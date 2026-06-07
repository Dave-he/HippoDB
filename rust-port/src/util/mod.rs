//!内部 utility 模块 — 对齐 C端 `src/util.c`家族。
//!
//! 按 L0阶段进度逐子模块展开:
//! - `alloc` — malloc/realloc/free内部 API(本任务 T-0002)
//! - `pattern` — GLOB/LIKE模式匹配(本任务 T-0003)
//! - `utf8` — UTF-8 编解码/计数(本任务 T-0004)

pub mod alloc;
pub mod pattern;
pub mod utf8;
