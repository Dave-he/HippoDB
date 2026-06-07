//! T-0002 集成测试根。
//!
//! `tests/util/alloc.rs` 子模块包含 8 个严格对齐 C 行为的单元测试。
//! 用 `#[path]` 把 alloc 拉到顶层,使 Cargo 把 `tests/util.rs` 识别为
//! 一个独立的 test binary(`cargo test --test util`)。

#[path = "util/alloc.rs"]
mod alloc;
