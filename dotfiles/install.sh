#!/usr/bin/env bash
# wezterm-gx installer (Linux, user-level, no sudo required).
#
# Deploys the gx fork build plus the machine snapshot stored in dotfiles/:
# binaries -> ~/.local/opt/wezterm-gx/<version>-<binhash>/ + wrapper ~/.local/bin/wezterm
# config   -> ~/.config/wezterm/                   (existing copy is backed up)
# plugins  -> ~/.local/share/wezterm/plugins/<escaped>/
# fonts    -> ~/.local/share/fonts/wezterm-gx/     + fc-cache
# desktop  -> ~/.local/share/applications/org.wezfurlong.wezterm.desktop
# zshrc    -> 默认不动 ~/.zshrc（cursor-mode 块归 oh-my-zsh gx 层）；
#             --zshrc 显式追加（无 gx 层的机器）；检测到 oh-my-zsh gx 层
#             接管时清理历史追加块
#
# Usage:
#   ./install.sh [--check] [--from-build DIR] [--bundle-root DIR]
#                [--im fcitx|ibus|none] [--no-desktop] [--zshrc]
#                [--no-fonts] [--force]
#
# --check       dry-run: run all preflight checks and print the plan, write nothing.
# --from-build  take the four binaries from a cargo build dir (e.g. target/release)
#               instead of <bundle-root>/bin/.
# --bundle-root dir that contains bin/, dotfiles/, manifest.env (default: script dir).
# --im          input-method env rendered into the desktop entry (default fcitx,
#               matching xim_im_name='fcitx' in config/general.lua).
# --zshrc       append the cursor-mode keybinding block to ~/.zshrc (default:
#               leave ~/.zshrc alone; the block's true source is the oh-my-zsh
#               gx layer).

set -euo pipefail

# 配置换入用的暂存目录（见 config 安装步骤）；异常退出时由 trap 清理
STAGED=""
cleanup_staged() {
   if [ -n "$STAGED" ] && [ -d "$STAGED" ]; then rm -rf "$STAGED"; fi
}
trap cleanup_staged EXIT

CHECK=0
FROM_BUILD=""
IM="fcitx"
WANT_DESKTOP=1
WANT_ZSHRC=0
WANT_FONTS=1
FORCE=0

while [ $# -gt 0 ]; do
   case "$1" in
      --check) CHECK=1 ;;
      --from-build) FROM_BUILD="${2:?--from-build needs a dir}"; shift ;;
      --bundle-root) BUNDLE_ROOT="${2:?--bundle-root needs a dir}"; shift ;;
      --im) IM="${2:?--im needs fcitx|ibus|none}"; shift ;;
      --no-desktop) WANT_DESKTOP=0 ;;
      --zshrc) WANT_ZSHRC=1 ;;
      --no-zshrc) WANT_ZSHRC=0 ;;
      --no-fonts) WANT_FONTS=0 ;;
      --force) FORCE=1 ;;
      -h|--help) sed -n '2,26p' "$0"; exit 0 ;;
      *) echo "install.sh: unknown option: $1" >&2; exit 2 ;;
   esac
   shift
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${BUNDLE_ROOT:-$SCRIPT_DIR}"
DOTFILES="$ROOT/dotfiles"
if [ ! -d "$DOTFILES" ]; then
   # tolerate being invoked from inside dotfiles/ itself
   if [ -d "$ROOT/wezterm-config" ]; then DOTFILES="$ROOT"; fi
fi
[ -d "$DOTFILES" ] || { echo "install.sh: dotfiles/ not found under $ROOT" >&2; exit 2; }

if [ -n "$FROM_BUILD" ]; then
   BIN_SRC="$FROM_BUILD"
else
   BIN_SRC="$ROOT/bin"
fi

log()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mWARN:\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31mERROR:\033[0m %s\n' "$*" >&2; exit 1; }
plan() { if [ "$CHECK" = 1 ]; then printf '  [plan] %s\n' "$*"; else log "$*"; fi; }

# ---------------------------------------------------------------- preflight --
command -v ldd >/dev/null || die "ldd not found; cannot verify binary compatibility"
ARCH="$(uname -m)"
[ "$ARCH" = "x86_64" ] || [ "$FORCE" = 1 ] || die "arch is $ARCH, binaries are x86_64 (use --force to override)"

GLIBC_MIN="2.31"
if [ -f "$ROOT/manifest.env" ]; then
   # shellcheck source=/dev/null
   . "$ROOT/manifest.env"
fi
# 注意：pipefail 下不要用 `cmd | head -n1`——head 早退会让上游吃 SIGPIPE(141)，
# 先整段落盘再取行。
GLIBC_FULL="$(ldd --version 2>/dev/null || true)"
GLIBC_NOW="$(printf '%s\n' "$GLIBC_FULL" | grep -oE '[0-9]+\.[0-9]+' | tail -1 || true)"
if [ -n "${GLIBC_NOW:-}" ]; then
   LOWEST="$(printf '%s\n%s\n' "$GLIBC_MIN" "$GLIBC_NOW" | sort -V | sed -n '1p')"
   if [ "$LOWEST" != "$GLIBC_MIN" ] && [ "$FORCE" = 0 ]; then
      die "system glibc $GLIBC_NOW < required $GLIBC_MIN; build from source instead: git clone + make gx-install (or --force at your own risk)"
   fi
else
   warn "cannot detect glibc version, skipping compatibility check"
fi

if [ -n "$FROM_BUILD" ]; then
   [ -x "$BIN_SRC/wezterm-gui" ] || die "$BIN_SRC/wezterm-gui not found (run the build first)"
fi
for b in wezterm wezterm-gui wezterm-mux-server strip-ansi-escapes; do
   [ -e "$BIN_SRC/$b" ] || die "missing binary: $BIN_SRC/$b"
done
GUI_BIN="$BIN_SRC/wezterm-gui"
MISSING_LIBS="$(ldd "$GUI_BIN" 2>/dev/null | grep 'not found' || true)"
if [ -n "$MISSING_LIBS" ]; then
   warn "shared libraries missing on this system (wezterm-gui may fail to start):"
   printf '%s\n' "$MISSING_LIBS" | sed 's/^/    /'
   cat >&2 <<'EOF'
    hint (Ubuntu): sudo apt install libx11-6 libxcb1 libxkbcommon0 \
      libxkbcommon-x11-0 libwayland-client0 libegl1 libgl1 libfontconfig1 \
      libfreetype6 libharfbuzz0b
EOF
   [ "$FORCE" = 1 ] || die "aborting; install the libs above or rerun with --force"
fi

VERSION="$("${BIN_SRC}/wezterm" --version 2>/dev/null || true)"
VERSION="${VERSION%%$'\n'*}"
VERSION="${VERSION#wezterm }"
[ -n "$VERSION" ] || VERSION="dev"

# fork: 四个二进制必须自报同一版本串——同目录混版构建曾让 GUI 停在旧版
# 而 CLI 已更新（WEZ-BUILD-01）；占位串（WEZ-BUILD-02 修复前）也会在此
# 被拦下。strip-ansi-escapes 的 --version 由 fork 补加。
version_string_of() {
   local out
   out="$("$1" --version 2>/dev/null || true)"
   out="${out%%$'\n'*}"
   printf '%s' "${out##* }"
}
VERSION_REF="$(version_string_of "$BIN_SRC/wezterm")"
for b in wezterm-gui wezterm-mux-server strip-ansi-escapes; do
   V_OTHER="$(version_string_of "$BIN_SRC/$b")"
   if [ "$V_OTHER" != "$VERSION_REF" ]; then
      die "version mismatch: wezterm=$VERSION_REF but $b=$V_OTHER; rebuild all four binaries (make build BUILD_OPTS=--release)"
   fi
done

# fork: 目录名带四二进制联合内容哈希——同一 commit 的脏树/异 feature
# 重构建不会静默覆盖同名目录，回滚目标始终可分辨（WEZ-BUILD-01）。
# 同名即同内容，幂等重装天然安全。
BIN_HASH="$(sha256sum "$BIN_SRC/wezterm" "$BIN_SRC/wezterm-gui" "$BIN_SRC/wezterm-mux-server" "$BIN_SRC/strip-ansi-escapes" | sha256sum | cut -c1-12)"
VERSION_DIR="$(printf '%s' "$VERSION" | tr '/: ' '___')-$BIN_HASH"
OPT_DIR="$HOME/.local/opt/wezterm-gx/$VERSION_DIR"
BIN_DIR="$OPT_DIR/bin"
case "$HOME" in
   *" "*) warn "HOME contains spaces; desktop entry may be broken" ;;
esac
if [ "$(id -u)" = 0 ]; then
   warn "running as root: everything installs under $HOME only"
fi
if [ -e "$HOME/.wezterm.lua" ]; then
   # shellcheck disable=SC2088 # 提示文案故意原样显示 ~
   warn "~/.wezterm.lua exists and takes precedence over ~/.config/wezterm/wezterm.lua"
fi

case "$IM" in
   # fork（WEZ-CFG-06）：wezterm 走 XIM（xim_im_name），GTK/QT_IM_MODULE
   # 对它无效却经环境继承泄进每个 pane 子进程；只注入 XMODIFIERS。
   fcitx) IM_ENV="/usr/bin/env XMODIFIERS=@im=fcitx " ;;
   ibus)  IM_ENV="/usr/bin/env XMODIFIERS=@im=ibus " ;;
   none)  IM_ENV="" ;;
   *) die "--im must be fcitx|ibus|none" ;;
esac

backup_existing() {
   [ ! -e "$1" ] && return 0
   local bk
   bk="$1.bak-gx-$(date +%Y%m%d-%H%M%S)"
   plan "backup $1 -> $bk"
   [ "$CHECK" = 1 ] && return 0
   cp -a "$1" "$bk"
}

# ------------------------------------------------------------------- install --
TOTAL=8
STEP=0
step() { STEP=$((STEP + 1)); plan "[$STEP/$TOTAL] $*"; }

step "binaries -> $BIN_DIR (from $BIN_SRC)"
if [ "$CHECK" = 0 ]; then
   mkdir -p "$BIN_DIR"
   for b in wezterm wezterm-gui wezterm-mux-server strip-ansi-escapes; do
      install -m 0755 "$BIN_SRC/$b" "$BIN_DIR/$b"
   done
   # fork: 正式生成 .gx-managed 元数据（WEZ-BUILD-01/WEZ-CFG-02）。
   # 源 commit：源码安装取 git HEAD，离线包取 manifest.env 的 COMMIT。
   SRC_COMMIT="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
   SRC_COMMIT="${SRC_COMMIT:-${COMMIT:-unknown}}"
   cat > "$OPT_DIR/.gx-managed" <<EOF
version=$VERSION
installed_at=$(date -Iseconds)
source_commit=$SRC_COMMIT
bin_hash=$BIN_HASH
EOF
fi

step "wrapper -> ~/.local/bin/wezterm"
if [ "$CHECK" = 0 ]; then
   mkdir -p "$HOME/.local/bin"
   WRAPPER="$HOME/.local/bin/wezterm"
   backup_existing "$WRAPPER"
   sed "s|__BIN_DIR__|$BIN_DIR|g" "$DOTFILES/templates/wezterm-wrapper.sh.template" > "$WRAPPER"
   chmod 0755 "$WRAPPER"
fi

step "config -> ~/.config/wezterm (from $DOTFILES/wezterm-config)"
CFG="$HOME/.config/wezterm"
if [ "$CHECK" = 1 ]; then
   if [ -e "$CFG" ]; then plan "backup $CFG -> $CFG.bak-gx-<ts> (rename 换入)"; fi
else
   # 先整树拷到暂存目录，再连续 rename 换入：若先 rm 旧目录再直接 cp，
   # 空窗期内启动的 wezterm 会报 wezterm.lua 缺失（19MB 快照拷贝需秒级）。
   STAGED="$HOME/.config/.wezterm.gx-new.$$"
   BK=""
   if [ -e "$CFG" ]; then BK="$CFG.bak-gx-$(date +%Y%m%d-%H%M%S)"; fi
   if [ -n "$BK" ]; then plan "backup $CFG -> $BK"; fi
   mkdir -p "$HOME/.config"
   rm -rf "$STAGED"
   cp -a "$DOTFILES/wezterm-config" "$STAGED"
   if [ -n "$BK" ]; then mv "$CFG" "$BK"; fi
   mv "$STAGED" "$CFG"
   STAGED=""
fi

step "plugins -> ~/.local/share/wezterm/plugins/"
if [ "$CHECK" = 0 ]; then
   PLUG_DEST="$HOME/.local/share/wezterm/plugins"
   mkdir -p "$PLUG_DEST"
   for p in "$DOTFILES"/plugins/*/; do
      [ -d "$p" ] || continue
      name="$(basename "$p")"
      mkdir -p "$PLUG_DEST/$name"
      cp -a "$p." "$PLUG_DEST/$name/"
      # wezterm.plugin.list() 需要把每个插件当 git 仓库打开（含 remote），
      # 快照把 .git 存成 gitdir/ 以免嵌套仓库，装完还原
      if [ -d "$PLUG_DEST/$name/gitdir" ]; then
         rm -rf "$PLUG_DEST/$name/.git"
         mv "$PLUG_DEST/$name/gitdir" "$PLUG_DEST/$name/.git"
      fi
   done
fi

if [ "$WANT_FONTS" = 1 ]; then
   step "fonts -> ~/.local/share/fonts/wezterm-gx/"
   if [ "$CHECK" = 0 ]; then
      FONT_DIR="$HOME/.local/share/fonts/wezterm-gx"
      mkdir -p "$FONT_DIR"
      cp -a "$DOTFILES"/fonts/*.ttf "$DOTFILES"/fonts/*.ttc "$FONT_DIR/" 2>/dev/null || true
      if command -v fc-cache >/dev/null; then
         fc-cache -f "$FONT_DIR" >/dev/null 2>&1 || warn "fc-cache failed"
      else
         warn "fc-cache not found; fonts activate after fontconfig is available"
      fi
   fi
else
   step "fonts skipped (--no-fonts)"
fi

if [ "$WANT_DESKTOP" = 1 ]; then
   step "desktop entry -> ~/.local/share/applications/org.wezfurlong.wezterm.desktop (im=$IM)"
   if [ "$CHECK" = 0 ]; then
      APPS="$HOME/.local/share/applications"
      mkdir -p "$APPS"
      ICON_DIR="$HOME/.local/share/icons/wezterm-gx"
      mkdir -p "$ICON_DIR"
      install -m 0644 "$DOTFILES/assets/org.wezfurlong.wezterm.png" "$ICON_DIR/org.wezfurlong.wezterm.png"
      ENTRY="$APPS/org.wezfurlong.wezterm.desktop"
      backup_existing "$ENTRY"
      sed -e "s|__ICON__|$ICON_DIR/org.wezfurlong.wezterm.png|g" \
          -e "s|__TRYEXEC__|$BIN_DIR/wezterm-gui|g" \
          -e "s|__EXEC__|$IM_ENV$BIN_DIR/wezterm-gui start --cwd .|g" \
          "$DOTFILES/templates/org.wezfurlong.wezterm.desktop.template" > "$ENTRY"
      if command -v update-desktop-database >/dev/null; then
         update-desktop-database "$APPS" || true
      fi
   fi
else
   step "desktop entry skipped (--no-desktop)"
fi

ZSHRC="$HOME/.zshrc"
# fork: ~/.zshrc 的真源归 oh-my-zsh gx 层（2026-09-21 拍板）。检测到
# gx 层接管（~/.oh-my-zsh/.gx-managed）时清理本安装器历史上追加的
# cursor-mode 标记块；该块内容已并入 oh-my-zsh gx/config/zshrc。
if [ -f "$ZSHRC" ] && grep -q '>>> wezterm-gx >>>' "$ZSHRC" 2>/dev/null \
   && [ -e "$HOME/.oh-my-zsh/.gx-managed" ]; then
   plan "zshrc: remove legacy cursor-mode block (owned by oh-my-zsh gx layer now)"
   if [ "$CHECK" = 0 ]; then
      backup_existing "$ZSHRC"
      sed -i '/# >>> wezterm-gx >>>/,/# <<< wezterm-gx <<</d' "$ZSHRC"
   fi
fi
if [ "$WANT_ZSHRC" = 1 ]; then
   if [ ! -f "$ZSHRC" ] || ! grep -q '>>> wezterm-gx >>>' "$ZSHRC" 2>/dev/null; then
      step "zshrc cursor-mode keybinding block -> $ZSHRC"
      if [ "$CHECK" = 0 ]; then
         cat "$DOTFILES/templates/zshrc-wezterm.sh" >> "$ZSHRC" 2>/dev/null \
            || { touch "$ZSHRC"; cat "$DOTFILES/templates/zshrc-wezterm.sh" >> "$ZSHRC"; }
      fi
   else
      step "zshrc block already present, skip"
   fi
else
   step "zshrc untouched (default; use --zshrc on machines without the oh-my-zsh gx layer)"
fi

# fork: 回收旧版本目录与备份（WEZ-HYG-02）。VERSION_DIR 带内容哈希后每次
# 新构建都产生新目录，不回收会无限累积（真机曾 1.1GB/7 份）。
# 保留：当前版本 + 最新 2 个旧版本；.bak-gx-* 备份保留最新 3 份。
prune_old() {
   # $1=glob 目录前缀, $2=keep 数量, $3=说明
   local parent="$1" keep="$2" what="$3" entry
   [ -d "$parent" ] || return 0
   local -a entries=()
   while IFS= read -r entry; do entries+=("$entry"); done < <(ls -1dt "$parent"/*/ 2>/dev/null)
   local i
   for i in "${!entries[@]}"; do
      [ "$i" -lt "$keep" ] && continue
      # 永不删除当前版本目录
      [ "${entries[$i]%/}" = "$OPT_DIR" ] && continue
      plan "prune $what: ${entries[$i]}"
      [ "$CHECK" = 0 ] && rm -rf "${entries[$i]}"
   done
}
prune_backups() {
   # $1=glob 模式（文件）, $2=keep, $3=说明
   local pattern="$1" keep="$2" what="$3"
   local -a entries=()
   local entry
   # shellcheck disable=SC2086 # glob 必须在此处展开
   while IFS= read -r entry; do entries+=("$entry"); done < <(ls -1dt $pattern 2>/dev/null)
   local i
   for i in "${!entries[@]}"; do
      [ "$i" -lt "$keep" ] && continue
      plan "prune $what: ${entries[$i]}"
      [ "$CHECK" = 0 ] && rm -rf "${entries[$i]}"
   done
}
step "prune old versions/backups (WEZ-HYG-02)"
prune_old "$HOME/.local/opt/wezterm-gx" 3 "old version dir"
prune_backups "$HOME/.config/wezterm.bak-gx-*" 3 "config backup"
prune_backups "$HOME/.local/bin/wezterm.bak-gx-*" 3 "wrapper backup"
prune_backups "$HOME/.local/share/applications/org.wezfurlong.wezterm.desktop.bak-gx-*" 3 "desktop entry backup"

echo
if [ "$CHECK" = 1 ]; then
   log "check mode: all preflight checks passed, nothing was written"
else
   log "installed wezterm-gx $VERSION"
   cat <<EOF

Next steps:
  - ensure ~/.local/bin is on PATH (relogin or: export PATH="\$HOME/.local/bin:\$PATH")
  - verify: wezterm --version && wezterm ls-fonts | head
  - config lives at ~/.config/wezterm; older installs kept under ~/.local/opt/wezterm-gx/
EOF
fi
