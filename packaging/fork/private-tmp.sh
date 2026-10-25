#!/usr/bin/env bash
# Run a command with its own empty temp directory, removed afterwards whatever happens (the
# command failing, or the job being cancelled), and exit with the command's status.
#
# The test suite leaves about 300 folders in the temp directory on every run: upstream's tests make
# them under std::env::temp_dir() and do not remove them. On the host that runs CI, 58,000 of them
# filled /tmp's 1,048,576 inodes, and later runs failed with "No space left on device" with the
# disk half empty. Fixing each test would mean changing upstream's tests in the fork, and again for
# every new one a sync brings; this contains them instead. The directory is under /tmp because
# qpdf, an oracle some tests run, may not read files anywhere else.
#
# Usage: packaging/fork/private-tmp.sh <command> [arguments...]
# Env: PRIVATE_TMP_PARENT (default /tmp) is where the directory is made.
set -uo pipefail
[ $# -gt 0 ] || { echo "usage: private-tmp.sh <command> [arguments...]" >&2; exit 2; }
dir="$(mktemp -d "${PRIVATE_TMP_PARENT:-/tmp}/pdfcraft-tests.XXXXXX")" || exit 1
trap 'rm -rf "$dir"' EXIT
# A cancelled job (SIGTERM, SIGINT) still removes it: the EXIT trap runs on the way out.
trap 'exit 143' TERM
trap 'exit 130' INT
TMPDIR="$dir" "$@"
status=$?
left=$(find "$dir" -mindepth 1 -maxdepth 1 | wc -l)
if [ "$left" -gt 0 ]; then
  echo "private-tmp: removed the $left entries the command left in its temp directory" >&2
fi
exit "$status"
