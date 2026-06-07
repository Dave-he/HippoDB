#!/bin/bash
# 包装 gh_poller — flock + PATH + 5 分钟自杀 (cron 会重启)
set -e
cd /Users/hyx/workspace/sqllite-project
export PATH="/Users/hyx/.nvm/versions/node/v24.12.0/bin:/Users/hyx/.cargo/bin:/usr/local/bin:/usr/bin:/bin:$PATH"

PYTHON_BIN="$(command -v python3.13 || command -v python3.11 || command -v python3.10 || command -v python3)"

# 用 timeout 限制单次运行最多 4.5 分钟 (cron 5min 触发, 留 30s buffer)
exec timeout 270 "$PYTHON_BIN" workflow/gh_poller.py
