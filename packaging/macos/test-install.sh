#!/usr/bin/env bash
# Test install.sh on any OS: the macOS tools it calls (uname, curl, shasum, ditto, codesign, pgrep,
# osascript, xattr, open) are stand-ins on PATH, and "GitHub" is a local directory of release
# files. Each case states what the installer must do, including what it must refuse.
#
# Usage: packaging/macos/test-install.sh   (exit 0 when every case passes)

# The stand-ins' bodies are written for the stand-in, not this shell: no expansion here.
# shellcheck disable=SC2016
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
STUBS="$T/stubs"
SERVE="$T/github"
mkdir -p "$STUBS" "$SERVE"

stub() {
  printf '#!/usr/bin/env bash\nset -euo pipefail\n%s\n' "$2" >"$STUBS/$1"
  chmod +x "$STUBS/$1"
}
stub uname 'case "${1:-}" in -s) echo "${STUB_UNAME_S:-Darwin}" ;; -m) echo "${STUB_UNAME_M:-arm64}" ;; *) echo Darwin ;; esac'
# curl: -I with -w %{url_effective} answers where releases/latest redirects; -o URL serves a file.
stub curl '
out="" url="" head=0
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w) shift 2 ;;
    -*I*) head=1; shift ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
echo "curl $url" >>"$STUB_LOG"
if [ "$head" = 1 ]; then printf "%s" "$STUB_LATEST"; exit 0; fi
f="$STUB_SERVE/${url#https://github.com/}"
[ -f "$f" ] || exit 22
cp "$f" "$out"'
stub shasum '[ "$1 $2" = "-a 256" ] || exit 2; sha256sum "$3"'
# ditto copying the new app records it if the app it replaces (STUB_APP_PID) is still running.
stub ditto 'if [ "$1" = -x ]; then mkdir -p "$4"; unzip -q "$3" -d "$4"; else
  [ "${STUB_DITTO_FAIL:-0}" = 0 ] || exit 1
  if [ -n "${STUB_APP_PID:-}" ] && kill -0 "$STUB_APP_PID" 2>/dev/null; then echo "copied while the app ran" >>"$STUB_LOG"; fi
  cp -a "$1" "$2"; fi'
stub codesign 'echo "codesign $*" >>"$STUB_LOG"; exit "${STUB_CODESIGN_RC:-0}"'
stub pgrep 'exit 1'
# hdiutil attach <...> -mountpoint M IMAGE opens the image at M; detach does nothing.
stub hdiutil 'if [ "$1" = attach ]; then m=""; img=""; while [ $# -gt 0 ]; do case "$1" in -mountpoint) m="$2"; shift 2 ;; -*) shift ;; attach) shift ;; *) img="$1"; shift ;; esac; done; mkdir -p "$m"; unzip -q "$img" -d "$m"; echo "hdiutil attach $img" >>"$STUB_LOG"; fi'
for t in osascript xattr open; do stub "$t" "echo \"$t \$*\" >>\"\$STUB_LOG\""; done

# A release v0.8.0-p1 whose app prints "new".
mkapp() {
  mkdir -p "$1/PdfCraft.app/Contents/MacOS"
  printf '#!/bin/sh\necho %s\n' "$2" >"$1/PdfCraft.app/Contents/MacOS/PdfCraft"
  chmod +x "$1/PdfCraft.app/Contents/MacOS/PdfCraft"
}
V=0.8.0-p1
ZIP="pdfcraft-$V-macos-aarch64.zip"
mkapp "$T/build" new
REL="$SERVE/krichelj/pdfcraft/releases/download/v$V"
mkdir -p "$REL"
(cd "$T/build" && zip -qry "$REL/$ZIP" PdfCraft.app)
(cd "$REL" && sha256sum -- * >SHA256SUMS.txt)
# Upstream's release of the same version: a universal DMG (here a zip the hdiutil stand-in opens).
UREL="$SERVE/storytold/pdfcraft/releases/download/v$V"
mkdir -p "$UREL"
mkapp "$T/ubuild" upstream
(cd "$T/ubuild" && zip -qry "$UREL/pdfcraft-$V-macos-universal.dmg" PdfCraft.app)
(cd "$UREL" && sha256sum -- * >SHA256SUMS.txt)

export STUB_SERVE="$SERVE" STUB_LOG="$T/log" PDFCRAFT_NO_OPEN=1
export STUB_LATEST="https://github.com/krichelj/pdfcraft/releases/tag/v$V"
pass=0
fail=0
# Usage: run <name> <expect: ok|refuse> <apps dir> [VAR=value…]: install.sh with that environment.
run() {
  local name="$1" expect="$2" apps="$3" rc=0
  shift 3
  : >"$STUB_LOG"
  env PATH="$STUBS:$PATH" PDFCRAFT_APPS="$apps" "$@" bash "$HERE/install.sh" >"$T/out" 2>&1 || rc=$?
  if { [ "$expect" = ok ] && [ "$rc" = 0 ]; } || { [ "$expect" = refuse ] && [ "$rc" != 0 ]; }; then
    return 0
  fi
  echo "FAIL $name: expected $expect, exit $rc" >&2
  sed 's/^/    /' "$T/out" >&2
  return 1
}
check() {
  if "${@:2}"; then pass=$((pass + 1)); else
    echo "FAIL $1" >&2
    fail=$((fail + 1))
  fi
}
app_says() { [ "$("$1/PdfCraft.app/Contents/MacOS/PdfCraft")" = "$2" ]; }
fresh() {
  rm -rf "$T/apps"
  mkdir -p "$T/apps"
  mkapp "$T/apps" old
}

fresh
check "installs the latest release over an old copy" run latest ok "$T/apps"
check "  the new app is in place" app_says "$T/apps" new
check "  no replaced copy is left behind" bash -c "! ls -A '$T/apps' | grep -q replaced"
check "  the signature was checked" grep -q '^codesign --verify --strict' "$T/log"

rm -rf "$T/empty"
check "installs where there was nothing" run empty ok "$T/empty"
check "  the app is there" app_says "$T/empty" new

fresh
check "a pinned version with a v prefix" run pinned ok "$T/apps" PDFCRAFT_VERSION="v$V"
check "  asks nothing about the latest release" bash -c "! grep -q 'releases/latest' '$T/log'"

fresh
cp "$REL/$ZIP" "$T/good.zip"
printf 'x' >>"$REL/$ZIP"
check "refuses a download that doesn't match SHA256SUMS.txt" run checksum refuse "$T/apps"
check "  and keeps the old copy" app_says "$T/apps" old
cp "$T/good.zip" "$REL/$ZIP"

fresh
check "refuses an app whose signature doesn't verify" run codesign refuse "$T/apps" STUB_CODESIGN_RC="1"
check "  and keeps the old copy" app_says "$T/apps" old

fresh
check "restores the old copy when the copy fails" run copy refuse "$T/apps" STUB_DITTO_FAIL="1"
check "  the old copy is back" app_says "$T/apps" old

fresh
check "refuses an Intel Mac" run intel refuse "$T/apps" STUB_UNAME_M="x86_64"
check "  before downloading anything" bash -c "! grep -q '^curl' '$T/log'"

fresh
check "refuses when there is no release" run none refuse "$T/apps" STUB_LATEST="https://github.com/krichelj/pdfcraft/releases"
check "  and keeps the old copy" app_says "$T/apps" old

fresh
check "refuses a release without the macOS zip" run missing refuse "$T/apps" PDFCRAFT_VERSION="9.9.9"

# Install and relaunch: the app starts the installer with its pid, then quits.
fresh
sleep 2 &
app=$!
check "waits for the app to quit, then swaps and reopens" run relaunch ok "$T/apps" PDFCRAFT_WAIT_PID="$app" STUB_APP_PID="$app" PDFCRAFT_NO_OPEN=0
check "  never while the app still runs" bash -c "! grep -q 'copied while the app ran' '$T/log'"
check "  the new app is in place" app_says "$T/apps" new
check "  and opened" grep -q "^open $T/apps/PdfCraft.app\$" "$T/log"
check "  without quitting anything itself" bash -c "! grep -q '^osascript' '$T/log'"

fresh
sleep 30 &
app=$!
check "gives up when the app never quits" run stuck refuse "$T/apps" PDFCRAFT_WAIT_PID="$app" PDFCRAFT_WAIT_SECS=1 PDFCRAFT_NO_OPEN=0
kill "$app" 2>/dev/null || true
check "  keeps the old copy" app_says "$T/apps" old
check "  says why in an alert" grep -q '^osascript .*display alert' "$T/log"
check "  and opens the installed copy again" grep -q "^open $T/apps/PdfCraft.app\$" "$T/log"

fresh
cp "$REL/$ZIP" "$T/good.zip"
printf 'x' >>"$REL/$ZIP"
check "a bad download after Install and relaunch" run relaunch-bad refuse "$T/apps" PDFCRAFT_WAIT_PID=999999999 PDFCRAFT_NO_OPEN=0
cp "$T/good.zip" "$REL/$ZIP"
check "  keeps the old copy" app_says "$T/apps" old
check "  says why in an alert" grep -q '^osascript .*display alert' "$T/log"

fresh
check "refuses a process id that isn't one" run bad-pid refuse "$T/apps" PDFCRAFT_WAIT_PID="12; rm -rf /"
check "  and keeps the old copy" app_says "$T/apps" old

# The original PdfCraft's release: its DMG, mounted and copied, checked the same way.
fresh
check "an upstream release installs from its DMG" run upstream ok "$T/apps" PDFCRAFT_REPO=storytold/pdfcraft PDFCRAFT_VERSION="$V"
check "  the upstream app is in place" app_says "$T/apps" upstream
check "  through the DMG" grep -q "^hdiutil attach .*pdfcraft-$V-macos-universal.dmg" "$T/log"
fresh
printf 'x' >>"$UREL/pdfcraft-$V-macos-universal.dmg"
check "refuses an upstream DMG that doesn't match its SHA256SUMS.txt" run upstream-bad refuse "$T/apps" PDFCRAFT_REPO=storytold/pdfcraft PDFCRAFT_VERSION="$V"
check "  and keeps the old copy" app_says "$T/apps" old
fresh
check "refuses any other repository" run elsewhere refuse "$T/apps" PDFCRAFT_REPO=evil/pdfcraft PDFCRAFT_VERSION="$V"
check "  before downloading anything" bash -c "! grep -q '^curl' '$T/log'"

echo "install.sh: $pass passed, $fail failed"
[ "$fail" = 0 ]
