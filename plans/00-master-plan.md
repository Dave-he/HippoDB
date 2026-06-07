# SQLite → Rust 1:1 重构主计划

> 这是**总计划**,不是子任务。子任务在 `backlog/` 下,以文件为单位排队。
> Hermes 调度器按本文件的模块顺序 + 依赖关系,从 `backlog/` 取任务派发。

## 目标(不可妥协)

1. **C ABI 兼容**:任何用官方 `libsqlite3` 编译的 C 程序都能在不改任何代码、不重链接的情况下
   替换为 `libsqllite_rs.so`(通过 `LD_PRELOAD` / `DYLD_INSERT_LIBRARIES`)。
2. **文件格式兼容**:同一 `.db` 文件 C 版和 Rust 版互相读写,内容 byte-for-byte 一致。
3. **SQL 方言兼容**:与官方 SQLite 同版本下,通过 `sqllogictest` 全量测试集。
4. **测试 fixture 兼容**:官方 `testfixture` 全部通过(包括非常规的"我想看 x 是 y 的边角"测试)。

## 模块依赖图(自底向上)

```
L0  基础设施(无依赖)
    ├─ util/        字符串/内存/哈希/随机/UTF8
    ├─ os/          OS 抽象(替代 os_unix.c/os_win.c,提供 VFS trait)
    └─ types/       公共类型:sqlite3_int64/SqliteError/Result

L1  存储(依赖 L0)
    ├─ pager/       Pager(页面缓存 + 事务状态机)
    ├─ wal/         Write-Ahead Log
    ├─ btree/       B-Tree(基于 Pager)
    └─ vfs/         VFS 抽象 + 内存 VFS + Unix VFS

L2  引擎核心(依赖 L1)
    ├─ tokenize/    SQL 词法分析器
    ├─ parse/       SQL 语法分析器(Lemon 移植,生成 parse.rs)
    ├─ expr/        表达式求值
    ├─ func/        内置函数库(算术/字符串/日期/json1)
    ├─ vdbe/        虚拟机 + 指令集
    └─ resolve/     名解析、表/列绑定

L3  优化(依赖 L2)
    ├─ where/       WHERE 子句优化器
    ├─ select/      SELECT 编译
    ├─ insert/      INSERT 编译
    ├─ update/      UPDATE 编译
    └─ delete/      DELETE 编译

L4  DDL(依赖 L2+L3)
    ├─ create/      CREATE TABLE/INDEX/VIEW/TRIGGER
    ├─ alter/       ALTER TABLE
    ├─ drop/        DROP
    └─ analyze/     ANALYZE(统计信息)

L5  Pragma & 系统(依赖 L2)
    ├─ pragma/      PRAGMA 命令
    └─ schema/      sqlite_schema / 元数据查询

L6  公开 API(依赖全部)
    ├─ api/         sqlite3_open/exec/prepare/step/finalize/...
    └─ callback/    用户函数注册 / collation / 作者钩子

L7  扩展(可选,按需)
    ├─ fts3/ fts5/  全文检索
    ├─ rtree/       R-Tree 空间索引
    ├─ session/     会话/变更集
    └─ json1/       JSON 函数(若未在 L2 实现)
```

## 重构顺序(每层内部按 C 文件拆分)

按"先核心后边缘"原则,每层完成后跑一次 `sqllogictest` 兼容子集验证不退步。

### 阶段 P0 — 启动(本计划本身)
- 写主计划 + 模块计划
- 搭建 Rust workspace
- 写 `oracle/` 拉取官方预编译产物
- 写差分测试 harness

### 阶段 P1 — L0 + L1 存储栈(让 1 个 SELECT 1 能跑)
目标:能用 Rust 版打开一个空 .db,执行 `CREATE TABLE t(x); INSERT INTO t VALUES(1); SELECT * FROM t;`
差分测试:与官方二进制交替操作同一个 .db,内容一致。

### 阶段 P2 — L2 + L3 完整查询路径
目标:支持 SELECT/WHERE/JOIN/ORDER BY/GROUP BY/聚合/subquery。

### 阶段 P3 — L4 + L5 DDL
目标:CREATE/ALTER/DROP/ANALYZE/PRAGMA 全量。

### 阶段 P4 — L6 公开 API 稳定
目标:`sqlite3_open_v2`/`prepare_v2`/`bind_*`/`step`/`column_*`/`finalize` 等
全部签名一致;`sqlite3_exec` 等价行为。

### 阶段 P5 — L7 扩展
目标:FTS5、RTREE、json1、session、math、regexp。

### 阶段 P6 — 全量测试
- `testfixture` 全部
- `sqllogictest` 全部
- 模糊测试差分

### 阶段 P7 — 性能/二进制体积优化(可选)

## 每个子任务的硬性规格

1. **文件**:对应一个 C 源文件或其中一段(> 2000 行 C 的文件必须拆子任务)
2. **测试**:必须包含 sqllogictest 兼容片段或直接调用公开 API 的 Rust 测试
3. **回归**:必须通过 oracle 差分(与 C 版同输入比输出)
4. **提交**:每次完成一个子任务,git commit 一次
5. **日志**:任务完成时把 diff 摘要 + 测试输出写进 `notes/`

## 与官方版本同步策略

固定一个 SQLite 版本(默认 `latest`,首次构建时记入 `notes/VERSION.txt`)。
官方大版本发布时(如 3.50 → 3.51)可选择性增量同步新增功能,不在本项目范围。

## 退出条件(全部满足才算完成)

- [ ] `cargo build --release -p libsqlite_rs` 成功,生成 .so/.dylib
- [ ] `LD_PRELOAD` 替换官方 libsqlite3,`sqlite3 --version` 输出与官方一致
- [ ] `./testfixture test/main.test`(官方 test suite)100% 通过
- [ ] `sqllogictest` 100% 通过
- [ ] 模糊测试 24h 无 crash
- [ ] docs 写完(README/CONTRIBUTING + 模块导航)
