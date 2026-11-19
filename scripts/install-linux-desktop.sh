#!/usr/bin/env bash
# Registers a NovelForge AppImage as a normal desktop application.
# ---------------------------------------------------------------------------
# An AppImage is just a file: double-clicking it runs the app, but nothing adds
# it to the application launcher. This script copies the bundle into
# ~/.local/bin, extracts its icon, writes a .desktop entry and refreshes the
# desktop/icon caches, so NovelForge shows up beside every other app.
#
# Everything stays user-owned, no root required:
#   ~/.local/bin/novelforge.AppImage
#   ~/.local/bin/novelforge                  (symlink)
#   ~/.local/share/icons/hicolor/<size>/apps/novelforge.png
#   ~/.local/share/applications/novelforge.desktop
#
# Usage:
#   ./scripts/install-linux-desktop.sh [path/to/NovelForge_*.AppImage] [--launch]
#
# The path is optional: the script looks for an AppImage beside itself (the
# release ZIP layout), in ./src-tauri/target/release/bundle/appimage (the
# build-tree layout), and in the current directory. Re-running is safe and
# replaces the previous copy.
# ---------------------------------------------------------------------------
set -euo pipefail

LAUNCH=0
APPIMAGE=""

usage() {
  sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
}

for arg in "$@"; do
  case "$arg" in
    --launch) LAUNCH=1 ;;
    -h|--help) usage; exit 0 ;;
    -*) echo "Unknown option: $arg" >&2; usage >&2; exit 2 ;;
    *)
      if [ -n "$APPIMAGE" ]; then
        echo "Only one AppImage path may be given." >&2
        exit 2
      fi
      APPIMAGE="$arg"
      ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ -z "$APPIMAGE" ]; then
  for candidate in \
    "$SCRIPT_DIR"/NovelForge_*.AppImage \
    "$REPO_DIR"/NovelForge_*.AppImage \
    "$REPO_DIR"/src-tauri/target/release/bundle/appimage/NovelForge_*.AppImage \
    ./NovelForge_*.AppImage
  do
    if [ -f "$candidate" ]; then
      APPIMAGE="$candidate"
      break
    fi
  done
fi

if [ -z "$APPIMAGE" ] || [ ! -f "$APPIMAGE" ]; then
  echo "No NovelForge AppImage found." >&2
  echo "Pass its path: $0 ./NovelForge_0.1.5_amd64.AppImage" >&2
  exit 1
fi

# Absolute path, so the copy below still works after a `cd`.
APPIMAGE="$(cd "$(dirname "$APPIMAGE")" && pwd)/$(basename "$APPIMAGE")"

BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
TARGET="$BIN_DIR/novelforge.AppImage"
DESKTOP_DIR="$DATA_HOME/applications"
DESKTOP_FILE="$DESKTOP_DIR/novelforge.desktop"
ICONS_ROOT="$DATA_HOME/icons/hicolor"

mkdir -p "$BIN_DIR" "$DESKTOP_DIR" "$ICONS_ROOT"

if [ "$APPIMAGE" != "$TARGET" ]; then
  install -m 0755 "$APPIMAGE" "$TARGET"
fi
ln -sfn "novelforge.AppImage" "$BIN_DIR/novelforge"

# Pull the branded icon out of the bundle instead of depending on a source
# checkout; the AppImage root carries a real 256x256 NovelForge.png.
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
icon=""
for pattern in 'NovelForge.png' 'novelforge.png' 'usr/share/icons/hicolor/*/apps/novelforge.png'; do
  (cd "$TMP_DIR" && "$TARGET" --appimage-extract "$pattern" >/dev/null 2>&1) || true
  for candidate in \
    "$TMP_DIR/squashfs-root/NovelForge.png" \
    "$TMP_DIR/squashfs-root/novelforge.png" \
    "$TMP_DIR"/squashfs-root/usr/share/icons/hicolor/*/apps/novelforge.png
  do
    if [ -f "$candidate" ]; then
      icon="$candidate"
      break
    fi
  done
  [ -n "$icon" ] && break
done

if [ -n "$icon" ]; then
  # Keep the size directory honest when the bundle only carries a themed icon.
  icon_size=256x256
  case "$icon" in
    */hicolor/*/apps/*) icon_size="$(basename "$(dirname "$(dirname "$icon")")")" ;;
  esac
  icon_dir="$ICONS_ROOT/$icon_size/apps"
  mkdir -p "$icon_dir"
  install -m 0644 "$icon" "$icon_dir/novelforge.png"
  ICON_LINE="Icon=novelforge"
else
  echo "warning: could not extract an icon from the AppImage; the launcher will use a generic one." >&2
  ICON_LINE="Icon=application-x-executable"
fi

cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=NovelForge
GenericName=Download Studio
Comment=A private download studio for webnovels, manga and e-books.
Exec="$TARGET"
$ICON_LINE
Terminal=false
Categories=Network;FileTransfer;
Keywords=novel;manga;ebook;download;
StartupNotify=true
StartupWMClass=novelforge
EOF
chmod 0644 "$DESKTOP_FILE"

# Best effort: missing cache tools are not fatal, the entry still works.
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$ICONS_ROOT" >/dev/null 2>&1 || true
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "note: $BIN_DIR is not on PATH; the launcher entry still works, and"
     echo "      you can run $TARGET directly." ;;
esac

echo "Installed NovelForge"
echo "  app:   $TARGET"
echo "  entry: $DESKTOP_FILE"
echo "Launch it from the application menu, or run: novelforge"

if [ "$LAUNCH" -eq 1 ]; then
  nohup "$TARGET" >/dev/null 2>&1 &
  disown 2>/dev/null || true
fi
