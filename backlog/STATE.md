# Backlog State

- **Total tasks**: 15
- **Done**: 1 (6.7%)
- **Queued**: 13
- **In progress**: 0
- **Blocked**: 0
- **Failed**: 1
- **Last update**: 2026-06-07T12:25:00.624409+00:00

## failed (1)
- `T-0002` [L0/util] port sqlite3_malloc family and SqliteError OOM deferred check — last error: no result; raw len=978

## queued (13)
- `T-0003` [L0/util] port sqlite3_strglob and sqlite3_strlike (pattern matching)
- `T-0004` [L0/util] port UTF-8 helpers (sqlite3Utf8Read/Write/Compare)
- `T-0005` [L0/util] port sqlite3_hash and GrowableArray
- `T-0006` [L0/util] port string compare family (strnicmp, sqlite3_stricmp, sqlite3_strnicmp)
- `T-0007` [L0/util] port sqlite3_mprintf / sqlite3_vsnprintf (printf-style format)
- `T-0008` [L0/util] port sqlite3_randomness (PRNG)
- `T-0009` [L0/util] port date/time functions (julianday, strftime, current_time)
- `T-0010` [L1/os] port OS VFS interface and unix VFS (slim subset: open/close/read/write)
- `T-0011` [L1/pager] port Pager struct and page cache (read path only, no write/journalling)
- `T-0012` [L1/pager] port Pager write path with rollback journal
- `T-0013` [L1/btree] port B-Tree read path (open cursor, first/next, key fetch)
- `T-0014` [L2/tokenize] port tokenizer (SQL lexer)
- `T-0015` [L2/parse] port Lemon parser generator and parse.y to Rust

## done (1)
- `T-0001` [P0/workspace] initialize Rust workspace and minimal FFI shim
