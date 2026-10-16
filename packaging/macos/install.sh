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
# Env: PDFCRAFT_VERSION    a release such as 0.8.0-p1 (default: the latest)
#      PDFCRAFT_APPS       where PdfCraft.app goes (default: /Applications)
#      PDFCRAFT_NO_OPEN    set to 1 to not open the app afterwards
#      PDFCRAFT_WAIT_PID   wait for this process to exit instead of quitting PdfCraft
#      PDFCRAFT_WAIT_SECS  how long to wait for it (default 3600)
set -euo pipefail
REPO=krichelj/pdfcraft
APPS="${PDFCRAFT_APPS:-/Applications}"
WAIT_PID="${PDFCRAFT_WAIT_PID:-}"
die() {
  echo "error: $*" >&2
  if [ -n "$WAIT_PID" ]; then
    # Started by the app, which is quitting or gone: say why, and bring back what is installed.
    osascript -e 'on run argv' -e 'display alert "PdfCraft could not update" message (item 1 of argv)' -e 'end run' "$*" \
      >/dev/null 2>&1 || true
    if [ "${PDFCRAFT_NO_OPEN:-}" != 1 ] && [ -d "$APPS/PdfCraft.app" ]; then open "$APPS/PdfCraft.app" || true; fi
  fi
  exit 1
}
[ -n "$WAIT_PID" ] && echo "==> $(date -u +%FT%TZ) update started by PdfCraft (pid $WAIT_PID)"
case "$WAIT_PID" in
  "" | *[!0-9]*) [ -z "$WAIT_PID" ] || die "PDFCRAFT_WAIT_PID is not a process id: $WAIT_PID" ;;
esac

[ "$(uname -s)" = Darwin ] || die "this installs the macOS app; run it on a Mac"
[ "$(uname -m)" = arm64 ] || die "these builds are for Apple silicon (M1 and later), and this Mac is $(uname -m)"

version="${PDFCRAFT_VERSION:-}"
if [ -z "$version" ]; then
  # The latest release's page redirects to its tag: no API call, so no API rate limit.
  url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")" \
    || die "couldn't reach GitHub"
  version="${url##*/tag/v}"
fi
version="${version#v}"
case "$version" in
  "" | *[!0-9A-Za-z.-]*) die "no PdfCraft release found (got '$version')" ;;
esac

base="https://github.com/$REPO/releases/download/v$version"
zip="pdfcraft-$version-macos-aarch64.zip"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "==> PdfCraft $version: downloading $zip"
curl -fSL --progress-bar -o "$work/$zip" "$base/$zip" || die "couldn't download $base/$zip"
curl -fsSL -o "$work/SHA256SUMS.txt" "$base/SHA256SUMS.txt" || die "couldn't download $base/SHA256SUMS.txt"
want="$(awk -v f="$zip" '$2 == f || $2 == "*" f { print $1 }' "$work/SHA256SUMS.txt")"
[ -n "$want" ] || die "$zip is not listed in SHA256SUMS.txt"
got="$(shasum -a 256 "$work/$zip" | awk '{ print $1 }')"
[ "$got" = "$want" ] || die "$zip doesn't match SHA256SUMS.txt (expected $want, got $got)"

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
elif pgrep -xq PdfCraft; then
  echo "==> Quitting PdfCraft"
  osascript -e 'tell application "PdfCraft" to quit' >/dev/null 2>&1 || true
  for _ in $(seq 1 60); do
    pgrep -xq PdfCraft || break
    sleep 1
  done
  ! pgrep -xq PdfCraft || die "PdfCraft is still running; quit it and run this again"
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
