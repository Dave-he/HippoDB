#!/bin/bash
# 注册 Hermes cron: 每 60 分钟跑一次 workflow/run-once.sh
# 用 crontab 形式,而不是 hermes cronjob 工具,
# 因为这个 workflow 需要 (a) 持续数月 (b) 不需要 user 决策 (c) 输出直接落日志即可

set -e

CRON_LINE="* * * * * /Users/hyx/workspace/sqllite-project/workflow/run-once.sh >> /Users/hyx/workspace/sqllite-project/workflow/cron.log 2>&1"

# 先看有没有
if crontab -l 2>/dev/null | grep -F "sqllite-project/workflow/run-once.sh" > /dev/null; then
    echo "cron already registered:"
    crontab -l | grep -F "sqllite-project/workflow/run-once.sh"
    exit 0
fi

# 加进去
( crontab -l 2>/dev/null; echo "$CRON_LINE" ) | crontab -
echo "registered:"
crontab -l | grep -F "sqllite-project/workflow/run-once.sh"
