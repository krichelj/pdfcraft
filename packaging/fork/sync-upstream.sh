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
# shellcheck source=lib.sh
. "$ROOT/packaging/fork/lib.sh"

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
        # Only catalog hashes differ on both sides; they are recomputed below for the merged files.
        ATTRIBUTION.toml) git checkout --theirs -- "$f" && git add "$f" ;;
        # A catalog is one string per line: both sides' new lines are kept (a duplicate key fails
        # the i18n tests in the PR's CI, which then stays open for a person).
        crates/ui-egui/src/i18n/*.tsv)
          git show ":2:$f" >"$f.ours" && git show ":1:$f" >"$f.base" && git show ":3:$f" >"$f.theirs"
          git merge-file --union -p "$f.ours" "$f.base" "$f.theirs" >"$f"
          rm -f "$f.ours" "$f.base" "$f.theirs"
          git add "$f"
          ;;
        .github/workflows/*)
          case " $FORK_WORKFLOWS " in
            *" ${f##*/} "*) git checkout --ours -- "$f" && git add "$f" ;;
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
  # The merged catalogs' hashes, then every asset must check out (AGENTS.md §1), or a person decides.
  rehash_catalogs
  cargo xtask assets >/dev/null 2>&1 || die "the merged assets don't match ATTRIBUTION.toml: run cargo xtask assets"
  if ! git diff --quiet ATTRIBUTION.toml; then
    git add ATTRIBUTION.toml
    git commit --quiet -m "Fork: record the merged translation catalogs' hashes

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
  fi
  # The fork's own workflows are the fork's: an upstream edit to one is not taken, even when it
  # merges cleanly, and the record says so.
  kept=""
  for w in $FORK_WORKFLOWS; do
    if ! git diff --quiet sync-base HEAD -- ".github/workflows/$w"; then
      git checkout sync-base -- ".github/workflows/$w"
      kept="$kept $w"
    fi
  done
  if [ -n "$kept" ]; then
    git commit --quiet -m "Fork: keep the fork's own workflows (upstream changed:$kept)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
    echo "sync: kept the fork's version of$kept"
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
    if [ -n "$kept" ]; then echo "- Keeps the fork's own workflows; upstream's changes to$kept were not taken."; fi
  } >"$summary"
  fill_summary "$record" "$summary"
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
  push_as_owner "$branch"
  pr=$(gh pr create --repo "$REPO" --base main --head "$branch" \
    --title "Sync upstream: storytold/pdfcraft@${up:0:12} (PdfCraft $version)" \
    --body-file "$record" | sed -n 's#.*/pull/\([0-9]*\)$#\1#p')
  [ -n "$pr" ] || die "couldn't open the PR"
  echo "sync: opened PR #$pr"
fi

merge_when_green "$pr" "${SYNC_CHECKS_TIMEOUT:-7200}"
echo "sync: merged PR #$pr; the push to main starts the Release workflow"
