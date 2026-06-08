#!/usr/bin/env bash
# Local thin client — every heavy lifting goes to BuildBuddy remote.
# 预期:
#   - 首次 (干净 checkout): 30-60s 冷启, 把 1st build 上传
#   - 二次 (同一 commit):  < 5s 全 NO-OP, 网络往返 1-2 次
#
# 用法:
#   bash scripts/local-build.sh                  # build everything
#   bash scripts/local-build.sh //src/util:util  # build single target
#   bash scripts/local-build.sh test //...:all   # 跑所有测试
#
# 切远端执行 (RBE workers) 加 --config=ci-remote:
#   bash scripts/local-build.sh --config=ci-remote //...
set -euo pipefail
cd "$(dirname "$0")/.."

USER_RC="$HOME/.config/bazel/bazelrc"
USER_DIR="$(dirname "$USER_RC")"

# 自动建 ~/.config/bazel 目录 (如果缺) ——
# (此操作是建在用户 $HOME 下, 不在项目内, 一次性的. 不会污染项目.)
if [ ! -d "$USER_DIR" ]; then
    echo "→ 创建 $USER_DIR (一次性)"
    mkdir -p "$USER_DIR"
    chmod 700 "$USER_DIR"
fi

# 验 BuildBuddy API key 已配
if [ ! -f "$USER_RC" ]; then
    echo "→ 创建 $USER_RC 模板 (含占位符, 请手动 paste 你的 key)"
    cat > "$USER_RC" <<'TEMPLATE'
# libsqlite_rs BuildBuddy credentials
# 不进 git. 必须保持 0600.
build --remote_header=x-buildbuddy-api-key=YOUR_BUILDBUDDY_API_KEY
TEMPLATE
    chmod 600 "$USER_RC"
    echo "  → 已建模板: $USER_RC"
    echo "  → 现在请: 1) 去 https://app.buildbuddy.io/ 拿 API key"
    echo "           2) 编辑 $USER_RC, 把 YOUR_BUILDBUDDY_API_KEY 替换成真 key"
    echo "           3) 重新跑此脚本"
    exit 1
fi

if ! grep -q "x-buildbuddy-api-key=YOUR_" "$USER_RC" 2>/dev/null && \
   ! grep -qE "x-buildbuddy-api-key=[^Y]" "$USER_RC" 2>/dev/null; then
    echo "⚠️  $USER_RC 仍含占位符 YOUR_BUILDBUDDY_API_KEY, build 会 401"
    echo "    编辑替换成真 key 后重试"
    exit 1
fi

# 默认走 remote cache, 远端执行可选
# --remote_download_minimal = 只下载 build 产物, 跑 test 才下源码 (省带宽)
exec bazel build --remote_download_minimal "$@"
