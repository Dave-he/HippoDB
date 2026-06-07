# Backlog State

- **Total tasks**: 18
- **Done**: 3 (16.7%)
- **Queued**: 13
- **In progress**: 1
- **Blocked**: 0
- **Failed**: 0
- **Last update**: 2026-06-07T15:32:12.756639+00:00

## in_progress (1)
- `T-0005` [L0/util] port sqlite3_hash and GrowableArray — last error: killed by Hermes after 22min no-result; Claude API appears to hang on str.rs (a small task)

## queued (13)
- `T-0006` [L0/util] port string compare family (strnicmp, sqlite3_stricmp, sqlite3_strnicmp) — last error: claude read 10 files but wrote 0; prompt may be too cautious
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

## done (3)
- `T-0001` [P0/workspace] initialize Rust workspace and minimal FFI shim
- `T-0002` [L0/util] port sqlite3_malloc family and SqliteError OOM deferred check
- `T-0003` [L0/util] port sqlite3_strglob and sqlite3_strlike (pattern matching)
