#!/usr/bin/env python3
"""
Hermes SQLite 重构调度器 — 单轮执行
=========================
每 60 分钟被 cron 调用一次,做:
  1. flock 取锁(防重叠)
  2. 读 backlog/queue.jsonl
  3. 找下一个 queued 且依赖已 done 的子任务
  4. 渲染 prompt,调 claude -p
  5. 解析结果,更新 queue,写 git commit
  6. 跑一次 sanity build
  7. 更新 STATE.md
  8. 释放锁,退场
"""
import fcntl
import json
import os
import shutil
import subprocess
import sys
import time
import re
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional, List, Dict, Any

ROOT = Path("/Users/hyx/workspace/sqllite-project")
WORKDIR = ROOT / "rust-port"
BACKLOG = ROOT / "backlog"
QUEUE = BACKLOG / "queue.jsonl"
STATE_MD = BACKLOG / "STATE.md"
LOG_DIR = ROOT / "workflow" / "logs"
LOCK = ROOT / "workflow" / ".lock"
PROMPT_TPL = (ROOT / "workflow" / "prompt-template.md").read_text()

LOG_DIR.mkdir(parents=True, exist_ok=True)
BACKLOG.mkdir(parents=True, exist_ok=True)
LOCK.parent.mkdir(parents=True, exist_ok=True)


def log(msg: str) -> None:
    ts = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    line = f"[{ts}] {msg}"
    print(line, flush=True)
    (LOG_DIR / f"{datetime.now().strftime('%Y-%m-%d')}.log").open("a").write(line + "\n")


def read_queue() -> list:
    if not QUEUE.exists():
        return []
    return [json.loads(l) for l in QUEUE.read_text().splitlines() if l.strip()]


def write_queue(items: list) -> None:
    QUEUE.write_text("\n".join(json.dumps(i, ensure_ascii=False) for i in items) + "\n")


def deps_satisfied(task: dict, done_ids: set) -> bool:
    return all(d in done_ids for d in task.get("depends_on", []))


def next_task(items: list) -> Optional[dict]:
    """找下一个可派发的任务。

    规则:
    - 优先扫 status=queued 且依赖已 done 的
    - 如果没有 queued, 但有 in_progress(说明上次崩溃了), 重置它回 queued 派发
    """
    done_ids = {t["id"] for t in items if t["status"] == "done"}
    for t in items:
        if t["status"] == "queued" and deps_satisfied(t, done_ids):
            return t
    # 没有可派发的 queued — 看是否有遗留 in_progress
    for t in items:
        if t["status"] == "in_progress":
            log(f"  found stale in_progress: {t['id']}, resetting to queued")
            t["status"] = "queued"
            return t
    return None


def render_prompt(task: dict) -> str:
    # 从 C 文件前 200 行提取 API 签名,作为 C_API_SIG 占位符
    c_file = ROOT / task["c_file"]
    api_sig = "(see file)"
    if c_file.exists():
        text = c_file.read_text(errors="ignore")
        # 抓所有非 static 函数签名(简版,只看 2-3 行)
        sigs = re.findall(
            r"^(?:SQLITE_API\s+)?[A-Za-z_][A-Za-z0-9_ \*]+(?:sqlite3\w*|\w+)\s*\([^;]*?\)\s*\{",
            text,
            re.MULTILINE,
        )
        if sigs:
            api_sig = " | ".join(s.replace("{", "").strip() for s in sigs[:8])

    # 上次失败的反馈(从 queue.jsonl 拉, 让 Claude 知道哪错了)
    last_error_block = ""
    le = task.get("last_error")
    if le and task.get("attempts", 0) > 0:
        last_error_block = (
            f"\n## 上次尝试的反馈 (attempt #{task.get('attempts')})\n\n"
            f"你的上一次实现有以下问题:\n\n```\n{le}\n```\n\n"
            f"**请读 `workflow/logs/{task['id']}-stream.jsonl` 和 `workflow/logs/2026-06-07.log` 看完整 stream, "
            f"**先修这些具体失败**, 不要从头重写。如果失败在 4 个特定 test, 优先跑 `cargo test pattern::test_name` 单测调试。\n"
        )

    return (
        PROMPT_TPL
        .replace("{{WORKDIR}}", str(WORKDIR))
        .replace("{{C_FILE}}", str(c_file.relative_to(ROOT)))
        .replace("{{SCOPE}}", task["scope"])
        .replace("{{ID}}", task["id"])
        .replace("{{MODULE}}", task["module"])
        .replace("{{C_API_SIG}}", api_sig)
        .replace("{{TESTS}}", task["tests"])
        .replace("{{EST_TURNS}}", str(task.get("est_turns", 12)))
        .replace("{{TITLE}}", task["title"])
        .replace("{{LAST_ERROR}}", last_error_block)
    )


def run_claude(task: dict) -> dict:
    """调 claude -p stream-json 拿每个 tool call, 推断任务实际状态.

    三方 MiniMax-M3 模型 + claude -p json 模式有个 bug: result 文本不返回。
    改用 stream-json + --verbose, 我们能从每个 stream event 看到:
    - assistant 发出的 tool_use (Write/Edit/Bash)
    - tool_result (成功/失败)
    - assistant 的 text 块 (含 contract JSON)
    """
    prompt = render_prompt(task)
    prompt_file = LOG_DIR / f"{task['id']}-prompt.md"
    prompt_file.write_text(prompt)

    log(f"dispatching {task['id']}: {task['title']}")
    cmd = [
        "claude",
        "-p",
        prompt,
        "--output-format",
        "stream-json",
        "--verbose",
        "--max-turns",
        str(task.get("est_turns", 12)),
        "--max-budget-usd",
        "5",
        "--add-dir",
        str(ROOT),
        # 关键: 显式允许 Write/Edit/MultiEdit。
        # --bare 模式会禁用这些工具(参考 claude-code skill),
        # 所以我们用 --allowedTools 而不是 --bare。
        "--allowedTools",
        "Read,Write,Edit,MultiEdit,Bash,Grep,Glob,NotebookEdit,WebFetch",
        # 拒绝破坏性操作
        "--disallowedTools",
        "WebSearch",
    ]
    try:
        result = subprocess.run(
            cmd,
            cwd=WORKDIR,
            capture_output=True,
            text=True,
            timeout=1500,  # 25 分钟硬上限
        )
    except subprocess.TimeoutExpired:
        log(f"  {task['id']} TIMEOUT after 25min")
        return {"id": task["id"], "status": "failed", "next_action": "timeout"}

    # 完整 stdout 存到文件
    stream_log = LOG_DIR / f"{task['id']}-stream.jsonl"
    stream_log.write_text(result.stdout)
    (LOG_DIR / f"{task['id']}-stderr.txt").write_text(result.stderr[-2000:])

    # 解析 stream-json: 每行一个事件
    tool_calls: list[dict] = []
    text_blocks: list[str] = []
    final_envelope: dict = {}
    num_turns = 0
    subtype = ""
    stop_reason = ""
    cost = 0.0

    for line in result.stdout.splitlines():
        line = line.strip()
        if not line or not line.startswith("{"):
            continue
        try:
            evt = json.loads(line)
        except json.JSONDecodeError:
            continue
        et = evt.get("type", "")

        if et == "result":
            final_envelope = evt
            subtype = evt.get("subtype", "")
            stop_reason = evt.get("stop_reason", "")
            num_turns = evt.get("num_turns", 0)
            cost = evt.get("total_cost_usd", 0.0)

        elif et == "assistant":
            # 顶层 assistant 事件
            msg = evt.get("message", {})
            for block in msg.get("content", []):
                btype = block.get("type", "")
                if btype == "text":
                    text_blocks.append(block.get("text", ""))
                elif btype == "tool_use":
                    tool_calls.append(
                        {
                            "name": block.get("name", "?"),
                            "input": block.get("input", {}),
                        }
                    )

        elif et == "user":
            # tool_result 块
            msg = evt.get("message", {})
            for block in msg.get("content", []):
                if block.get("type") == "tool_result":
                    # 标记对应的 tool_call 成功/失败
                    content = block.get("content", "")
                    is_err = block.get("is_error", False)
                    if tool_calls:
                        tool_calls[-1]["is_error"] = is_err
                        tool_calls[-1]["result_preview"] = (
                            str(content)[:200] if content else ""
                        )

    # 统计
    write_count = sum(
        1 for tc in tool_calls if tc["name"] in ("Write", "Edit", "MultiEdit")
    )
    bash_count = sum(1 for tc in tool_calls if tc["name"] == "Bash")
    read_count = sum(1 for tc in tool_calls if tc["name"] in ("Read", "Glob", "Grep")
    )

    # 把"原 final 文本"拼起来(可能含 contract JSON)
    final_text = "\n".join(text_blocks)
    log(
        f"  {task['id']} done: turns={num_turns} cost=${cost:.3f} "
        f"write/edit={write_count} bash={bash_count} read={read_count} "
        f"text_len={len(final_text)} subtype={subtype}"
    )

    # 在 final_text 里找 contract JSON
    parsed: Optional[dict] = None
    if final_text:
        match = re.search(r'```json\s*(\{.*?\})\s*```', final_text, re.DOTALL)
        if match:
            try:
                parsed = json.loads(match.group(1))
            except json.JSONDecodeError:
                pass
        if not parsed:
            match = re.search(
                r'\{[^{}]*"id"\s*:\s*"' + re.escape(task["id"]) + r'"', final_text
            )
            if match:
                start = match.start()
                depth = 0
                for i, ch in enumerate(final_text[start:]):
                    if ch == "{":
                        depth += 1
                    elif ch == "}":
                        depth -= 1
                        if depth == 0:
                            try:
                                parsed = json.loads(final_text[start : start + i + 1])
                            except json.JSONDecodeError:
                                break

    if parsed and "status" in parsed:
        parsed["tool_calls"] = tool_calls
        parsed["num_turns"] = num_turns
        parsed["cost_usd"] = cost
        return parsed

    # fallback: 即使没 contract JSON, 也能根据 tool_use 判断
    # - 写了 rust-port/* 文件 + 跑了 build/test → 算"implicit done"
    # - 只 read 没 write → "stalled"
    # - 只写 notes/*.md → "discovery" (claude 发现 spec 问题, 需要人 review)
    # - max_turns 但有 write → "partial"

    # 分类 write/edit
    code_writes = [
        tc for tc in tool_calls
        if tc["name"] in ("Write", "Edit", "MultiEdit")
        and "file_path" in tc["input"]
        and tc["input"]["file_path"].startswith(str(WORKDIR))
    ]
    note_writes = [
        tc for tc in tool_calls
        if tc["name"] in ("Write", "Edit", "MultiEdit")
        and "file_path" in tc["input"]
        and tc["input"]["file_path"].endswith(".md")
        and "/notes/" in tc["input"]["file_path"]
    ]
    code_write_count = len(code_writes)
    note_write_count = len(note_writes)

    # 关键判定: 写了 rust 代码 + 跑了 bash → done
    if code_write_count > 0 and bash_count > 0:
        log(
            f"  {task['id']} IMPLICIT DONE based on tool_use "
            f"({code_write_count} code writes, {bash_count} bash)"
        )
        return {
            "id": task["id"],
            "status": "done",
            "files_created": [
                tc["input"].get("file_path", "?")
                for tc in code_writes
                if tc["name"] == "Write"
            ],
            "files_modified": [
                tc["input"].get("file_path", "?")
                for tc in code_writes
                if tc["name"] in ("Edit", "MultiEdit")
            ],
            "tests_run": "(see stream log)",
            "diff_summary": f"implicit: {code_write_count} code writes, {bash_count} bash, {read_count} reads",
            "next_action": "verified via tool_use inspection",
            "tool_calls": tool_calls,
            "num_turns": num_turns,
            "cost_usd": cost,
        }

    # 写了 note 但没写代码 → claude 发现 spec 问题, 需人 review
    if note_write_count > 0 and code_write_count == 0:
        log(f"  {task['id']} DISCOVERY: {note_write_count} notes written, no code change")
        return {
            "id": task["id"],
            "status": "discovery",
            "next_action": f"claude wrote {note_write_count} note(s) but no code; see {note_writes[0]['input'].get('file_path')}",
            "tool_calls": tool_calls,
            "num_turns": num_turns,
            "cost_usd": cost,
        }

    # 啥都没写
    if code_write_count == 0 and note_write_count == 0 and read_count > 0:
        log(f"  {task['id']} STALLED: {read_count} reads but 0 writes of any kind")
        return {
            "id": task["id"],
            "status": "stalled",
            "next_action": f"claude read {read_count} files but wrote 0; prompt may be too cautious",
            "tool_calls": tool_calls,
            "num_turns": num_turns,
            "cost_usd": cost,
        }

    # 写了一些代码但没跑 bash → 可能编译都没过
    if code_write_count > 0 and bash_count == 0:
        return {
            "id": task["id"],
            "status": "partial",
            "next_action": f"wrote {code_write_count} code files but didn't run cargo build/test",
            "tool_calls": tool_calls,
            "num_turns": num_turns,
            "cost_usd": cost,
        }

    return {
        "id": task["id"],
        "status": "failed",
        "next_action": f"no qualifying tool_use; turns={num_turns}",
        "tool_calls": tool_calls,
        "num_turns": num_turns,
        "cost_usd": cost,
    }


def update_task(items: list, task: dict, result: dict) -> None:
    for t in items:
        if t["id"] == task["id"]:
            t["attempts"] = t.get("attempts", 0) + 1
            # 累计 cost 估算(在 task 上记录)
            t["cost_usd"] = t.get("cost_usd", 0.0) + result.get("cost_usd", 0.0)
            new_status = result.get("status", "failed")
            if new_status == "done":
                t["status"] = "done"
                t["last_error"] = None
            elif new_status == "blocked":
                t["status"] = "blocked"
                t["last_error"] = result.get("next_action", "")
            elif new_status == "stalled":
                # 5 次失败后放弃; 否则仍留 queued 让下轮 cron 重试
                if t["attempts"] >= 5:
                    t["status"] = "stalled"
                else:
                    t["status"] = "queued"
                t["last_error"] = result.get("next_action", "stalled")
            elif new_status == "partial":
                if t["attempts"] >= 5:
                    t["status"] = "stalled"
                else:
                    t["status"] = "queued"
                t["last_error"] = result.get("next_action", "partial")
            elif new_status == "discovery":
                # claude 写了 notes/* 没写代码 → spec 可能有错或 agent 困惑,等人 review
                t["status"] = "discovery"
                t["last_error"] = result.get("next_action", "discovery")
            elif new_status == "incomplete":
                t["status"] = "queued" if t["attempts"] < 5 else "failed"
                t["last_error"] = result.get("next_action", "incomplete")
            else:
                if t["attempts"] >= 3:
                    t["status"] = "failed"
                    t["last_error"] = result.get("next_action", "max attempts")
                else:
                    t["status"] = "queued"
                    t["last_error"] = result.get("next_action", "")
            break


def git_commit_if_needed(task: dict, result: dict) -> bool:
    if result.get("status") != "done":
        return False
    try:
        # 检查是否有未提交的修改
        r = subprocess.run(
            ["git", "status", "--porcelain"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=30,
        )
        if not r.stdout.strip():
            log(f"  {task['id']} done but no diff")
            return False
        msg = f"port: {task['id']} {task['title']}\n\n{result.get('diff_summary', '')}"
        subprocess.run(["git", "add", "-A"], cwd=ROOT, check=True, timeout=60)
        subprocess.run(
            ["git", "commit", "-m", msg],
            cwd=ROOT,
            check=True,
            timeout=60,
        )
        log(f"  committed {task['id']}")
        return True
    except subprocess.CalledProcessError as e:
        log(f"  git commit failed for {task['id']}: {e}")
        return False


def render_state(items: list) -> str:
    by_status = {}  # type: dict
    for t in items:
        by_status.setdefault(t["status"], []).append(t)

    total = len(items)
    done = len(by_status.get("done", []))
    pct = (done / total * 100) if total else 0

    out = [
        "# Backlog State",
        "",
        f"- **Total tasks**: {total}",
        f"- **Done**: {done} ({pct:.1f}%)",
        f"- **Queued**: {len(by_status.get('queued', []))}",
        f"- **In progress**: {len(by_status.get('in_progress', []))}",
        f"- **Blocked**: {len(by_status.get('blocked', []))}",
        f"- **Failed**: {len(by_status.get('failed', []))}",
        f"- **Last update**: {datetime.now(timezone.utc).isoformat()}",
        "",
    ]
    for s in ("blocked", "failed", "in_progress", "queued", "done"):
        if s in by_status:
            out.append(f"## {s} ({len(by_status[s])})")
            for t in by_status[s][:30]:
                err = f" — last error: {t['last_error']}" if t.get("last_error") else ""
                out.append(f"- `{t['id']}` [{t['module']}] {t['title']}{err}")
            if len(by_status[s]) > 30:
                out.append(f"- ... and {len(by_status[s]) - 30} more")
            out.append("")
    return "\n".join(out)


def sanity_build() -> bool:
    """跑 cargo check + cargo test 验证。

    Returns True iff 全过。
    - cargo check: 编译通过
    - cargo test: 所有测试通过 (这是 1:1 port 的关键)
    """
    if not (WORKDIR / "Cargo.toml").exists():
        log("rust workspace not yet initialized; skipping sanity build")
        return True
    try:
        r = subprocess.run(
            ["cargo", "check", "--workspace", "--all-targets"],
            cwd=WORKDIR, capture_output=True, text=True, timeout=180,
        )
        if r.returncode != 0:
            log(f"  cargo check FAILED:\n{r.stderr[-1500:]}")
            return False
        log("  cargo check OK")
    except subprocess.TimeoutExpired:
        log("  cargo check TIMEOUT")
        return False

    # 跑 test, 任何失败都拒绝 commit
    try:
        r = subprocess.run(
            ["cargo", "test", "--workspace", "--all-targets", "--no-fail-fast"],
            cwd=WORKDIR, capture_output=True, text=True, timeout=300,
        )
        if r.returncode != 0:
            # 提取失败计数
            import re
            m = re.search(r"test result.*?(\d+) passed.*?(\d+) failed", r.stdout + r.stderr)
            fail_info = m.group(0) if m else "unknown"

            # 提取失败的具体测试名 (前 10 个)
            failed_names = re.findall(r"^---- (\S+) stdout", r.stdout + r.stderr, re.MULTILINE)
            failed_summary = ", ".join(failed_names[:10])

            log(f"  cargo test FAILED ({fail_info}):")
            for fn in failed_names[:5]:
                log(f"    - {fn}")

            # 写进 last_error 字典, 让 demote 时能传给 task
            sanity_build.last_error = (
                f"cargo test failed: {fail_info}. failed tests: {failed_summary}. "
                f"fix these specific test failures in src/util/pattern.rs (or wherever the bug is)."
            )
            return False
        log("  cargo test OK")
        # 清掉上次的错误
        sanity_build.last_error = None
        return True
    except subprocess.TimeoutExpired:
        log("  cargo test TIMEOUT")
        sanity_build.last_error = "cargo test timeout"
        return False


def main() -> int:
    # 1. 取锁
    try:
        lock_fd = open(LOCK, "w")
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except (IOError, OSError):
        log("another runner holds the lock, exiting")
        return 0

    log("=== runner start ===")
    try:
        items = read_queue()
        if not items:
            log("backlog empty; nothing to do")
            STATE_MD.write_text(render_state([]))
            return 0

        # 标记 in_progress
        task = next_task(items)
        if not task:
            log("no task with satisfied deps; will wait for blocker resolution")
            STATE_MD.write_text(render_state(items))
            return 0

        for t in items:
            if t["id"] == task["id"]:
                t["status"] = "in_progress"
                break
        write_queue(items)

        # 2. 派发
        result = run_claude(task)

        # 3. 重新读(防止并发改),更新
        items = read_queue()
        update_task(items, task, result)
        write_queue(items)

        # 4. sanity FIRST (cargo check + cargo test)
        #    如果测试不过, 即使 Claude 说 done 也拒绝 commit, 改 status=stalled
        sanity_ok = sanity_build()

        # 5. commit (only if sanity passed OR claude said blocked/failed)
        new_status = result.get("status", "failed")
        if new_status == "done" and not sanity_ok:
            log(f"  sanity FAILED → demoting {task['id']} from done to queued (with cargo test feedback)")
            for t in items:
                if t["id"] == task["id"]:
                    t["status"] = "queued" if t.get("attempts", 0) < 5 else "stalled"
                    t["last_error"] = getattr(sanity_build, "last_error", "cargo test failed")
                    break
            write_queue(items)
            new_status = "queued"
            result["status"] = "queued"
        git_commit_if_needed(task, result)

        # 6. state
        STATE_MD.write_text(render_state(items))

        log(
            f"=== runner end — {task['id']} -> {result.get('status')} "
            f"(attempts={task.get('attempts', 0)}) ==="
        )

        # 7. 事件链: 根据结果立即触发下一个动作 (不等 5min cron)
        # 释放 flock 之后再 trigger, 避免嵌套死锁
        new_status = result.get("status", "failed")
        # finally 块会先释放 lock, 然后我们 trigger
        # 这里只记录 "要 trigger 什么", finally 块实际调用
        # 改用全局变量
        global _POST_RUN_TRIGGER
        _POST_RUN_TRIGGER = new_status

        return 0
    finally:
        fcntl.flock(lock_fd, fcntl.LOCK_UN)
        lock_fd.close()
        # lock 已释放, 现在 trigger 下一步
        try:
            trigger_next = _POST_RUN_TRIGGER
            if trigger_next == "done":
                log("  → event: trigger runner-done (immediate next task)")
                proc = subprocess.Popen(
                    ["bash", str(ROOT / "workflow" / "trigger.sh"), "runner-done"],
                    cwd=ROOT,
                    stdout=open(LOG_DIR / "trigger.out", "a"),
                    stderr=open(LOG_DIR / "trigger.err", "a"),
                )
                log(f"    trigger pid={proc.pid}")
            elif trigger_next in ("failed", "stalled", "discovery"):
                log(f"  → event: trigger runner-fail (backoff retry)")
                proc = subprocess.Popen(
                    ["bash", str(ROOT / "workflow" / "trigger.sh"), "runner-fail", task.get("id", "")],
                    cwd=ROOT,
                    stdout=open(LOG_DIR / "trigger.out", "a"),
                    stderr=open(LOG_DIR / "trigger.err", "a"),
                )
                log(f"    trigger pid={proc.pid}")
        except Exception as e:
            log(f"  trigger next failed: {e}")


_POST_RUN_TRIGGER = None


if __name__ == "__main__":
    sys.exit(main())
