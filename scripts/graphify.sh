#!/usr/bin/env bash
# wezterm Graphify 固定入口：只索引产品 Rust 源码（.graphifyignore 排除
# 文档/资源/vendored/生成数据表），代码-only AST 抽取，无需任何 API key。
# 子命令：rebuild|check|query|path|explain
set -euo pipefail

WRAPPER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${WRAPPER_DIR}/.." && pwd)"
GRAPH_DIR="${ROOT}/graphify-out"
GRAPH_JSON="${GRAPH_DIR}/graph.json"
REPORT="${GRAPH_DIR}/GRAPH_REPORT.md"
PINNED_VERSION="0.9.20"
# 输出目录由项目控制，避免继承外部 GRAPHIFY_OUT。
export GRAPHIFY_OUT=graphify-out

die() {
    echo "[graphify] ERROR: $*" >&2
    exit 1
}

resolve_cli() {
    local configured="${WEZTERM_GRAPHIFY_CLI:-}"
    local cli
    if [ -n "${configured}" ]; then
        if [[ "${configured}" == */* ]]; then
            [ -x "${configured}" ] || die "WEZTERM_GRAPHIFY_CLI 不可执行：${configured}"
            cli="${configured}"
        else
            cli="$(command -v "${configured}" 2>/dev/null)" || die "找不到 WEZTERM_GRAPHIFY_CLI：${configured}"
        fi
    elif [ -x "${ROOT}/.local/tools/venv/bin/graphify" ]; then
        cli="${ROOT}/.local/tools/venv/bin/graphify"
    else
        cli="$(command -v graphify 2>/dev/null)" \
            || die "未安装 graphify；请运行 make setup（钉版装入 .local/tools/venv）"
    fi
    if [ "${WEZTERM_GRAPHIFY_ALLOW_ANY_VERSION:-0}" != "1" ]; then
        local version
        version="$("${cli}" --version 2>/dev/null | awk '{print $2}')"
        [ "${version}" = "${PINNED_VERSION}" ] \
            || die "graphify 版本 ${version:-unknown} != 固定 ${PINNED_VERSION}；升级前先在临时副本比对节点稳定性（或设 WEZTERM_GRAPHIFY_ALLOW_ANY_VERSION=1 明确放行）"
    fi
    printf '%s\n' "${cli}"
}

reject_graph_override() {
    local arg
    for arg in "$@"; do
        case "${arg}" in
            --graph|--graph=*) die "禁止覆盖项目固定图谱：${GRAPH_JSON}" ;;
        esac
    done
}

rebuild() {
    local cli="$1"
    # 固定 hash seed，避免社区划分与报告随解释器随机漂移。
    export PYTHONHASHSEED=0
    cd "${ROOT}"
    # 排除规则在 .graphifyignore（仓库根=扫描根）。
    "${cli}" extract . --out . --force --code-only --no-cluster --max-workers 4
    [ -f "${GRAPH_JSON}" ] || die "抽取未生成图谱：${GRAPH_JSON}"
    "${cli}" cluster-only . --graph "${GRAPH_JSON}" --no-viz --no-label
    [ -f "${REPORT}" ] || die "聚类未生成报告：${REPORT}"
    python3 "${WRAPPER_DIR}/graphify_fingerprint.py" write
    python3 "${WRAPPER_DIR}/graphify_fingerprint.py" check
    echo "[graphify] 图谱已重建 -> graphify-out/graph.json（排除表 .graphifyignore；指纹已更新）"
}

command_name="${1:-}"
case "${command_name}" in
    rebuild)
        [ "$#" -eq 1 ] || die "rebuild 不接受额外参数"
        rebuild "$(resolve_cli)"
        ;;
    check)
        [ "$#" -eq 1 ] || die "check 不接受额外参数"
        cd "${ROOT}"
        python3 "${WRAPPER_DIR}/graphify_fingerprint.py" check
        echo "[graphify] 源码指纹与图谱产物一致"
        ;;
    query|path|explain)
        [ -f "${GRAPH_JSON}" ] || die "图谱尚未构建，请先运行 make graph"
        shift
        reject_graph_override "$@"
        cli="$(resolve_cli)"
        cd "${ROOT}"
        exec "${cli}" "${command_name}" "$@" --graph "${GRAPH_JSON}"
        ;;
    extract|update|cluster-only|label|merge-graphs|watch|install|uninstall|add|clone|global*)
        die "禁止直接执行 ${command_name}；本项目固定使用 make graph 全量重建"
        ;;
    ""|help|-h|--help)
        echo "用法：scripts/graphify.sh rebuild|check|query|path|explain ..."
        ;;
    *)
        die "不支持的命令：${command_name}"
        ;;
esac
