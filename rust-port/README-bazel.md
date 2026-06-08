# libsqlite_rs — Bazel Build System

> 用户 pivot 2026-06-08: **本地不编译**, 全部走 Bazel + BuildBuddy SaaS 远端.
> 详见 `plans/03-bazel-remote-build.md` 选型过程.

## 1. 架构

```
本机 (Mac)  ──bazel build──>  BuildBuddy SaaS (cloud.buildbuddy.io)
   │                                 │
   │  • thin client (bazelisk 9.1.1)│  • remote cache (gRPC + TLS)
   │  • 0 编译, 只解析 + manifest    │  • remote exec (RBE workers, 可选)
   │  • 5s 内 NO-OP (cache hit)      │  • BES 上报 (UI: app.buildbuddy.io)
   │                                 │
   └──~/.config/bazel/bazelrc──────┘
         (chmod 0600, 不进 git, 含 API key)
```

## 2. 一次性安装

```bash
brew install bazelisk                # 9.1.1, 已装
mkdir -p ~/.config/bazel             # 已建
chmod 700 ~/.config/bazel
touch ~/.config/bazel/bazelrc
chmod 600 ~/.config/bazel/bazelrc

# paste 你的 BuildBuddy API key:
echo 'build --remote_header=x-buildbuddy-api-key=YOUR_KEY_HERE' >> ~/.config/bazel/bazelrc
# (替换 YOUR_KEY_HERE)
```

> 警告: **不要**把 API key 写进 `rust-port/.bazelrc` (它会进 git).

## 3. 日常命令

| 操作 | 命令 | 预期耗时 |
|---|---|---|
| 编全 workspace | `bash scripts/local-build.sh //...` | 首次 30-60s, 二次 < 5s |
| 跑测试 | `bazel test //...:all` | 5-15s (远端) |
| 强制走 RBE | `bash scripts/local-build.sh --config=ci-remote //...` | 30-60s |
| 清本地 cache | `bazel clean` | 5-10s |
| 查远端 invocation | `open https://app.buildbuddy.io/invocation/` | - |

## 4. Claude 任务 SANITY 等价

旧 (cargo):
```bash
cargo check --workspace && cargo test --lib --workspace
```

新 (bazel + remote):
```bash
bazel build //... && bazel test //...:all
```

`workflow/runner.py` 已经改完, 自动用 bazel. 见 `workflow/prompt-template.md` 的
`{{SANITY_CMD}}` 占位符.

## 5. 故障排查

**"another runner holds the lock"** — 参考 plans/03 流水线陷阱, 删 `rust-port/.bazelisk-spawn` 或重启 bazel daemon (`bazel shutdown`).

**"could not download Bazel"** — `~/.cache/bazelisk` 损坏, `rm -rf ~/.cache/bazelisk` 重拉.

**"BES upload failed"** — 远端没收到结果, 但 build 成功了. 不用管, 仍可 cache hit.

**Cache miss 率 > 30%** — 99% 是 `BUILD.bazel` 写错了 source 依赖. 跑 `bazel query 'deps(//:libsqlite_rs)'` 看实际依赖图.
