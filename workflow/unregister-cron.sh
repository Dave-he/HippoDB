#!/bin/bash
# 取消 Hermes cron
if crontab -l 2>/dev/null | grep -F "sqllite-project/workflow/run-once.sh" > /dev/null; then
    crontab -l | grep -v -F "sqllite-project/workflow/run-once.sh" | crontab -
    echo "unregistered"
else
    echo "not registered"
fi
