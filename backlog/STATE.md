# Backlog State

- **Total tasks**: 18
- **Done**: 3 (16.7%)
- **Queued**: 15
- **In progress**: 0
- **Blocked**: 0
- **Failed**: 0
- **Last update**: 2026-06-07T16:12:59.830332+00:00

## queued (15)
- `T-0004` [L0/util] port UTF-8 helpers (sqlite3Utf8Read/Write/Compare) — last error: claude read 6 files but wrote 0; prompt may be too cautious
- `T-0005` [L0/util] port sqlite3_hash and GrowableArray — last error: HashMap type annotation. Manual fix applied: 17 occurrences of to_ptr() -> to_ptr::<i32>() in tests. Hash.rs compiles. Re-run should pass cargo test.
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
