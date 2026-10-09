#!/usr/bin/env bash
# Cross-build the Windows portable zip on a Linux host:
#
#   $DIST/pdfcraft-<version>-windows-<arch>-portable.zip   pdfcraft.exe + pdfcraft-cli.exe + portable.txt
#
# The fork's runners are Linux only, so this replaces package.ps1 there: zig links the GNU-ABI
# targets (x64: x86_64-pc-windows-gnu, arm64: aarch64-pc-windows-gnullvm). No MSI (WiX needs
# Windows' msi.dll) and no Authenticode signature, so SmartScreen warns on first run.
#
# Usage: packaging/windows/cross-package.sh --arch x64|arm64
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/windows"

ARCH=""
while [ $# -gt 0 ]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  x64) TARGET=x86_64-pc-windows-gnu ;;
  arm64) TARGET=aarch64-pc-windows-gnullvm ;;
  *) echo "usage: $0 --arch x64|arm64" >&2; exit 2 ;;
esac
# Per-run zig caches: concurrent jobs sharing ~/.cache/cargo-zigbuild and zig's global cache
# raced (run 37861883023: "getcwd() failed", CurrentDirUnlinked in the Windows x64 link).
ZIG_SCRATCH="${RUNNER_TEMP:-$CARGO_TARGET_DIR}/zig-cache"
export ZIG_GLOBAL_CACHE_DIR="$ZIG_SCRATCH/global" ZIG_LOCAL_CACHE_DIR="$ZIG_SCRATCH/local" \
  CARGO_ZIGBUILD_CACHE_DIR="$ZIG_SCRATCH/cargo-zigbuild"
for tool in zig cargo-zigbuild zip; do
  command -v "$tool" >/dev/null || { echo "error: $tool not found on $(hostname)" >&2; exit 1; }
done

echo "==> PdfCraft $VERSION for Windows ($TARGET), cross-built on $(hostname)"
(cd "$ROOT" && cargo zigbuild --release --locked -p pdfcraft -p pdfcraft-cli --target "$TARGET")
BIN="$CARGO_TARGET_DIR/$TARGET/release"

NAME="pdfcraft-$VERSION-windows-$ARCH-portable"
WORK="$CARGO_TARGET_DIR/windows-cross-package"
rm -rf "${WORK:?}/$NAME"
mkdir -p "$WORK/$NAME" "$DIST"
cp "$BIN/pdfcraft.exe" "$BIN/pdfcraft-cli.exe" "$HERE/portable.txt" "$WORK/$NAME/"
copy_docs "$WORK/$NAME"
copy_font_licences "$WORK/$NAME"
ZIP="$DIST/$NAME.zip"
rm -f "$ZIP"
(cd "$WORK" && zip -qr "$ZIP" "$NAME")
ls -l "$ZIP"
