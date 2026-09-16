#!/usr/bin/env bash
# UI 冒烟截图：隔离 Xvfb 显示上启动 wezterm-gui，xwd 抓屏并转 PNG。
# 只杀自己启动的进程；显示号被占用即失败（不抢已有显示）。
#
# 用法：scripts/ui_smoke.sh [--out DIR]
#   DIR 缺省 .ui-evidence/smoke/<UTC时间戳>/；建议先 make evidence TASK=<任务>
#   分配批次目录后传 --out（result.json 由两者之一写入，内容如实）。
#
# 前置：target/debug/wezterm-gui（cargo build -p wezterm-gui）、Xvfb、xwd、ffmpeg。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GUI="${ROOT}/target/debug/wezterm-gui"
EVIDENCE="${ROOT}/.ui-evidence/smoke/$(date -u +%Y%m%dT%H%M%SZ)-$$"
if [ "${1:-}" = "--out" ] && [ -n "${2:-}" ]; then
    EVIDENCE="${ROOT}/${2#/}"
fi

fail() { echo "[ui-smoke] ERROR: $*" >&2; exit 1; }
note() { printf '[ui-smoke] %s\n' "$*"; }

[ -x "${GUI}" ] || fail "缺少 ${GUI}；先运行 cargo build -p wezterm-gui（若报 x11/wayland 缺库，先 sudo ./get-deps 装系统依赖，或 --no-default-features 关 wayland）"
command -v Xvfb >/dev/null 2>&1 || fail "缺少 Xvfb；系统包管理器安装（debian: xvfb）"
command -v xwd >/dev/null 2>&1 || fail "缺少 xwd；系统包管理器安装（debian: x11-apps）"
command -v ffmpeg >/dev/null 2>&1 || fail "缺少 ffmpeg（xwd->PNG 转换）；系统包管理器安装"

XVFB_PID=""
GUI_PID=""
cleanup() {
    [ -n "${GUI_PID}" ] && kill "${GUI_PID}" 2>/dev/null || true
    [ -n "${XVFB_PID}" ] && kill "${XVFB_PID}" 2>/dev/null || true
    wait 2>/dev/null || true
}
trap cleanup EXIT

# 选一个未被占用的显示号（socket 不存在 + Xvfb 能活 1s 即认为可用）。
pick_display() {
    local candidate
    for candidate in $((RANDOM % 40 + 60)) $((RANDOM % 40 + 110)) $((RANDOM % 40 + 160)); do
        [ -e "/tmp/.X11-unix/X${candidate}" ] && continue
        echo "${candidate}"
        return 0
    done
    fail "未能分配空闲显示号"
}

DISPLAY_NUM="$(pick_display)"
export DISPLAY=":${DISPLAY_NUM}"
note "启动 Xvfb on ${DISPLAY} (1280x800x24)"
Xvfb "${DISPLAY}" -screen 0 1280x800x24 -nolisten tcp &
XVFB_PID=$!
sleep 1
kill -0 "${XVFB_PID}" 2>/dev/null || fail "Xvfb 启动即退出（显示号 ${DISPLAY_NUM} 可能被占用）"

MARKER="WEZTERM-UI-SMOKE-OK-1234567890-终端冒烟"
mkdir -p "${EVIDENCE}"
note "启动 wezterm-gui（证据目录 ${EVIDENCE#$ROOT/}）"
"${GUI}" start --always-new-process --class wezterm-smoke \
    --config 'font_size=14;warn_about_missing_glyphs=false' \
    -- /bin/sh -c "printf '%s\n\n%s\n' '${MARKER}' '${MARKER}'; sleep 600" &
GUI_PID=$!

sleep 8
kill -0 "${GUI_PID}" 2>/dev/null || fail "wezterm-gui 提前退出（检查 Xvfb/字体/配置）"

xwd -root -silent > "${EVIDENCE}/screen.xwd"
ffmpeg -hide_banner -loglevel error -y -i "${EVIDENCE}/screen.xwd" "${EVIDENCE}/before.png"
rm -f "${EVIDENCE}/screen.xwd"
[ -s "${EVIDENCE}/before.png" ] || fail "截图为空"

COMMIT="$(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo no-git)"
cat > "${EVIDENCE}/result.json" <<EOF
{
  "status": "CAPTURED",
  "task": "ui-smoke",
  "branch": "$(git -C "${ROOT}" branch --show-current 2>/dev/null || echo no-git)",
  "commit": "${COMMIT}",
  "display": "${DISPLAY}",
  "command": "scripts/ui_smoke.sh",
  "screenshots": ["before.png"],
  "images_reviewed": false,
  "note": "截图已落盘但尚未读回；执行者必须读回 before.png 核对 '${MARKER}' 可见后将 images_reviewed 置 true 并把 status 改 PASS/FAIL"
}
EOF
note "完成：${EVIDENCE#$ROOT/}/before.png —— 必须读回图片核对标记文本后再判定通过"
