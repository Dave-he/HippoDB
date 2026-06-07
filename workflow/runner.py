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


def read_queue() -> list[dict]:
    if not QUEUE.exists():
        return []
    return [json.loads(l) for l in QUEUE.read_text().splitlines() if l.strip()]


def write_queue(items: list[dict]) -> None:
    QUEUE.write_text("\n".join(json.dumps(i, ensure_ascii=False) for i in items) + "\n")


def deps_satisfied(task: dict, done_ids: set[str]) -> bool:
    return all(d in done_ids for d in task.get("depends_on", []))


def next_task(items: list[dict]) -> dict | None:
    done_ids = {t["id"] for t in items if t["status"] == "done"}
    for t in items:
        if t["status"] == "queued" and deps_satisfied(t, done_ids):
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
    )


def run_claude(task: dict) -> dict:
    """调 claude -p,返回解析后的输出 dict."""
    prompt = render_prompt(task)
    prompt_file = LOG_DIR / f"{task['id']}-prompt.md"
    prompt_file.write_text(prompt)

    log(f"dispatching {task['id']}: {task['title']}")
    cmd = [
        "claude",
        "-p",
        prompt,
        "--output-format",
        "json",
        "--max-turns",
        str(task.get("est_turns", 12)),
        "--max-budget-usd",
        "5",
        "--bare",  # 不读 CLAUDE.md hooks,避免与我们的 PreToolUse 钩子冲突
        "--add-dir",
        str(ROOT),
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

    (LOG_DIR / f"{task['id']}-stdout.txt").write_text(result.stdout[-5000:])
    (LOG_DIR / f"{task['id']}-stderr.txt").write_text(result.stderr[-2000:])

    # claude -p --output-format json 把整个会话结果放在 stdout 第一行
    parsed: dict = {}
    try:
        # 输出可能有多行,取最后一个 JSON object
        for line in reversed(result.stdout.strip().splitlines()):
            line = line.strip()
            if line.startswith("{"):
                parsed = json.loads(line)
                break
    except json.JSONDecodeError:
        pass

    # 提取最终结果(可能在 result 字段或 type=result 里)
    final = parsed.get("result", "")
    if not final and parsed.get("type") == "result":
        final = parsed.get("result", "")
    if not final:
        # fallback: 取最后 2KB 文本
        final = result.stdout[-2000:]

    # 在 final 里找我们约定的 JSON 结构({id, status, ...})
    match = re.search(r'```json\s*(\{.*?\})\s*```', final, re.DOTALL)
    if match:
        try:
            return json.loads(match.group(1))
        except json.JSONDecodeError:
            pass
    # 也可能 JSON 直接出现在文本里
    match = re.search(r'\{[^{}]*"id"\s*:\s*"' + re.escape(task["id"]) + r'"', final)
    if match:
        start = match.start()
        # 找匹配的右括号
        depth = 0
        for i, ch in enumerate(final[start:]):
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    try:
                        return json.loads(final[start : start + i + 1])
                    except json.JSONDecodeError:
                        break

    return {
        "id": task["id"],
        "status": "failed",
        "next_action": f"no parseable JSON; raw len={len(result.stdout)}",
    }


def update_task(items: list[dict], task: dict, result: dict) -> None:
    for t in items:
        if t["id"] == task["id"]:
            t["attempts"] = t.get("attempts", 0) + 1
            new_status = result.get("status", "failed")
            if new_status == "done":
                t["status"] = "done"
                t["last_error"] = None
            elif new_status == "blocked":
                t["status"] = "blocked"
                t["last_error"] = result.get("next_action", "")
            else:
                if t["attempts"] >= 3:
                    t["status"] = "failed"
                    t["last_error"] = result.get("next_action", "max attempts")
                else:
                    t["status"] = "queued"  # retry
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


def render_state(items: list[dict]) -> str:
    by_status: dict[str, list[dict]] = {}
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
    """检查 rust workspace 是否还能编译。如果还没建,返回 True(暂时跳过)。"""
    if not (WORKDIR / "Cargo.toml").exists():
        log("rust workspace not yet initialized; skipping sanity build")
        return True
    try:
        r = subprocess.run(
            ["cargo", "check", "--workspace"],
            cwd=WORKDIR,
            capture_output=True,
            text=True,
            timeout=180,
        )
        if r.returncode != 0:
            log(f"  cargo check FAILED:\n{r.stderr[-1500:]}")
            return False
        log("  cargo check OK")
        return True
    except subprocess.TimeoutExpired:
        log("  cargo check TIMEOUT")
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

        # 4. commit
        git_commit_if_needed(task, result)

        # 5. sanity
        sanity_build()

        # 6. state
        STATE_MD.write_text(render_state(items))

        log(
            f"=== runner end — {task['id']} -> {result.get('status')} "
            f"(attempts={task.get('attempts', 0)}) ==="
        )
        return 0
    finally:
        fcntl.flock(lock_fd, fcntl.LOCK_UN)
        lock_fd.close()


if __name__ == "__main__":
    sys.exit(main())
