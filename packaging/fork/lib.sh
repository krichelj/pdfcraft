#!/usr/bin/env bash
# Shared by the fork's automation (sync-upstream.sh, release-bump.sh, auto-merge.sh): push and merge
# as the owner, and hand what a merge breaks to the on-call LLM. Sourced; needs REPO (and, for the
# LLM, ROOT) set.

die() {
  echo "::error::$*" >&2
  exit 1
}

# Push a branch with the owner's gh login: the workflow's own token can't push workflow files, and
# its pushes start no other workflow.
push_as_owner() {
  git -c credential.helper= -c credential.helper='!gh auth git-credential' push --quiet -u origin "$1"
}

# MERGE_IGNORE_CHECK names a check left out (the job running this, which is never "completed" while
# it waits).
# Usage: wait_for_checks <pr> <timeout seconds>. Wait for every check on the PR's head: returns 0
# when they all passed, 1 when one failed, 2 when they didn't finish in time. CI's test job must be
# among them, so a PR whose checks haven't started yet is never "green".
wait_for_checks() {
  local pr="$1" timeout="$2" head state deadline
  head=$(gh pr view "$pr" --repo "$REPO" --json headRefOid --jq .headRefOid)
  deadline=$(($(date +%s) + timeout))
  while :; do
    # shellcheck disable=SC2016 # a jq program, not shell
    state=$(gh api "repos/$REPO/commits/$head/check-runs?per_page=100&filter=latest" --jq '
      [.check_runs[] | select(.name != env.MERGE_IGNORE_CHECK) | {name, status, conclusion}] as $r
      | if ($r | map(select(.conclusion == "failure" or .conclusion == "cancelled" or .conclusion == "timed_out" or .conclusion == "action_required")) | length) > 0 then "FAILURE"
        elif ($r | map(select(.name == "test (shared code)" and .conclusion == "success")) | length) > 0
          and ($r | all(.status == "completed")) then "SUCCESS"
        else "PENDING" end' 2>/dev/null || echo PENDING)
    case "$state" in
      SUCCESS) return 0 ;;
      FAILURE) return 1 ;;
    esac
    [ "$(date +%s)" -lt "$deadline" ] || return 2
    sleep 30
  done
}

# Usage: merge_when_green <pr> <timeout seconds>. Wait for every check on the PR's head, then merge it
# (the branch is deleted); a failing check leaves it open.
merge_when_green() {
  local pr="$1" rc=0
  wait_for_checks "$pr" "$2" || rc=$?
  case "$rc" in
    0) gh pr merge "$pr" --repo "$REPO" --merge --delete-branch ;;
    1) die "PR #$pr has failing checks; left open for a person" ;;
    *) die "PR #$pr's checks didn't finish in time; left open" ;;
  esac
}

# The on-call LLM (oncall-llm, the owner's private tool on the runner hosts; its server and keys
# are configured there, never here): it resolves merge conflicts and repairs failing checks.
# ONCALL_LLM names the command; SYNC_NO_LLM=1 leaves both to a person, as before it existed.
llm_available() {
  [ "${SYNC_NO_LLM:-}" != 1 ] && command -v "${ONCALL_LLM:-oncall-llm}" >/dev/null 2>&1
}

# The checks the LLM's fix loop must make pass: CI's test job (ci.yml), in its order, into FIX_ARGS.
# SYNC_FIX_CHECKS (one NAME=COMMAND per line) replaces them (tests); SYNC_LLM_ROUNDS caps the rounds.
fix_check_args() {
  local line
  FIX_ARGS=()
  if [ -n "${SYNC_FIX_CHECKS:-}" ]; then
    while IFS= read -r line; do
      [ -z "$line" ] || FIX_ARGS+=(--check "$line")
    done <<<"$SYNC_FIX_CHECKS"
  else
    FIX_ARGS=(
      --check "fmt=cargo fmt --all -- --check" --autofix "fmt=cargo fmt --all"
      --check "clippy=cargo clippy --workspace --all-targets -- -D warnings"
      --check "test=cargo nextest run --workspace --no-fail-fast && cargo test --workspace --doc"
      --check "layers=cargo xtask layers"
      --check "assets=cargo xtask assets"
      --check "cli-without-mcp=cargo check -p pdfcraft-cli --no-default-features"
    )
  fi
  FIX_ARGS+=(--max-rounds "${SYNC_LLM_ROUNDS:-8}" --instructions "$ROOT/packaging/fork/oncall-instructions.md")
}

# Usage: llm_fix <commit prefix> [context file]. The LLM edits the tree at $ROOT until every check
# passes, committing each round's edits with the prefix; fails when they still fail.
llm_fix() {
  local context=()
  fix_check_args
  [ -z "${2:-}" ] || context=(--context "$2")
  "${ONCALL_LLM:-oncall-llm}" fix --repo "$ROOT" "${FIX_ARGS[@]}" "${context[@]}" --commit-prefix "$1"
}

# Usage: ci_failure_log <pr>. The failed jobs' logs on the PR's head commit, their last 200 KB.
ci_failure_log() {
  local head id
  head=$(gh pr view "$1" --repo "$REPO" --json headRefOid --jq .headRefOid)
  for id in $(gh run list --repo "$REPO" --commit "$head" --status failure --json databaseId --jq '.[].databaseId'); do
    gh run view "$id" --repo "$REPO" --log-failed || true
  done | tail -c 200000
}

# Usage: merge_when_green_or_fix <pr> <timeout seconds> <branch>. merge_when_green, except that a
# failing check goes to the on-call LLM with the failed jobs' logs, at most SYNC_CI_FIX_RETRIES
# (default 2) times: its fix is pushed to the PR's branch (checked out at $ROOT) and the new head's
# checks are awaited. Without the LLM, or past the retries, the PR is left open for a person.
merge_when_green_or_fix() {
  local pr="$1" timeout="$2" branch="$3" tries=0 rc log
  while :; do
    rc=0
    wait_for_checks "$pr" "$timeout" || rc=$?
    case "$rc" in
      0)
        gh pr merge "$pr" --repo "$REPO" --merge --delete-branch
        return
        ;;
      2) die "PR #$pr's checks didn't finish in time; left open" ;;
    esac
    if [ "$tries" -ge "${SYNC_CI_FIX_RETRIES:-2}" ] || ! llm_available; then
      die "PR #$pr has failing checks; left open for a person"
    fi
    tries=$((tries + 1))
    echo "sync: PR #$pr has failing checks; the on-call LLM fixes them (attempt $tries)"
    log="$(mktemp)"
    ci_failure_log "$pr" >"$log"
    llm_fix "Sync fix (CI, attempt $tries)" "$log" \
      || die "the on-call LLM couldn't fix PR #$pr's failing checks; left open for a person"
    rm -f "$log"
    push_as_owner "$branch"
  done
}

# Usage: fill_summary <record> <file>. Replace the record's "- TODO: ..." line with the file's lines
# (cargo xtask fork-notes generate writes that placeholder).
fill_summary() {
  awk -v f="$2" '/^- TODO: / { while ((getline l < f) > 0) print l; next } { print }' "$1" >"$1.new"
  mv "$1.new" "$1"
}

# Re-record the SHA-256 of every translation catalog in ATTRIBUTION.toml. Both sides add strings to
# the catalogs, so after an upstream merge their recorded hashes are upstream's or the fork's, never
# those of the merged files; the catalogs are the only assets the fork changes (AGENTS.md §1).
rehash_catalogs() {
  local f h
  for f in crates/ui-egui/src/i18n/*.tsv; do
    h=$(sha256sum "$f" | cut -d' ' -f1)
    awk -v p="path = \"$f\"" -v h="$h" '
      $0 == p { want = 1 }
      want && /^sha256 = "/ { $0 = "sha256 = \"" h "\""; want = 0 }
      { print }' ATTRIBUTION.toml >ATTRIBUTION.toml.new
    mv ATTRIBUTION.toml.new ATTRIBUTION.toml
  done
}
