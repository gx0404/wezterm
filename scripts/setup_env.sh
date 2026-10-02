#!/usr/bin/env bash
# wezterm 一键环境安装（checkout-local 模式，参考 xyz-csm/herdr）：
# 把本仓开发框架需要的工具钉版安装到仓库内 .local/tools/（gitignored，不写用户全局状态）。
#
# 安装项：
#   .local/tools/nextest/bin/cargo-nextest   # Makefile test 目标的既定运行器
#   .local/tools/venv/                        # graphifyy(图谱) + tomli(py3.10 TOML)
#   .local/tools/stylua/bin/stylua            # lua 代码块格式化（generated-check/
#                                             # update-derived-files 与 docs 构建同一约定）
#   .local/tools/lua/bin/lua54                 # scripts/tests/*.lua 纯 Lua 单测的运行器
#                                             #（dotfiles tests/pure_fn_test.lua 不走它，
#                                             # 经 wezterm mlua 跑）
#
# Windows（uname -s 为 MINGW*/MSYS*/CYGWIN*）：nextest 与 stylua 换装 Windows 预编译包
# （bin 下为 cargo-nextest.exe / stylua.exe）；框架 venv 跳过（仅图谱与 py3.10 需要，
# python3>=3.11 自带 tomllib，resolver 可直接运行）。
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
# Windows 用 x86_64-pc-windows-msvc 包（根目录即 cargo-nextest.exe），同样与版本成对维护。
NEXTEST_SHA256_WINDOWS="b0d6a6569d4ef63a095c5a574a6856c17fb755b51a02a17e4651e738e9192831"
GRAPHIFY_VERSION="0.9.20"
# musl 静态二进制（glibc 版本解耦）；与 ci/stylua.toml 共同决定键表派生物格式。
STYLUA_VERSION="2.5.2"
STYLUA_URL="https://github.com/JohnnyMorganz/StyLua/releases/download/v${STYLUA_VERSION}/stylua-linux-x86_64-musl.zip"
STYLUA_SHA256="ca6f1cf52eaf69e6632b81acef9c197aa24b85eb30d2455a35e7dbe28ae77c72"
# Windows 用 stylua-windows-x86_64.zip（内含 stylua.exe）。
STYLUA_SHA256_WINDOWS="e77d0ea1226b8b389b43f702240091249a96eea25857281f90ea24d0eb9eb969"
LUA_VERSION="5.4.8"
# LuaBinaries 预编译包（Windows x86_64；lua54.exe + lua54.dll，附 lua.exe 别名）。
LUA_URL_WINDOWS="https://sourceforge.net/projects/luabinaries/files/${LUA_VERSION}/Tools%20Executables/lua-${LUA_VERSION}_Win64_bin.zip/download"
LUA_SHA256_WINDOWS="20321e893509e575d2454dd7bbf05342c1f3cb1b3788c0ec5a55ae4279dde169"
# 官方源码包：Linux 直接编译（posix 无 readline 依赖），Windows 预编译失败时回退 mingw 编译。
LUA_TARBALL_URL="https://www.lua.org/ftp/lua-${LUA_VERSION}.tar.gz"
LUA_TARBALL_SHA256="4f18ddae154e793e46eeab727c59ef1c0c0c2b744e7b94219710d76f530629ae"

# ---- 平台分支：Windows（Git Bash/MSYS2/Cygwin）换用 Windows 预编译包 ----
IS_WINDOWS=0
EXE=""
PREBUILT_KIND="musl 静态"
WIN_CURL=""
case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*)
        IS_WINDOWS=1
        # $WEZTERM_TOOLCHAIN_ROOT 可能写成 D:\...；GNU tar 会把 "D:" 当远程主机，统一转成 /d/... 形式。
        TOOLS="$(cygpath -u "${TOOLS}")"
        EXE=".exe"
        PREBUILT_KIND="Windows x86_64"
        NEXTEST_PLATFORM="x86_64-pc-windows-msvc"
        NEXTEST_URL="https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-${NEXTEST_VERSION}/cargo-nextest-${NEXTEST_VERSION}-${NEXTEST_PLATFORM}.tar.gz"
        NEXTEST_SHA256="${NEXTEST_SHA256_WINDOWS}"
        STYLUA_URL="https://github.com/JohnnyMorganz/StyLua/releases/download/v${STYLUA_VERSION}/stylua-windows-x86_64.zip"
        STYLUA_SHA256="${STYLUA_SHA256_WINDOWS}"
        # 优先系统 curl.exe（Schannel，信任 Windows 证书库）：MSYS2 的 curl 用自带 CA 包，
        # 遇到杀软/企业代理解密 HTTPS 时校验失败。完整性仍由上面的 sha256 钉版保证。
        WIN_SYSROOT="$(cygpath -u "${SYSTEMROOT:-C:\\Windows}" 2>/dev/null || true)"
        if [ -n "${WIN_SYSROOT}" ] && [ -x "${WIN_SYSROOT}/System32/curl.exe" ]; then
            WIN_CURL="${WIN_SYSROOT}/System32/curl.exe"
        fi
        ;;
esac

fail=0
note()  { printf '[setup-env] %s\n' "$*"; }
ok()    { printf '[setup-env] OK %s\n' "$*"; }
miss()  { printf '[setup-env] 缺少 %s — %s\n' "$1" "$2"; fail=1; }

need_cmd() {
    command -v "$1" >/dev/null 2>&1 && ok "$1" || miss "$1" "$2"
}

# WindowsApps 下的 python3.exe 可能只是占位符：command -v 找得到，运行却静默失败（退出码 49）。
need_python3() {
    if [ "${IS_WINDOWS}" = 1 ] && command -v python3 >/dev/null 2>&1 \
        && ! python3 -c "" </dev/null >/dev/null 2>&1; then
        miss python3 "$(command -v python3) 无法运行（WindowsApps 占位符？）；把可用的 Python>=3.11 前置到 PATH"
    else
        need_cmd python3 "$1"
    fi
}

# Windows 不装框架 venv（graphifyy 仅图谱需要，tomli 仅 py3.10 需要），只提示 resolver 能否直接运行。
note_venv_windows() {
    if python3 -c "import tomllib" </dev/null >/dev/null 2>&1; then
        note "框架 venv：Windows 分支跳过（仅图谱与 py3.10 需要）；python3 自带 tomllib，resolver 可直接运行"
    else
        note "框架 venv：Windows 分支跳过；当前 python3 无 tomllib，resolver 需要 python3>=3.11（或自装 tomli）"
    fi
}

# Git Bash 与 MSYS2 的 /tmp 指向不同目录，两套工具混用（如 MSYS2 bash 调 Git 自带的 unzip）时
# 默认 mktemp 路径对另一方不可见；Windows 上把临时目录建在盘符路径下的 ${TOOLS} 里。
new_tmpdir() {
    if [ "${IS_WINDOWS}" = 1 ]; then
        mktemp -d "${TOOLS}/.tmp.XXXXXX"
    else
        mktemp -d
    fi
}

# 系统 curl.exe 是原生程序，输出路径先经 cygpath -m 转成 D:/... 形式。
download() {
    if [ -n "${WIN_CURL}" ]; then
        "${WIN_CURL}" -fsSL "$1" -o "$(cygpath -m "$2")"
    else
        curl -fsSL "$1" -o "$2"
    fi
}

check_all() {
    need_cmd cargo "安装 rustup（https://rustup.rs）；引导工具不入本仓"
    need_cmd rustc "随 rustup 安装"
    need_python3 "系统包管理器安装（>=3.10；3.10 需 tomli，框架 venv 会带）"
    command -v git >/dev/null 2>&1 && ok git || miss git "系统包管理器安装"
    if [ -x "${TOOLS}/nextest/bin/cargo-nextest${EXE}" ]; then
        ok "cargo-nextest $("${TOOLS}/nextest/bin/cargo-nextest${EXE}" --version 2>/dev/null | awk 'NR==1{print $2}')（项目钉版）"
    elif command -v cargo-nextest >/dev/null 2>&1; then
        note "cargo-nextest 走系统 PATH（建议安装项目钉版：scripts/setup_env.sh）"
    else
        miss cargo-nextest "运行 scripts/setup_env.sh 安装项目钉版"
    fi
    if [ "${IS_WINDOWS}" = 1 ]; then
        note_venv_windows
    elif [ -x "${TOOLS}/venv/bin/python" ]; then
        ok "框架 venv $("${TOOLS}/venv/bin/python" --version 2>&1 | awk '{print $2}')（tomli+graphifyy）"
    else
        note "框架 venv 未安装（resolver 在 py3.10 上依赖其 tomli）：scripts/setup_env.sh"
    fi
    if [ -x "${TOOLS}/stylua/bin/stylua${EXE}" ]; then
        ok "stylua $("${TOOLS}/stylua/bin/stylua${EXE}" --version 2>/dev/null | awk '{print $NF}')（项目钉版）"
    elif command -v stylua >/dev/null 2>&1; then
        note "stylua 走系统 PATH（建议安装项目钉版：scripts/setup_env.sh）"
    else
        miss stylua "运行 scripts/setup_env.sh 安装项目钉版（键表派生物比对/写入用）"
    fi
    if [ -x "${TOOLS}/lua/bin/lua54${EXE}" ]; then
        ok "lua $("${TOOLS}/lua/bin/lua54${EXE}" -v 2>/dev/null | awk 'NR==1{print $2}')（项目钉版，scripts/tests 的 lua 单测）"
    elif command -v lua54 >/dev/null 2>&1; then
        note "lua54 走系统 PATH（建议安装项目钉版：scripts/setup_env.sh）"
    else
        miss lua54 "运行 scripts/setup_env.sh 安装项目钉版（scripts/tests 的 lua 单测运行器）"
    fi
    [ "$fail" -eq 0 ] && echo "[setup-env] 诊断通过" || echo "[setup-env] 存在缺项（见上）"
    return "$fail"
}

install_nextest() {
    local dest="${TOOLS}/nextest"
    if [ -x "${dest}/bin/cargo-nextest${EXE}" ] \
        && "${dest}/bin/cargo-nextest${EXE}" --version 2>/dev/null | grep -q "${NEXTEST_VERSION}"; then
        note "cargo-nextest ${NEXTEST_VERSION} 已是钉版，跳过"
        return 0
    fi
    mkdir -p "${dest}"
    local tmp
    tmp="$(new_tmpdir)"
    trap 'rm -rf "${tmp}"' RETURN
    if [ "${NEXTEST_SHA256}" != "PENDING-COMPUTED-ON-FIRST-INSTALL" ]; then
        note "下载 cargo-nextest ${NEXTEST_VERSION}（${PREBUILT_KIND}）"
        if download "${NEXTEST_URL}" "${tmp}/nextest.tgz"; then
            echo "${NEXTEST_SHA256}  ${tmp}/nextest.tgz" | sha256sum -c - \
                || { echo "[setup-env] sha256 校验失败，拒绝安装" >&2; return 1; }
            tar -xzf "${tmp}/nextest.tgz" -C "${tmp}"
            install -m 0755 "${tmp}/cargo-nextest${EXE}" "${dest}/bin/cargo-nextest${EXE}" 2>/dev/null \
                || { mkdir -p "${dest}/bin"; install -m 0755 "${tmp}/cargo-nextest${EXE}" "${dest}/bin/cargo-nextest${EXE}"; }
            ok "cargo-nextest ${NEXTEST_VERSION} -> ${dest}/bin/"
            return 0
        fi
        if [ "${IS_WINDOWS}" = 1 ]; then
            # 下方回退只取 Linux 包，Windows 直接报错。
            echo "[setup-env] 下载失败；也可手动 cargo install cargo-nextest --version =${NEXTEST_VERSION} --locked --root ${dest}" >&2
            return 1
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

install_stylua() {
    local dest="${TOOLS}/stylua"
    if [ -x "${dest}/bin/stylua${EXE}" ] \
        && "${dest}/bin/stylua${EXE}" --version 2>/dev/null | grep -q "${STYLUA_VERSION}"; then
        note "stylua ${STYLUA_VERSION} 已是钉版，跳过"
        return 0
    fi
    mkdir -p "${dest}/bin"
    local tmp
    tmp="$(new_tmpdir)"
    trap 'rm -rf "${tmp}"' RETURN
    note "下载 stylua ${STYLUA_VERSION}（${PREBUILT_KIND}）"
    download "${STYLUA_URL}" "${tmp}/stylua.zip" \
        || { echo "[setup-env] 下载失败；也可手动从 StyLua releases 安装到 ${dest}/bin/" >&2; return 1; }
    echo "${STYLUA_SHA256}  ${tmp}/stylua.zip" | sha256sum -c - \
        || { echo "[setup-env] sha256 校验失败，拒绝安装" >&2; return 1; }
    unzip -o -q "${tmp}/stylua.zip" -d "${tmp}"
    install -m 0755 "${tmp}/stylua${EXE}" "${dest}/bin/stylua${EXE}"
    ok "stylua ${STYLUA_VERSION} -> ${dest}/bin/"
}

install_lua() {
    local dest="${TOOLS}/lua"
    if [ -x "${dest}/bin/lua54${EXE}" ] \
        && "${dest}/bin/lua54${EXE}" -v 2>/dev/null | grep -q "Lua ${LUA_VERSION}"; then
        note "lua ${LUA_VERSION} 已是钉版，跳过"
        return 0
    fi
    mkdir -p "${dest}/bin"
    local tmp
    tmp="$(new_tmpdir)"
    trap 'rm -rf "${tmp}"' RETURN
    if [ "${IS_WINDOWS}" = 1 ]; then
        note "下载 lua ${LUA_VERSION}（Windows x86_64 预编译）"
        if download "${LUA_URL_WINDOWS}" "${tmp}/lua.zip"; then
            echo "${LUA_SHA256_WINDOWS}  ${tmp}/lua.zip" | sha256sum -c - \
                || { echo "[setup-env] sha256 校验失败，拒绝安装" >&2; return 1; }
            unzip -o -q "${tmp}/lua.zip" -d "${tmp}/lua-bin"
            install -m 0755 "${tmp}/lua-bin/lua54.exe" "${tmp}/lua-bin/lua54.dll" "${dest}/bin/"
            # lua 名字别名：lua.exe 与 lua54.dll 同目录，加载链不受影响
            cp -f "${dest}/bin/lua54.exe" "${dest}/bin/lua.exe"
            ok "lua ${LUA_VERSION} -> ${dest}/bin/"
            return 0
        fi
        note "预编译包下载失败，回退源码编译（需要 gcc 与 make 在 PATH）"
    fi
    note "下载 lua ${LUA_VERSION} 源码（编译安装）"
    download "${LUA_TARBALL_URL}" "${tmp}/lua.tgz" \
        || { echo "[setup-env] 下载失败" >&2; return 1; }
    echo "${LUA_TARBALL_SHA256}  ${tmp}/lua.tgz" | sha256sum -c - \
        || { echo "[setup-env] sha256 校验失败，拒绝安装" >&2; return 1; }
    tar -xzf "${tmp}/lua.tgz" -C "${tmp}"
    local plat="posix" # 无 readline 依赖，任意 Linux/mac 开发机可编
    [ "${IS_WINDOWS}" = 1 ] && plat="mingw"
    (cd "${tmp}/lua-${LUA_VERSION}" && make "${plat}" -j4) \
        || { echo "[setup-env] 编译失败（需要 gcc 与 make 在 PATH）" >&2; return 1; }
    if [ "${IS_WINDOWS}" = 1 ]; then
        install -m 0755 "${tmp}/lua-${LUA_VERSION}/src/lua.exe" \
            "${tmp}/lua-${LUA_VERSION}/src/lua54.dll" "${dest}/bin/"
        cp -f "${dest}/bin/lua.exe" "${dest}/bin/lua54.exe"
    else
        install -m 0755 "${tmp}/lua-${LUA_VERSION}/src/lua" "${dest}/bin/lua54"
        ln -sf lua54 "${dest}/bin/lua"
    fi
    ok "lua ${LUA_VERSION}（源码编译）-> ${dest}/bin/"
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
    need_python3 "系统包管理器安装"
    [ "$fail" -ne 0 ] && { echo "[setup-env] 必备引导工具缺失，先按提示安装" >&2; exit 1; }
    install_nextest
    if [ "${IS_WINDOWS}" = 1 ]; then
        note_venv_windows
    else
        install_venv
    fi
    install_stylua
    install_lua
    if [ "${IS_WINDOWS}" = 1 ]; then
        echo "[setup-env] 完成：make test / make generated-check 现在使用项目钉版工具（Windows 未装框架 venv，make graph 需自备 graphify）"
    else
        echo "[setup-env] 完成：make test / make graph / make generated-check / make framework-check 现在使用项目钉版工具"
    fi
fi
