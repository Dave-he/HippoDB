# 子任务派发给 Claude 的 prompt 模板

> 这是 `claude -p` 收到的内容。每次执行一个新子任务,会替换 `{{...}}` 占位符。

---

你是一个 SQLite → Rust 重构 agent,工作目录 `/Users/hyx/workspace/sqllite-project/rust-port`。
**只做这一件事**:把 `sqlite-source/src/malloc.c` 中的 `在 src/util/alloc.rs 实现内部 API: Sqlite::Malloc::malloc/malloc64/realloc/free,以及 db.malloc_failed 标志。**严格对齐 C 源码** (sqlite-source/src/malloc.c): malloc(n<=0) 返回 NULL; malloc64(n==0 || n>SQLITE_MAX_ALLOCATION_SIZE) 返回 NULL; realloc 失败时原指针保留; free(NULL) no-op; OOM 不会 panic, 写 db.malloc_failed。用 std::alloc::Global + 显式 Layout。在 src/api.rs 替换 sqlite3_malloc/malloc64/free 的桩为 thin wrapper 调内部 API。**不要**写'假非 null'指针(那是 T-0001 的临时桩, 现在要替换)。写 src/util/mod.rs 包含 alloc 子模块, 在 lib.rs 注册 mod util;` 用 1:1 Rust 行为等价地实现。

## 必读(已为你提供关键上下文,你必须先读)

1. `plans/00-master-plan.md` — 总目标与契约
2. `plans/02-c-porting-conventions.md` — C 到 Rust 的命名/错误/内存/FFI 约定
3. `sqlite-source/src/malloc.c` — 官方参考实现(只读)
4. `rust-port/src/lib.rs` 和 `rust-port/src/error.rs` — 已有公共 API 与错误类型
5. `notes/T-0002.md` — 本任务已有的笔记(可能含之前的发现/spec 修正)
6. `rust-port/src/` — 现有所有 Rust 代码(已 done 的任务产物)

## 本子任务

- **ID**:`T-0002`
- **模块**:`L0/util`
- **范围**:`在 src/util/alloc.rs 实现内部 API: Sqlite::Malloc::malloc/malloc64/realloc/free,以及 db.malloc_failed 标志。**严格对齐 C 源码** (sqlite-source/src/malloc.c): malloc(n<=0) 返回 NULL; malloc64(n==0 || n>SQLITE_MAX_ALLOCATION_SIZE) 返回 NULL; realloc 失败时原指针保留; free(NULL) no-op; OOM 不会 panic, 写 db.malloc_failed。用 std::alloc::Global + 显式 Layout。在 src/api.rs 替换 sqlite3_malloc/malloc64/free 的桩为 thin wrapper 调内部 API。**不要**写'假非 null'指针(那是 T-0001 的临时桩, 现在要替换)。写 src/util/mod.rs 包含 alloc 子模块, 在 lib.rs 注册 mod util;`
- **C 入口函数**(从 `c_file` 提取):`int sqlite3_release_memory(int n) | sqlite3_mutex *sqlite3MallocMutex(void) | int sqlite3_memory_alarm(
  void(*xCallback)(void *pArg, sqlite3_int64 used,int N),
  void *pArg,
  sqlite3_int64 iThreshold
) | sqlite3_int64 sqlite3_soft_heap_limit64(sqlite3_int64 n) | void sqlite3_soft_heap_limit(int n) | sqlite3_int64 sqlite3_hard_heap_limit64(sqlite3_int64 n) | int sqlite3MallocInit(void) | int sqlite3HeapNearlyFull(void)`
- **要写测试**:`tests/util/alloc.rs: 8 单元测试严格对齐 C 行为 — malloc(0) 返回 NULL, malloc64(0) 返回 NULL, free(NULL) no-op, realloc 失败原指针保留, 大块 alloc, 对齐, OOM 标志读写, 0 字节 realloc。`
- **估计轮数**:`25`

## ⚠️ 关键行为要求(违反任何一个直接 `failed`)

1. **第一轮就写代码,不要通读**。你最多花 2 轮(Read + Glob)理解上下文,然后**立刻 Write**。
   如果你读 10+ 文件还不写任何东西, 任务会被判 stalled。
2. **如果任务 spec 与 C 源码不一致,优先 C 源码**(`sqlite-source/src/malloc.c` 是真相源)。
   在 `notes/T-0002-discovery.md` 写 5 行说明, 然后**继续按 C 源码实现,不要停下来**。
3. **如果现有桩代码错了(比如 T-0001 的 malloc(0) 假非 null 指针),直接替换它**,
   不要保留旧行为。
4. **写完代码后必须跑 `cargo check` 和 `cargo test`**, 在返回的 JSON 里报告结果。

## 硬性要求

1. **行为 1:1**:错误码、返回值、边界条件(byte-level)全部与 C 版一致。
   不能用"差不多"或"看起来对"蒙混。
2. **unsafe 最小化**:只在和 C 交互、或必须做指针/对齐操作时用 `unsafe`,
   加 `// SAFETY:` 注释说明。
3. **错误处理**:统一用 `Result<T, SqliteError>`,不要 `unwrap()`/`expect()` 除非
   已经从数学上证明不可能失败。
4. **测试**:写一个 `#[test]` 跑通官方对应行为,再写一个差分测试
   (用 `oracle/sqlite3` 跑同样输入比输出)。
5. **不要扩大范围**:本任务只做 在 src/util/alloc.rs 实现内部 API: Sqlite::Malloc::malloc/malloc64/realloc/free,以及 db.malloc_failed 标志。**严格对齐 C 源码** (sqlite-source/src/malloc.c): malloc(n<=0) 返回 NULL; malloc64(n==0 || n>SQLITE_MAX_ALLOCATION_SIZE) 返回 NULL; realloc 失败时原指针保留; free(NULL) no-op; OOM 不会 panic, 写 db.malloc_failed。用 std::alloc::Global + 显式 Layout。在 src/api.rs 替换 sqlite3_malloc/malloc64/free 的桩为 thin wrapper 调内部 API。**不要**写'假非 null'指针(那是 T-0001 的临时桩, 现在要替换)。写 src/util/mod.rs 包含 alloc 子模块, 在 lib.rs 注册 mod util;,不要顺手"优化"或"顺便"重构别的。
6. **不要重写 C 文件以外的代码**,除非你发现现有 Rust 代码有明显错误(那要先
   在 `notes/T-0002-discovery.md` 记录,再决定是否动)。
7. **commit**:完成后 `git add -A && git commit -m "port: T-0002 port sqlite3_malloc family and SqliteError OOM deferred check"`
   (commit 失败也别慌,记录即可)。

## 输出格式(必须)

完成后用 `--output-format json` 模式返回以下结构(写在最终一条消息):

```json
{
  "id": "T-0002",
  "status": "done" | "blocked" | "failed",
  "files_created": ["..."],
  "files_modified": ["..."],
  "tests_run": "cargo test ... (输出末 20 行)",
  "diff_summary": "1-2 句中文描述",
  "next_action": "如果 status=blocked,写明等什么;如果 done,写下一任务建议"
}
```

如果 `status=blocked` 或 `failed`,**先**在 `notes/T-0002.md` 写清原因,
再返回 JSON。

## 硬性约束(违反任何一个直接 `failed`)

- ❌ 改动 `sqlite-source/` 下任何文件
- ❌ `cargo build` 失败但 commit
- ❌ 跑测试不通过就标 done
- ❌ 写 `unwrap()` 处理 OOM(`db.malloc_failed` 是 C 风格的延后错误,要复刻)
- ❌ 改公开 API 签名(只允许新增,不允许改/删)
- ❌ 跳过读 `plans/02-c-porting-conventions.md`
