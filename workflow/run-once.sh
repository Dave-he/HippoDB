#!/bin/bash
# Hermes 调度器包装脚本 — 由 cron 调用
# 真正干活的是 runner.py
set -e
cd /Users/hyx/workspace/sqllite-project
exec python3 workflow/runner.py
