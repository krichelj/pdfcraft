#!/usr/bin/env bash
# Bring storytold/pdfcraft's new commits into this fork through a PR, and merge it once CI passes.
#
#   1. Fetch upstream main; stop when the fork already has every commit.
#   2. On branch sync/upstream-<upstream sha>: merge it, keep the fork's rules (CLAUDE.md): only the
#      fork's workflows stay, and Cargo.toml / Cargo.lock conflicts take upstream's and re-apply the
#      fork version. The on-call LLM (lib.sh) resolves every other conflict; without it, or when it
#      can't, the sync stops with the files listed.
#   3. The on-call LLM runs CI's checks on the merged tree and fixes what fails, one commit per
#      round ("Sync fix: ..."); the sync stops if they still fail.
#   4. Bump the version: <base>-p<N+1>, or <new upstream base>-p1 when upstream's version moved,
#      and write the release record (cargo xtask fork-notes) listing the upstream commits it brings
#      and what the LLM resolved and fixed.
#   5. Push and open the PR, wait for its checks, merge it (the branch is deleted on merge); a
#      failing check goes back to the LLM with its log, at most twice. The push to main starts the
#      Release workflow, which builds and publishes the DMG.
#
# Runs in the Sync upstream workflow with the owner's gh login on the runner host: commits made
# with the workflow's own token can't touch workflow files, and don't start other workflows.
# Env: SYNC_DRY_RUN=1 stops before pushing; SYNC_CHECKS_TIMEOUT (seconds, default 7200);
#      SYNC_NO_LLM=1 leaves conflicts and failing checks to a person; ONCALL_LLM, SYNC_LLM_ROUNDS,
#      SYNC_CI_FIX_RETRIES and SYNC_FIX_CHECKS as lib.sh says;
#      SYNC_FORK_REF syncs from that commit instead of origin/main, and SYNC_UPSTREAM_URL from another
#      repository's main instead of storytold/pdfcraft's (tests).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
UPSTREAM_URL="${SYNC_UPSTREAM_URL:-https://github.com/storytold/pdfcraft.git}"
UPSTREAM_REF=refs/fork-notes/upstream
REPO=krichelj/pdfcraft
# The fork's workflows; any other one an upstream merge brings is deleted (CLAUDE.md fork rules).
FORK_WORKFLOWS="ci.yml own-runners-guard.yml packaging-lint.yml release.yml sync-upstream.yml fork-automation.yml auto-merge.yml"
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
stale=""
if [ -n "$pr" ]; then
  git fetch --quiet origin "$branch"
  pr_head=$(git rev-parse FETCH_HEAD)
  if git merge-base --is-ancestor sync-base "$pr_head"; then
    # An earlier run opened the PR on today's main: work on its head, where a fix for a failing
    # check goes. A dry run pushes nothing, so it stops here.
    if [ "${SYNC_DRY_RUN:-}" = 1 ]; then
      echo "sync: dry run; PR #$pr is open on the current main; stopping before its checks are fixed or it is merged"
      exit 0
    fi
    git checkout --quiet -B "$branch" "$pr_head"
  else
    # The PR predates main's latest commits (a fix to this automation, say): the merge is made again
    # from today's main and replaces the PR's branch.
    echo "sync: PR #$pr is behind main; rebuilding it from the current main"
    stale=1
  fi
fi
if [ -z "$pr" ] || [ -n "$stale" ]; then
  current=$(cargo xtask version 2>/dev/null | tail -1)
  git checkout --quiet -B "$branch"
  took_deny=""
  llm_resolved=""
  RUNNER_TMP="$(mktemp -d)"
  if ! git merge --no-edit --no-ff -m "Merge storytold/pdfcraft main (${up:0:12})" "$UPSTREAM_REF" >/dev/null 2>&1; then
    for f in $(git diff --name-only --diff-filter=U); do
      case "$f" in
        Cargo.toml | Cargo.lock) git checkout --theirs -- "$f" && git add "$f" ;;
        # Only catalog hashes differ on both sides; they are recomputed below for the merged files.
        ATTRIBUTION.toml) git checkout --theirs -- "$f" && git add "$f" ;;
        # The dependency policy: resolved below, once the rest of the tree is merged.
        deny.toml)
          git show ":2:$f" >"$RUNNER_TMP/deny.ours" && git show ":1:$f" >"$RUNNER_TMP/deny.base" && git show ":3:$f" >"$RUNNER_TMP/deny.theirs"
          took_deny=1
          ;;
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
    # deny.toml: upstream's file, else both sides' entries (both usually add advisory ignores);
    # whichever the merged tree passes `cargo deny` with, or a person decides.
    if [ -n "$took_deny" ]; then
      command -v cargo-deny >/dev/null || die "deny.toml conflicted and cargo-deny isn't on the runner: see own-ci-runners"
      cp "$RUNNER_TMP/deny.theirs" deny.toml
      if cargo deny --all-features check >/dev/null 2>&1; then
        echo "sync: took upstream's deny.toml; cargo deny passes"
      else
        git merge-file --union -p "$RUNNER_TMP/deny.ours" "$RUNNER_TMP/deny.base" "$RUNNER_TMP/deny.theirs" >deny.toml || true
        cargo deny --all-features check >/dev/null 2>&1 || die "the merged tree fails cargo deny with either deny.toml; a person decides"
        echo "sync: deny.toml keeps both sides' entries; cargo deny passes"
      fi
      git add deny.toml
    fi
    left=$(git diff --name-only --diff-filter=U)
    # Everything the rules above don't cover goes to the on-call LLM, which stages what it resolves.
    if [ -n "$left" ] && llm_available; then
      echo "sync: the on-call LLM resolves $(echo "$left" | wc -l) conflicted file(s): $(echo "$left" | tr '\n' ' ')"
      if "${ONCALL_LLM:-oncall-llm}" resolve --repo "$ROOT" --instructions "$ROOT/packaging/fork/oncall-instructions.md" \
        --report "$RUNNER_TMP/resolve.json"; then
        llm_resolved="$left"
      else
        echo "sync: the on-call LLM left conflicts unresolved (exit $?)"
      fi
      left=$(git diff --name-only --diff-filter=U)
    fi
    [ -z "$left" ] || die "upstream merge conflicts that need a person: $(echo "$left" | tr '\n' ' ')"
    git -c core.editor=true commit --quiet --no-edit
  fi
  # The merged catalogs' hashes, then every asset must check out (AGENTS.md §1), or a person decides.
  rehash_catalogs
  if ! assets_out=$(cargo xtask assets 2>&1); then
    printf '%s\n' "$assets_out" | tail -20
    # With the LLM, its fix loop runs this same check (and the build it needs: a merge that broke
    # xtask fails here too), so the sync goes on to it; without, a person decides.
    llm_available || die "the merged assets don't check out (cargo xtask assets above)"
    echo "sync: the merged assets don't check out yet; the on-call LLM's fix loop runs that check"
  fi
  # Cargo.lock as cargo writes it for the merged manifests: upstream's lock, taken on a conflict,
  # lacks the fork's own dependencies, and the first build rewrites it; and with every package the
  # fork had newer kept at the fork's version. Committed here, so the fix loop starts from a clean
  # tree (it refuses to commit over uncommitted tracked work).
  cargo metadata --format-version 1 >/dev/null 2>&1 || true
  git show sync-base:Cargo.lock >"$RUNNER_TMP/fork.lock"
  keep_newer_locked "$RUNNER_TMP/fork.lock"
  if ! git diff --quiet Cargo.lock; then
    git add Cargo.lock
    git commit --quiet -m "Fork: Cargo.lock as cargo writes it for the merged manifests

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
  fi
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
  if ! git diff --cached --quiet; then
    git commit --quiet -m "Fork: only the fork's workflows stay

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
  fi

  # The merged tree must pass CI's checks: the on-call LLM fixes what fails, one commit per round.
  if llm_available; then
    echo "sync: the on-call LLM runs CI's checks on the merged tree"
    llm_fix "Sync fix" || die "the merged tree still fails its checks after the on-call LLM's rounds; a person decides"
  else
    echo "sync: no on-call LLM; the PR's CI checks the merged tree"
  fi
  fixes=$(git log --first-parent --format=%s sync-base..HEAD | grep '^Sync fix' || true)

  base_now="${current%%-*}"
  base_up=$(git show "$UPSTREAM_REF:Cargo.toml" | sed -n '/^version = "/{s/^version = "\([^"]*\)".*/\1/p;q;}')
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
    # -n, not `| head`: under pipefail, head leaving early kills git log with SIGPIPE (exit 141).
    git log --no-merges -n 40 --format='%s' "$UPSTREAM_REF" --not sync-base | sed -e 's/^/  - /' -e 's/TODO/to-do/g'
    if [ "$count" -gt 40 ]; then echo "  - and $((count - 40)) more"; fi
    if [ -n "$kept" ]; then echo "- Keeps the fork's own workflows; upstream's changes to$kept were not taken."; fi
    if [ -n "$llm_resolved" ]; then
      files=""
      for f in $llm_resolved; do files="${files:+$files, }\`$f\`"; done
      echo "- The on-call LLM resolved the merge conflicts in: $files."
    fi
    if [ -n "$fixes" ]; then
      echo "- The on-call LLM fixed what the merge broke, so CI's checks pass:"
      echo "$fixes" | sed -e 's/^/  - /' -e 's/TODO/to-do/g'
    fi
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
  title="Sync upstream: storytold/pdfcraft@${up:0:12} (PdfCraft $version)"
  if [ -n "$stale" ]; then
    force_push_as_owner "$branch" "$pr_head"
    gh pr edit "$pr" --repo "$REPO" --title "$title" --body-file "$record" >/dev/null
    echo "sync: rebuilt PR #$pr"
  else
    push_as_owner "$branch"
    pr=$(gh pr create --repo "$REPO" --base main --head "$branch" --title "$title" \
      --body-file "$record" | sed -n 's#.*/pull/\([0-9]*\)$#\1#p')
    [ -n "$pr" ] || die "couldn't open the PR"
    echo "sync: opened PR #$pr"
  fi
fi

merge_when_green_or_fix "$pr" "${SYNC_CHECKS_TIMEOUT:-7200}" "$branch"
echo "sync: merged PR #$pr; the push to main starts the Release workflow"
