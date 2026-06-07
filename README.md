# sqllite-project — 1:1 Rust 重构 SQLite

目标:用 Rust 1:1 重构 SQLite C 源码,所有公开 API、二进制文件格式、行为、错误码、
测试结果与官方一致。不接受 PR 回上游(SQLite 官方 AGENTS.md 明确拒绝 agentic code),
本仓库仅作为学习/兼容性工程。

## 仓库结构

```
sqllite-project/
├── sqlite-source/         # 官方 SQLite 源码(只读,作为参考真相源)
├── rust-port/             # Rust 重构产物
│   ├── src/               # 重构代码
│   ├── tests/             # 重构后的单元/集成测试
│   ├── benches/           # 性能基准
│   ├── docs/              # 内部文档
│   ├── scripts/           # 本地工具脚本
│   └── .ref/              # 来自 sqlite-source 的不可变参考材料
├── plans/                 # 重构计划(模块级 + 总计划)
├── backlog/               # 已识别但未启动的子任务
├── notes/                 # 架构笔记、踩坑记录
├── oracle/                # 官方 SQLite 编译产物 + 测试集(用于差分测试)
└── workflow/              # Hermes 调度器产物(日志、状态机、锁)
```

## 工作流

由 Hermes 在 `workflow/` 下的调度器(每 60 分钟)驱动:

1. **扫描**:读 `backlog/` 取下一个 `status=queued` 的子任务
2. **派发**:用 `claude -p` print 模式执行该子任务(单任务,不超 30 轮)
3. **记录**:把子任务转 `in_progress` → `done` 或 `blocked`,写日志
4. **自检**:每 N 个子任务跑一次 oracle 差分测试,确保兼容性不退化
5. **续单**:若仍有 `queued`,本轮继续派下一个;若 backlog 空,根据里程碑
   自动生成下一批(分模块拆分)

## 兼容性契约

- **C ABI**:`sqlite3_open`/`sqlite3_close`/`sqlite3_exec`/... 全部导出
  (通过 `unsafe extern "C"`),用 `cdylib` crate type 构建
- **错误码**:`SQLITE_OK=0` 起,与官方完全一致
- **文件格式**:`*.db`/`*.db-journal`/`*.db-wal`/`*.db-shm` 与官方 byte-for-byte
  兼容(同一数据库文件可以被 C 版和 Rust 版交替读写,得到相同结果)
- **SQL 方言**:支持 SQL:2023 子集 + SQLite 扩展(ATTACH/CTE/window/json1/fts5/rtree)
- **测试通过率**:
  - `testfixture` 全部 → 目标 100%(分批推进)
  - `sqllogictest` 全部 → 目标 100%
  - `dbsqlfuzz` 模糊测试 → 与官方相同种子下零 crash

## 调度器入口

- 主动触发:`bash workflow/run-once.sh`
- 守护:`cron` 每 60 分钟调一次 `bash workflow/run-once.sh`
- 停止守护:删除 cron 行即可

## 当前状态

详见 `backlog/STATE.md`。
