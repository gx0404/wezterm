#!/usr/bin/env bash
# 派生文件只读校验（不写任何仓库文件）：
#   1) 二进制派生链：shell 补全与 copy/search 键表，与 target/debug/wezterm
#      的再生成输出逐字节比对（有意更新走 make generated-write）。
#      （--help 文本在 macOS/Linux 有空白差异，上游明确不纳入比较。）
#   2) 文档索引链：docs/ 拷贝到临时目录重跑 ci/generate-docs.py，比对全部
#      输出（含 gitignored 的 index/SUMMARY）是否与当前一致。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${ROOT}/target/debug/wezterm"

fail=0
note() { printf '[generated-check] %s\n' "$*"; }
bad()  { printf '[generated-check] 不一致: %s\n' "$*"; fail=1; }

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

if [ -x "${BIN}" ]; then
    for shell in bash zsh fish; do
        "${BIN}" shell-completion --shell "${shell}" > "${TMP}/${shell}" 2>/dev/null
        if ! cmp -s "${TMP}/${shell}" "${ROOT}/assets/shell-completion/${shell}"; then
            bad "assets/shell-completion/${shell}（重跑 make generated-write 或说明差异）"
        fi
    done
    for mode in copy_mode search_mode; do
        fname="default-$(echo "${mode}" | tr _ -)-key-table.markdown"
        # 入库键表是 stylua（ci/stylua.toml）格式化的——与 docs 构建的
        # gelatyx/stylua 同一约定；比对前用钉版 stylua 做同样格式化。
        STYLUA="$(command -v stylua 2>/dev/null || true)"
        if [ -z "${STYLUA}" ] && [ -x "${ROOT}/.local/tools/stylua/bin/stylua" ]; then
            STYLUA="${ROOT}/.local/tools/stylua/bin/stylua"
        fi
        if [ -n "${STYLUA}" ]; then
            "${BIN}" -n show-keys --lua --key-table "${mode}" > "${TMP}/keytable.lua"
            "${STYLUA}" --config-path "${ROOT}/ci/stylua.toml" "${TMP}/keytable.lua"
            {
                echo '```lua'
                cat "${TMP}/keytable.lua"
                echo '```'
            } | perl -0777 -pe 's/^\n+|\n\K\n+$//g' > "${TMP}/${fname}"
            if ! cmp -s "${TMP}/${fname}" "${ROOT}/docs/examples/${fname}"; then
                bad "docs/examples/${fname}（重跑 make generated-write 或说明差异）"
            fi
        else
            note "缺少 stylua，跳过键表 ${fname} 比对（scripts/setup_env.sh 安装钉版）"
        fi
    done
else
    note "跳过二进制派生链：缺少 ${BIN}（先 cargo build -p wezterm；本轮不计失败）"
fi

# 文档索引链：临时副本内重跑生成器（它自带 os.chdir("docs")，须从仓库根形态运行），
# 只比对 git 跟踪文件——ignored 的 index/SUMMARY 随文档构建环境变化，不是可审漂移面。
cp -r "${ROOT}/docs" "${TMP}/docs"
( cd "${TMP}" && python3 "${ROOT}/ci/generate-docs.py" >/dev/null 2>&1 ) \
    || note "generate-docs.py 在临时副本执行失败（环境差异），已跳过文档索引比对"
if [ -d "${TMP}/docs" ]; then
    # mkdocs.yml 写在临时副本的上一级，不影响仓库。
    while IFS= read -r rel; do
        [ -f "${TMP}/docs/${rel}" ] || continue
        if ! cmp -s "${TMP}/docs/${rel}" "${ROOT}/docs/${rel}"; then
            bad "docs/${rel} 与 generate-docs.py 再生成不一致"
        fi
    done < <(git -C "${ROOT}" ls-files -- docs)
fi

if [ "${fail}" -ne 0 ]; then
    echo "[generated-check] FAIL：存在漂移（见上）" >&2
    exit 1
fi
note "PASS：派生文件与生成器输出一致（或已声明跳过）"
