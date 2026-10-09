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
#   .local/tools/nasm/bin/nasm.exe           # 仅 Windows：MSVC 打包的 vendored OpenSSL 汇编器
#   .local/tools/perl/{perl,c}/bin            # 仅 Windows：Strawberry Perl portable（OpenSSL Configure）
#   .local/tools/make/bin/make.exe           # 仅 Windows：同一 Perl 包的 GNU Make 4.4.1 与依赖 DLL
#
# Windows（uname -s 为 MINGW*/MSYS*/CYGWIN*）：nextest 与 stylua 换装 Windows 预编译包
# （bin 下为 cargo-nextest.exe / stylua.exe）；lua 用 LuaBinaries 预编译包；框架 venv
# 同样安装（Windows venv 是 Scripts/ 布局，补 python3.exe 与 bin/graphify shim）。
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
GRAPHIFY_VERSION="0.9.73"
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
# NASM（仅 Windows 用）：官方 releasebuilds 目录不发布 sha256 校验文件，
# 以下值为 2026-10-03 从官方 URL 下载后自行计算（来源 nasm.us 本身，非第三方镜像）。
NASM_VERSION="3.02"
NASM_URL_WINDOWS="https://www.nasm.us/pub/nasm/releasebuilds/${NASM_VERSION}/win64/nasm-${NASM_VERSION}-win64.zip"
NASM_SHA256_WINDOWS="161d0bfaff53c2f9e9f3e69fd0672323ebabafd1268976a5cec11be92a19aee7"
# Strawberry Perl portable（仅 Windows 用）：sha256 取自 GitHub release SP_54231_64bit
# 资产 digest（2026-10-03 读取）。zip 约 290 MB，解压后保留 perl/bin 与 c/bin 布局。
PERL_VERSION="5.42.3.1"
PERL_URL_WINDOWS="https://github.com/StrawberryPerl/Perl-Dist-Strawberry/releases/download/SP_54231_64bit/strawberry-perl-${PERL_VERSION}-64bit-portable.zip"
PERL_SHA256_WINDOWS="6a081a811781c30aca51dbc036afd93092af91e3297901f02c17043795a10690"

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

need_python3() {
    local py
    if py="$(find_real_python)"; then
        ok "Python>=3.10（${py}）"
    else
        miss python3 "请提供可运行的 Python>=3.10；WindowsApps 别名不作为解释器"
    fi
}

version_is() {
    local expected="$1" field="$2" output
    shift 2
    output="$("$@" 2>&1)" || return 1
    [ "$(printf '%s\n' "${output}" | awk -v field="${field}" 'NR==1 {print $field}')" = "${expected}" ]
}

nextest_ready() { version_is "${NEXTEST_VERSION}" 2 "${TOOLS}/nextest/bin/cargo-nextest${EXE}" --version; }
stylua_ready() { version_is "${STYLUA_VERSION}" 2 "${TOOLS}/stylua/bin/stylua${EXE}" --version; }
lua_ready() { version_is "${LUA_VERSION}" 2 "${TOOLS}/lua/bin/lua54${EXE}" -v; }
nasm_ready() { version_is "${NASM_VERSION}" 3 "${TOOLS}/nasm/bin/nasm.exe" -v; }
perl_ready() {
    [ -f "${TOOLS}/perl/.version" ] && [ "$(<"${TOOLS}/perl/.version")" = "${PERL_VERSION}" ] \
        && version_is "v${PERL_VERSION%.*}" 1 "${TOOLS}/perl/perl/bin/perl.exe" -e 'print "$^V\n"'
}
make_ready() { version_is "4.4.1" 3 "${TOOLS}/make/bin/make.exe" --version; }

# venv 健康检查（两平台统一）：Linux/mac 用 bin/，Windows venv 用 Scripts/ 布局。
venv_python() {
    if [ -x "${TOOLS}/venv/bin/python" ]; then
        printf '%s\n' "${TOOLS}/venv/bin/python"
    elif [ -x "${TOOLS}/venv/Scripts/python.exe" ]; then
        printf '%s\n' "${TOOLS}/venv/Scripts/python.exe"
    else
        return 1
    fi
}

venv_packages_ready() {
    local py
    py="$(venv_python)" || return 1
    "${py}" -c "import graphify, tomli, importlib.metadata as m, sys; sys.exit(0 if sys.version_info >= (3, 10) and m.version('graphifyy') == '${GRAPHIFY_VERSION}' else 1)" >/dev/null 2>&1
}

venv_ready() {
    venv_packages_ready || return 1
    [ -x "${TOOLS}/venv/bin/graphify" ] || return 1
    if [ "${IS_WINDOWS}" = 1 ]; then
        "${TOOLS}/venv/Scripts/python3.exe" -c 'import sys; assert sys.version_info >= (3, 10); assert sys.prefix != sys.base_prefix' >/dev/null 2>&1 || return 1
    fi
    "${TOOLS}/venv/bin/graphify" --version >/dev/null 2>&1
}

new_tmpdir() {
    mkdir -p "${TOOLS}"
    mktemp -d "${TOOLS}/.tmp.XXXXXX"
}

# 系统 curl.exe 是原生程序，输出路径先经 cygpath -m 转成 D:/... 形式。
download() {
    if [ -n "${WIN_CURL}" ]; then
        "${WIN_CURL}" -fsSL "$1" -o "$(cygpath -m "$2")"
    else
        curl -fsSL "$1" -o "$2"
    fi
}

check_tool() {
    if "$1"; then
        ok "$2（项目钉版）"
    else
        miss "$2" "运行 scripts/setup_env.sh 修复项目工具（缺失、无法运行或版本不符）"
    fi
}

check_all() {
    fail=0
    if [ "${IS_WINDOWS}" != 1 ]; then
        cargo --version >/dev/null 2>&1 && ok cargo || miss cargo "安装 rustup；引导工具不入本仓"
        rustc --version >/dev/null 2>&1 && ok rustc || miss rustc "随 rustup 安装"
    fi
    need_python3
    git --version >/dev/null 2>&1 && ok git || miss git "安装 Git"
    check_tool nextest_ready "cargo-nextest ${NEXTEST_VERSION}"
    check_tool venv_ready "框架 venv（tomli + graphifyy ${GRAPHIFY_VERSION}）"
    check_tool stylua_ready "stylua ${STYLUA_VERSION}"
    check_tool lua_ready "lua ${LUA_VERSION}"
    if [ "${IS_WINDOWS}" = 1 ]; then
        check_tool nasm_ready "nasm ${NASM_VERSION}"
        check_tool perl_ready "Strawberry Perl ${PERL_VERSION}"
        check_tool make_ready "GNU Make 4.4.1"
        if MSYS2_ARG_CONV_EXCL='*' cmd.exe /d /c call "$(cygpath -w "${ROOT}/scripts/gx_msvc_env.cmd")" --check; then
            ok "MSVC 编译前置"
        else
            miss "MSVC 编译前置" "运行 scripts\\gx_msvc_env.cmd --check 查看诊断；系统组件由用户安装"
        fi
    fi
    [ "$fail" -eq 0 ] && echo "[setup-env] 诊断通过" || echo "[setup-env] 存在缺项（见上）"
    return "$fail"
}

install_nextest() {
    local dest="${TOOLS}/nextest"
    if nextest_ready; then
        note "cargo-nextest ${NEXTEST_VERSION} 已是钉版，跳过"
        return 0
    fi
    mkdir -p "${dest}/bin"
    local tmp
    tmp="$(new_tmpdir)"
    trap 'rm -rf "${tmp}"' RETURN
    note "下载 cargo-nextest ${NEXTEST_VERSION}（${PREBUILT_KIND}）"
    download "${NEXTEST_URL}" "${tmp}/nextest.tgz" \
        || { echo "[setup-env] 下载失败；检查网络后重跑 scripts/setup_env.sh（不使用未校验回退）" >&2; return 1; }
    echo "${NEXTEST_SHA256}  ${tmp}/nextest.tgz" | sha256sum -c - \
        || { echo "[setup-env] sha256 校验失败，拒绝安装" >&2; return 1; }
    tar -xzf "${tmp}/nextest.tgz" -C "${tmp}"
    install -m 0755 "${tmp}/cargo-nextest${EXE}" "${dest}/bin/cargo-nextest${EXE}"
    nextest_ready || { echo "[setup-env] cargo-nextest 运行/版本校验失败" >&2; return 1; }
    ok "cargo-nextest ${NEXTEST_VERSION} -> ${dest}/bin/"
}

install_stylua() {
    local dest="${TOOLS}/stylua"
    if stylua_ready; then
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
    if lua_ready; then
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

# 仅 Windows：NASM 钉版装到 ${TOOLS}/nasm/bin/。
install_nasm() {
    [ "${IS_WINDOWS}" = 1 ] || return 0
    local dest="${TOOLS}/nasm"
    if nasm_ready; then
        note "nasm ${NASM_VERSION} 已是钉版，跳过"
        return 0
    fi
    mkdir -p "${dest}/bin"
    local tmp
    tmp="$(new_tmpdir)"
    trap 'rm -rf "${tmp}"' RETURN
    note "下载 nasm ${NASM_VERSION}（Windows x64）"
    download "${NASM_URL_WINDOWS}" "${tmp}/nasm.zip"         || { echo "[setup-env] nasm 下载失败（${NASM_URL_WINDOWS}）；检查网络后重跑 scripts/setup_env.sh" >&2; return 1; }
    echo "${NASM_SHA256_WINDOWS}  ${tmp}/nasm.zip" | sha256sum -c -         || { echo "[setup-env] nasm sha256 校验失败，拒绝安装" >&2; return 1; }
    unzip -o -q "${tmp}/nasm.zip" -d "${tmp}/nasm-x"
    install -m 0755 "${tmp}/nasm-x/nasm-${NASM_VERSION}/nasm.exe"         "${tmp}/nasm-x/nasm-${NASM_VERSION}/ndisasm.exe" "${dest}/bin/"
    ok "nasm ${NASM_VERSION} -> ${dest}/bin/"
}

# 仅 Windows：Strawberry Perl portable 钉版解压到 ${TOOLS}/perl/（perl/bin、c/bin 布局）。
install_perl() {
    [ "${IS_WINDOWS}" = 1 ] || return 0
    local dest="${TOOLS}/perl"
    if perl_ready; then
        note "perl ${PERL_VERSION} 已是钉版，跳过"
        return 0
    fi
    mkdir -p "${TOOLS}"
    local tmp
    tmp="$(new_tmpdir)"
    trap 'rm -rf "${tmp}"' RETURN
    note "下载 Strawberry Perl ${PERL_VERSION} portable（约 290 MB，请耐心等待）"
    download "${PERL_URL_WINDOWS}" "${tmp}/perl.zip"         || { echo "[setup-env] perl 下载失败（${PERL_URL_WINDOWS}）；检查网络后重跑 scripts/setup_env.sh" >&2; return 1; }
    echo "${PERL_SHA256_WINDOWS}  ${tmp}/perl.zip" | sha256sum -c -         || { echo "[setup-env] perl sha256 校验失败，拒绝安装" >&2; return 1; }
    rm -rf "${dest}"
    mkdir -p "${dest}"
    unzip -o -q "${tmp}/perl.zip" -d "${dest}"         || { echo "[setup-env] perl 解压失败" >&2; rm -rf "${dest}"; return 1; }
    [ -x "${dest}/perl/bin/perl.exe" ]         || { echo "[setup-env] 解压后缺少 perl/bin/perl.exe，布局异常" >&2; return 1; }
    printf '%s' "${PERL_VERSION}" > "${dest}/.version"
    ok "perl ${PERL_VERSION} -> ${dest}/"
}

install_make() {
    [ "${IS_WINDOWS}" = 1 ] || return 0
    perl_ready && version_is "4.4.1" 3 "${TOOLS}/perl/c/bin/gmake.exe" --version \
        || { echo "[setup-env] Strawberry Perl 包内 GNU Make 4.4.1 校验失败" >&2; return 1; }
    if make_ready; then
        note "GNU Make 4.4.1 已是钉版，跳过"
        return 0
    fi
    mkdir -p "${TOOLS}/make/bin"
    install -m 0755 "${TOOLS}/perl/c/bin/gmake.exe" "${TOOLS}/make/bin/make.exe"
    install -m 0755 "${TOOLS}/perl/c/bin/libintl-8.dll" "${TOOLS}/perl/c/bin/libiconv-2.dll" "${TOOLS}/make/bin/"
    make_ready || { echo "[setup-env] GNU Make 运行/版本校验失败" >&2; return 1; }
    ok "GNU Make 4.4.1 -> ${TOOLS}/make/bin/（许可保留于 ${TOOLS}/perl/licenses/）"
}

probe_python() {
    local path="$1" real
    case "$(printf '%s' "${path}" | tr '[:upper:]' '[:lower:]')" in *windowsapps*) return 1 ;; esac
    [ -x "${path}" ] || return 1
    real="$("${path}" -c 'import sys; sys.exit(1) if sys.version_info < (3, 10) else print(sys.executable)' </dev/null 2>/dev/null)" || return 1
    [ -n "${real}" ] || return 1
    case "$(printf '%s' "${real}" | tr '[:upper:]' '[:lower:]')" in *windowsapps*) return 1 ;; esac
    if [ "${IS_WINDOWS}" = 1 ]; then real="$(cygpath -u "${real}")"; fi
    printf '%s\n' "${real}"
}

find_real_python() {
    local cand path root
    for path in "${TOOLS}/venv/Scripts/python.exe" "${TOOLS}/venv/bin/python"; do
        probe_python "${path}" && return 0
    done
    for cand in python3 python; do
        while IFS= read -r path; do
            probe_python "${path}" && return 0
        done < <(type -a -p "${cand}" 2>/dev/null || true)
    done
    if [ "${IS_WINDOWS}" = 1 ] && [ -n "${LOCALAPPDATA:-}" ]; then
        root="$(cygpath -u "${LOCALAPPDATA}")"
        for path in "${root}"/Python/pythoncore-3.*-64/python.exe "${root}"/Programs/Python/Python3*/python.exe; do
            probe_python "${path}" && return 0
        done
    fi
    return 1
}

install_venv() {
    if venv_ready; then
        note "框架 venv（tomli + graphifyy ${GRAPHIFY_VERSION}）已就绪，跳过"
        return 0
    fi
    local venv="${TOOLS}/venv" pyexe
    if ! venv_packages_ready; then
        if ! pyexe="$(venv_python)" || ! probe_python "${pyexe}" >/dev/null; then
            pyexe="$(find_real_python)" \
                || { echo "[setup-env] 没有可运行的 Python>=3.10，无法建框架 venv" >&2; return 1; }
            if command -v uv >/dev/null 2>&1; then
                uv venv --python "${pyexe}" "${venv}" >/dev/null
            else
                "${pyexe}" -m venv "${venv}"
            fi
        fi
        if command -v uv >/dev/null 2>&1; then
            uv pip install --python "$(venv_python)" "graphifyy==${GRAPHIFY_VERSION}" tomli >/dev/null
        else
            "$(venv_python)" -m pip install --quiet "graphifyy==${GRAPHIFY_VERSION}" tomli
        fi
    fi
    if [ "${IS_WINDOWS}" = 1 ]; then
        cp -f "${venv}/Scripts/python.exe" "${venv}/Scripts/python3.exe"
        mkdir -p "${venv}/bin"
        printf '#!/usr/bin/env bash\nexec "$(cd "$(dirname "${BASH_SOURCE[0]}")/../Scripts" && pwd)/graphify.exe" "$@"\n' \
            > "${venv}/bin/graphify"
        chmod +x "${venv}/bin/graphify"
    fi
    venv_ready || { echo "[setup-env] 框架 venv 校验失败" >&2; return 1; }
    ok "框架 venv -> ${venv}（tomli + graphifyy ${GRAPHIFY_VERSION}）"
}

main() {
    export PATH="${TOOLS}/make/bin:${TOOLS}/venv/Scripts:${TOOLS}/venv/bin:${TOOLS}/nextest/bin:${TOOLS}/stylua/bin:${TOOLS}/lua/bin:${TOOLS}/nasm/bin:${TOOLS}/perl/perl/bin:${PATH}"
    export PYTHONDONTWRITEBYTECODE=1
    if [ "$MODE" = "check" ]; then
        check_all
        return
    fi
    mkdir -p "${ROOT}/.local/tmp" "${ROOT}/.local/cache"
    export TMPDIR="${ROOT}/.local/tmp"
    export TMP="${TMPDIR}" TEMP="${TMPDIR}"
    export PIP_CACHE_DIR="${ROOT}/.local/cache/pip" UV_CACHE_DIR="${ROOT}/.local/cache/uv"
    if [ "${IS_WINDOWS}" = 1 ]; then
        export TMP="$(cygpath -m "${TMPDIR}")" TEMP="$(cygpath -m "${TMPDIR}")"
        export PIP_CACHE_DIR="$(cygpath -m "${PIP_CACHE_DIR}")" UV_CACHE_DIR="$(cygpath -m "${UV_CACHE_DIR}")"
    fi
    need_cmd cargo "安装 rustup；引导工具不入本仓"
    need_python3
    [ "$fail" -ne 0 ] && { echo "[setup-env] 必备引导工具缺失，先按提示安装" >&2; return 1; }
    install_nextest
    install_venv
    install_stylua
    install_nasm
    install_perl
    install_make
    install_lua
    check_all
    echo "[setup-env] 完成：make test / make graph / make generated-check / make framework-check 现在使用项目钉版工具"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    main
fi
