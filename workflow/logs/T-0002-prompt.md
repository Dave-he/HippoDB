# 子任务派发给 Claude 的 prompt 模板

> 这是 `claude -p` 收到的内容。每次执行一个新子任务,会替换 `{{...}}` 占位符。

---

你是一个 SQLite → Rust 重构 agent,工作目录 `/Users/hyx/workspace/sqllite-project/rust-port`。
**只做这一件事**:把 `sqlite-source/src/util.c` 中的 `替换 src/api.rs 中 sqlite3_malloc/malloc64/free 的桩为完整实现: 内部 API 在 src/util/alloc.rs(ZeroMalloc/Malloc/Realloc/Free),行为对齐 C: malloc(0) 返回非 null 可 free; realloc 失败保留原指针; free(NULL) no-op; 用 std::alloc::Global + 显式 Layout。把 db.malloc_failed 的 set/check 接入到 malloc 失败路径(对齐 C 端 sqlite3Oom 的延后错误语义)。T-0001 的 SqliteDb 已有 AtomicBool 字段可用。` 用 1:1 Rust 行为等价地实现。

## 必读(已为你提供关键上下文,你必须先读)

1. `plans/00-master-plan.md` — 总目标与契约
2. `plans/02-c-porting-conventions.md` — C 到 Rust 的命名/错误/内存/FFI 约定
3. `sqlite-source/src/util.c` — 官方参考实现(只读)
4. `rust-port/src/lib.rs` 和 `rust-port/src/error.rs` — 已有公共 API 与错误类型
5. `notes/` 下和本任务相关的现有笔记

## 本子任务

- **ID**:`T-0002`
- **模块**:`L0/util`
- **范围**:`替换 src/api.rs 中 sqlite3_malloc/malloc64/free 的桩为完整实现: 内部 API 在 src/util/alloc.rs(ZeroMalloc/Malloc/Realloc/Free),行为对齐 C: malloc(0) 返回非 null 可 free; realloc 失败保留原指针; free(NULL) no-op; 用 std::alloc::Global + 显式 Layout。把 db.malloc_failed 的 set/check 接入到 malloc 失败路径(对齐 C 端 sqlite3Oom 的延后错误语义)。T-0001 的 SqliteDb 已有 AtomicBool 字段可用。`
- **C 入口函数**(从 `c_file` 提取):`int sqlite3FaultSim(int iTest) | int sqlite3IsNaN(double x) | int sqlite3IsOverflow(double x) | int sqlite3Strlen30(const char *z) | char *sqlite3ColumnType(Column *pCol, char *zDflt) | static SQLITE_NOINLINE void  sqlite3ErrorFinish(sqlite3 *db, int err_code) | void sqlite3Error(sqlite3 *db, int err_code) | void sqlite3ErrorClear(sqlite3 *db)`
- **要写测试**:`tests/util/alloc.rs: 8 单元测试覆盖 size=0 / null free / realloc 失败保留 / 连续 alloc 后 free 全部 / double-free 检测(模拟)/ 对齐 8 字节 / OOM 标志读写。`
- **估计轮数**:`25`

## 硬性要求

1. **行为 1:1**:错误码、返回值、边界条件(byte-level)全部与 C 版一致。
   不能用"差不多"或"看起来对"蒙混。
2. **unsafe 最小化**:只在和 C 交互、或必须做指针/对齐操作时用 `unsafe`,
   加 `// SAFETY:` 注释说明。
3. **错误处理**:统一用 `Result<T, SqliteError>`,不要 `unwrap()`/`expect()` 除非
   已经从数学上证明不可能失败。
4. **测试**:写一个 `#[test]` 跑通官方对应行为,再写一个差分测试
   (用 `oracle/sqlite3` 跑同样输入比输出)。
5. **不要扩大范围**:本任务只做 替换 src/api.rs 中 sqlite3_malloc/malloc64/free 的桩为完整实现: 内部 API 在 src/util/alloc.rs(ZeroMalloc/Malloc/Realloc/Free),行为对齐 C: malloc(0) 返回非 null 可 free; realloc 失败保留原指针; free(NULL) no-op; 用 std::alloc::Global + 显式 Layout。把 db.malloc_failed 的 set/check 接入到 malloc 失败路径(对齐 C 端 sqlite3Oom 的延后错误语义)。T-0001 的 SqliteDb 已有 AtomicBool 字段可用。,不要顺手"优化"或"顺便"重构别的。
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
