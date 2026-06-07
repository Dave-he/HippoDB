# C → Rust 移植约定

写给所有执行 `claude -p` 的 agent。**每个子任务开始前必读。**

## 1. 命名

| C 名称                  | Rust 名称              | 备注                                |
|------------------------|------------------------|-------------------------------------|
| `sqlite3_malloc`       | `SQLite::malloc`       | 公开 API 保持 `sqlite3_malloc`     |
| `sqlite3Malloc`        | `SQLite::Malloc`       | 内部 API 改 PascalCase             |
| `BtCursor`             | `BtCursor`             | 仍用 PascalCase (类型名一致)       |
| `pBt->nKey`            | `cursor.n_key`         | snake_case + struct field 风格     |
| `SQLITE_OK`            | `SqliteError::OK` 常量 | `pub const OK: SqliteError = SqliteError(0);` |

## 2. 错误处理

- **C**:函数返回 `i32`,`SQLITE_OK=0`,失败 `< 0`,`db->mallocFailed` 延后检查
- **Rust**:两种风格并存(根据场景):
  - `pub fn foo(...) -> Result<(), SqliteError>` — 默认
  - `pub unsafe fn bar(...) -> i32` — 当 C ABI 必须保持时(导出符号)

**实现惯例**:内部用 `Result`;`unsafe extern "C"` 包装层用 `i32` 并设置
`db.malloc_failed` 标志(对齐 C 行为)。

## 3. 内存

- C 的 `sqlite3_malloc` → Rust 用 `Global.alloc(Layout::...)`,失败返回 null
- C 的 `sqlite3_free(NULL)` 是 no-op → Rust 端 `Option<NonNull<u8>>`
- C 的 `sqlite3Realloc` 失败时**保留原指针** → Rust 端 `try_realloc` 必须显式
  模拟此行为,不能用 `Vec::resize` (Vec 在 OOM 时会 abort)

## 4. 字符串

- C 的 `char*` → `CStr` / `CString`
- C 的 `const char*` 输入 → `&CStr` 或 `&str`(C 端用 `CStr`,FFI 入口用)
- 内部字符串用 `&str` + 显式长度(因为很多 SQLite 字符串不是 NUL 结尾的)

## 5. 整数宽度

| C 类型             | Rust 类型      | 备注                            |
|-------------------|---------------|---------------------------------|
| `i64`             | `i64`         | 直译                            |
| `sqlite3_int64`   | `i64`         | type alias: `pub type SqliteInt64 = i64;` |
| `u32`             | `u32`         | 直译                            |
| `u64`             | `u64`         | 直译                            |
| `int`             | `i32`         | 直译                            |
| `unsigned int`    | `u32`         | 直译                            |
| `sqlite3_uint64`  | `u64`         | type alias                      |
| `double`          | `f64`         | 直译                            |
| `unsigned char`   | `u8`          | 直译                            |

**禁止**使用 `usize`/`isize` 替代(因为数据库格式里的 offset 是 32-bit)。

## 6. 宏 → const / 编译期计算

| C 宏                       | Rust 替代                                  |
|---------------------------|--------------------------------------------|
| `#define MAX(a,b) ((a)>(b)?(a):(b))` | `fn max_i32(a: i32, b: i32) -> i32 { a.max(b) }` |
| `#define ALWAYS(x)`        | `assert!(x);`                              |
| `#define NEVER(x)`         | `assert!(!x);`                             |
| `#define UNUSED_PARAMETER` | 删掉,或 `let _ = param;` 显式标注          |
| `#define sqlite3Toupper`   | `fn to_upper(c: u8) -> u8`                 |

## 7. 全局变量

SQLite 大量使用 `static`/文件级全局变量。Rust 等价物:

```rust
// 不用 unsafe + lazy_static,而是:
use std::sync::OnceLock;
static GLOBAL_COUNTER: AtomicI64 = AtomicI64::new(0);
```

或对 `sqlite3Malloc` 这类真正全局的服务,用 `crate::Allocator` 单例 + DI。

## 8. FFI 层

公开 API 在 `src/api.rs` 中,每个函数签名:

```rust
#[no_mangle]
pub unsafe extern "C" fn sqlite3_open(
    filename: *const c_char,
    pp_db: *mut *mut SqliteDb,
) -> c_int {
    // SAFETY: 见头注释
    let _ = crate::open(...);
    SQLITE_OK
}
```

每个 `unsafe extern "C"` 函数必须有 `// SAFETY:` 注释 + RustDoc 说明 C 端
调用契约。

## 9. 测试约定

- 单元测试与 C 版同位置:`util.c` → `src/util.rs::tests`
- 集成测试用 `tests/` 下的 `pub mod` 直接调内部 API(不通过 FFI)
- 差分测试在 `tests/diff/` 下,用 `oracle/sqlite3` 当对照组
- 性能基准放 `benches/`,用 `criterion`

## 10. 提交粒度

一个 C 文件/一段 = 一个 commit = 一个 backlog 子任务。
Commit message 格式:`port: <id> <short title>`(如 `port: T-0001 malloc family`)。

## 11. 不可改的文件

- `sqlite-source/` 全部(只读,作为真相源)
- `oracle/` 下的预编译产物(只读)
- `plans/00-master-plan.md`(主计划,需要修订时人工改)

## 12. 必查清单(每个子任务 commit 前)

- [ ] `cargo build` 通过
- [ ] `cargo test` 通过
- [ ] `cargo clippy --all-targets -- -D warnings` 通过
- [ ] 没有 `unwrap()` / `expect()` (除了 #![allow] 标注的少数)
- [ ] 没有引入新依赖(若需要,先在 `notes/<id>-discovery.md` 解释)
- [ ] 公开 API 没有签名变化
