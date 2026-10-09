#!/usr/bin/env bash
# Cross-build the Apple Silicon (M1 and later) macOS artifacts on a Linux host:
#
#   $DIST/pdfcraft-<version>-macos-aarch64.zip       PdfCraft.app, ad-hoc signed
#   $DIST/pdfcraft-cli-<version>-macos-aarch64.zip   the headless CLI
#
# The fork's runners are Linux only, so this replaces package.sh there: zig links against a macOS
# SDK copied from the owner's Xcode, rcodesign signs ad-hoc. No DMG and no notarization (hdiutil
# and notarytool are macOS-only); macOS asks once on first open (right-click > Open).
#
# Needs (env): MACOS_SDKROOT (a MacOSX*.sdk directory). Tools on PATH: zig, cargo-zigbuild,
# rcodesign, zip; the aarch64-apple-darwin Rust target.
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
for tool in zig cargo-zigbuild rcodesign zip; do
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

CLI_DIR="$WORK/pdfcraft-cli-$VERSION-macos-aarch64"
mkdir -p "$CLI_DIR"
cp "$BIN/pdfcraft-cli" "$CLI_DIR/"
copy_docs "$CLI_DIR"
rcodesign sign --code-signature-flags runtime "$CLI_DIR/pdfcraft-cli"
rm -f "$CLI_ZIP"
(cd "$WORK" && zip -qry "$CLI_ZIP" "$(basename "$CLI_DIR")")
ls -l "$APP_ZIP" "$CLI_ZIP"
