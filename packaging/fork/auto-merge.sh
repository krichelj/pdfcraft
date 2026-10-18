#!/usr/bin/env bash
# Merge a PR once every check on its head has passed (Auto-merge workflow). The merge deletes the
# branch; a failing check leaves the PR open for a person. Pushes and merges with the owner's gh
# login on the runner host (lib.sh), so the merge starts Release like any other.
# Usage: packaging/fork/auto-merge.sh <pr number>
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPO=krichelj/pdfcraft
# shellcheck source=lib.sh
. "$ROOT/packaging/fork/lib.sh"
pr="${1:?usage: auto-merge.sh <pr number>}"
case "$pr" in *[!0-9]*) die "not a PR number: $pr" ;; esac
export MERGE_IGNORE_CHECK="${MERGE_IGNORE_CHECK:-merge when green}"
merge_when_green "$pr" "${AUTO_MERGE_TIMEOUT:-7200}"
echo "auto-merge: merged PR #$pr"
