#!/bin/bash
# 事件触发器 — 由各种事件源调用, 把事件路由到 runner / claude
# 复用 5min cron 的 lock 机制, 但事件来了就能立即跑, 不等 5min
#
# 用法:
#   trigger.sh runner-fail T-0002
#   trigger.sh runner-done
#   trigger.sh gh-pr 123
#   trigger.sh gh-actions 456
#   trigger.sh fs

set -e
cd /Users/hyx/workspace/sqllite-project
export PATH="/Users/hyx/.nvm/versions/node/v24.12.0/bin:/Users/hyx/.cargo/bin:/usr/local/bin:/usr/bin:/bin:$PATH"

PYTHON_BIN="$(command -v python3.13 || command -v python3.11 || command -v python3.10 || command -v python3)"

case "$1" in
    runner-fail)
        exec "$PYTHON_BIN" workflow/event_router.py --source=runner-fail --task-id="$2"
        ;;
    runner-done)
        exec "$PYTHON_BIN" workflow/event_router.py --source=runner-done
        ;;
    gh-pr)
        exec "$PYTHON_BIN" workflow/event_router.py --source=gh-pr --pr="$2"
        ;;
    gh-actions)
        exec "$PYTHON_BIN" workflow/event_router.py --source=gh-actions --run="$2"
        ;;
    fs)
        exec "$PYTHON_BIN" workflow/event_router.py --source=fs
        ;;
    *)
        echo "usage: $0 {runner-fail TASK | runner-done | gh-pr N | gh-actions RUN_ID | fs}" >&2
        exit 1
        ;;
esac
