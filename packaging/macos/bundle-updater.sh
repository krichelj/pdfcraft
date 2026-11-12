#!/usr/bin/env bash
# Put PdfCraft Updater (apps/pdfcraft-updater) inside PdfCraft.app as a helper app:
#
#   bundle-updater.sh <pdfcraft-updater binary> <PdfCraft.app> <version>
#
# Help > Check for updates opens PdfCraft.app/Contents/Helpers/PdfCraft Updater.app for Merge and
# update and for Install and relaunch (apps/pdfcraft/src/updates.rs). Run before the bundle is
# signed; the signing step signs the helper first, then PdfCraft.app. Used by package.sh and
# cross-package.sh.
set -euo pipefail
[ $# -eq 3 ] || { echo "usage: $0 <pdfcraft-updater binary> <PdfCraft.app> <version>" >&2; exit 2; }
bin="$1" app="$2" version="$3"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
[ -x "$bin" ] || { echo "error: $bin is not a program" >&2; exit 1; }
[ -f "$app/Contents/Resources/PdfCraft.icns" ] || { echo "error: $app has no PdfCraft.icns yet" >&2; exit 1; }
helper="$app/Contents/Helpers/PdfCraft Updater.app"
rm -rf "$helper"
mkdir -p "$helper/Contents/MacOS" "$helper/Contents/Resources"
cp "$bin" "$helper/Contents/MacOS/pdfcraft-updater"
cp "$app/Contents/Resources/PdfCraft.icns" "$helper/Contents/Resources/PdfCraft.icns"
sed -e "s/@VERSION@/$version/g" -e "s/@SHORT_VERSION@/${version%%-*}/g" \
  "$here/Updater-Info.plist.in" >"$helper/Contents/Info.plist"
printf 'APPL????' >"$helper/Contents/PkgInfo"
echo "$helper"
