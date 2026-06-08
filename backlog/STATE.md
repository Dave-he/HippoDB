# Backlog State

- **Total tasks**: 19 (T-0001 .. T-0016)
- **Done**: 3 (15.8%)
- **Queued**: 15
- **In progress**: 0
- **Blocked**: 0
- **Failed**: 0
- **Stalled**: 0 (T-0004/5/6 RESET 2026-06-08, 重新入队待 T-0016 落定)
- **Last update**: 2026-06-08T09:25:00+08:00
- **Major pivot**: Build system → Bazel + remote cache (云端优先, 本地 0 编译)
  - 详见 `plans/03-bazel-remote-build.md`
  - 决策点: 用户尚未选 A/B/C/D 中任一远端后端
  - T-0016 入队为 prereq, T-0004/5/6 解锁条件 = T-0016 done

## queued (16)
- `T-0004` [L0/util] port UTF-8 helpers (sqlite3Utf8Read/Write/Compare) — **blocked_by: T-0016**
- `T-0005` [L0/util] port sqlite3_hash and GrowableArray — **blocked_by: T-0016**
- `T-0006` [L0/util] port string compare family (strnicmp, sqlite3_stricmp, sqlite3_strnicmp) — **blocked_by: T-0016**
- `T-0007a` [L0/util] port integer printf (d/i/u/x/o/p/%%)
- `T-0007b` [L0/util] port string printf (s/.*)
- `T-0007c` [L0/util] port float printf (f/e/g)
- `T-0007d` [L0/util] port SQLite-specific printf (q/Q/w/z)
- `T-0008` [L0/util] port sqlite3_randomness (PRNG)
- `T-0009` [L0/util] port date/time functions (julianday, strftime, current_time)
- `T-0010` [L1/os] port OS VFS interface and unix VFS (slim subset: open/close/read/write)
- `T-0011` [L1/pager] port Pager struct and page cache (read path only, no write/journalling)
- `T-0012` [L1/pager] port Pager write path with rollback journal
- `T-0013` [L1/btree] port B-Tree read path (open cursor, first/next, key fetch)
- `T-0014` [L2/tokenize] port tokenizer (SQL lexer)
- `T-0015` [L2/parse] port Lemon parser generator and parse.y to Rust
- `T-0016` [P0/build-system] **Migrate to Bazel + remote build** — 选定 A/B/C/D 后即可执行

## done (3)
- `T-0001` [P0/workspace] initialize Rust workspace and minimal FFI shim
- `T-0002` [L0/util] port sqlite3_malloc family and SqliteError OOM deferred check
- `T-0003` [L0/util] port sqlite3_strglob and sqlite3_strlike (pattern matching)

## Infrastructure
- **Cron**: 2026-06-08 09:25 — sqllite 两条 cron entry 已删除 (run-once.sh + run-gh-poller.sh). 备份 /tmp/cron.bak.*. kafka 那条保留.
- **gh-poller**: 自爆 — `timeout: not found` (macOS 没 GNU timeout). 删除 gh-cron.log.
- **Orphan deps**: 8 个 `target/debug/deps` 进程被 kill -9 清掉.
- **Lock**: 0B 孤儿 lock 已删.
- **Next**: 用户选 T-0016 的云端后端 (A/B/C/D), 我写 Bazel 配置 + 改 runner.py 的 SANITY_CMD.
