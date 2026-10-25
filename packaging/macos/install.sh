#!/usr/bin/env bash
# Install or update PdfCraft on a Mac with Apple silicon, from krichelj/pdfcraft's latest release:
#
#   curl -fsSL https://raw.githubusercontent.com/krichelj/pdfcraft/main/packaging/macos/install.sh | bash
#
# The fork's builds are ad-hoc signed, not notarized (notarizing needs a paid Apple Developer ID).
# A browser marks what it downloads with the quarantine attribute, and Gatekeeper refuses to open a
# quarantined app Apple hasn't notarized ("Apple could not verify ..."). curl marks nothing, so the
# copy installed here opens directly. The download is checked against the release's
# SHA256SUMS.txt and the app's signature before anything in the Applications folder is replaced.
#
# PdfCraft's Help > Check for updates > Install and relaunch runs the copy inside the app bundle
# (Contents/Resources/install.sh) with PDFCRAFT_WAIT_PID set to the app's process: everything is
# downloaded and checked while the app quits, the swap waits until it has, and a failure is shown
# in an alert and the installed version opened again.
#
# Only this fork's builds are installed: the original PdfCraft's would replace the fork's changes.
# When the original has a newer release, Check for updates > Merge and update starts the fork's
# Sync upstream workflow (which merges it into the fork, tests it and releases <its version>-p1)
# and runs this script with PDFCRAFT_AWAIT_BASE set to that version: it waits until the fork's
# latest release is built on it, then always asks whether to install it now (Later, or no answer
# within an hour, leaves everything as it is; Check for updates offers the release afterwards).
#
# One installer runs at a time, whatever started it: a lock (a symbolic link to the installer's
# process id, created atomically) is held from the wait for a release, or from the swap, to the
# end. An installer that finds the release already installed does nothing more.
#
# Env: PDFCRAFT_VERSION      a release such as 0.8.0-p1 (default: the latest)
#      PDFCRAFT_APPS         where PdfCraft.app goes (default: /Applications)
#      PDFCRAFT_NO_OPEN      set to 1 to not open the app afterwards
#      PDFCRAFT_WAIT_PID     wait for this process to exit instead of quitting PdfCraft
#      PDFCRAFT_WAIT_SECS    how long to wait for it (default 3600)
#      PDFCRAFT_AWAIT_BASE   wait for the fork's latest release to be built on this version (X.Y.Z)
#      PDFCRAFT_AWAIT_SECS   how long to wait for that release (default 21600, six hours)
#      PDFCRAFT_AWAIT_POLL   seconds between looks at the latest release (default 60)
#      PDFCRAFT_LOCK         the installers' lock (default: $TMPDIR/pdfcraft-installer.lock, per user)
set -euo pipefail
REPO=krichelj/pdfcraft
APPS="${PDFCRAFT_APPS:-/Applications}"
WAIT_PID="${PDFCRAFT_WAIT_PID:-}"
AWAIT_BASE="${PDFCRAFT_AWAIT_BASE:-}"
LOCK="${PDFCRAFT_LOCK:-${TMPDIR:-/tmp}/pdfcraft-installer.lock}"
own_lock=""
work=""
trap '[ -z "$work" ] || rm -rf "$work"; [ -z "$own_lock" ] || rm -f "$LOCK"' EXIT
die() {
  echo "error: $*" >&2
  if [ -n "$WAIT_PID" ] || [ -n "$AWAIT_BASE" ]; then
    # Started by the app, which may be quitting, gone or busy with something else: say why.
    osascript -e 'on run argv' -e 'display alert "PdfCraft could not update" message (item 1 of argv)' -e 'end run' "$*" \
      >/dev/null 2>&1 || true
  fi
  if [ -n "$WAIT_PID" ]; then
    # The app quit for this update: bring back what is installed.
    if [ "${PDFCRAFT_NO_OPEN:-}" != 1 ] && [ -d "$APPS/PdfCraft.app" ]; then open "$APPS/PdfCraft.app" || true; fi
  fi
  exit 1
}
[ -n "$WAIT_PID" ] && echo "==> $(date -u +%FT%TZ) update started by PdfCraft (pid $WAIT_PID)"
case "$WAIT_PID" in
  "" | *[!0-9]*) [ -z "$WAIT_PID" ] || die "PDFCRAFT_WAIT_PID is not a process id: $WAIT_PID" ;;
esac

# X.Y.Z without leading zeros and at most nine digits a part, so base_at_least can't overflow.
is_base() { [[ "$1" =~ ^(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})$ ]]; }
# Whether version core $1 is at least $2, comparing X, then Y, then Z as numbers (0.12.0 > 0.9.9).
base_at_least() {
  local -a a b
  local i
  IFS=. read -r -a a <<<"$1"
  IFS=. read -r -a b <<<"$2"
  for i in 0 1 2; do
    if ((a[i] > b[i])); then return 0; fi
    if ((a[i] < b[i])); then return 1; fi
  done
  return 0
}
if [ -n "$AWAIT_BASE" ]; then
  AWAIT_BASE="${AWAIT_BASE#v}"
  is_base "$AWAIT_BASE" || die "PDFCRAFT_AWAIT_BASE is not a version like 0.12.0: $PDFCRAFT_AWAIT_BASE"
  [ -z "$WAIT_PID" ] || die "PDFCRAFT_AWAIT_BASE and PDFCRAFT_WAIT_PID can't be used together"
  [ -z "${PDFCRAFT_VERSION:-}" ] || die "PDFCRAFT_AWAIT_BASE waits for the latest release; don't set PDFCRAFT_VERSION too"
  echo "==> $(date -u +%FT%TZ) waiting for the fork's release built on PdfCraft $AWAIT_BASE"
fi

[ "$(uname -s)" = Darwin ] || die "this installs the macOS app; run it on a Mac"
[ "$(uname -m)" = arm64 ] || die "these builds are for Apple silicon (M1 and later), and this Mac is $(uname -m)"

# The live installer holding the lock, or nothing.
lock_holder() {
  local pid
  pid="$(readlink "$LOCK" 2>/dev/null || true)"
  case "$pid" in "" | *[!0-9]*) return 0 ;; esac
  if kill -0 "$pid" 2>/dev/null; then echo "$pid"; fi
}
# Usage: take_lock <seconds to wait for another installer>. Fails when it is still held after that.
take_lock() {
  local waited=0 stale
  # Anything but our kind of lock there (a directory, a file) is not ours to remove.
  if [ -e "$LOCK" ] && [ ! -L "$LOCK" ]; then return 1; fi
  # -n: never follow an existing link (into a directory it would create the link inside it).
  until ln -sn "$$" "$LOCK" 2>/dev/null; do
    [ -L "$LOCK" ] || return 1
    if [ -z "$(lock_holder)" ]; then
      # Its installer died: take the lock over, unless another installer just did.
      stale="$(readlink "$LOCK" 2>/dev/null || true)"
      if [ "$(readlink "$LOCK" 2>/dev/null || true)" = "$stale" ]; then rm -f "$LOCK" || return 1; fi
      continue
    fi
    [ "$waited" -lt "$1" ] || return 1
    sleep 1
    waited=$((waited + 1))
  done
  own_lock=1
}
# The version of the PdfCraft installed in $APPS (its Info.plist's PdfCraftVersion), or nothing.
installed_version() {
  plutil -extract PdfCraftVersion raw -o - "$APPS/PdfCraft.app/Contents/Info.plist" 2>/dev/null || true
}

# The latest release's page redirects to its tag: no API call, so no API rate limit.
latest() {
  local url
  url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")" || return 1
  printf '%s\n' "${url##*/tag/v}"
}

version="${PDFCRAFT_VERSION:-}"
if [ -n "$AWAIT_BASE" ]; then
  # A second Merge and update, or one while an update installs, adds nothing.
  if ! take_lock 0; then
    echo "==> another installer (pid $(lock_holder)) is already running; nothing to do"
    exit 0
  fi
  poll="${PDFCRAFT_AWAIT_POLL:-60}"
  case "$poll" in "" | *[!0-9]*) die "PDFCRAFT_AWAIT_POLL is not a number of seconds: $poll" ;; esac
  waited=0
  while :; do
    # A failed look (no network for a moment) counts as a release that isn't out yet.
    candidate="$(latest || true)"
    if is_base "${candidate%%-*}" && base_at_least "${candidate%%-*}" "$AWAIT_BASE"; then
      version="$candidate"
      echo "==> the fork's release $version is built on PdfCraft $AWAIT_BASE"
      break
    fi
    [ "$waited" -lt "${PDFCRAFT_AWAIT_SECS:-21600}" ] \
      || die "the fork's release built on PdfCraft $AWAIT_BASE didn't appear in time (latest: ${candidate:-none}); Check for updates offers it once it is out"
    sleep "$poll"
    waited=$((waited + (poll > 0 ? poll : 1)))
  done
elif [ -z "$version" ]; then
  version="$(latest)" || die "couldn't reach GitHub"
fi
version="${version#v}"
case "$version" in
  "" | *[!0-9A-Za-z.-]*) die "no PdfCraft release found (got '$version')" ;;
esac

base="https://github.com/$REPO/releases/download/v$version"
zip="pdfcraft-$version-macos-aarch64.zip"
work="$(mktemp -d)"

echo "==> PdfCraft $version: downloading $zip"
curl -fsSL -o "$work/SHA256SUMS.txt" "$base/SHA256SUMS.txt" || die "couldn't download $base/SHA256SUMS.txt"
curl -fSL --progress-bar -o "$work/$zip" "$base/$zip" || die "couldn't download $base/$zip"
want="$(awk -v f="$zip" '$2 == f || $2 == "*" f { print $1 }' "$work/SHA256SUMS.txt")"
[ -n "$want" ] || die "SHA256SUMS.txt doesn't list $zip"
got="$(shasum -a 256 "$work/$zip" | awk '{ print $1 }')"
[ "$got" = "$want" ] || die "$zip doesn't match SHA256SUMS.txt (expected $want, got $got)"

mkdir -p "$work/unpacked"
ditto -x -k "$work/$zip" "$work/unpacked" || die "couldn't unpack $zip"
new="$work/unpacked/PdfCraft.app"
[ -x "$new/Contents/MacOS/PdfCraft" ] || die "$zip holds no PdfCraft.app"
codesign --verify --strict "$new" || die "the signature of PdfCraft.app in $zip doesn't verify"

# Replace a running copy only once it has quit (it asks about unsaved changes first).
if [ -n "$WAIT_PID" ]; then
  echo "==> Waiting for PdfCraft (pid $WAIT_PID) to quit"
  waited=0
  while kill -0 "$WAIT_PID" 2>/dev/null; do
    [ "$waited" -lt "${PDFCRAFT_WAIT_SECS:-3600}" ] || die "PdfCraft didn't quit, so the update to $version wasn't installed"
    sleep 1
    waited=$((waited + 1))
  done
  take_lock "${PDFCRAFT_WAIT_SECS:-3600}" || die "another PdfCraft installer (pid $(lock_holder)) is still running"
elif [ -z "$AWAIT_BASE" ]; then
  take_lock 0 || die "another PdfCraft installer (pid $(lock_holder)) is running; try again once it has finished"
fi

# Another installer may have put this release in place already: then there is nothing to do.
if [ "$(installed_version)" = "$version" ]; then
  echo "==> PdfCraft $version is already installed in $APPS"
  if [ -n "$WAIT_PID" ] && [ "${PDFCRAFT_NO_OPEN:-}" != 1 ] && ! pgrep -xq PdfCraft; then open "$APPS/PdfCraft.app"; fi
  exit 0
fi

if [ -n "$AWAIT_BASE" ]; then
  # Hours may have passed since Merge and update: always ask, and only quit PdfCraft on a yes.
  running=""
  if pgrep -xq PdfCraft; then running=1; fi
  then_what="PdfCraft opens when it is in place."
  [ -z "$running" ] || then_what="PdfCraft quits first, asking about unsaved changes, and reopens."
  # shellcheck disable=SC2016 # AppleScript, not shell
  answer="$(osascript -e 'on run argv' \
    -e 'display dialog ("PdfCraft " & item 1 of argv & " is ready: your fork with PdfCraft " & item 2 of argv & " merged in. Install it now? " & item 3 of argv) buttons {"Later", "Install now"} default button "Install now" with title "Update PdfCraft" giving up after 3600' \
    -e 'end run' "$version" "$AWAIT_BASE" "$then_what" 2>/dev/null || true)"
  case "$answer" in
    *"button returned:Install now"*) ;;
    *)
      echo "==> PdfCraft $version left for later; Check for updates offers it"
      exit 0
      ;;
  esac
fi
if [ -z "$WAIT_PID" ] && pgrep -xq PdfCraft; then
  echo "==> Quitting PdfCraft"
  osascript -e 'tell application "PdfCraft" to quit' >/dev/null 2>&1 || true
  # As long as Install and relaunch waits: the user may be saving work in a Save panel.
  waited=0
  while pgrep -xq PdfCraft; do
    [ "$waited" -lt "${PDFCRAFT_WAIT_SECS:-3600}" ] || die "PdfCraft is still running, so $version wasn't installed; quit it and update again"
    sleep 1
    waited=$((waited + 1))
  done
fi

mkdir -p "$APPS"
[ -w "$APPS" ] || die "can't write to $APPS; run this as an admin user, or set PDFCRAFT_APPS=\$HOME/Applications"
dest="$APPS/PdfCraft.app"
old=""
if [ -e "$dest" ]; then
  old="$APPS/.PdfCraft.app.replaced.$$"
  mv "$dest" "$old"
fi
if ! ditto "$new" "$dest"; then
  rm -rf "$dest"
  [ -z "$old" ] || mv "$old" "$dest"
  die "couldn't copy PdfCraft.app into $APPS; the previous version is still there"
fi
[ -z "$old" ] || rm -rf "$old"
# Nothing here was quarantined; this is only in case the copy picked up a mark anyway.
xattr -dr com.apple.quarantine "$dest" 2>/dev/null || true

echo "==> PdfCraft $version is installed in $dest"
[ "${PDFCRAFT_NO_OPEN:-}" = 1 ] || open "$dest"
