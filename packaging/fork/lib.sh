#!/usr/bin/env bash
# Shared by the fork's automation (sync-upstream.sh, release-bump.sh): push and merge as the owner.
# Sourced; needs REPO set.

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
# Usage: merge_when_green <pr> <timeout seconds>. Wait for every check on the PR's head, then merge it
# (the branch is deleted); a failing check leaves it open. CI's test job must be among them, so a PR
# whose checks haven't started yet is never "green".
merge_when_green() {
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
      SUCCESS) break ;;
      FAILURE) die "PR #$pr has failing checks; left open for a person" ;;
    esac
    [ "$(date +%s)" -lt "$deadline" ] || die "PR #$pr's checks didn't finish in time; left open"
    sleep 30
  done
  gh pr merge "$pr" --repo "$REPO" --merge --delete-branch
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
