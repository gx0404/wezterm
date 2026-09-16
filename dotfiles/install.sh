#!/usr/bin/env bash
# wezterm-gx installer (Linux, user-level, no sudo required).
#
# Deploys the gx fork build plus the machine snapshot stored in dotfiles/:
# binaries -> ~/.local/opt/wezterm-gx/<version>/   + wrapper ~/.local/bin/wezterm
# config   -> ~/.config/wezterm/                   (existing copy is backed up)
# plugins  -> ~/.local/share/wezterm/plugins/<escaped>/
# fonts    -> ~/.local/share/fonts/wezterm-gx/     + fc-cache
# desktop  -> ~/.local/share/applications/org.wezfurlong.wezterm.desktop
# zshrc    -> marked append block (cursor-mode keybindings)
#
# Usage:
#   ./install.sh [--check] [--from-build DIR] [--bundle-root DIR]
#                [--im fcitx|ibus|none] [--no-desktop] [--no-zshrc]
#                [--no-fonts] [--force]
#
# --check       dry-run: run all preflight checks and print the plan, write nothing.
# --from-build  take the four binaries from a cargo build dir (e.g. target/release)
#               instead of <bundle-root>/bin/.
# --bundle-root dir that contains bin/, dotfiles/, manifest.env (default: script dir).
# --im          input-method env rendered into the desktop entry (default fcitx,
#               matching xim_im_name='fcitx' in config/general.lua).

set -euo pipefail

CHECK=0
FROM_BUILD=""
IM="fcitx"
WANT_DESKTOP=1
WANT_ZSHRC=1
WANT_FONTS=1
FORCE=0

while [ $# -gt 0 ]; do
   case "$1" in
      --check) CHECK=1 ;;
      --from-build) FROM_BUILD="${2:?--from-build needs a dir}"; shift ;;
      --bundle-root) BUNDLE_ROOT="${2:?--bundle-root needs a dir}"; shift ;;
      --im) IM="${2:?--im needs fcitx|ibus|none}"; shift ;;
      --no-desktop) WANT_DESKTOP=0 ;;
      --no-zshrc) WANT_ZSHRC=0 ;;
      --no-fonts) WANT_FONTS=0 ;;
      --force) FORCE=1 ;;
      -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
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
plan() { [ "$CHECK" = 1 ] && printf '  [plan] %s\n' "$*" || log "$*"; }

# ---------------------------------------------------------------- preflight --
command -v ldd >/dev/null || die "ldd not found; cannot verify binary compatibility"
ARCH="$(uname -m)"
[ "$ARCH" = "x86_64" ] || [ "$FORCE" = 1 ] || die "arch is $ARCH, binaries are x86_64 (use --force to override)"

GLIBC_MIN="2.31"
if [ -f "$ROOT/manifest.env" ]; then
   # shellcheck disable=SC1091
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
VERSION_DIR="$(printf '%s' "$VERSION" | tr '/: ' '___')"
OPT_DIR="$HOME/.local/opt/wezterm-gx/$VERSION_DIR"
BIN_DIR="$OPT_DIR/bin"
case "$HOME" in
   *" "*) warn "HOME contains spaces; desktop entry may be broken" ;;
esac
if [ "$(id -u)" = 0 ]; then
   warn "running as root: everything installs under $HOME only"
fi
if [ -e "$HOME/.wezterm.lua" ]; then
   warn "~/.wezterm.lua exists and takes precedence over ~/.config/wezterm/wezterm.lua"
fi

case "$IM" in
   fcitx) IM_ENV="/usr/bin/env XMODIFIERS=@im=fcitx GTK_IM_MODULE=fcitx QT_IM_MODULE=fcitx " ;;
   ibus)  IM_ENV="/usr/bin/env XMODIFIERS=@im=ibus GTK_IM_MODULE=ibus QT_IM_MODULE=ibus " ;;
   none)  IM_ENV="" ;;
   *) die "--im must be fcitx|ibus|none" ;;
esac

backup_existing() {
   [ ! -e "$1" ] && return 0
   local bk="$1.bak-gx-$(date +%Y%m%d-%H%M%S)"
   plan "backup $1 -> $bk"
   [ "$CHECK" = 1 ] && return 0
   cp -a "$1" "$bk"
}

# ------------------------------------------------------------------- install --
TOTAL=7
STEP=0
step() { STEP=$((STEP + 1)); plan "[$STEP/$TOTAL] $*"; }

step "binaries -> $BIN_DIR (from $BIN_SRC)"
if [ "$CHECK" = 0 ]; then
   mkdir -p "$BIN_DIR"
   for b in wezterm wezterm-gui wezterm-mux-server strip-ansi-escapes; do
      install -m 0755 "$BIN_SRC/$b" "$BIN_DIR/$b"
   done
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
if [ "$CHECK" = 0 ]; then
   CFG="$HOME/.config/wezterm"
   mkdir -p "$HOME/.config"
   if [ -e "$CFG" ]; then
      backup_existing "$CFG"
      rm -rf "$CFG"
   fi
   cp -a "$DOTFILES/wezterm-config" "$CFG"
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
      command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" || true
   fi
else
   step "desktop entry skipped (--no-desktop)"
fi

if [ "$WANT_ZSHRC" = 1 ]; then
   ZSHRC="$HOME/.zshrc"
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
   step "zshrc skipped (--no-zshrc)"
fi

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
