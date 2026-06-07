# Backlog State

- **Total tasks**: 5
- **Done**: 0 (0.0%)
- **Queued**: 5
- **In progress**: 0
- **Blocked**: 0
- **Failed**: 0
- **Last update**: 2026-06-07T00:00:00Z

## queued (5)
- `T-0001` [P0/workspace] initialize Rust workspace and minimal FFI shim
- `T-0002` [L0/util] port sqlite3_malloc family and SqliteError OOM deferred check
- `T-0003` [L0/util] port sqlite3_strglob and sqlite3_strlike (pattern matching)
- `T-0004` [L0/util] port UTF-8 helpers (sqlite3Utf8Read/Write/Compare)
- `T-0005` [L0/util] port sqlite3_hash and GrowableArray

## Next milestone

完成 T-0001~T-0005 后,会自动从 plans/00-master-plan.md 的 L0/util 模块计划中
**继续生成 T-0006~T-0020**:字符串函数(strnicmp/strformat/str_vappendf)、
日期/时间(date.c)、随机数(random.c)、数值转换(printf 浮点精度)。
