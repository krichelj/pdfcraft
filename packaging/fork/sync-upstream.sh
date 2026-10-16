#!/usr/bin/env bash
# Bring storytold/pdfcraft's new commits into this fork through a PR, and merge it once CI passes.
#
#   1. Fetch upstream main; stop when the fork already has every commit.
#   2. On branch sync/upstream-<upstream sha>: merge it, keep the fork's rules (CLAUDE.md): only the
#      fork's workflows stay, and Cargo.toml / Cargo.lock conflicts take upstream's and re-apply the
#      fork version. Any other conflict stops the sync with the files listed.
#   3. Bump the version: <base>-p<N+1>, or <new upstream base>-p1 when upstream's version moved,
#      and write the release record (cargo xtask fork-notes) listing the upstream commits it brings.
#   4. Push and open the PR, wait for its checks, merge it (the branch is deleted on merge).
#      The push to main starts the Release workflow, which builds and publishes the DMG.
#
# Runs in the Sync upstream workflow with the owner's gh login on the runner host: commits made
# with the workflow's own token can't touch workflow files, and don't start other workflows.
# Env: SYNC_DRY_RUN=1 stops before pushing; SYNC_CHECKS_TIMEOUT (seconds, default 7200);
#      SYNC_FORK_REF syncs from that commit instead of origin/main, and SYNC_UPSTREAM_URL from another
#      repository's main instead of storytold/pdfcraft's (tests).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
UPSTREAM_URL="${SYNC_UPSTREAM_URL:-https://github.com/storytold/pdfcraft.git}"
UPSTREAM_REF=refs/fork-notes/upstream
REPO=krichelj/pdfcraft
# The fork's workflows; any other one an upstream merge brings is deleted (CLAUDE.md fork rules).
FORK_WORKFLOWS="ci.yml own-runners-guard.yml packaging-lint.yml release.yml sync-upstream.yml"
die() {
  echo "::error::$*" >&2
  exit 1
}

git config user.name >/dev/null || git config user.name "krichelj"
git config user.email >/dev/null || git config user.email "krichelj@users.noreply.github.com"
if [ -z "${SYNC_FORK_REF:-}" ]; then git fetch --quiet origin main; fi
git checkout --quiet -B sync-base "${SYNC_FORK_REF:-origin/main}"
git fetch --quiet "$UPSTREAM_URL" "+refs/heads/main:$UPSTREAM_REF"
new=$(git rev-list --count HEAD.."$UPSTREAM_REF")
if [ "$new" = 0 ]; then
  echo "sync: the fork has every storytold/pdfcraft commit; nothing to do"
  exit 0
fi
up=$(git rev-parse "$UPSTREAM_REF")
branch="sync/upstream-${up:0:12}"
echo "sync: $new new upstream commits, up to ${up:0:12}; branch $branch"

pr=$(gh pr list --repo "$REPO" --head "$branch" --state open --json number --jq '.[0].number // empty')
if [ -z "$pr" ]; then
  current=$(cargo xtask version 2>/dev/null | tail -1)
  git checkout --quiet -B "$branch"
  if ! git merge --no-edit --no-ff -m "Merge storytold/pdfcraft main (${up:0:12})" "$UPSTREAM_REF" >/dev/null 2>&1; then
    for f in $(git diff --name-only --diff-filter=U); do
      case "$f" in
        Cargo.toml | Cargo.lock) git checkout --theirs -- "$f" && git add "$f" ;;
        .github/workflows/*)
          case " $FORK_WORKFLOWS " in
            *" ${f##*/} "*) die "upstream changed the fork's own workflow $f; resolve it by hand" ;;
            *) git rm --quiet -f -- "$f" ;;
          esac
          ;;
        *) ;;
      esac
    done
    left=$(git diff --name-only --diff-filter=U)
    [ -z "$left" ] || die "upstream merge conflicts that need a person: $(echo "$left" | tr '\n' ' ')"
    git -c core.editor=true commit --quiet --no-edit
  fi
  # Only the fork's workflows stay (upstream adds ones for other platforms and GitHub's runners).
  for w in .github/workflows/*.yml; do
    case " $FORK_WORKFLOWS " in *" ${w##*/} "*) ;; *) git rm --quiet -- "$w" && echo "sync: removed upstream workflow $w" ;; esac
  done

  base_now="${current%%-*}"
  base_up=$(git show "$UPSTREAM_REF:Cargo.toml" | sed -n 's/^version = "\([^"]*\)".*/\1/p' | head -1)
  base_up="${base_up%%-*}"
  if [ "$base_up" = "$base_now" ]; then
    level="${current##*-p}"
    case "$level" in "" | *[!0-9]*) die "the fork version $current has no -p<N>" ;; esac
    version="$base_now-p$((level + 1))"
  else
    version="$base_up-p1"
  fi
  cargo xtask version set "$version" >/dev/null
  echo "sync: version $current -> $version"

  cargo xtask fork-notes generate "$version" --commit HEAD --upstream "$UPSTREAM_REF" >/dev/null
  record="docs/fork/releases/v$version.md"
  count=$(git rev-list --no-merges --count "$UPSTREAM_REF" --not sync-base)
  summary="$(mktemp)"
  {
    echo "- Brings in $count upstream storytold/pdfcraft commits, up to [\`${up:0:12}\`](https://github.com/storytold/pdfcraft/commit/$up):"
    git log --no-merges --format='%s' "$UPSTREAM_REF" --not sync-base | head -40 | sed -e 's/^/  - /' -e 's/TODO/to-do/g'
    if [ "$count" -gt 40 ]; then echo "  - and $((count - 40)) more"; fi
  } >"$summary"
  awk -v f="$summary" '/^- TODO: / { while ((getline l < f) > 0) print l; next } { print }' "$record" >"$record.new"
  mv "$record.new" "$record"
  rm -f "$summary"
  git add -A Cargo.toml Cargo.lock "$record"
  git commit --quiet -m "Release: PdfCraft $version (upstream ${up:0:12})

Merges $count storytold/pdfcraft commits; the record is $record.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
  cargo xtask fork-notes check "$version" --commit HEAD --upstream "$UPSTREAM_REF" >/dev/null

  if [ "${SYNC_DRY_RUN:-}" = 1 ]; then
    echo "sync: dry run; stopping before the push (branch $branch, version $version)"
    exit 0
  fi
  git -c credential.helper= -c credential.helper='!gh auth git-credential' push --quiet -u origin "$branch"
  pr=$(gh pr create --repo "$REPO" --base main --head "$branch" \
    --title "Sync upstream: storytold/pdfcraft@${up:0:12} (PdfCraft $version)" \
    --body-file "$record" | sed -n 's#.*/pull/\([0-9]*\)$#\1#p')
  [ -n "$pr" ] || die "couldn't open the PR"
  echo "sync: opened PR #$pr"
fi

# Wait for every check on the PR's head, then merge; a failing check leaves the PR open. CI's test
# job must be among them, so a PR whose checks haven't started yet is never "green".
head=$(gh pr view "$pr" --repo "$REPO" --json headRefOid --jq .headRefOid)
deadline=$(($(date +%s) + ${SYNC_CHECKS_TIMEOUT:-7200}))
while :; do
  # shellcheck disable=SC2016 # a jq program, not shell
  state=$(gh api "repos/$REPO/commits/$head/check-runs?per_page=100" --jq '
    [.check_runs[] | {name, status, conclusion}] as $r
    | if ($r | map(select(.conclusion == "failure" or .conclusion == "cancelled" or .conclusion == "timed_out" or .conclusion == "action_required")) | length) > 0 then "FAILURE"
      elif ($r | map(select(.name == "test (shared code)" and .conclusion == "success")) | length) > 0
        and ($r | all(.status == "completed")) then "SUCCESS"
      else "PENDING" end' 2>/dev/null || echo PENDING)
  case "$state" in
    SUCCESS) break ;;
    FAILURE) die "PR #$pr has failing checks; left open for a person" ;;
  esac
  [ "$(date +%s)" -lt "$deadline" ] || die "PR #$pr's checks didn't finish in time; left open"
  sleep 30
done
gh pr merge "$pr" --repo "$REPO" --merge --delete-branch
echo "sync: merged PR #$pr; the push to main starts the Release workflow"
