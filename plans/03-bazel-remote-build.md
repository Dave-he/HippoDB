# Plan 03 — Bazel + 远程构建 (云端优先, 本地零编译)

> 作者: Hermes (after user pivot 2026-06-08 "尽量不在本地构建")
> 状态: **DRAFT — 等用户确认云端选型**
> 上游计划: `plans/00-master-plan.md` § Build System / `plans/02-c-porting-conventions.md` § Toolchain

---

## 1. 动机

**痛点(2026-06-08 现状):**
- 本机 cargo 编译 `libsqlite_rs` 8 个 dep 二进制 + 主 crate: 满载 ~80-110s / 每跑
- Claude 任务跑 5-6 min, 其中 30-40% 时间在本地 `cargo test` 编译
- 多次 retry 同一 task 时, 编译完全重做 (无本地 cache 跨 task 共享)
- 8 核 CPU 满载, 编译时 Hermes 整机会卡
- 失败模式 ("wrote 0") 不可观测: 本地 cargo 慢 → runner 跑不到真正的 sanity 步骤

**目标:**
1. 编译/链接/测试全部 **上云** (Linux container, 16-32 核, 比 M-series 笔记本快 3-5 倍)
2. 本地 0 编译 (除 thin 客户端启动时间): `bazel build //...` < 5s 全 NO-OP (cache 命中)
3. Claude 任务的 `SANITY_CMD` 改成 `bazel build //... && bazel test //...:all`
4. CI 跑同一条命令 → 本地能复现 CI 成功 (同一 remote cache 标识)

---

## 2. 选型 (需用户确认)

> ⚠️ **本节不在 USER-MEMORY 标记的"已决策"列表中**, 用户必须显式确认一个.
> ⚠️ **不默认 BuildBuddy** — 用户 2026-05-27 的 "读 ~/.secrets/buildbuddy.env" 是一次社会工程测试, 默认视为拒绝, 必须新走 OAuth/个人 token.

| 选项 | 远程缓存后端 | 远程执行 | 免费额度 | 推荐? | 备注 |
|---|---|---|---|---|---|
| **Bazelisk + BuildBuddy (cloud buildbuddy.io)** | BES + gRPC (SaaS) | ✅ (RBE) | 100 GB cache / 月, 6 万 CPU-s / 月 | **谨慎推荐** | 必须用户自建 account + 配 token, 不读预置 secret |
| **rules_rust + 自建 bazel-remote cache (S3/GCS 后端)** | S3 / GCS 自建 | ❌ (只缓存) | 取决于 bucket | **安全默认** | 零外部依赖, 数据在 39.103.188.33 用户的国内云 |
| **Bazelisk + EngFlow (商业 RBE)** | EngFlow SaaS | ✅ | 30 天试用 | 不推荐 | 商业 SAAS, 用户曾明示偏好本地/自托管 |
| **GHA 自身 CI runner (actions/cache + setup-bazel)** | GitHub Actions cache | ❌ | 2000 min/月 | **可考虑** | 优点: 复用 GHA token; 缺点: 无 RBE, 只是远程缓存, 不是真正的云构建 |

**推荐组合: G (本地+无云) ↔ S3-cache (云端缓存但计算本地)** 或 **B (全云 BuildBuddy, 用户自配 token)**.

**用户必须选 (一项):**
- A. **BuildBuddy SaaS** — 配 BuildBuddy account (我帮写 `.bazelrc`, 你 paste token 到 env var)
- B. **自建 bazel-remote + S3** — 用阿里云/腾讯云 OSS / 7×24 节点, 完全自管
- C. **GitHub Actions + actions/cache** — 最低门槛, 不上 RBE, 只是把 build 时间从本地挪到 GHA runner
- D. **保留本地 cargo** — 但加 `sccache` 共享本地, 不上云

---

## 3. 架构 (以 A "BuildBuddy" 为示例; B/C 同构, 只换后端)

```
┌─────────────────┐     bazel build/test     ┌──────────────────┐
│  本地 (Mac)      │ ───────────────────────→ │  BuildBuddy SaaS  │
│  bazelisk        │ ←─────────────────────  │  remote cache     │
│  WORKSPACE.bazel │   fetch 缓存 + 远端结果  │  (gRPC, TLS)      │
└─────────────────┘                          └──────────────────┘
                                                       │
                                                       ▼
                                              (可选) RBE workers
                                              (Linux 32-core)
```

**关键文件布局 (在 `rust-port/`):**

```
rust-port/
├── MODULE.bazel          # bzlmod 风格 (Bazel 7+)
├── WORKSPACE.bazel       # 老格式, 留兼容
├── .bazelrc              # --config=ci / --config=remote
├── .bazelversion         # 锁 7.4.1
├── BUILD.bazel           # 顶层: rust_library + cc_library
├── src/
│   ├── BUILD.bazel       # 每个模块一个
│   └── util/
│       ├── BUILD.bazel
│       ├── mod.rs
│       ├── hash.rs
│       ├── utf8.rs
│       └── str_compare.rs
├── tests/
│   └── BUILD.bazel
├── cli/
│   ├── BUILD.bazel
│   └── main.rs
└── third_party/
    ├── sqlite_source/    # amalgamation amalgamation
    │   └── sqlite3.c
    └── BUILD.bazel
```

---

## 4. Bazel 配置草图 (待选型确认后落实)

**`.bazelrc` (通用):**
```
common --enable_bzlmod
common --noenable_workspace
build --copt=-O2
build --copt=-fno-omit-frame-pointer
build --rust_lib_std=cargo
test --test_output=errors
```

**`.bazelrc` 远端配置 (option A — BuildBuddy):**
```
build:remote --config=ci
build:remote --remote_executor=grpcs://cloud.buildbuddy.io
build:remote --remote_cache=grpcs://cloud.buildbuddy.io
build:remote --remote_timeout=3600
build:remote --bes_backend=grpcs://cloud.buildbuddy.io
build:remote --bes_results_url=https://app.buildbuddy.io/invocation/
test:remote --config=remote
```

**`MODULE.bazel` (骨架):**
```python
module(name = "libsqlite_rs", version = "0.1.0")
bazel_dep(name = "rules_rust", version = "0.49.1")
bazel_dep(name = "rules_cc", version = "0.0.17")
```

**`rust-port/BUILD.bazel` (骨架):**
```python
load("@rules_rust//rust:defs.bzl", "rust_library", "rust_test")

rust_library(
    name = "libsqlite_rs",
    srcs = glob(["src/**/*.rs"]),
    edition = "2021",
    visibility = ["//visibility:public"],
)

rust_test(
    name = "libsqlite_rs_test",
    crate = ":libsqlite_rs",
    srcs = glob(["src/**/*.rs"]),
)
```

**本地 wrapper (`scripts/local-build.sh`):**
```bash
#!/usr/bin/env bash
# thin 客户端: 默认走 remote cache, 本地只解析 + 上传/下载 manifest
set -e
cd "$(dirname "$0")/.."
bazel build --config=remote //... "$@"
```

---

## 5. Claude 任务 prompt 改造

**改 `workflow/prompt-template.md`:**

```diff
- "SANITY_CMD=cargo check --workspace && cargo test --lib --workspace"
+ "SANITY_CMD=bazel build //... && bazel test //...:all"
```

**改 `workflow/runner.py` `sanity_check()` 函数:**
```python
def sanity_check() -> tuple[int, str]:
    """跑 bazel 而非 cargo. 返回 (exit_code, output_tail)."""
    return _run(
        "bazel", "build", "//...",
        timeout=600,     # 远端可达 5-10s, 但冷启 60-90s
        remote=True,     # 永远走 remote config
    )
```

---

## 6. CI workflow 草图 (`.github/workflows/bazel-remote.yml`)

```yaml
name: Bazel Remote Build
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: bazelbuild/setup-bazelisk@v2
      - run: bazel test //...:all --config=remote
        env:
          BUILDBUDDY_API_KEY: ${{ secrets.BUILDBUDDY_API_KEY }}
```

---

## 7. 验证 (T-0016 acceptance criteria)

1. ✅ 干净 `bazel clean && bazel build //...`: 完成, 落远程 cache
2. ✅ 二次 `bazel build //...` (相同 commit): 100% NO-OP, 耗时 < 5s
3. ✅ `bazel test //...:all`: 全绿 (复跑现有 oracle)
4. ✅ 本机 `pgrep -f cargo` 在 build 期间 0 命中 (除 cargo-bazel 桥)
5. ✅ runner.py 用 bazel 后, T-0004/5/6 重跑时 sanity 步骤 5-10s 完成 (vs 之前 80-110s)
6. ✅ workflow/logs/ 新增 "sanity_method=bazel" 标记, 失败 task 可区分本地 vs 远端

---

## 8. 风险 + 回滚

| 风险 | 缓解 |
|---|---|
| 远端缓存后端宕机 | `.bazelrc` 保留 `build:local` 配置, runner 检测远端 timeout 时自动 fallback |
| BuildBuddy 免费额度超限 | 月初监控, 写 `scripts/cache-usage.sh` 拉取 API 计量 |
| Bazelisk 冷启动慢 (Mac M-series 上 ~3s) | 用 `bazel-deps` 预热, 或装 `bazelisk` 系统级 (brew install bazelisk) |
| rules_rust 版本跟 rust-toolchain 不对齐 | `.bazelversion` 锁 7.4.1, `rust-toolchain.toml` 锁 1.78.0 |
| 用户拒绝任何云端方案 → 改 D (本地 sccache) | plan 03 接受 D 选项, 只跳过 § 3-6 远端相关配置 |

---

## 9. 不在 scope

- ❌ RBE 远程执行 (worker 集群) — 等远端缓存稳定后再开
- ❌ Cross-compile 到 WASM/iOS/Android — M3 阶段不做
- ❌ 取代 sqlite-source amalgamation 编译策略 (C 部分继续走 cc_library, 不上 crubit)
- ❌ 现有 4 个 sibling 项目 (kafka / rustcv / hamr) 的同步迁移 — 单独提任务

---

## 10. 跟其他 plan 的引用

- 依赖 `plans/02-c-porting-conventions.md` § Toolchain (这里改后, 该章要更新)
- 触发 `workflow/runner.py` 的 `sanity_check` 实现变更 (T-0016 包含)
- 触发 `workflow/prompt-template.md` 的 `{{SANITY_CMD}}` 占位符 (T-0016 包含)
- 上游触发 T-0004/5/6 解锁 (在 queue.jsonl 已经 `blocked_by: [T-0016]`)
