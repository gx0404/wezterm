#!/usr/bin/env bash
# make generated-write 的实现：重建派生文件。
#   1. 跑上游 ci/update-derived-files.sh（补全/键表/--help 概要）；
#   2. 对两张键表 markdown 按 stylua（ci/stylua.toml）格式化——与
#      generated_check 的比对路径和 docs 构建的 gelatyx/stylua 同一约定。
# 上游脚本本体不动（fork 治理：上游文件不做语义改动）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

STYLUA="$(command -v stylua 2>/dev/null || true)"
if [ -z "${STYLUA}" ] && [ -x "${ROOT}/.local/tools/stylua/bin/stylua" ]; then
    STYLUA="${ROOT}/.local/tools/stylua/bin/stylua"
fi
if [ -z "${STYLUA}" ]; then
    echo "[generated-write] 缺少 stylua（scripts/setup_env.sh 安装钉版）；" >&2
    echo "[generated-write] 键表将以原始输出写入，与入库约定不一致，中止。" >&2
    exit 1
fi

"${ROOT}/ci/update-derived-files.sh" "$@"

tmp="$(mktemp)"
trap 'rm -f "${tmp}"' EXIT
for f in \
    "${ROOT}/docs/examples/default-copy-mode-key-table.markdown" \
    "${ROOT}/docs/examples/default-search-mode-key-table.markdown"
do
    # 文件结构固定为 ```lua 围栏包一段 lua
    sed -n '2,$p' "${f}" | head -n -1 > "${tmp}"
    "${STYLUA}" --config-path "${ROOT}/ci/stylua.toml" "${tmp}"
    {
        echo '```lua'
        cat "${tmp}"
        echo '```'
    } > "${f}"
    echo "[generated-write] ${f#${ROOT}/}（stylua 格式化完成）"
done
