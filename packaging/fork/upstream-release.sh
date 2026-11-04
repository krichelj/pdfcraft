#!/usr/bin/env bash
# Print storytold/pdfcraft's newest release tag (vX.Y.Z, no pre-release suffix) when the fork does
# not have its commit yet, and nothing when it does: the Sync upstream workflow then merges that
# tag (sync-upstream.sh with SYNC_UPSTREAM_TAG). Only the newest tag matters, since merging it
# brings every older one.
#
# Env: SYNC_UPSTREAM_URL reads another repository's tags, and SYNC_FORK_REF compares against that
#      commit instead of origin/main (tests).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
UPSTREAM_URL="${SYNC_UPSTREAM_URL:-https://github.com/storytold/pdfcraft.git}"
tag=$(git ls-remote --tags --refs "$UPSTREAM_URL" | awk '{print $2}' | sed 's#^refs/tags/##' \
  | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -n 1 || true)
[ -n "$tag" ] || exit 0
if [ -z "${SYNC_FORK_REF:-}" ]; then git fetch --quiet origin main; fi
git fetch --quiet --no-tags "$UPSTREAM_URL" "+refs/tags/$tag:refs/fork-notes/release"
commit=$(git rev-parse "refs/fork-notes/release^{commit}")
if git merge-base --is-ancestor "$commit" "${SYNC_FORK_REF:-origin/main}"; then
  echo "release: the fork has $tag (${commit:0:12})" >&2
  exit 0
fi
echo "release: the fork lacks $tag (${commit:0:12})" >&2
echo "$tag"
