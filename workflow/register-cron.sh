#!/bin/bash
# 把 runner (5min) + gh-poller (5min) 两个 cron 行都注册
set -e

RUNNER_LINE="*/5 * * * * /Users/hyx/workspace/sqllite-project/workflow/run-once.sh >> /Users/hyx/workspace/sqllite-project/workflow/cron.log 2>&1"
GH_LINE="*/5 * * * * /Users/hyx/workspace/sqllite-project/workflow/run-gh-poller.sh >> /Users/hyx/workspace/sqllite-project/workflow/gh-cron.log 2>&1"

# 移除旧的 (兼容)
( crontab -l 2>/dev/null | grep -v -F "sqllite-project/workflow/" ) | crontab -

# 加新行
( crontab -l 2>/dev/null; echo "$RUNNER_LINE"; echo "$GH_LINE" ) | crontab -

echo "registered:"
crontab -l | grep sqllite-project
