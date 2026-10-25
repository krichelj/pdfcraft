#!/usr/bin/env bash
# End to end, headless, on Linux: Help > Check for updates > Install and relaunch, against the real
# krichelj/pdfcraft releases. A PdfCraft binary stamped with an older version runs from a stand-in
# PdfCraft.app (with the bundled install.sh) under Xvfb. Its launch check asks GitHub, finds the
# newer release and opens the Updates dialog; the test clicks Install and relaunch over the control
# channel. The app quits, and the installer downloads the real release zip and SHA256SUMS.txt with
# curl, checks them, swaps the bundle and "opens" it. Only the macOS-only tools are stand-ins:
# uname (Darwin, arm64), ditto (unzip, cp), codesign (rcodesign reads the real signature), open,
# osascript, xattr and pgrep.
#
# Usage: packaging/macos/e2e-update.sh <pdfcraft binary> <pdfcraft-cli binary>
#   Build both with a version older than the latest release, e.g.
#   cargo xtask version set 0.8.0 && cargo build -p pdfcraft -p pdfcraft-cli, then restore.
#   The fork's latest release must be built on the original PdfCraft's latest (as the daily sync
#   keeps it): while the original is ahead, the dialog offers Merge and update instead, which
#   test-install.sh and the ui-egui updates tests cover with stand-ins.
# Needs: xvfb-run, curl, unzip, sha256sum, rcodesign, file; network access to github.com.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
[ $# = 2 ] || { echo "usage: $0 <pdfcraft binary> <pdfcraft-cli binary>" >&2; exit 2; }
APP_BIN="$(realpath "$1")" CLI="$(realpath "$2")"
for t in xvfb-run curl unzip sha256sum rcodesign file; do
  command -v "$t" >/dev/null || { echo "error: $t not found" >&2; exit 1; }
done
E="$(mktemp -d)"
echo "e2e: working in $E (kept for inspection)"
BUNDLE="$E/Applications/PdfCraft.app"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources" "$E/stubs" "$E/home" "$E/ctl"
chmod 700 "$E/ctl"
cp "$APP_BIN" "$BUNDLE/Contents/MacOS/PdfCraft"
install -m 755 "$HERE/install.sh" "$BUNDLE/Contents/Resources/install.sh"
OLD="$("$BUNDLE/Contents/MacOS/PdfCraft" --version)"

stub() {
  printf '#!/usr/bin/env bash\nset -euo pipefail\n%s\n' "$2" >"$E/stubs/$1"
  chmod +x "$E/stubs/$1"
}
# shellcheck disable=SC2016
{
  stub uname 'case "${1:-}" in -s) echo Darwin ;; -m) echo arm64 ;; *) echo Darwin ;; esac'
  stub shasum '[ "$1 $2" = "-a 256" ] || exit 2; sha256sum "$3"'
  stub ditto 'if [ "$1" = -x ]; then mkdir -p "$4"; unzip -q "$3" -d "$4"; else cp -a "$1" "$2"; fi'
  stub codesign 'for a; do app="$a"; done; rcodesign print-signature-info "$app/Contents/MacOS/PdfCraft" | grep -q code_directory'
  stub pgrep 'exit 1'
  for t in osascript xattr open; do stub "$t" "echo \"$t \$*\" >>\"$E/calls\""; done
}

ui() { "$CLI" ui --control "$E/ctl/control.json" "$@"; }
# Usage: until_true <seconds> <what> <command…>
until_true() {
  local secs="$1" what="$2" i
  shift 2
  for ((i = 0; i < secs * 2; i++)); do
    if "$@" >/dev/null 2>&1; then return 0; fi
    sleep 0.5
  done
  echo "FAIL: timed out after ${secs}s waiting for $what" >&2
  exit 1
}

echo "e2e: starting $OLD from $BUNDLE under Xvfb"
env PATH="$E/stubs:$PATH" HOME="$E/home" XDG_DATA_HOME="$E/home/data" XDG_CONFIG_HOME="$E/home/config" \
  xvfb-run -a -s "-screen 0 1400x900x24" "$BUNDLE/Contents/MacOS/PdfCraft" --control "$E/ctl/control.json" \
  >"$E/app.out" 2>&1 &
XVFB=$!
app_running() { /usr/bin/pgrep -f "^$BUNDLE/Contents/MacOS/PdfCraft --control" >/dev/null; }
until_true 120 "the control channel" ui state
until_true 90 "the launch check to offer Install and relaunch" bash -c "'$CLI' ui --control '$E/ctl/control.json' inspect query='install and relaunch' | grep -q 'Install and relaunch'"
ui screenshot --out "$E/updates-dialog.png" >/dev/null
echo "e2e: the launch check opened the dialog (screenshot $E/updates-dialog.png)"
ui click label="Install and relaunch" >/dev/null
until_true 60 "the app to quit" bash -c "! /usr/bin/pgrep -f '^$BUNDLE/Contents/MacOS/PdfCraft --control' >/dev/null"
wait "$XVFB" 2>/dev/null || true
echo "e2e: the app quit; waiting for the installer"
until_true 600 "the installer to finish" bash -c "grep -qE 'is installed in|error:' \"\$(find '$E/home' -name update.log | head -1)\""
LOG="$(find "$E/home" -name update.log | head -1)"
sed 's/^/    log: /' "$LOG"
grep -q 'is installed in' "$LOG" || { echo "FAIL: the installer reported an error" >&2; exit 1; }

NEW_BIN="$BUNDLE/Contents/MacOS/PdfCraft"
file "$NEW_BIN" | grep -q 'Mach-O 64-bit arm64' || { echo "FAIL: $NEW_BIN is not the release's arm64 app: $(file "$NEW_BIN")" >&2; exit 1; }
NEW_VERSION="$(grep -A1 CFBundleShortVersionString "$BUNDLE/Contents/Info.plist" | sed -n 's:.*<string>\(.*\)</string>.*:\1:p')"
grep -q "^open $BUNDLE\$" "$E/calls" || { echo "FAIL: the new app wasn't opened" >&2; exit 1; }
! app_running || { echo "FAIL: the old app is still running" >&2; exit 1; }
echo "e2e: PASS: $OLD -> PdfCraft $NEW_VERSION ($(file -b "$NEW_BIN" | cut -d, -f1)), reopened from $BUNDLE"
