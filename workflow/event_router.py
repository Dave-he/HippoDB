#!/usr/bin/env python3
"""
事件路由器 — 把多种事件信号路由到 runner

输入信号 (任一):
  1. --source=runner-fail  : 上一次 runner 失败 (exit code != 0 或 status=failed)
  2. --source=runner-done  : 上一次 runner 成功 (status=done)
  3. --source=gh-pr        : GitHub PR 状态变化 (需要 --pr=NUMBER)
  4. --source=gh-actions   : GitHub Actions run 失败 (需要 --run=ID)
  5. --source=fs           : 文件系统事件 (lock 创建/queue.jsonl 修改)

行为:
  - 计算 "触发" 或 "忽略":
    * runner-fail → 立即重试同一任务 (带 backoff: 30s/60s/120s/300s)
    * runner-done → 立即派下一个 queued 任务
    * gh-pr       → code-review 模式: 把 PR 内容喂给 claude, 决定 approve/request-changes
    * gh-actions  → fix 模式: 把失败 log 喂给 claude, 修代码
    * fs          → 当前是 5min cron 的替代, 触发一次 runner
  - 调用 runner.py (lock-protected)

用法:
  event_router.py --source=runner-fail --task-id=T-0002
  event_router.py --source=runner-done
  event_router.py --source=gh-pr --pr=123
  event_router.py --source=gh-actions --run=456

本脚本设计成幂等 + lock-protected, 多个事件并发也会被 runner 的 flock 串行化。
"""
import argparse
import json
import os
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path("/Users/hyx/workspace/sqllite-project")
LOG = ROOT / "workflow" / "logs" / "events.log"
LOG.parent.mkdir(parents=True, exist_ok=True)


def log(msg: str) -> None:
    ts = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    line = f"[{ts}] [event] {msg}"
    print(line, flush=True)
    with open(LOG, "a") as f:
        f.write(line + "\n")


def run_runner() -> int:
    """调一次 runner.py (lock-protected by runner itself)"""
    result = subprocess.run(
        ["bash", str(ROOT / "workflow" / "run-once.sh")],
        cwd=ROOT,
        env={
            **os.environ,
            "PATH": "/Users/hyx/.nvm/versions/node/v24.12.0/bin:/Users/hyx/.cargo/bin:/usr/local/bin:/usr/bin:/bin",
        },
        capture_output=True,
        text=True,
        timeout=1500,
    )
    log(f"runner exit={result.returncode}")
    return result.returncode


def on_runner_fail(task_id: str) -> None:
    """runner 失败: 退避后立即重试同一任务 (实际由 next_task 找 queued 项)

    backoff: 第一次 30s, 第二次 60s, 第三次 120s, 之后 300s
    """
    queue = ROOT / "backlog" / "queue.jsonl"
    items = [json.loads(l) for l in queue.read_text().splitlines() if l.strip()]
    task = next((t for t in items if t["id"] == task_id), None)
    if not task:
        log(f"task {task_id} not found")
        return
    attempts = task.get("attempts", 0)
    backoff = [30, 60, 120, 300][min(attempts, 3)]
    log(f"runner-fail for {task_id} attempts={attempts}, backoff {backoff}s")
    time.sleep(backoff)
    run_runner()


def on_runner_done() -> None:
    """runner 成功: 立即派下一个 queued 任务 (不等 5min cron)"""
    log("runner-done → immediate next dispatch")
    run_runner()


def on_gh_pr(pr_number: int) -> None:
    """GitHub PR: 用 claude 跑 code review, 自动 approve / request-changes

    实现:
      1. gh pr view <N> --json 拿 PR metadata
      2. gh pr diff <N> 拿 diff
      3. 喂给 claude (用 review prompt template)
      4. claude 决定 approve/request-changes
      5. gh pr review <N> --approve / --request-changes
    """
    log(f"gh-pr {pr_number}: fetching metadata + diff")
    try:
        meta = subprocess.run(
            ["gh", "pr", "view", str(pr_number), "--json",
             "title,body,author,baseRefName,headRefName,files,additions,deletions"],
            cwd=ROOT, capture_output=True, text=True, timeout=30,
        )
        if meta.returncode != 0:
            log(f"gh pr view failed: {meta.stderr[:200]}")
            return
        pr = json.loads(meta.stdout)
        diff = subprocess.run(
            ["gh", "pr", "diff", str(pr_number)],
            cwd=ROOT, capture_output=True, text=True, timeout=60,
        ).stdout
    except Exception as e:
        log(f"gh pr fetch error: {e}")
        return

    # 调 claude review
    review_prompt = build_review_prompt(pr, diff)
    log(f"  dispatching claude review ({len(diff)} bytes diff)")
    review_result = subprocess.run(
        [
            "claude", "-p", review_prompt,
            "--output-format", "stream-json", "--verbose",
            "--max-turns", "8", "--max-budget-usd", "2",
            "--add-dir", str(ROOT),
            "--allowedTools", "Read,Bash,Grep,Glob",
        ],
        cwd=ROOT,
        capture_output=True, text=True, timeout=600,
    )
    (LOG.parent / f"pr-{pr_number}-review.jsonl").write_text(review_result.stdout)

    # 解析 claude 决定
    decision = parse_review_decision(review_result.stdout)
    log(f"  claude decision: {decision}")

    if decision["action"] == "approve":
        subprocess.run(
            ["gh", "pr", "review", str(pr_number), "--approve",
             "--body", decision.get("body", "auto-approved")],
            cwd=ROOT, capture_output=True, text=True, timeout=30,
        )
        log(f"  approved PR #{pr_number}")
    elif decision["action"] == "request-changes":
        subprocess.run(
            ["gh", "pr", "review", str(pr_number), "--request-changes",
             "--body", decision.get("body", "auto-requested changes")],
            cwd=ROOT, capture_output=True, text=True, timeout=30,
        )
        log(f"  requested changes on PR #{pr_number}")
    else:
        log(f"  left PR #{pr_number} alone (decision={decision['action']})")


def on_gh_actions(run_id: str) -> None:
    """GitHub Actions 失败: 把失败 log 喂给 claude, 派修复任务

    实现:
      1. gh run view <ID> --log-failed 拿失败 log
      2. 喂给 claude (fix 模式)
      3. claude 写修复, commit, push (如果授权)
    """
    log(f"gh-actions run={run_id}: fetching failed log")
    try:
        log_text = subprocess.run(
            ["gh", "run", "view", run_id, "--log-failed"],
            cwd=ROOT, capture_output=True, text=True, timeout=60,
        ).stdout
    except Exception as e:
        log(f"gh run view error: {e}")
        return

    if not log_text.strip():
        log(f"  run {run_id} has no failed log")
        return

    # 修: 加一个 backlog 任务
    fix_id = f"FIX-{run_id[:8]}"
    fix_prompt = (
        f"GitHub Actions run {run_id} failed. 修复以下问题:\n\n"
        f"```\n{log_text[:8000]}\n```\n\n"
        f"1. 先读代码找到根因\n"
        f"2. 改代码\n"
        f"3. 跑 cargo check + cargo test 确认\n"
        f"4. git add -A && git commit -m 'fix: Actions run {run_id}'\n\n"
        f"完成后输出 JSON: {{\"id\": \"{fix_id}\", \"status\": \"done\" | \"blocked\", "
        f"\"diff_summary\": \"...\", \"next_action\": \"...\"}}"
    )
    log(f"  dispatching claude fix")
    fix_result = subprocess.run(
        [
            "claude", "-p", fix_prompt,
            "--output-format", "stream-json", "--verbose",
            "--max-turns", "20", "--max-budget-usd", "5",
            "--add-dir", str(ROOT),
            "--allowedTools", "Read,Write,Edit,MultiEdit,Bash,Grep,Glob",
        ],
        cwd=ROOT,
        capture_output=True, text=True, timeout=1500,
    )
    (LOG.parent / f"fix-{run_id}.jsonl").write_text(fix_result.stdout)
    log(f"  fix dispatch exit={fix_result.returncode}")


def build_review_prompt(pr: dict, diff: str) -> str:
    return f"""你是 sqllite-project (SQLite 1:1 Rust 重构) 的 code reviewer。

需要 review 下面这个 PR:

# {pr['title']}
Author: {pr['author']['login']}
Base: {pr['baseRefName']} ← Head: {pr['headRefName']}
Files: {len(pr.get('files', []))}, +{pr.get('additions', 0)} -{pr.get('deletions', 0)} lines

{pr.get('body', '(no description)')}

## Diff (truncated to 8000 chars)

```diff
{diff[:8000]}
```

## 评审标准 (对齐 plans/02-c-porting-conventions.md)

1. **行为 1:1 与 C 源码一致** — 错误码、边界条件、unsafe 注释
2. **测试覆盖** — 至少 1 个 test per public API
3. **不扩大范围** — 没顺手改不属于本 PR 的文件
4. **commit message 格式** — `port: T-NNNN <title>`
5. **不破坏现有测试** — `cargo test` 应仍全过

## 输出

严格按以下 JSON (放在 ```json ``` 块内):

```json
{{
  "action": "approve" | "request-changes" | "comment",
  "body": "1-3 句话总结评审意见, 给 PR 作者看",
  "issues": [
    {{"severity": "critical" | "important" | "minor", "file": "path/to/file.rs", "line": 42, "msg": "..."}}
  ]
}}
```
"""


def parse_review_decision(stream_text: str) -> dict:
    """从 claude stream-json 输出提取 review 决定"""
    import re
    # 找 ```json { ... } ``` 块
    match = re.search(r'```json\s*(\{.*?\})\s*```', stream_text, re.DOTALL)
    if match:
        try:
            return json.loads(match.group(1))
        except json.JSONDecodeError:
            pass
    # 兜底: 找 action 字段
    match = re.search(r'"action"\s*:\s*"(\w+)"', stream_text)
    if match:
        return {"action": match.group(1), "body": "(parsed from stream)"}
    return {"action": "comment", "body": "could not parse review"}


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--source", required=True,
                   choices=["runner-fail", "runner-done", "gh-pr", "gh-actions", "fs"])
    p.add_argument("--task-id", default="")
    p.add_argument("--pr", type=int, default=0)
    p.add_argument("--run", default="")
    args = p.parse_args()

    log(f"event_router source={args.source} task={args.task_id} pr={args.pr} run={args.run}")

    if args.source == "runner-fail":
        on_runner_fail(args.task_id)
    elif args.source == "runner-done":
        on_runner_done()
    elif args.source == "gh-pr":
        on_gh_pr(args.pr)
    elif args.source == "gh-actions":
        on_gh_actions(args.run)
    elif args.source == "fs":
        run_runner()
    return 0


if __name__ == "__main__":
    sys.exit(main())
