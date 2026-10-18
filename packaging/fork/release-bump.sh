#!/usr/bin/env bash
# After a merge to main changes the fork since its last release, put out the next patch release:
# <base>-p<N> becomes <base>-p<N+1> (the fork's own line, beside upstream's version; CLAUDE.md),
# on branch release/v<next>, with its record listing the PRs merged since the last release. The PR
# is merged once its checks pass, and that merge publishes the release (release.yml).
#
# Run by Release's bump job when a push to main finds its version already tagged. Pushes and merges
# with the owner's gh login on the runner host (lib.sh). Env: BUMP_DRY_RUN=1 stops before pushing.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
REPO=krichelj/pdfcraft
# shellcheck source=lib.sh
. "$ROOT/packaging/fork/lib.sh"

git config user.name >/dev/null || git config user.name "krichelj"
git config user.email >/dev/null || git config user.email "krichelj@users.noreply.github.com"
git fetch --quiet --tags origin main
git checkout --quiet -B bump-base origin/main
current=$(cargo xtask version 2>/dev/null | tail -1)
tag="v$current"
if ! git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null; then
  echo "bump: $current isn't released yet; Release publishes it"
  exit 0
fi
if git diff --quiet "$tag" HEAD -- . ':(exclude)docs/fork/releases'; then
  echo "bump: nothing changed since $tag"
  exit 0
fi
# Behind a newer upstream release, the next release is the sync's <its version>-p1, not -p<N+1>.
if [ "${BUMP_SKIP_UPSTREAM_CHECK:-}" != 1 ] && ! cargo xtask version check-upstream >/dev/null 2>&1; then
  echo "bump: upstream released a newer version; the sync releases it as <version>-p1"
  exit 0
fi
level="${current##*-p}"
case "$level" in "" | *[!0-9]*) die "the fork version $current has no -p<N>" ;; esac
next="${current%%-*}-p$((level + 1))"
branch="release/v$next"
echo "bump: changes since $tag; next release $next on $branch"

pr=$(gh pr list --repo "$REPO" --head "$branch" --state open --json number --jq '.[0].number // empty')
if [ -z "$pr" ]; then
  git checkout --quiet -B "$branch"
  cargo xtask version set "$next" >/dev/null
  cargo xtask fork-notes generate "$next" --commit HEAD >/dev/null
  record="docs/fork/releases/v$next.md"
  summary="$(mktemp)"
  # One line per PR merged since the last release (its title); direct commits by their subject.
  for c in $(git rev-list --first-parent --reverse "$tag..HEAD"); do
    subject=$(git log -1 --format=%s "$c")
    case "$subject" in
      "Merge pull request #"*)
        n="${subject#Merge pull request #}"
        n="${n%% *}"
        title=$(git log -1 --format=%b "$c" | sed -n '1{s/^[[:space:]]*//;p}')
        echo "- ${title:-$subject} (#$n)" ;;
      *) echo "- $subject" ;;
    esac
  done | sed 's/TODO/to-do/g' >"$summary"
  [ -s "$summary" ] || echo "- Fork changes since $tag." >"$summary"
  fill_summary "$record" "$summary"
  rm -f "$summary"
  git add -A Cargo.toml Cargo.lock "$record"
  git commit --quiet -m "Release: PdfCraft $next

The fork's changes since $tag; the record is $record.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
  cargo xtask fork-notes check "$next" --commit HEAD >/dev/null
  if [ "${BUMP_DRY_RUN:-}" = 1 ]; then
    echo "bump: dry run; stopping before the push ($branch)"
    exit 0
  fi
  push_as_owner "$branch"
  pr=$(gh pr create --repo "$REPO" --base main --head "$branch" --title "Release: PdfCraft $next" \
    --body-file "$record" | sed -n 's#.*/pull/\([0-9]*\)$#\1#p')
  [ -n "$pr" ] || die "couldn't open the PR"
  echo "bump: opened PR #$pr"
fi
merge_when_green "$pr" "${BUMP_CHECKS_TIMEOUT:-7200}"
echo "bump: merged PR #$pr; the push to main publishes PdfCraft $next"
