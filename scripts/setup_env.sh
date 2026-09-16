#!/usr/bin/env bash
# wezterm 一键环境安装（checkout-local 模式，参考 xyz-csm/herdr）：
# 把本仓开发框架需要的工具钉版安装到仓库内 .local/tools/（gitignored，不写用户全局状态）。
#
# 安装项：
#   .local/tools/nextest/bin/cargo-nextest   # Makefile test 目标的既定运行器
#   .local/tools/venv/                        # graphifyy(图谱) + tomli(py3.10 TOML)
#
# 解析序：Makefile 已把上述 bin 目录前置到 PATH；$WEZTERM_TOOLCHAIN_ROOT
# 可整体重定向 .local/tools（CI 或离线复用）。
#
# 用法：scripts/setup_env.sh           安装/补齐（幂等：已装且校验通过则跳过）
#       scripts/setup_env.sh --check   只读诊断，不安装任何东西
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS="${WEZTERM_TOOLCHAIN_ROOT:-${ROOT}/.local/tools}"
MODE="install"
if [ "${1:-}" = "--check" ]; then
    MODE="check"
elif [ $# -gt 0 ]; then
    echo "用法：scripts/setup_env.sh [--check]" >&2
    exit 2
fi

# ---- 钉版清单（升级 = 改这里 + 临时副本验证产物稳定后再提交）----
NEXTEST_VERSION="0.9.144"
NEXTEST_PLATFORM="x86_64-unknown-linux-musl"
NEXTEST_URL="https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-${NEXTEST_VERSION}/cargo-nextest-${NEXTEST_VERSION}-${NEXTEST_PLATFORM}.tar.gz"
# musl 静态二进制（glibc 版本解耦）；sha256 与版本成对维护。
NEXTEST_SHA256="20ed0a7d3d6f8dda9bb1b0bcb5838aea5784d3e2360746280868996d709dde0a"
GRAPHIFY_VERSION="0.9.20"

fail=0
note()  { printf '[setup-env] %s\n' "$*"; }
ok()    { printf '[setup-env] OK %s\n' "$*"; }
miss()  { printf '[setup-env] 缺少 %s — %s\n' "$1" "$2"; fail=1; }

need_cmd() {
    command -v "$1" >/dev/null 2>&1 && ok "$1" || miss "$1" "$2"
}

check_all() {
    need_cmd cargo "安装 rustup（https://rustup.rs）；引导工具不入本仓"
    need_cmd rustc "随 rustup 安装"
    need_cmd python3 "系统包管理器安装（>=3.10；3.10 需 tomli，框架 venv 会带）"
    command -v git >/dev/null 2>&1 && ok git || miss git "系统包管理器安装"
    if [ -x "${TOOLS}/nextest/bin/cargo-nextest" ]; then
        ok "cargo-nextest $("${TOOLS}/nextest/bin/cargo-nextest" --version 2>/dev/null | awk '{print $NF}')（项目钉版）"
    elif command -v cargo-nextest >/dev/null 2>&1; then
        note "cargo-nextest 走系统 PATH（建议安装项目钉版：scripts/setup_env.sh）"
    else
        miss cargo-nextest "运行 scripts/setup_env.sh 安装项目钉版"
    fi
    if [ -x "${TOOLS}/venv/bin/python" ]; then
        ok "框架 venv $("${TOOLS}/venv/bin/python" --version 2>&1 | awk '{print $2}')（tomli+graphifyy）"
    else
        note "框架 venv 未安装（resolver 在 py3.10 上依赖其 tomli）：scripts/setup_env.sh"
    fi
    [ "$fail" -eq 0 ] && echo "[setup-env] 诊断通过" || echo "[setup-env] 存在缺项（见上）"
    return "$fail"
}

install_nextest() {
    local dest="${TOOLS}/nextest"
    if [ -x "${dest}/bin/cargo-nextest" ] \
        && "${dest}/bin/cargo-nextest" --version 2>/dev/null | grep -q "${NEXTEST_VERSION}"; then
        note "cargo-nextest ${NEXTEST_VERSION} 已是钉版，跳过"
        return 0
    fi
    mkdir -p "${dest}"
    local tmp
    tmp="$(mktemp -d)"
    trap 'rm -rf "${tmp}"' RETURN
    if [ "${NEXTEST_SHA256}" != "PENDING-COMPUTED-ON-FIRST-INSTALL" ]; then
        note "下载 cargo-nextest ${NEXTEST_VERSION}（musl 静态）"
        if curl -fsSL "${NEXTEST_URL}" -o "${tmp}/nextest.tgz"; then
            echo "${NEXTEST_SHA256}  ${tmp}/nextest.tgz" | sha256sum -c - \
                || { echo "[setup-env] sha256 校验失败，拒绝安装" >&2; return 1; }
            tar -xzf "${tmp}/nextest.tgz" -C "${tmp}"
            install -m 0755 "${tmp}/cargo-nextest" "${dest}/bin/cargo-nextest" 2>/dev/null \
                || { mkdir -p "${dest}/bin"; install -m 0755 "${tmp}/cargo-nextest" "${dest}/bin/cargo-nextest"; }
            ok "cargo-nextest ${NEXTEST_VERSION} -> ${dest}/bin/"
            return 0
        fi
        note "预编译包下载失败，回退 cargo install（编译约数分钟）"
    else
        note "首次安装：下载并记录 sha256（确认后写回本脚本钉版）"
    fi
    curl -fsSL "${NEXTEST_URL_BASE}/linux-tar-gzip" -o "${tmp}/nextest.tgz" \
        || { echo "[setup-env] 下载失败；也可手动 cargo install cargo-nextest --locked --root ${dest}" >&2; return 1; }
    local sum
    sum="$(sha256sum "${tmp}/nextest.tgz" | awk '{print $1}')"
    echo "[setup-env] sha256(${NEXTEST_VERSION}) = ${sum}   # 写回 setup_env.sh 的 NEXTEST_SHA256 以钉版"
    tar -xzf "${tmp}/nextest.tgz" -C "${tmp}"
    mkdir -p "${dest}/bin"
    install -m 0755 "${tmp}/cargo-nextest" "${dest}/bin/cargo-nextest"
    ok "cargo-nextest ${NEXTEST_VERSION} -> ${dest}/bin/"
}

install_venv() {
    local venv="${TOOLS}/venv"
    if [ -x "${venv}/bin/python" ] && "${venv}/bin/python" -c "import tomli" 2>/dev/null \
        && "${venv}/bin/graphify" --version 2>/dev/null | grep -q "${GRAPHIFY_VERSION}"; then
        note "框架 venv（tomli + graphifyy ${GRAPHIFY_VERSION}）已就绪，跳过"
        return 0
    fi
    if command -v uv >/dev/null 2>&1; then
        uv venv --python "$(command -v python3)" "${venv}" >/dev/null
        uv pip install --python "${venv}/bin/python" "graphifyy==${GRAPHIFY_VERSION}" tomli >/dev/null
    else
        python3 -m venv "${venv}"
        "${venv}/bin/pip" install --quiet "graphifyy==${GRAPHIFY_VERSION}" tomli
    fi
    "${venv}/bin/python" -c "import tomli" || return 1
    "${venv}/bin/graphify" --version | grep -q "${GRAPHIFY_VERSION}" || return 1
    ok "框架 venv -> ${venv}（tomli + graphifyy ${GRAPHIFY_VERSION}）"
}

if [ "$MODE" = "check" ]; then
    check_all
else
    need_cmd cargo "安装 rustup；引导工具不入本仓"
    need_cmd python3 "系统包管理器安装"
    [ "$fail" -ne 0 ] && { echo "[setup-env] 必备引导工具缺失，先按提示安装" >&2; exit 1; }
    install_nextest
    install_venv
    echo "[setup-env] 完成：make test / make graph / make framework-check 现在使用项目钉版工具"
fi
