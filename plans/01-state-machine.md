# Refactor 状态机

## 全局状态

- `bootstrapping` — 工程脚手架阶段
- `phase:P0` … `phase:P7` — 阶段进度
- `differential-regression` — 检测到差分回归,需修
- `paused` — 用户主动暂停
- `done` — 全部完成(退出条件全满足)

## 单个子任务状态

`queued` → `in_progress` → (`done` | `blocked` | `failed`)

- `blocked` — 需要用户决策/前置依赖缺失,需 prompt
- `failed` — 连续 3 次派发都失败,转入人工

## 文件位置

- 队列:`backlog/queue.jsonl`(每行一个 JSON 子任务)
- 状态:`backlog/STATE.md`(人读,自动从 queue 生成)
- 日志:`workflow/logs/<date>.log`
- 锁:`workflow/.lock`(防止 cron 重叠)

## 单子任务 JSON schema

```json
{
  "id": "T-0001",
  "module": "L0/util",
  "c_file": "sqlite-source/src/util.c",
  "title": "port sqlite3_malloc family",
  "scope": "实现 sqlite3_malloc/sqlite3_malloc64/sqlite3_realloc/sqlite3_free,行为与官方一致,带 OOM 注入",
  "tests": "tests/util/malloc.rs + sqllogictest 兼容片段",
  "depends_on": [],
  "est_turns": 8,
  "status": "queued",
  "attempts": 0,
  "last_error": null,
  "notes_ref": "notes/T-0001.md"
}
```

## 调度器行为(每 60 分钟)

1. `flock` 拿锁,失败则退出
2. 读 `backlog/queue.jsonl`,扫前 N 个 `status=queued` 且依赖已 `done` 的任务
3. 对每个候选任务:
   - 渲染 prompt(模板见 `workflow/prompt-template.md`)
   - `claude -p <prompt> --max-turns <est> --max-budget-usd 5`
   - 解析退出码 + 提取 `notes/<id>.md` 摘要
   - 成功:`status=done`,写 git commit
   - 失败:attempts++ ,若 ≥3 则 `failed`,否则保留 `queued`
4. 更新 `STATE.md`
5. 每 6 小时额外跑 `oracle/` 差分测试
6. 若 `status=done` 任务占比 < 阶段目标,自动从 `module-plan` 模板批量生成新子任务入队
7. 释放锁,退场
