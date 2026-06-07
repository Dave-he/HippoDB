#!/usr/bin/env python3
"""
GitHub 事件 poller — 替代 webhook server (无需公网/反向代理)

每 5 分钟用 gh CLI 拉:
  - 该 repo 的 open PR
  - 该 repo 的失败 Actions run

发现新事件 (PR created/updated, Actions 失败) 触发 event_router 处理。

跟 cron 一样用 flock 防重叠, daemon 风格运行 (foreground, 配 launchd / cron)
"""
import fcntl
import json
import os
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path("/Users/hyx/workspace/sqllite-project")
LOCK = ROOT / "workflow" / ".gh-poller.lock"
STATE = ROOT / "workflow" / "gh-state.json"
LOG_DIR = ROOT / "workflow" / "logs"
LOG = LOG_DIR / "gh-poller.log"
LOG_DIR.mkdir(parents=True, exist_ok=True)
REPO = os.environ.get("SQ_REPO", "Dave-he/sqllite-project")  # 改为你自己的

POLL_INTERVAL = int(os.environ.get("GH_POLL_INTERVAL", "300"))  # 默认 5min


def log(msg: str) -> None:
    ts = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    line = f"[{ts}] [gh-poller] {msg}"
    print(line, flush=True)
    with open(LOG, "a") as f:
        f.write(line + "\n")


def load_state() -> dict:
    if STATE.exists():
        try:
            return json.loads(STATE.read_text())
        except json.JSONDecodeError:
            pass
    return {"seen_prs": {}, "seen_runs": {}}


def save_state(state: dict) -> None:
    STATE.write_text(json.dumps(state, ensure_ascii=False, indent=2))


def poll_prs(state: dict) -> list:
    """返回有变化的 PR 列表"""
    try:
        result = subprocess.run(
            ["gh", "pr", "list", "--repo", REPO,
             "--json", "number,title,updatedAt,state,isDraft",
             "--state", "open", "--limit", "20"],
            cwd=ROOT, capture_output=True, text=True, timeout=30,
        )
        if result.returncode != 0:
            log(f"gh pr list failed: {result.stderr[:200]}")
            return []
        prs = json.loads(result.stdout)
    except Exception as e:
        log(f"gh pr list error: {e}")
        return []

    new_events = []
    for pr in prs:
        n = pr["number"]
        updated = pr.get("updatedAt", "")
        seen = state["seen_prs"].get(str(n), {})
        # 新 PR 或 updatedAt 变了 → 触发
        if seen.get("updatedAt") != updated or seen.get("state") != pr.get("state"):
            new_events.append(pr)
            state["seen_prs"][str(n)] = {
                "updatedAt": updated,
                "state": pr.get("state"),
                "seen_at": datetime.now(timezone.utc).isoformat(),
            }
    return new_events


def poll_actions(state: dict) -> list:
    """返回失败的 Actions run 列表"""
    try:
        result = subprocess.run(
            ["gh", "run", "list", "--repo", REPO,
             "--json", "databaseId,name,conclusion,createdAt,headBranch,event",
             "--limit", "20"],
            cwd=ROOT, capture_output=True, text=True, timeout=30,
        )
        if result.returncode != 0:
            log(f"gh run list failed: {result.stderr[:200]}")
            return []
        runs = json.loads(result.stdout)
    except Exception as e:
        log(f"gh run list error: {e}")
        return []

    new_events = []
    for run in runs:
        if run.get("conclusion") != "failure":
            continue
        rid = str(run["databaseId"])
        seen = state["seen_runs"].get(rid, {})
        if not seen:
            new_events.append(run)
            state["seen_runs"][rid] = {
                "name": run.get("name"),
                "headBranch": run.get("headBranch"),
                "seen_at": datetime.now(timezone.utc).isoformat(),
            }
    return new_events


def trigger_pr_review(pr: dict) -> None:
    log(f"  → trigger gh-pr review for PR #{pr['number']}")
    subprocess.Popen(
        ["bash", str(ROOT / "workflow" / "trigger.sh"), "gh-pr", str(pr["number"])],
        cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )


def trigger_actions_fix(run: dict) -> None:
    log(f"  → trigger gh-actions fix for run {run['databaseId']}")
    subprocess.Popen(
        ["bash", str(ROOT / "workflow" / "trigger.sh"), "gh-actions", str(run["databaseId"])],
        cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )


def main_loop() -> int:
    log(f"start (repo={REPO}, interval={POLL_INTERVAL}s)")

    while True:
        try:
            state = load_state()
            prs = poll_prs(state)
            runs = poll_actions(state)
            save_state(state)
            log(f"polled: {len(prs)} new PRs, {len(runs)} new failed runs")
            for pr in prs:
                trigger_pr_review(pr)
            for run in runs:
                trigger_actions_fix(run)
        except Exception as e:
            log(f"poll error: {e}")
        time.sleep(POLL_INTERVAL)


def main() -> int:
    # flock 防重叠
    try:
        lock_fd = open(LOCK, "w")
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except (IOError, OSError):
        log("another gh-poller holds the lock, exiting")
        return 0
    try:
        return main_loop()
    finally:
        fcntl.flock(lock_fd, fcntl.LOCK_UN)
        lock_fd.close()


if __name__ == "__main__":
    sys.exit(main())
