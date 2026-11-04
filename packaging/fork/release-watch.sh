#!/usr/bin/env bash
# The reliable trigger for syncing a new storytold/pdfcraft release: GitHub starts this repo's
# scheduled workflows hours late (the 06:00 sync has started at 13:38), so a timer on the runner
# host runs this every 5 minutes. When upstream has a vX.Y.Z tag the fork lacks
# (upstream-release.sh) and no Sync upstream run is queued or running, it starts one; that run
# merges the tag. The workflow's own 15-minute schedule stays as the fallback.
#
#   packaging/fork/release-watch.sh            one check (what the timer runs)
#   packaging/fork/release-watch.sh install    install and start the systemd --user timer
#
# Uses the owner's gh login on the host (own-ci-runners). Env: RELEASE_WATCH_DIR is the fork clone
# it checks from (default ~/.cache/pdfcraft-release-watch); SYNC_UPSTREAM_URL and SYNC_FORK_REF as
# upstream-release.sh says (tests).
set -euo pipefail
REPO=krichelj/pdfcraft
DIR="${RELEASE_WATCH_DIR:-$HOME/.cache/pdfcraft-release-watch}"

if [ "${1:-}" = install ]; then
  units="$HOME/.config/systemd/user"
  mkdir -p "$units" "$(dirname "$DIR")"
  self="$DIR/packaging/fork/release-watch.sh"
  [ -d "$DIR/.git" ] || git clone --quiet "https://github.com/$REPO.git" "$DIR"
  cat >"$units/pdfcraft-release-watch.service" <<UNIT
[Unit]
Description=PdfCraft fork: start the upstream sync when storytold/pdfcraft tags a release

[Service]
Type=oneshot
ExecStart=/bin/bash $self
Environment=PATH=$PATH
UNIT
  cat >"$units/pdfcraft-release-watch.timer" <<UNIT
[Unit]
Description=PdfCraft fork: check for a new upstream release every 5 minutes

[Timer]
OnBootSec=2min
OnUnitActiveSec=5min

[Install]
WantedBy=timers.target
UNIT
  systemctl --user daemon-reload
  systemctl --user enable --now pdfcraft-release-watch.timer
  exit 0
fi

# The clone follows the fork's main, so the watcher's own code updates with the fork.
if [ -z "${SYNC_FORK_REF:-}" ]; then
  git -C "$DIR" fetch --quiet origin main
  git -C "$DIR" checkout --quiet --detach origin/main
fi
tag=$("$DIR/packaging/fork/upstream-release.sh")
if [ -z "$tag" ]; then exit 0; fi
busy=$(gh run list --repo "$REPO" --workflow sync-upstream.yml --json status \
  --jq '[.[] | select(.status != "completed")] | length')
if [ "$busy" != 0 ]; then
  echo "release-watch: $tag is new; a Sync upstream run is already queued or running"
  exit 0
fi
gh workflow run sync-upstream.yml --repo "$REPO" --ref main
echo "release-watch: $tag is new; started Sync upstream"
