//! T-0002 / T-0003 / T-0004 / T-0005 / T-0006 / T-0007a 集成测试根。
//!
//! - `tests/util/alloc.rs` 子模块包含8个严格对齐 C行为的单元测试。
//! - `tests/util/pattern.rs` 子模块包含23个严格对齐 C行为的单元测试。
//! - `tests/util/utf8.rs` 子模块包含31个严格对齐 C行为的单元测试。
//! - `tests/util/hash.rs` 子模块包含24个 Hash + GrowableArray 测试(本任务 T-0005)。
//! - `tests/util/str.rs` 子模块包含严格字符串比较测试 (T-0006)。
//! - `tests/util/printf_int.rs` 子模块包含 40+ printf 整数格式测试 (T-0007a)。
//!
//! 用 `#[path]` 把 alloc/pattern/utf8/hash/str/printf_int 拉到顶层,使 Cargo 把
//! `tests/util.rs`识别为 一个独立的 test binary(`cargo test --test util`)。

#[path = "util/alloc.rs"]
mod alloc;

#[path = "util/datetime.rs"]
mod datetime;

#[path = "util/hash.rs"]
mod hash;

#[path = "util/pattern.rs"]
mod pattern;

#[path = "util/printf_float.rs"]
mod printf_float;

#[path = "util/printf_int.rs"]
mod printf_int;

#[path = "util/printf_sqlite.rs"]
mod printf_sqlite;

#[path = "util/printf_str.rs"]
mod printf_str;

#[path = "util/random.rs"]
mod random;

#[path = "util/str.rs"]
mod str;

#[path = "util/utf8.rs"]
mod utf8;
