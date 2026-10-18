#!/usr/bin/env bash
# Scenario tests for the fork's automation: sync-upstream.sh, release-bump.sh and lib.sh's
# merge_when_green, run as they are against synthetic histories. Each sync case starts an upstream
# from the commit the fork's current release records as its upstream base, adds that case's commits,
# and dry-runs the sync in a fresh clone of this checkout; each states what must happen, including
# when the sync must stop for a person. No network: everything is a local repository.
#
# Usage: packaging/fork/test-sync.sh   (exit 0 when every case passes; needs cargo, git, jq)
set -euo pipefail
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
T="$(mktemp -d)"
if [ "${TEST_SYNC_KEEP:-}" = 1 ]; then echo "kept in $T" >&2; else trap 'rm -rf "$T"' EXIT; fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$SRC/target}"
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@example.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@example.com
version=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$SRC/Cargo.toml" | head -1)
base=$(grep -o 'storytold/pdfcraft/commit/[0-9a-f]\{40\}' "$SRC/docs/fork/releases/v$version.md" | head -1 | cut -d/ -f4)
[ -n "$base" ] || { echo "no upstream base recorded for $version" >&2; exit 1; }
level="${version##*-p}"
next="${version%%-*}-p$((level + 1))"
pass=0 fail=0

# The fork as it is now (this checkout's HEAD, uncommitted changes included) on branch main.
git -C "$SRC" stash create >"$T/stash" || true
fork_rev=$(cat "$T/stash")
fork_rev=${fork_rev:-$(git -C "$SRC" rev-parse HEAD)}
git init -q -b main "$T/fork"
git -C "$T/fork" fetch -q "$SRC" "$fork_rev" "+refs/tags/*:refs/tags/*"
git -C "$T/fork" reset -q --hard FETCH_HEAD
git -C "$T/fork" checkout -q -B main

# Usage: upstream <name>; then commit into "$T/up-<name>" with up <name> <git args…>.
upstream() {
  git init -q -b main "$T/up-$1"
  git -C "$T/up-$1" fetch -q "$T/fork" "$base"
  git -C "$T/up-$1" reset -q --hard FETCH_HEAD
}
up() {
  local n="$1"
  shift
  git -C "$T/up-$n" "$@"
}
# Usage: sync <name> [fork-side command]: dry-run the sync of that upstream in a fresh clone at
# "$T/s-<name>", after running the command there (a fork commit the case needs) when one is given.
sync() {
  local d="$T/s-$1" rc=0
  git clone -q "$T/fork" "$d"
  if [ -n "${2:-}" ]; then (cd "$d" && eval "$2"); fi
  (cd "$d" && SYNC_DRY_RUN=1 SYNC_FORK_REF=HEAD SYNC_UPSTREAM_URL="$T/up-$1" bash packaging/fork/sync-upstream.sh) >"$d.out" 2>&1 || rc=$?
  echo "$rc" >"$d.rc"
}
check() {
  if "${@:2}"; then pass=$((pass + 1)); else
    echo "FAIL $1" >&2
    fail=$((fail + 1))
  fi
}
ok() { [ "$(cat "$T/s-$1.rc")" = 0 ]; }
stopped() { [ "$(cat "$T/s-$1.rc")" != 0 ] && grep -q "$2" "$T/s-$1.out"; }
in_tree() { [ -e "$T/s-$1/$2" ]; }
said() { grep -q "$2" "$T/s-$1.out"; }
version_is() { grep -q "^version = \"$2\"" "$T/s-$1/Cargo.toml"; }
notes_ok() { (cd "$T/s-$1" && cargo xtask fork-notes check "$2" --commit HEAD --upstream refs/fork-notes/upstream >/dev/null 2>&1); }
assets_ok() { (cd "$T/s-$1" && cargo xtask assets >/dev/null 2>&1); }

# 1. Nothing new upstream.
upstream none
sync none
check "nothing new: does nothing" ok none
check "  and says so" said none "nothing to do"

# 2. An ordinary upstream change.
upstream plain
echo "upstream note" >>"$T/up-plain/NOTICE"
up plain commit -qam "upstream: an ordinary change"
sync plain
check "an upstream change is merged" ok plain
check "  with the change in the tree" grep -q "upstream note" "$T/s-plain/NOTICE"
check "  the fork's next patch version, $next" version_is plain "$next"
check "  and its record, which passes, naming the commit" notes_ok plain "$next"
check "  the record lists the upstream commit" grep -q "upstream: an ordinary change" "$T/s-plain/docs/fork/releases/v$next.md"
check "  assets still check out" assets_ok plain

# 3. Upstream adds a workflow for another platform or GitHub's runners.
upstream wf
printf 'name: X\non: push\njobs:\n  x:\n    runs-on: ubuntu-latest\n    steps:\n      - run: true\n' >"$T/up-wf/.github/workflows/x.yml"
up wf add -A
up wf commit -qm "upstream: a workflow the fork doesn't run"
sync wf
check "a new upstream workflow: merged" ok wf
check "  and removed" bash -c "! test -e '$T/s-wf/.github/workflows/x.yml'"
check "  the fork's workflows stay" in_tree wf .github/workflows/sync-upstream.yml

# 4. Upstream moves its version: the fork's line restarts at <new base>-p1.
upstream bump
# As `cargo xtask version set 9.9.0` does upstream: the workspace version and every internal crate's.
old=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$T/up-bump/Cargo.toml" | head -1)
sed -i "s/\"$old\"/\"9.9.0\"/g" "$T/up-bump/Cargo.toml"
up bump commit -qam "upstream: release 9.9.0"
sync bump
check "an upstream version bump: merged" ok bump
check "  the fork version restarts at 9.9.0-p1" version_is bump "9.9.0-p1"
check "  and its record passes" notes_ok bump "9.9.0-p1"

# 5. Upstream adds to a translation catalog the fork also changed: the hash lines conflict.
upstream cat
printf '\tAn upstream string\tアップストリームの文字列\n' >>"$T/up-cat/crates/ui-egui/src/i18n/ja.tsv"
h=$(sha256sum "$T/up-cat/crates/ui-egui/src/i18n/ja.tsv" | cut -d' ' -f1)
awk -v h="$h" '$0 == "path = \"crates/ui-egui/src/i18n/ja.tsv\"" { w = 1 } w && /^sha256 = "/ { $0 = "sha256 = \"" h "\""; w = 0 } { print }' \
  "$T/up-cat/ATTRIBUTION.toml" >"$T/a" && mv "$T/a" "$T/up-cat/ATTRIBUTION.toml"
up cat commit -qam "upstream: a new string"
sync cat
check "catalog hash conflict: merged" ok cat
check "  with upstream's string" grep -q "An upstream string" "$T/s-cat/crates/ui-egui/src/i18n/ja.tsv"
check "  and the fork's strings" grep -q "Install and relaunch" "$T/s-cat/crates/ui-egui/src/i18n/ja.tsv"
check "  and every hash right" assets_ok cat

# 5b. Upstream adds to deny.toml where the fork did: upstream's policy is taken if the tree passes it.
upstream deny
sed -i '/RUSTSEC-2024-0370/a\    { id = "RUSTSEC-2099-0001", reason = "upstream test entry" },' "$T/up-deny/deny.toml"
up deny commit -qam "upstream: a dependency policy change"
# The fork adds an entry at the same place, so the two always conflict, whatever both files hold.
sync deny "sed -i '/RUSTSEC-2024-0370/a\\    { id = \"RUSTSEC-2099-0002\", reason = \"fork test entry\" },' deny.toml && git commit -qam 'Fork: a deny entry'"
check "deny.toml conflict: merged" ok deny
check "  with upstream's entry" grep -q "RUSTSEC-2099-0001" "$T/s-deny/deny.toml"
check "  and the fork's entry is kept or upstream's policy passes alone" grep -q "RUSTSEC-2026-0371" "$T/s-deny/deny.toml"
check "  checked by cargo deny" said deny "cargo deny passes"

# 6. Upstream edits a line of code the fork changed: a person decides.
upstream code
sed -i 's#^pub const RELEASES_PAGE: &str = .*#pub const RELEASES_PAGE: \&str = "https://example.com/upstream";#' "$T/up-code/crates/ui-egui/src/updates.rs"
up code commit -qam "upstream: change the releases page"
sync code
check "a code conflict stops the sync" stopped code "conflicts that need a person"
check "  naming the file" said code "crates/ui-egui/src/updates.rs"

# 7. Upstream edits the fork's own workflow where the fork changed it.
upstream ownwf
sed -i 's/^name: CI$/name: CI (upstream)/; s/runs-on: \[self-hosted, linux\]/runs-on: ubuntu-latest/' "$T/up-ownwf/.github/workflows/ci.yml"
up ownwf commit -qam "upstream: CI on GitHub's runners"
sync ownwf
check "an upstream edit of the fork's own workflow: merged" ok ownwf
check "  keeping the fork's version" bash -c "grep -q '^name: CI$' '$T/s-ownwf/.github/workflows/ci.yml' && ! grep -q ubuntu-latest '$T/s-ownwf/.github/workflows/ci.yml'"
check "  and the record says so" grep -q "upstream's changes to ci.yml were not taken" "$T/s-ownwf/docs/fork/releases/v$next.md"

# 8. release-bump: tagged and unchanged, tagged and changed, untagged.
bump_case() {
  local d="$T/b-$1" rc=0
  git clone -q "$T/fork" "$d"
  (cd "$d" && eval "$2" && BUMP_DRY_RUN=1 BUMP_SKIP_UPSTREAM_CHECK=1 bash packaging/fork/release-bump.sh) >"$d.out" 2>&1 || rc=$?
  echo "$rc" >"$d.rc"
}
git -C "$T/fork" tag -f "v$version" >/dev/null
bump_case same "true"
check "bump: nothing changed since the release" grep -q "nothing changed since v$version" "$T/b-same.out"
before=$(git -C "$T/fork" rev-parse HEAD)
(cd "$T/fork" && git checkout -q -b feature && echo x >>NOTICE && git commit -qam 'A feature' && git checkout -q main \
  && git merge -q --no-ff -m 'Merge pull request #99 from someone/feature' -m 'A feature for people' feature)
bump_case changed "true"
git -C "$T/fork" reset -q --hard "$before"
check "bump: a merged PR makes the next patch release" grep -q "next release $next" "$T/b-changed.out"
check "  listing the PR" grep -q "A feature for people (#99)" "$T/b-changed/docs/fork/releases/v$next.md"
git -C "$T/fork" tag -d "v$version" >/dev/null
bump_case untagged "true"
check "bump: an unreleased version is left to Release" grep -q "isn't released yet" "$T/b-untagged.out"

# 9. merge_when_green with a stand-in gh replaying check-run states.
mkdir -p "$T/gh"
cat >"$T/gh/gh" <<'EOF'
#!/usr/bin/env bash
case "$1 $2" in
  "pr view") echo deadbeef ;;
  "pr merge") echo "merged $3" >>"$GH_LOG" ;;
  "api "*)
    n=$(($(cat "$GH_STEP") + 1)); echo "$n" >"$GH_STEP"
    f=$(sed -n "${n}p" "$GH_STATES"); [ -n "$f" ] || f=$(tail -1 "$GH_STATES")
    jq -r "${@: -1}" <<<"$f" ;;
esac
EOF
chmod +x "$T/gh/gh"
run_merge() {
  printf '%s\n' "$@" >"$T/states"
  echo 0 >"$T/step"
  : >"$T/ghlog"
  (export PATH="$T/gh:$PATH" GH_LOG="$T/ghlog" GH_STEP="$T/step" GH_STATES="$T/states" REPO=x/y
    sleep() { :; }
    # shellcheck source=lib.sh
    . "$SRC/packaging/fork/lib.sh"
    merge_when_green 7 60) >/dev/null 2>&1
}
pending='{"check_runs":[{"name":"test (shared code)","status":"in_progress","conclusion":null}]}'
green='{"check_runs":[{"name":"test (shared code)","status":"completed","conclusion":"success"},{"name":"lint","status":"completed","conclusion":"success"}]}'
red='{"check_runs":[{"name":"test (shared code)","status":"completed","conclusion":"failure"}]}'
other_only='{"check_runs":[{"name":"lint","status":"completed","conclusion":"success"}]}'
check "merge: waits through pending, merges when green" bash -c "$(declare -f run_merge); T='$T' SRC='$SRC'; run_merge '$pending' '$pending' '$green' && grep -q 'merged 7' '$T/ghlog'"
check "merge: a failing check leaves the PR open" bash -c "$(declare -f run_merge); T='$T' SRC='$SRC'; ! run_merge '$pending' '$red' && ! grep -q merged '$T/ghlog'"
check "merge: green without CI's test job never merges" bash -c "$(declare -f run_merge); T='$T' SRC='$SRC'; printf '%s\n' '$other_only' >'$T/states'; ! timeout 5 bash -c '$(declare -f run_merge); T=$T SRC=$SRC; run_merge \"$other_only\"' ; ! grep -q merged '$T/ghlog'"

echo "fork automation: $pass passed, $fail failed"
[ "$fail" = 0 ]
