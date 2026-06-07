#!/bin/bash
# Hermes 调度器包装脚本 — 由 cron 调用
# 真正干活的是 runner.py
#
# 注意: cron 环境默认 PATH=/usr/bin:/bin, 缺少 nvm/conda 等。
# 必须显式扩展 PATH 才能找到 claude / python3.13。
set -e
cd /Users/hyx/workspace/sqllite-project

# 显式扩展 PATH,让 claude / python3.13 / cargo 都能被找到
export PATH="/Users/hyx/.nvm/versions/node/v24.12.0/bin:/Users/hyx/.cargo/bin:/usr/local/bin:/usr/bin:/bin:$PATH"
# cargo / rustc 在此
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"

PYTHON_BIN="$(command -v python3.13 || command -v python3.11 || command -v python3.10 || command -v python3)"
if [ -z "$PYTHON_BIN" ]; then
    echo "no python3.10+ found" >&2
    exit 1
fi
# 双保险: claude 必须在 PATH 里
if ! command -v claude >/dev/null 2>&1; then
    echo "claude not in PATH; refusing to run" >&2
    exit 1
fi

exec "$PYTHON_BIN" workflow/runner.py
