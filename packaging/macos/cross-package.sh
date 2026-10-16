#!/usr/bin/env bash
# Cross-build the Apple Silicon (M1 and later) macOS artifacts on a Linux host:
#
#   $DIST/pdfcraft-<version>-macos-aarch64.dmg       PdfCraft.app on a drag-to-Applications DMG
#   $DIST/pdfcraft-<version>-macos-aarch64.zip       PdfCraft.app, ad-hoc signed
#   $DIST/pdfcraft-cli-<version>-macos-aarch64.zip   the headless CLI
#
# The fork's runners are Linux only, so this replaces package.sh there: zig links against a macOS
# SDK copied from the owner's Xcode, rcodesign signs ad-hoc, and the DMG is an HFS+ image written
# with hfsprogs' mkfs.hfsplus and libdmg-hfsplus (`hfsplus`, `dmg`), the way Firefox builds its
# DMGs on Linux. No notarization (notarytool is macOS-only); macOS asks once on first open.
#
# Needs (env): MACOS_SDKROOT (a MacOSX*.sdk directory). Tools on PATH: zig, cargo-zigbuild,
# rcodesign, zip, mkfs.hfsplus, hfsplus, dmg; the aarch64-apple-darwin Rust target.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/macos"
TARGET=aarch64-apple-darwin

# Per-run zig caches: concurrent jobs sharing ~/.cache/cargo-zigbuild and zig's global cache
# raced (run 37861883023: "getcwd() failed", CurrentDirUnlinked in the Windows x64 link).
ZIG_SCRATCH="${RUNNER_TEMP:-$CARGO_TARGET_DIR}/zig-cache"
export ZIG_GLOBAL_CACHE_DIR="$ZIG_SCRATCH/global" ZIG_LOCAL_CACHE_DIR="$ZIG_SCRATCH/local" \
  CARGO_ZIGBUILD_CACHE_DIR="$ZIG_SCRATCH/cargo-zigbuild"
for tool in zig cargo-zigbuild rcodesign zip mkfs.hfsplus hfsplus dmg; do
  command -v "$tool" >/dev/null || { echo "error: $tool not found on $(hostname)" >&2; exit 1; }
done
[ -f "${MACOS_SDKROOT:-}/usr/lib/libSystem.tbd" ] || {
  echo "error: MACOS_SDKROOT='${MACOS_SDKROOT:-}' is not a macOS SDK" >&2; exit 1; }

# Keep in sync with LSMinimumSystemVersion in Info.plist.in.
export MACOSX_DEPLOYMENT_TARGET=11.0 SDKROOT="$MACOS_SDKROOT"
# aws-lc requires NEON and the crypto extensions at compile time; every Apple Silicon chip has them.
export CFLAGS_aarch64_apple_darwin=-mcpu=apple-m1 CXXFLAGS_aarch64_apple_darwin=-mcpu=apple-m1
PDFCRAFT_REAL_ZIG="$(command -v zig)"
export PDFCRAFT_REAL_ZIG CARGO_ZIGBUILD_ZIG_PATH="$HERE/zig-cc.sh"
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/macos-cross-package"
APP="$WORK/PdfCraft.app"
APP_ZIP="$DIST/pdfcraft-$VERSION-macos-aarch64.zip"
DMG="$DIST/pdfcraft-$VERSION-macos-aarch64.dmg"
CLI_ZIP="$DIST/pdfcraft-cli-$VERSION-macos-aarch64.zip"

echo "==> PdfCraft $VERSION for macOS ($TARGET), cross-built on $(hostname)"
(cd "$ROOT" && cargo zigbuild --release --locked -p pdfcraft -p pdfcraft-cli --target "$TARGET")
BIN="$CARGO_TARGET_DIR/$TARGET/release"

rm -rf "$WORK"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$DIST"
cp "$BIN/pdfcraft" "$APP/Contents/MacOS/PdfCraft"
cp "$ROOT/assets/app-icon/pdfcraft.icns" "$APP/Contents/Resources/PdfCraft.icns"
copy_font_licences "$APP/Contents/Resources"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" \
  -e "s/@BUILD_SHA@/${PDFCRAFT_BUILD_SHA:-unknown}/g" \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
python3 -c 'import plistlib,sys; plistlib.load(open(sys.argv[1],"rb"))' "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"

# Ad-hoc signature (no certificate): Apple Silicon refuses to run unsigned arm64 code.
rcodesign sign --code-signature-flags runtime --entitlements-xml-file "$HERE/entitlements.plist" "$APP"
# `rcodesign verify` rejects ad-hoc signatures (no CMS blob); check the signature is there instead.
rcodesign print-signature-info "$APP/Contents/MacOS/PdfCraft" | grep -q "code_directory"

rm -f "$APP_ZIP"
(cd "$WORK" && zip -qry "$APP_ZIP" PdfCraft.app)

# ---- DMG: the same layout as package.sh (packaging/macos/dmg/README.md) ----------------------
STAGE="$WORK/dmg"
mkdir -p "$STAGE/.background"
cp -a "$APP" "$STAGE/PdfCraft.app"
ln -s /Applications "$STAGE/Applications"
cp "$HERE/dmg/background.tiff" "$STAGE/.background/background.tiff"
cp "$HERE/dmg/dmg-layout.DS_Store" "$STAGE/.DS_Store"
# An HFS+ volume with 25% and 16 MiB of room over the staged size. The volume name has no
# version: .DS_Store finds the background through an alias that includes it.
HFS="$WORK/pdfcraft.hfs"
STAGE_KIB=$(du -sk "$STAGE" | cut -f1)
rm -f "$HFS" "$DMG"
truncate -s "$(( (STAGE_KIB * 5 / 4 + 16384) * 1024 ))" "$HFS"
mkfs.hfsplus -v PdfCraft "$HFS" >/dev/null
# clone_link keeps the Applications link a link; modes (the executable bits) are copied.
hfsplus "$HFS" --symlinks clone_link addall "$STAGE" >/dev/null
dmg build "$HFS" "$DMG" >/dev/null
# The image must hold the signed app, the link and the layout, or it doesn't ship.
CHECK="$WORK/dmg-check.hfs"
rm -f "$CHECK"
dmg extract "$DMG" "$CHECK" >/dev/null
for entry in PdfCraft.app Applications .DS_Store .background; do
  hfsplus "$CHECK" ls / | grep -q "$entry" || { echo "error: $entry missing from $DMG" >&2; exit 1; }
done
hfsplus "$CHECK" ls /PdfCraft.app/Contents/MacOS | grep -q 'PdfCraft' || { echo "error: no app binary in $DMG" >&2; exit 1; }
rm -f "$HFS" "$CHECK"

CLI_DIR="$WORK/pdfcraft-cli-$VERSION-macos-aarch64"
mkdir -p "$CLI_DIR"
cp "$BIN/pdfcraft-cli" "$CLI_DIR/"
copy_docs "$CLI_DIR"
rcodesign sign --code-signature-flags runtime "$CLI_DIR/pdfcraft-cli"
rm -f "$CLI_ZIP"
(cd "$WORK" && zip -qry "$CLI_ZIP" "$(basename "$CLI_DIR")")
ls -l "$DMG" "$APP_ZIP" "$CLI_ZIP"
