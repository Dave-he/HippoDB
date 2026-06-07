# Backlog State

- **Total tasks**: 15
- **Done**: 1
- **Queued**: 14
- **Stalled**: 0
- **In progress**: 0
- **Last update**: 2026-06-07T20:52:00Z

## done (1)
- `T-0001` [P0/workspace] initialize Rust workspace and minimal FFI shim
  — 8 FFI functions, 14 tests passing, commit 200c475

## queued (14)
- `T-0002` (新 spec, 严格对齐 C malloc.c; 等待 runner 重派)
- `T-0003` through `T-0015`

## Event-driven 升级 (commit 976bc83 + 最新未提交)

新增组件:
- `workflow/event_router.py` — 4 类事件源路由: runner-fail/runner-done/gh-pr/gh-actions
- `workflow/trigger.sh` — 事件触发 wrapper
- `workflow/gh_poller.py` — GitHub PR + Actions 状态 polling (替代 webhook)
- `workflow/run-gh-poller.sh` — poller 包装 (cron 5min)

Runner 增强:
- 派发完成后自动 trigger 下一步 (不等 cron 5min)
- sanity_build 现在跑 `cargo check` + `cargo test` (之前只 check)
- 测试失败时把 done demote 成 stalled, 阻止 commit 假 done

注册 cron:
- */5 * * * * workflow/run-once.sh       (主 runner)
- */5 * * * * workflow/run-gh-poller.sh  (GitHub poller)
