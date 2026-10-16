#!/usr/bin/env bash
# Put PdfCraft.app on a drag-to-Applications DMG: the app, an `Applications` link, and the Finder
# window layout from this directory (README.md). One staging for every build of the image:
#
#   package.sh (macOS)        `hdiutil create`, then mounts the image and checks the app inside
#   cross-package.sh (Linux)  an HFS+ volume from hfsprogs' mkfs.hfsplus and libdmg-hfsplus's
#                             `hfsplus` and `dmg` (as Firefox builds its DMGs on Linux), then
#                             extracts the image and checks the app, the link and the layout
#
# Usage: packaging/macos/dmg/make-dmg.sh <PdfCraft.app> <out.dmg> <work-dir>
# The image is not signed here; package.sh signs and notarizes it afterwards.
set -euo pipefail
# shellcheck source=../../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../../env.sh"
DMG_DIR="$ROOT/packaging/macos/dmg"
[ $# = 3 ] || { echo "usage: $0 <PdfCraft.app> <out.dmg> <work-dir>" >&2; exit 2; }
APP="$1" DMG="$2" WORK="$3"
[ -x "$APP/Contents/MacOS/PdfCraft" ] || { echo "error: $APP has no Contents/MacOS/PdfCraft" >&2; exit 1; }

echo "==> building $DMG"
STAGE="$WORK/dmg"
rm -rf "$STAGE"
mkdir -p "$STAGE"
if command -v ditto >/dev/null; then ditto "$APP" "$STAGE/PdfCraft.app"; else cp -a "$APP" "$STAGE/PdfCraft.app"; fi
ln -s /Applications "$STAGE/Applications"
# Finder window layout: background, icon size and positions (README.md).
mkdir -p "$STAGE/.background"
cp "$DMG_DIR/background.tiff" "$STAGE/.background/background.tiff"
cp "$DMG_DIR/dmg-layout.DS_Store" "$STAGE/.DS_Store"
rm -f "$DMG"

if [ "$(uname -s)" = Darwin ]; then
  # `hdiutil create -srcfolder` writes the files as they are. `makehybrid -hfs` gave every file a
  # com.apple.FinderInfo (a Finder location of -1,-1), which `codesign --verify --strict` rejects as
  # "Finder information, or similar detritus" on the app in the image and on a copy installed from it,
  # so MDM and compliance checks reported a correctly notarized app as damaged (#390). `create`
  # attaches a device while it copies, which fails now and then on CI runners ("Resource busy"), so
  # it gets a few tries. The volume name has no version: .DS_Store finds the background through an
  # alias that includes it.
  # Usage: hdiutil_retry <what> <hdiutil args…>; tries five times with growing pauses.
  hdiutil_retry() {
    local what="$1" attempt
    shift
    for attempt in 1 2 3 4 5; do
      if hdiutil "$@"; then
        return 0
      fi
      [ "$attempt" = 5 ] && { echo "hdiutil $what failed 5 times" >&2; return 1; }
      warn "hdiutil $what failed (attempt $attempt of 5); retrying in $((attempt * 10))s"
      sleep $((attempt * 10))
    done
  }
  hdiutil_retry create create -srcfolder "$STAGE" -volname "PdfCraft" -fs HFS+ -format UDZO -imagekey zlib-level=9 -ov "$DMG"
  # The app inside the image must still pass --strict, so a makehybrid-style regression can't ship.
  MOUNT="$WORK/dmg-check"
  rm -rf "$MOUNT"
  mkdir -p "$MOUNT"
  hdiutil_retry attach attach -nobrowse -readonly -mountpoint "$MOUNT" "$DMG" >/dev/null
  if ! codesign --verify --strict --deep --verbose=2 "$MOUNT/PdfCraft.app"; then
    hdiutil_retry detach detach "$MOUNT" >/dev/null || true
    exit 1
  fi
  hdiutil_retry detach detach "$MOUNT" >/dev/null
else
  for tool in mkfs.hfsplus hfsplus dmg; do
    command -v "$tool" >/dev/null || { echo "error: $tool not found on $(hostname)" >&2; exit 1; }
  done
  # An HFS+ volume with 25% and 16 MiB of room over the staged size, named as above.
  HFS="$WORK/pdfcraft.hfs"
  STAGE_KIB=$(du -sk "$STAGE" | cut -f1)
  rm -f "$HFS"
  truncate -s "$(((STAGE_KIB * 5 / 4 + 16384) * 1024))" "$HFS"
  mkfs.hfsplus -v PdfCraft "$HFS" >/dev/null
  # clone_link keeps the Applications link a link; modes (the executable bits) are copied.
  hfsplus "$HFS" --symlinks clone_link addall "$STAGE" >/dev/null
  dmg build "$HFS" "$DMG" >/dev/null
  # The image must hold the app, the link and the layout, or it doesn't ship.
  CHECK="$WORK/dmg-check.hfs"
  rm -f "$CHECK"
  dmg extract "$DMG" "$CHECK" >/dev/null
  ROOT_LIST="$(hfsplus "$CHECK" ls /)"
  for entry in PdfCraft.app Applications .DS_Store .background; do
    grep -q " $entry\$" <<<"$ROOT_LIST" || { echo "error: $entry missing from $DMG" >&2; exit 1; }
  done
  hfsplus "$CHECK" ls /PdfCraft.app/Contents/MacOS | grep -q '^100[1357][0-7][0-7] .* PdfCraft$' \
    || { echo "error: no executable app binary in $DMG" >&2; exit 1; }
  rm -f "$HFS" "$CHECK"
fi
rm -rf "$STAGE"
