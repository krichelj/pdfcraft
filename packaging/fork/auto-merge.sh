#!/usr/bin/env bash
# Merge a PR once every check on its head has passed (Auto-merge workflow). The merge deletes the
# branch; a failing check leaves the PR open for a person, and this run red. Pushes and merges with the owner's gh
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
state=$(gh pr view "$pr" --repo "$REPO" --json state --jq .state)
if [ "$state" != OPEN ]; then
  echo "auto-merge: PR #$pr is $state already; nothing to do"
  exit 0
fi
# AUTO_MERGE_EVENT=workflow_run: a check just finished (a re-run that now passes, say). Merge if every
# check has passed now; otherwise leave the PR to its own run, which waits and reports a failing check.
# This run never waits and never fails for a check, so it never marks a commit red.
if [ "${AUTO_MERGE_EVENT:-}" = workflow_run ]; then
  rc=0
  wait_for_checks "$pr" 0 || rc=$?
  if [ "$rc" != 0 ]; then
    echo "auto-merge: PR #$pr's checks haven't all passed; its own run decides"
    exit 0
  fi
  merge_pr "$pr"
else
  merge_when_green "$pr" "${AUTO_MERGE_TIMEOUT:-7200}"
fi
echo "auto-merge: merged PR #$pr"
