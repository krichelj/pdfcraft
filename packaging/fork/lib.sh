#!/usr/bin/env bash
# Shared by the fork's automation (sync-upstream.sh, release-bump.sh, auto-merge.sh): push and merge
# as the owner, and hand what a merge breaks to the on-call LLM. Sourced; needs REPO (and, for the
# LLM, ROOT) set.

die() {
  echo "::error::$*" >&2
  exit 1
}

# Usage: ask_marker <title> <body>. The line ask_person writes under an ask, naming exactly what was
# asked, so a later run can tell whether the open issue already says it.
ask_marker() {
  printf '<!-- sync-ask %s -->' "$(printf '%s\n%s' "$1" "$2" | git hash-object --stdin)"
}

# Usage: ask_person <key> <title> <body>. Tell the owner the automation stopped for a person: a GitHub
# issue (GitHub notifies the owner), or a comment on the open issue whose title has <key> (one issue
# per sync branch, however many runs stop on it). A dry run only says what it would ask.
ask_person() {
  local n body
  if [ "${SYNC_DRY_RUN:-}" = 1 ]; then
    echo "sync: dry run; would ask a person: $2"
    return 0
  fi
  body="$3"$'\n\n'"$(ask_marker "$2" "$3")"
  n=$(gh issue list --repo "$REPO" --state open --search "\"$1\" in:title" --json number --jq '.[0].number // empty' 2>/dev/null) || n=""
  # Telling the owner must never hide why the sync stopped: a failure here is a warning, and the
  # caller's own error follows (issues were disabled on the fork when this first ran).
  if [ -n "$n" ]; then
    if gh issue comment "$n" --repo "$REPO" --body "$2"$'\n\n'"$body" >/dev/null; then
      echo "sync: asked a person again on issue #$n: $2"
    else
      echo "::warning::couldn't comment on issue #$n to ask a person: $2"
    fi
  elif gh issue create --repo "$REPO" --title "$1: $2" --body "$body" >/dev/null; then
    echo "sync: asked a person in a new issue: $2"
  else
    echo "::warning::couldn't open an issue to ask a person: $2"
  fi
}

# Usage: stop_for_person <key> <title> <body> <error>. Stop for a person: ask (ask_person) and fail
# with <error>. When the open issue for <key> already holds this exact ask, the owner has been told:
# the run only warns, records SYNC_WAITING=1 for the workflow's later steps, and ends cleanly, so an
# hourly run stopped on the same thing sends no failure mail and adds no comment.
stop_for_person() {
  local n said
  if [ "${SYNC_DRY_RUN:-}" != 1 ]; then
    n=$(gh issue list --repo "$REPO" --state open --search "\"$1\" in:title" --json number --jq '.[0].number // empty' 2>/dev/null) || n=""
    if [ -n "$n" ]; then
      said=$(gh issue view "$n" --repo "$REPO" --json body,comments --jq '.body, .comments[].body' 2>/dev/null) || said=""
      if printf '%s' "$said" | grep -qF "$(ask_marker "$2" "$3")"; then
        echo "::warning::still waiting for a person on issue #$n: $4"
        if [ -n "${GITHUB_ENV:-}" ]; then echo "SYNC_WAITING=1" >>"$GITHUB_ENV"; fi
        exit 0
      fi
    fi
  fi
  ask_person "$1" "$2" "$3"
  die "$4"
}

# Push a branch with the owner's gh login: the workflow's own token can't push workflow files, and
# its pushes start no other workflow.
push_as_owner() {
  git -c credential.helper= -c credential.helper='!gh auth git-credential' push --quiet -u origin "$1"
}

# Usage: force_push_as_owner <branch> <sha>. Replace the remote branch, only if it is still at <sha>
# (a rebuilt sync PR: never over a commit someone pushed since it was read).
force_push_as_owner() {
  git -c credential.helper= -c credential.helper='!gh auth git-credential' \
    push --quiet -u --force-with-lease="$1:$2" origin "$1"
}

# MERGE_IGNORE_CHECK names a check left out (the job running this, which is never "completed" while
# it waits).
# Usage: wait_for_checks <pr> <timeout seconds>. Wait for every check on the PR's head: returns 0
# when they all passed, 1 when one failed, 2 when they didn't finish in time. CI's test job must be
# among them, so a PR whose checks haven't started yet is never "green". CHECKED_HEAD is set to the
# commit whose checks were read, the only one merge_pr will merge.
wait_for_checks() {
  local pr="$1" timeout="$2" head state deadline
  head=$(gh pr view "$pr" --repo "$REPO" --json headRefOid --jq .headRefOid)
  CHECKED_HEAD="$head"
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

# Usage: merge_pr <pr>. Merge it, deleting its branch, but only at CHECKED_HEAD (the commit whose
# checks passed: a push after them is never merged untested). A PR another run merged first counts as
# merged, so two runs that both saw it green never turn the second one red: when GitHub refuses the
# merge because the other run's is still under way ("Merge already in progress", as on #47), this
# one waits up to MERGE_SETTLE_SECONDS (default 120) for it to land.
merge_pr() {
  local waited=0
  gh pr merge "$1" --repo "$REPO" --merge --delete-branch ${CHECKED_HEAD:+--match-head-commit "$CHECKED_HEAD"} && return 0
  while :; do
    [ "$(gh pr view "$1" --repo "$REPO" --json state --jq .state)" = MERGED ] && return 0
    [ "$waited" -lt "${MERGE_SETTLE_SECONDS:-120}" ] || return 1
    sleep 5
    waited=$((waited + 5))
  done
}

# Usage: merge_when_green <pr> <timeout seconds>. Wait for every check on the PR's head, then merge it
# (the branch is deleted); a failing check leaves it open.
merge_when_green() {
  local pr="$1" rc=0
  wait_for_checks "$pr" "$2" || rc=$?
  case "$rc" in
    0) merge_pr "$pr" ;;
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

# The checks the LLM's fix loop must make pass: CI's test job (ci.yml), in its order, then Packaging
# lint's checks of the macOS packaging (packaging-lint.yml), into FIX_ARGS.
# SYNC_FIX_CHECKS (one NAME=COMMAND per line) replaces them (tests); SYNC_LLM_ROUNDS caps the rounds.
# The mechanical fixes apply whatever the checks are: a formatter for fmt, and for assets the
# catalog re-hash (an edited translation catalog changes its SHA-256, which no LLM can compute).
fix_check_args() {
  local line
  FIX_ARGS=()
  if [ -n "${SYNC_FIX_CHECKS:-}" ]; then
    while IFS= read -r line; do
      [ -z "$line" ] || FIX_ARGS+=(--check "$line")
    done <<<"$SYNC_FIX_CHECKS"
  else
    FIX_ARGS=(
      --check "fmt=cargo fmt --all -- --check"
      --check "clippy=cargo clippy --workspace --all-targets -- -D warnings"
      --check "test=packaging/fork/private-tmp.sh bash -c 'cargo nextest run --workspace --no-fail-fast && cargo test --workspace --doc'"
      --check "layers=cargo xtask layers"
      --check "assets=cargo xtask assets"
      --check "cli-without-mcp=cargo check -p pdfcraft-cli --no-default-features"
      # Packaging lint's (packaging-lint.yml): a merged script that doesn't parse fails CI too.
      --check "packaging-lint=shellcheck -x packaging/env.sh packaging/icons.sh packaging/macos/*.sh packaging/macos/dmg/*.sh packaging/fork/*.sh && xmllint --noout packaging/macos/*.plist* && packaging/macos/test-install.sh"
    )
  fi
  FIX_ARGS+=(
    --autofix "fmt=cargo fmt --all"
    --autofix "assets=bash -c '. packaging/fork/lib.sh && rehash_catalogs'"
    --max-rounds "${SYNC_LLM_ROUNDS:-8}" --instructions "$ROOT/packaging/fork/oncall-instructions.md"
  )
}

# What the on-call LLM's fixes may never change: what CI runs and how, the fork's automation and its
# rules. A path ending in / covers everything under it. The LLM is told (oncall-instructions.md),
# oncall-llm refuses such edits itself when it can (--protect), and llm_guard checks the commits.
LLM_PROTECTED=(.github/ packaging/fork/ xtask/src/workflows.rs deny.toml .repo-structure.toml CLAUDE.md AGENTS.md)

# Usage: llm_supports <flag>. Whether the installed oncall-llm's fix takes <flag> (older ones don't).
llm_supports() {
  local help
  help=$("${ONCALL_LLM:-oncall-llm}" fix --help 2>/dev/null) || return 1
  [[ "$help" == *"$1"* ]]
}

# Usage: llm_guard <rev>. Die, before anything is pushed, when the commits after <rev> (the LLM's)
# change a protected path, or add a line that disables a test (#[ignore], #[cfg(any())], a skip
# decorator), hides a failing command (`|| true`, `|| :` in a script or workflow), or comments out a
# line it removed. A weaker test or a muted check is never a fix; a person decides.
llm_guard() {
  local since="$1" path p bad="" muted g=(git -C "${ROOT:-.}")
  while IFS= read -r path; do
    for p in "${LLM_PROTECTED[@]}"; do
      case "$p" in
        */) if [[ "$path" == "$p"* ]]; then bad="$bad $path"; fi ;;
        *) if [ "$path" = "$p" ]; then bad="$bad $path"; fi ;;
      esac
    done
  done < <("${g[@]}" diff --name-only "$since" HEAD)
  [ -z "$bad" ] || die "the on-call LLM's fix changes protected path(s):$bad; a person decides"
  muted=$({
    "${g[@]}" diff --unified=0 "$since" HEAD | grep -E '^\+[^+]' \
      | grep -E '#\[ignore|#\[cfg\(any\(\)\)\]|@(pytest\.mark\.skip|unittest\.skip)' || true
    # A removed line put back behind a comment opener: code commented out to quiet a check.
    "${g[@]}" diff --unified=0 "$since" HEAD | awk '
      /^-[^-]/ { line = substr($0, 2); gsub(/^[ \t]+|[ \t]+$/, "", line); if (length(line) > 3) gone[line] = 1 }
      /^\+[^+]/ { line = substr($0, 2); if (match(line, /^[ \t]*(#+|\/\/+|--+|\/\*+)[ \t]*/)) {
        rest = substr(line, RLENGTH + 1); gsub(/[ \t]+$/, "", rest); if (rest in gone) print "+" line } }' || true
    "${g[@]}" diff --unified=0 "$since" HEAD -- '*.sh' '*.bash' '*.yml' '*.yaml' '*Makefile' | grep -E '^\+[^+]' \
      | grep -E '\|\|[[:space:]]*(true|:)([[:space:];)]|$)' || true
  })
  [ -z "$muted" ] || die "the on-call LLM's fix disables a test or hides a failure; a person decides:
$muted"
}

# Usage: llm_fix <commit prefix> [context file]. The LLM edits the tree at $ROOT until every check
# passes, committing each round's edits with the prefix; fails when they still fail, and dies when
# its commits break llm_guard. The files the merge changed (since sync-base) are named in its prompts.
llm_fix() {
  local extra=() before p
  fix_check_args
  [ -z "${2:-}" ] || extra+=(--context "$2")
  if llm_supports --since && git rev-parse -q --verify sync-base >/dev/null; then extra+=(--since sync-base); fi
  if llm_supports --protect; then
    for p in "${LLM_PROTECTED[@]}"; do
      case "$p" in */) extra+=(--protect "$p**") ;; *) extra+=(--protect "$p") ;; esac
    done
  fi
  before=$(git -C "$ROOT" rev-parse HEAD)
  "${ONCALL_LLM:-oncall-llm}" fix --repo "$ROOT" "${FIX_ARGS[@]}" "${extra[@]}" --commit-prefix "$1" || return
  llm_guard "$before"
}

# Usage: ci_failure_log <pr>. What the failed jobs' logs on the PR's head commit say about the
# failure (ci_log_digest): a whole Test log is ~900 KB, far past the model's context.
ci_failure_log() {
  local head id
  head=$(gh pr view "$1" --repo "$REPO" --json headRefOid --jq .headRefOid)
  for id in $(gh run list --repo "$REPO" --commit "$head" --status failure --json databaseId --jq '.[].databaseId'); do
    gh run view "$id" --repo "$REPO" --log-failed || true
  done | ci_log_digest
}

# Usage: ci_log_digest <log. GitHub's job/step/time prefixes and colour codes stripped; nextest's
# PASS/SKIP lines and cargo's progress lines dropped; every line that mentions an error, a panic, a
# failure, an assertion or a warning kept with 3 lines before and 12 after, gaps marked "...",
# repeated lines collapsed; at most SYNC_CI_LOG_BYTES (default 40000) bytes, from the start. A log
# with none of those lines gives its last SYNC_CI_LOG_BYTES instead.
ci_log_digest() {
  local esc=$'\033'
  sed -E -e "s/${esc}\[[0-9;]*[A-Za-z]//g" -e 's/^[^\t]*\t[^\t]*\t[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z ?//' \
    | awk -v before=3 -v after=12 -v max="${SYNC_CI_LOG_BYTES:-40000}" '
      function out(s) { bytes += length(s) + 1; if (bytes > max) return 0; print s; return 1 }
      /^[[:space:]]*(PASS|SKIP) \[/ { next }
      /^[[:space:]]*(Compiling|Checking|Downloaded|Downloading|Fresh|Blocking|Running|Doc-tests) / { next }
      {
        n++; line[n] = $0
        if (tolower($0) ~ /error|panicked|failed|fail \[|assertion|warning:/) {
          any = 1
          for (i = n - before; i <= n + after; i++) keep[i] = 1
        }
      }
      END {
        if (!any) {
          first = n + 1
          while (first > 1 && total + length(line[first - 1]) + 1 <= max) { first--; total += length(line[first]) + 1 }
          for (i = first; i <= n; i++) print line[i]
          exit
        }
        for (i = 1; i <= n; i++) {
          if (!keep[i]) { gap = 1; continue }
          if (gap && printed && !out("...")) exit
          gap = 0
          if (printed && line[i] == prev) continue
          if (!out(line[i])) exit
          prev = line[i]; printed = 1
        }
      }'
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
        merge_pr "$pr"
        return
        ;;
      2) die "PR #$pr's checks didn't finish in time; left open" ;;
    esac
    if [ "$tries" -ge "${SYNC_CI_FIX_RETRIES:-2}" ] || ! llm_available; then
      stop_for_person "$branch" "PR #$pr has failing checks" \
        "The sync PR #$pr ($branch) has failing checks; it stays open until they pass. Fix them on its branch: the next sync run merges it once every check is green." \
        "PR #$pr has failing checks; left open for a person"
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

# Usage: locked <Cargo.lock>. Each registry package as `name@line version`, sorted, its line the semver
# range a version stays compatible within (the major version; 0.minor; 0.0.patch).
locked() {
  awk '
    function out(p) {
      split(ver, p, /[.+-]/)
      print name "@" (p[1] != "0" ? p[1] : p[2] != "0" ? "0." p[2] : "0.0." p[3]), ver
    }
    /^\[\[package\]\]/ { if (src ~ /^registry\+/) out(); name = ver = src = "" }
    /^name = "/ { name = $3; gsub(/"/, "", name) }
    /^version = "/ { ver = $3; gsub(/"/, "", ver) }
    /^source = "/ { src = $3; gsub(/"/, "", src) }
    END { if (src ~ /^registry\+/) out() }' "$1" | sort
}

# Usage: keep_newer_locked <the fork's Cargo.lock>. Raise each registry package the merged Cargo.lock
# has older than the fork's lock does, on the same semver line, to the fork's version. Upstream's lock,
# taken on a conflict, must not undo the fork's dependency updates: the 0.13.0 sync took tokio back
# from 1.54.1 to 1.53.1, the version of an advisory (RUSTSEC-2026-0386) the fork had moved past.
keep_newer_locked() {
  local key fork merged
  join <(locked "$1") <(locked Cargo.lock) | while read -r key fork merged; do
    [ "$fork" != "$merged" ] || continue
    [ "$(printf '%s\n%s\n' "$merged" "$fork" | sort -V | tail -1)" = "$fork" ] || continue
    if cargo update --quiet -p "${key%@*}@$merged" --precise "$fork" 2>/dev/null; then
      echo "sync: Cargo.lock keeps the fork's ${key%@*} $fork (upstream's lock had $merged)"
    else
      echo "sync: Cargo.lock has ${key%@*} $merged: the merged manifests don't take the fork's $fork"
    fi
  done
}

# Re-record the SHA-256 of every translation catalog in ATTRIBUTION.toml. Both sides add strings to
# the catalogs, so after an upstream merge their recorded hashes are upstream's or the fork's, never
# those of the merged files; the catalogs are the only assets the fork changes (AGENTS.md §1).
rehash_catalogs() {
  local f h
  for f in crates/ui-egui/src/i18n/*.tsv; do
    # None left (upstream moved them to storytold/craft-i18n): the glob stays unmatched.
    [ -e "$f" ] || continue
    h=$(sha256sum "$f" | cut -d' ' -f1)
    # Only the catalog's own [[asset]] block: an entry without a sha256 line must not pass the
    # hash on to the next asset's (upstream 0.13.0's catalogs have such entries, and the hashes of
    # two icons and a PNG were overwritten).
    awk -v p="path = \"$f\"" -v h="$h" '
      /^\[\[asset\]\]/ { want = 0 }
      $0 == p { want = 1 }
      want && /^sha256 = "/ { $0 = "sha256 = \"" h "\""; want = 0 }
      { print }' ATTRIBUTION.toml >ATTRIBUTION.toml.new
    mv ATTRIBUTION.toml.new ATTRIBUTION.toml
  done
}

# Usage: fold_test_modules FILE (mergiraf's output, conflict markers in diff3 style). Both sides
# adding a `#[cfg(test)] mod tests { … }` where the base had none conflicts in mergiraf, which won't
# merge two items of one name: such a region becomes one module, the fork's tests then upstream's
# (upstream's `use super::*;` dropped when the fork's has it). Any other region is left as it is.
# Exit 0 when no conflict is left in FILE.
fold_test_modules() {
  awk '
    function head(a, n,   i, j) { for (i = 1; i <= n && a[i] ~ /^[[:space:]]*$/; i++); return i }
    function tail_(a, n,   i) { for (i = n; i >= 1 && a[i] ~ /^[[:space:]]*$/; i--); return i }
    function is_mod(a, n,   i, j) {
      i = head(a, n); j = tail_(a, n)
      return i < j && a[i] == "#[cfg(test)]" && a[i + 1] == "mod tests {" && a[j] == "}"
    }
    function flush(   i, j, k, use) {
      blank = 1
      for (i = 1; i <= nb; i++) if (b[i] !~ /^[[:space:]]*$/) blank = 0
      if (blank && is_mod(o, no) && is_mod(t, nt)) {
        use = 0
        j = tail_(o, no)
        for (i = 1; i < j; i++) { print o[i]; if (o[i] == "    use super::*;") use = 1 }
        k = head(t, nt) + 2
        if (use && t[k] == "    use super::*;") { k++; if (t[k] ~ /^[[:space:]]*$/) k++ }
        print ""
        for (i = k; i <= nt; i++) print t[i]
      } else {
        print om; for (i = 1; i <= no; i++) print o[i]
        print bm; for (i = 1; i <= nb; i++) print b[i]
        print "======="; for (i = 1; i <= nt; i++) print t[i]
        print tm; left = 1
      }
    }
    s == 0 && /^<<<<<<< / { s = 1; om = $0; no = nb = nt = 0; next }
    s == 1 && /^\|\|\|\|\|\|\| / { s = 2; bm = $0; next }
    s == 2 && /^=======$/ { s = 3; next }
    s == 3 && /^>>>>>>> / { tm = $0; flush(); s = 0; next }
    s == 1 { o[++no] = $0; next }
    s == 2 { b[++nb] = $0; next }
    s == 3 { t[++nt] = $0; next }
    { print }
    END { if (s != 0) left = 1; exit left }' "$1" >"$1.fold"
  local rc=$?
  mv "$1.fold" "$1"
  return "$rc"
}

# Usage: add_upstream_stubs <fork file> <upstream file>. For a fork-owned Rust file: each pub or
# pub(crate) method upstream's version has in an inherent `impl <Type> {` block and the fork's
# doesn't is appended to the fork's file with a `Default::default()` body, under its cfg
# attributes, so upstream's callers (its other code and tests) still compile. Prints the names added.
add_upstream_stubs() {
  local fork="$1" theirs="$2" have stubs
  have=$(grep -oE '\bfn [A-Za-z_][A-Za-z0-9_]*' "$fork" | cut -c4- | sort -u | tr '\n' ' ')
  stubs=$(awk -v have=" $have" '
    function flush() {
      if (out == "") return
      printf "\n/// Upstream'\''s API that the fork'\''s own code does not use: added by the sync so upstream'\''s\n/// callers compile.\nimpl %s {%s}\n", ty, out
      out = ""
    }
    /^impl [A-Za-z_][A-Za-z0-9_]* \{$/ { ty = $2; inimpl = 1; attrs = ""; next }
    inimpl && /^\}/ { flush(); inimpl = 0; next }
    !inimpl { next }
    sig != "" {
      sig = sig "\n" $0
      if ($0 ~ /\{ *$/) { emit(); }
      next
    }
    /^    #\[/ { attrs = attrs $0 "\n"; next }
    /^    pub(\(crate\))? (const )?fn [A-Za-z_]/ {
      name = $0; sub(/.*fn /, "", name); sub(/[^A-Za-z0-9_].*/, "", name)
      if (index(have, " " name " ") == 0) { sig = $0; if ($0 ~ /\{ *$/) emit(); else next }
      attrs = ""; next
    }
    /^    \/\/\// { next }
    { attrs = "" }
    function emit() {
      sub(/ *\{ *$/, "", sig)
      out = out "\n" attrs "    #[allow(dead_code, unused_variables)]\n" sig " {\n        Default::default()\n    }\n"
      printf "%s\n", name > "/dev/stderr"
      sig = ""; attrs = ""
    }
  ' "$theirs" 2>"$fork.stubnames") || return 1
  # Before the test module, if there is one: clippy refuses items after it.
  if [ -n "$stubs" ]; then
    printf '%s\n\n' "$stubs" >"$fork.stubs"
    awk -v f="$fork.stubs" '!done && /^#\[cfg\(test\)\]/ { while ((getline l < f) > 0) print l; done = 1 } { print }
      END { if (!done) while ((getline l < f) > 0) print l }' "$fork" >"$fork.new" && mv "$fork.new" "$fork"
    rm -f "$fork.stubs"
  fi
  paste -sd' ' "$fork.stubnames"
  rm -f "$fork.stubnames"
}

# Usage: skip_stubbed_tests <file> <name...>. Compiles out (`#[cfg(any())]`, never true) each
# `#[test]` in <file> that calls one of the stubbed methods <name>, itself or through a helper
# function in the same file, and those helpers: they check upstream's own updater, which the fork's
# replaces, and use its types. Prints how many tests.
skip_stubbed_tests() {
  local f="$1" n
  shift
  n=$(awk -v names=" $* " '
    function calls(body, set,   k) { for (k in set) if (index(body, k "(")) return 1; return 0 }
    NR == FNR {
      line[NR] = $0
      if (fname == "" && match($0, /^ *(pub(\(crate\))? )?fn [A-Za-z_][A-Za-z0-9_]*/)) {
        fname = substr($0, RSTART, RLENGTH); sub(/.*fn /, "", fname)
        ind = $0; sub(/[^ ].*/, "", ind); start = NR; body = ""
        istest[fname] = (prev ~ /^ *#\[test\]/ || prev2 ~ /^ *#\[test\]/); at[fname] = start
      }
      if (fname != "") {
        body = body $0 "\n"
        if ($0 == ind "}" || ($0 ~ /\}$/ && NR == start)) { bodies[fname] = body; fname = "" }
      }
      prev2 = prev; prev = $0; next
    }
    FNR == 1 {
      split(names, a, " "); for (i in a) if (a[i] != "") hit["." a[i]] = 1
      for (round = 0; round < 4; round++)
        for (g in bodies) if (!istest[g] && !(g in hit) && calls(bodies[g], hit)) hit[g] = 1
      for (g in bodies) if ((istest[g] && calls(bodies[g], hit)) || (!istest[g] && (g in hit))) {
        for (j = at[g]; j > 1 && line[j - 1] ~ /^ *(#\[|\/\/\/)/; j--) ;
        if (line[j - 1] !~ /cfg\(any\(\)\)/) { mark[j] = 1; if (istest[g]) count++ }
      }
    }
    {
      if (FNR in mark) {
        ind = $0; sub(/[^ ].*/, "", ind)
        print ind "#[cfg(any())] // fork: upstream'\''s updater, which the fork replaces"
      }
      print > "/dev/stdout"
    }
    END { print count + 0 > "/dev/stderr" }
  ' "$f" "$f" 2>&1 >"$f.ign")
  mv "$f.ign" "$f"
  echo "$n"
}
