#!/usr/bin/env bash
# fork(gx): 源码构建安装路径（在线目标机）。
# 链路：rust 工具链检查 → 系统依赖（./get-deps，需 sudo，交互确认）→
#       release 构建四件套 → dotfiles/install.sh 用户级部署。
# 离线机器请改用 dist/ 离线包（make gx-bundle，见 dotfiles/README.md）。

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

say()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
ask()  { printf '%s [y/N] ' "$*"; read -r ans; [[ "$ans" == y || "$ans" == Y ]]; }

# 1) rust 工具链：缺失时给出安装命令，确认后代装（rustup 官方脚本）
if ! command -v cargo >/dev/null; then
   say "cargo not found"
   if ask "install rustup (stable, minimal profile) now?"; then
      curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
      # shellcheck source=/dev/null
      . "$HOME/.cargo/env"
   else
      echo "install rust first: https://wezterm.org/install/source.html" >&2
      exit 1
   fi
fi
MIN_RUST="$(sed -n 's/^min_rust="\(.*\)"/\1/p' ci/check-rust-version.sh)"
say "rust: $(cargo --version) (repo minimum: ${MIN_RUST:-unknown})"
if ! ./ci/check-rust-version.sh; then
   echo "rust toolchain too old; run: rustup update stable" >&2
   exit 1
fi

# 2) 系统依赖：get-deps 是唯一真源；已满足时它幂等快速通过
if ask "install system build deps via ./get-deps (needs sudo)?"; then
   ./get-deps
else
   say "skipping get-deps (assuming build deps already present)"
fi

# 3) release 构建（复用上游 Makefile build 语义）
# fork: 默认总是重建——cargo 增量下无改动时秒级完成，而「存在即复用」
# 曾静默部署落后 HEAD 数天的陈旧二进制（WEZ-BUILD-01）。仅 --reuse-build
# 显式跳过。
REUSE_BUILD=0
for arg in "$@"; do
   case "$arg" in
      --reuse-build) REUSE_BUILD=1 ;;
      *) echo "gx_install.sh: unknown option: $arg" >&2; exit 2 ;;
   esac
done
if [ "$REUSE_BUILD" = 0 ]; then
   say "building release binaries (cargo incremental; first run takes a while)"
   make build BUILD_OPTS=--release
elif [ ! -x target/release/wezterm-gui ]; then
   say "--reuse-build given but no prior build; building now"
   make build BUILD_OPTS=--release
else
   say "reusing target/release binaries (--reuse-build)"
fi

# 4) 用户级部署（配置/插件/字体/desktop entry/zshrc）
say "deploying via dotfiles/install.sh"
bash dotfiles/install.sh --from-build "$REPO/target/release" --bundle-root "$REPO"
