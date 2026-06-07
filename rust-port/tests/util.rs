//! T-0002 / T-0003 / T-0004 集成测试根。
//!
//! - `tests/util/alloc.rs` 子模块包含8个严格对齐 C行为的单元测试。
//! - `tests/util/pattern.rs` 子模块包含23个严格对齐 C行为的单元测试。
//! - `tests/util/utf8.rs` 子模块包含31个严格对齐 C行为的单元测试。
//!
//! 用 `#[path]` 把 alloc/pattern/utf8 拉到顶层,使 Cargo 把 `tests/util.rs`识别为
//! 一个独立的 test binary(`cargo test --test util`)。

#[path = "util/alloc.rs"]
mod alloc;

#[path = "util/pattern.rs"]
mod pattern;

#[path = "util/utf8.rs"]
mod utf8;
