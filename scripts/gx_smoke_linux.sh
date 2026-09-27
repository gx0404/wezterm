#!/usr/bin/env bash
# System package lifecycle smoke; intended for a disposable Ubuntu runner.
set -euo pipefail
if [ "${GITHUB_ACTIONS:-}" != true ] && [ "${1:-}" != --allow-system-install ]; then
    echo 'Use a disposable runner, or explicitly pass --allow-system-install.' >&2
    exit 2
fi
if dpkg-query -W -f='${db:Status-Status}' wezterm-gx 2>/dev/null | grep -qx installed; then
    echo 'Refusing to replace an existing wezterm-gx installation in a lifecycle test.' >&2
    exit 2
fi
repo="$(cd "$(dirname "$0")/.." && pwd)"
privileged=(sudo)
as_user=()
if [ "$(id -u)" -eq 0 ]; then
    : "${GX_SMOKE_USER:?Set GX_SMOKE_USER to a non-root test account}"
    [ "$(id -u "$GX_SMOKE_USER")" -ne 0 ]
    privileged=()
    as_user=(runuser -u "$GX_SMOKE_USER" --)
fi
packages=("$repo"/dist/wezterm-gx_*_amd64.deb)
[ "${#packages[@]}" -eq 1 ] && [ -f "${packages[0]}" ]
"${privileged[@]}" apt-get install -y "${packages[0]}"
test_home="$(mktemp -d)"
evidence="$repo/.ui-evidence/gx-package-linux/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$evidence"
cat /etc/os-release > "$evidence/os-release"
sha256sum "${packages[0]}" > "$evidence/package.sha256"
dpkg-query -W -f='${Package} ${Version}\n' wezterm-gx libc6 > "$evidence/installed-packages.txt"
if [ "${#as_user[@]}" -gt 0 ]; then chown "$GX_SMOKE_USER" "$test_home" "$evidence"; fi
trap '"${privileged[@]}" apt-get remove -y wezterm-gx' EXIT
run_user() {
    "${as_user[@]}" env HOME="$test_home" XDG_CONFIG_HOME="$test_home/.config" XDG_DATA_HOME="$test_home/.local/share" "$@"
}
run_user wezterm-gx --gx-initialize-only
test -f "$test_home/.config/wezterm/wezterm.lua"
test "$(find "$test_home/.local/share/wezterm/plugins" -name HEAD -path '*/.git/HEAD' | wc -l)" -eq 4
run_user wezterm-gx ls-fonts > "$evidence/fonts.log" 2>&1
if grep -Eq 'plugin load failed|Error loading configuration' "$evidence/fonts.log"; then
    cat "$evidence/fonts.log" >&2
    exit 1
fi
grep -q '/usr/share/fonts/truetype/wezterm-gx/JetBrainsMonoNerdFont-Regular.ttf' "$evidence/fonts.log"
grep -q '/usr/share/fonts/truetype/wezterm-gx/NotoSansCJK-Regular.ttc' "$evidence/fonts.log"
state="$test_home/.local/share/wezterm/plugins/httpssCssZssZsgithubsDscomsZsMLFlexersZsresurrectsDswezterm/state/workspace/gx-preserve.json"
run_user sh -c 'printf "%s\n" "{\"preserve\":true}" > "$1"' sh "$state"
printf '\n-- gx-upgrade-preserve-marker\n' >> "$test_home/.config/wezterm/wezterm.lua"
run_user sh -c 'printf "%s\n" "{\"wallpaper\":\"user wallpaper.png\"}" > "$HOME/.config/wezterm/gui-settings.json"
    mkdir -p "$XDG_DATA_HOME/wezterm/plugins/user-plugin/plugin"
    printf "return {}\n" > "$XDG_DATA_HOME/wezterm/plugins/user-plugin/plugin/init.lua"'
sha256sum "$test_home/.config/wezterm/wezterm.lua" "$state" \
    "$test_home/.config/wezterm/gui-settings.json" \
    "$test_home/.local/share/wezterm/plugins/user-plugin/plugin/init.lua" > "$evidence/user-data.sha256"
"${privileged[@]}" apt-get install -y --reinstall "${packages[0]}"
run_user sh -c 'printf "older\n" > "$HOME/.local/share/wezterm-gx/resource-version"'
run_user wezterm-gx --gx-initialize-only
grep -q gx-upgrade-preserve-marker "$test_home/.config/wezterm/wezterm.lua"
grep -q preserve "$state"
test -d "$test_home/.local/share/wezterm-gx/backups"
sha256sum -c "$evidence/user-data.sha256"
# Exercise the packaged default shell, including its normal startup files.
run_user tee "$test_home/.zshrc" >/dev/null <<'ZSH'
print 'GX package smoke - default zsh, fonts and plugins loaded'
print '中文字体验证'
print "zsh version: $ZSH_VERSION"
sleep 14
exit
ZSH
run_user timeout 45 xvfb-run -a bash -c '
    set -euo pipefail
    LIBGL_ALWAYS_SOFTWARE=1 wezterm-gx-gui --config front_end=\"OpenGL\" --config enable_wayland=false \
        start --always-new-process --no-auto-connect > "$1/gui.log" 2>&1 &
    gui_pid=$!
    trap "kill $gui_pid 2>/dev/null || true" EXIT
    sleep 7
    kill -0 "$gui_pid"
    xwd -root -silent > "$1/screen.xwd"
    ffmpeg -hide_banner -loglevel error -y -i "$1/screen.xwd" -frames:v 1 "$1/linux.png"
    wait "$gui_pid"
' bash "$evidence"
"${privileged[@]}" apt-get remove -y wezterm-gx
trap - EXIT
test -f "$test_home/.config/wezterm/wezterm.lua"
test -f "$state"
test ! -e /usr/bin/wezterm-gx
sha256sum -c "$evidence/user-data.sha256"
echo "PASS: deb installation, first launch, reinstallation, GUI and uninstall; test home: $test_home" | tee "$evidence/result.txt"
