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
# curl: -I with -w %{url_effective} answers where releases/latest redirects (STUB_LATEST, or the
# next of the space-separated STUB_LATEST_SEQ on each look, then its last); -o URL serves a file.
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
if [ "$head" = 1 ]; then
  if [ -n "${STUB_LATEST_SEQ:-}" ]; then
    read -r -a seq <<<"$STUB_LATEST_SEQ"
    n=$(($(cat "$STUB_LOG.looks" 2>/dev/null || echo 0) + 1))
    echo "$n" >"$STUB_LOG.looks"
    i=$((n <= ${#seq[@]} ? n - 1 : ${#seq[@]} - 1))
    printf "%s" "https://github.com/krichelj/pdfcraft/releases/tag/${seq[i]}"
  else
    printf "%s" "$STUB_LATEST"
  fi
  exit 0
fi
f="$STUB_SERVE/${url#https://github.com/}"
[ -f "$f" ] || exit 22
cp "$f" "$out"'
stub shasum '[ "$1 $2" = "-a 256" ] || exit 2; sha256sum "$3"'
# ditto copying the new app records it if the app it replaces (STUB_APP_PID) is still running.
stub ditto 'if [ "$1" = -x ]; then mkdir -p "$4"; unzip -q "$3" -d "$4"; else
  [ "${STUB_DITTO_FAIL:-0}" = 0 ] || exit 1
  if [ -n "${STUB_APP_PID:-}" ] && kill -0 "$STUB_APP_PID" 2>/dev/null; then echo "copied while the app ran" >>"$STUB_LOG"; fi
  if [ -e "${STUB_RUNNING:-/nonexistent}" ]; then echo "copied while the app ran" >>"$STUB_LOG"; fi
  cp -a "$1" "$2"; fi'
stub codesign 'echo "codesign $*" >>"$STUB_LOG"; exit "${STUB_CODESIGN_RC:-0}"'
# pgrep -xq PdfCraft: "running" while the file STUB_RUNNING exists.
stub pgrep '[ -e "${STUB_RUNNING:-/nonexistent}" ]'
# osascript: records its arguments; a dialog answers STUB_DIALOG; "quit" ends the running app,
# after STUB_QUIT_DELAY seconds (a user answering unsaved-changes prompts), or never when "never".
stub osascript 'echo "osascript $*" >>"$STUB_LOG"
case "$*" in
  *"display dialog"*) printf "%s\n" "${STUB_DIALOG:-}" ;;
  *"to quit"*)
    case "${STUB_QUIT_DELAY:-0}" in
      never) ;;
      0) rm -f "${STUB_RUNNING:-/nonexistent}" ;;
      *) (sleep "$STUB_QUIT_DELAY"; rm -f "${STUB_RUNNING:-/nonexistent}") >/dev/null 2>&1 & ;;
    esac ;;
esac'
for t in xattr open; do stub "$t" "echo \"$t \$*\" >>\"\$STUB_LOG\""; done
# plutil -extract PdfCraftVersion raw -o - <Info.plist>: the key's value, as mkapp writes it.
stub plutil 'f="${!#}"; [ -f "$f" ] || exit 1
sed -n "s:.*<key>PdfCraftVersion</key><string>\(.*\)</string>.*:\1:p" "$f" | grep . || exit 1'

# Usage: mkapp <dir> <what it prints> [<its version>]: a PdfCraft.app with an Info.plist.
mkapp() {
  mkdir -p "$1/PdfCraft.app/Contents/MacOS"
  printf '#!/bin/sh\necho %s\n' "$2" >"$1/PdfCraft.app/Contents/MacOS/PdfCraft"
  chmod +x "$1/PdfCraft.app/Contents/MacOS/PdfCraft"
  printf '<plist><dict><key>PdfCraftVersion</key><string>%s</string></dict></plist>\n' "${3:-0.0.1}" \
    >"$1/PdfCraft.app/Contents/Info.plist"
}
# Usage: release <version> <what its app prints>: publish the fork's release on the stand-in GitHub.
release() {
  local rel="$SERVE/krichelj/pdfcraft/releases/download/v$1"
  rm -rf "$T/build-$1"
  mkapp "$T/build-$1" "$2" "$1"
  mkdir -p "$rel"
  (cd "$T/build-$1" && zip -qry "$rel/pdfcraft-$1-macos-aarch64.zip" PdfCraft.app)
  (cd "$rel" && sha256sum -- * >SHA256SUMS.txt)
}
V=0.8.0-p1
ZIP="pdfcraft-$V-macos-aarch64.zip"
release "$V" new
REL="$SERVE/krichelj/pdfcraft/releases/download/v$V"
# The fork's release built on the original PdfCraft 0.12.0 (what Merge and update waits for).
release 0.12.0-p1 merged

export STUB_SERVE="$SERVE" STUB_LOG="$T/log" PDFCRAFT_NO_OPEN=1 TMPDIR="$T/tmp"
mkdir -p "$TMPDIR"
export STUB_LATEST="https://github.com/krichelj/pdfcraft/releases/tag/v$V"
pass=0
fail=0
# Usage: run <name> <expect: ok|refuse> <apps dir> [VAR=value…]: install.sh with that environment.
run() {
  local name="$1" expect="$2" apps="$3" rc=0
  shift 3
  : >"$STUB_LOG"
  rm -f "$STUB_LOG.looks"
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
looks() { [ "$(cat "$STUB_LOG.looks" 2>/dev/null || echo 0)" = "$1" ]; }
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

# Only the fork's builds are installed: the original PdfCraft's would drop the fork's changes.
fresh
check "ignores a request for the original PdfCraft's build" run upstream ok "$T/apps" PDFCRAFT_REPO=storytold/pdfcraft
check "  and installs the fork's" app_says "$T/apps" new
check "  from the fork's releases only" bash -c "! grep -q 'storytold' '$T/log'"

# Merge and update: wait for the fork's release built on the original PdfCraft's new version.
AWAIT=(PDFCRAFT_AWAIT_BASE=0.12.0 PDFCRAFT_AWAIT_POLL=0 PDFCRAFT_AWAIT_SECS=20)
fresh
check "waits until the fork's release is built on the merged version" run await ok "$T/apps" "${AWAIT[@]}" \
  STUB_LATEST_SEQ="v0.11.0-p2 v0.11.0-p3 v0.12.0-p1" STUB_DIALOG="button returned:Install now, gave up:false"
check "  looking three times" looks 3
check "  then installs it" app_says "$T/apps" merged
check "  checked like any release" grep -q '^codesign --verify --strict' "$T/log"

fresh
check "compares versions as numbers (0.9.9 and 0.10.0 are older than 0.12.0)" run await-numeric ok "$T/apps" \
  "${AWAIT[@]}" STUB_LATEST_SEQ="v0.9.9-p1 v0.10.0-p4 v0.12.0-p1" STUB_DIALOG="button returned:Install now, gave up:false"
check "  installs only the release built on 0.12.0" app_says "$T/apps" merged
check "  after three looks" looks 3

fresh
check "gives up when the release never appears" run await-timeout refuse "$T/apps" PDFCRAFT_AWAIT_BASE=0.12.0 \
  PDFCRAFT_AWAIT_POLL=0 PDFCRAFT_AWAIT_SECS=3 STUB_LATEST_SEQ="v0.11.0-p2"
check "  keeps the old copy" app_says "$T/apps" old
check "  says why in an alert" grep -q '^osascript .*display alert' "$T/log"
check "  downloading nothing" bash -c "! grep -q 'releases/download' '$T/log'"
check "  and leaves no lock behind" bash -c "! ls -A '$TMPDIR' | grep -q pdfcraft-installer"

for bad in 0.12 abc "0.12.0; rm -rf /" 0.012.0 1.2.3.4 "" ; do
  [ -n "$bad" ] || continue
  fresh
  check "refuses PDFCRAFT_AWAIT_BASE='$bad'" run await-bad refuse "$T/apps" PDFCRAFT_AWAIT_BASE="$bad" \
    PDFCRAFT_AWAIT_POLL=0 PDFCRAFT_AWAIT_SECS=2 STUB_LATEST_SEQ="v0.12.0-p1"
  check "  before asking GitHub anything" bash -c "! grep -q '^curl' '$T/log'"
  check "  keeping the old copy" app_says "$T/apps" old
done

fresh
touch "$T/running"
check "asks before quitting a running PdfCraft, and Later leaves it alone" run await-later ok "$T/apps" \
  "${AWAIT[@]}" STUB_LATEST_SEQ="v0.12.0-p1" STUB_RUNNING="$T/running" STUB_DIALOG="button returned:Later"
check "  the question was asked" grep -q '^osascript .*display dialog' "$T/log"
check "  PdfCraft was not quit" bash -c "! grep -q 'to quit' '$T/log'"
check "  and the old copy stays" app_says "$T/apps" old

fresh
touch "$T/running"
check "Install now quits PdfCraft, then installs" run await-now ok "$T/apps" "${AWAIT[@]}" \
  STUB_LATEST_SEQ="v0.12.0-p1" STUB_RUNNING="$T/running" STUB_DIALOG="button returned:Install now, gave up:false"
check "  quitting it first" grep -q "^osascript .*tell application \"PdfCraft\" to quit" "$T/log"
check "  never copying while it ran" bash -c "! grep -q 'copied while the app ran' '$T/log'"
check "  the merged release is in place" app_says "$T/apps" merged

fresh
touch "$T/running"
check "an unanswered question (gave up) counts as Later" run await-gaveup ok "$T/apps" "${AWAIT[@]}" \
  STUB_LATEST_SEQ="v0.12.0-p1" STUB_RUNNING="$T/running" STUB_DIALOG="button returned:, gave up:true"
check "  the old copy stays" app_says "$T/apps" old
rm -f "$T/running"

LOCK="$TMPDIR/pdfcraft-installer.lock"
fresh
sleep 30 &
waiting=$!
ln -sfn "$waiting" "$LOCK"
check "a second Merge and update while one waits does nothing" run await-twice ok "$T/apps" "${AWAIT[@]}" \
  STUB_LATEST_SEQ="v0.12.0-p1"
check "  asking GitHub nothing" bash -c "! grep -q '^curl' '$T/log'"
check "  and keeping the old copy" app_says "$T/apps" old
check "  and the other installer's lock" test "$(readlink "$LOCK")" = "$waiting"
check "the one-line install refuses while another installer runs" run plain-locked refuse "$T/apps"
check "  keeping the old copy" app_says "$T/apps" old
check "  and the other installer's lock" test "$(readlink "$LOCK")" = "$waiting"
kill "$waiting" 2>/dev/null || true
wait "$waiting" 2>/dev/null || true
check "a lock left by an installer that died is taken over" run await-stale ok "$T/apps" "${AWAIT[@]}" \
  STUB_LATEST_SEQ="v0.12.0-p1" STUB_DIALOG="button returned:Install now, gave up:false"
check "  and the release installed" app_says "$T/apps" merged
check "  and its lock released" test ! -e "$LOCK"

# Merge and update always asks, also when PdfCraft isn't running (hours may have passed).
fresh
check "asks even when PdfCraft isn't running, and Later installs nothing" run await-closed-later ok "$T/apps" \
  "${AWAIT[@]}" STUB_LATEST_SEQ="v0.12.0-p1" STUB_DIALOG="button returned:Later" PDFCRAFT_NO_OPEN=0
check "  the question was asked" grep -q '^osascript .*display dialog' "$T/log"
check "  the old copy stays" app_says "$T/apps" old
check "  and nothing was opened" bash -c "! grep -q '^open' '$T/log'"
fresh
check "  Install now installs it and opens it" run await-closed-now ok "$T/apps" "${AWAIT[@]}" \
  STUB_LATEST_SEQ="v0.12.0-p1" STUB_DIALOG="button returned:Install now, gave up:false" PDFCRAFT_NO_OPEN=0
check "  the merged release is in place" app_says "$T/apps" merged
check "  and opened" grep -q "^open $T/apps/PdfCraft.app\$" "$T/log"

# Install and relaunch while Merge and update's installer holds the lock: it waits its turn.
fresh
sleep 3 &
holder=$!
ln -sfn "$holder" "$LOCK"
check "Install and relaunch waits for the installer holding the lock" run relaunch-waits ok "$T/apps" \
  PDFCRAFT_WAIT_PID=999999999 PDFCRAFT_WAIT_SECS=20
check "  then installs" app_says "$T/apps" new
wait "$holder" 2>/dev/null || true
fresh
sleep 30 &
holder=$!
ln -sfn "$holder" "$LOCK"
check "  and gives up when it never finishes" run relaunch-locked refuse "$T/apps" PDFCRAFT_WAIT_PID=999999999 \
  PDFCRAFT_WAIT_SECS=1 PDFCRAFT_NO_OPEN=0
check "  keeping the old copy" app_says "$T/apps" old
check "  saying why" grep -q '^osascript .*display alert' "$T/log"
check "  and reopening it" grep -q "^open $T/apps/PdfCraft.app\$" "$T/log"
kill "$holder" 2>/dev/null || true
wait "$holder" 2>/dev/null || true
rm -f "$LOCK"

# A release another installer already put in place is not installed twice.
rm -rf "$T/apps" && mkdir -p "$T/apps" && mkapp "$T/apps" current "$V"
check "the installed release is not installed again" run already ok "$T/apps" PDFCRAFT_WAIT_PID=999999999 PDFCRAFT_NO_OPEN=0
check "  nothing is copied" app_says "$T/apps" current
check "  and PdfCraft is reopened" grep -q "^open $T/apps/PdfCraft.app\$" "$T/log"
rm -rf "$T/apps" && mkdir -p "$T/apps" && mkapp "$T/apps" current 0.12.0-p1
check "Merge and update doesn't ask about a release already installed" run await-already ok "$T/apps" \
  "${AWAIT[@]}" STUB_LATEST_SEQ="v0.12.0-p1" STUB_DIALOG="button returned:Install now, gave up:false"
check "  asking nothing" bash -c "! grep -q 'display dialog' '$T/log'"
check "  and copying nothing" app_says "$T/apps" current

# Install now gives PdfCraft as long as Install and relaunch does to quit, not a fixed minute.
fresh
touch "$T/running"
check "Install now waits for a slow quit (a Save panel)" run await-slow-quit ok "$T/apps" "${AWAIT[@]}" \
  STUB_LATEST_SEQ="v0.12.0-p1" STUB_RUNNING="$T/running" STUB_DIALOG="button returned:Install now, gave up:false" \
  STUB_QUIT_DELAY=3 PDFCRAFT_WAIT_SECS=30
check "  then installs" app_says "$T/apps" merged
fresh
touch "$T/running"
started=$SECONDS
check "  and gives up after PDFCRAFT_WAIT_SECS when it never quits" run await-never-quits refuse "$T/apps" \
  "${AWAIT[@]}" STUB_LATEST_SEQ="v0.12.0-p1" STUB_RUNNING="$T/running" \
  STUB_DIALOG="button returned:Install now, gave up:false" STUB_QUIT_DELAY=never PDFCRAFT_WAIT_SECS=2
check "  within seconds, not a minute" test $((SECONDS - started)) -lt 30
check "  keeping the old copy" app_says "$T/apps" old
rm -f "$T/running"

fresh
check "refuses PDFCRAFT_AWAIT_BASE together with PDFCRAFT_WAIT_PID" run await-pid refuse "$T/apps" \
  "${AWAIT[@]}" PDFCRAFT_WAIT_PID=1 STUB_LATEST_SEQ="v0.12.0-p1"
check "  keeping the old copy" app_says "$T/apps" old

echo "install.sh: $pass passed, $fail failed"
[ "$fail" = 0 ]
