//!内部 utility 模块 — 对齐 C端 `src/util.c`家族。
//!
//! 按 L0阶段进度逐子模块展开:
//! - `alloc` — malloc/realloc/free内部 API(本任务 T-0002)
//! - `pattern` — GLOB/LIKE模式匹配(本任务 T-0003)
//! - `utf8` — UTF-8 编解码/计数(本任务 T-0004)
//! - `hash` — 通用 hash table + GrowableArray(本任务 T-0005)
//! - `str` — 大小写不敏感字符串比较(本任务 T-0006)
//! - `printf` — printf 家族(本任务 T-0007a-d: 整数/字符串/浮点/sqlite 扩展)
//! - `random` — ChaCha20-based PRNG (本任务 T-0008)
//! - `datetime` — date/time functions (本任务 T-0009)

pub mod alloc;
pub mod datetime;
pub mod hash;
pub mod pattern;
pub mod printf;
pub mod random;
pub mod str;
pub mod utf8;
