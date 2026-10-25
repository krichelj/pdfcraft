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

# One xtask binary for every clone: `cargo xtask` in a clone would rebuild it (a new path), and the
# clones' builds would queue on one build directory. xtask reads its checkout from
# CARGO_MANIFEST_DIR at run time (gates::root), so the shim points it at the clone it runs in.
(cd "$SRC" && cargo build -q -p xtask)
XTASK_BIN="$CARGO_TARGET_DIR/debug/xtask"
REAL_CARGO="$(command -v cargo)"
mkdir -p "$T/bin"
cat >"$T/bin/cargo" <<EOF
#!/usr/bin/env bash
if [ "\${1:-}" = xtask ]; then
  shift
  CARGO_MANIFEST_DIR="\$(git rev-parse --show-toplevel)/xtask" exec "$XTASK_BIN" "\$@"
fi
exec "$REAL_CARGO" "\$@"
EOF
chmod +x "$T/bin/cargo"
export PATH="$T/bin:$PATH"

# A stand-in for the on-call LLM, keeping oncall-llm's CLI contract (lib.sh), driven by STUB_LLM:
#   ok (default)  resolve: each conflicted file becomes both sides' lines (git merge-file --union),
#                 staged; fix: runs each --check in order; a failing check with an --autofix runs it
#                 and commits "<prefix>: <name>: mechanical fix"; a failing `test -f FILE` check is
#                 made to pass by creating FILE and committed with the --commit-prefix; any other
#                 failure exits 1. `fix --help` lists --since and --protect, as the real tool's does
#   down          every command exits 2, as when the server can't be reached
#   fixfail       fix exits 1, as when the checks still fail after its rounds
#   protected     fix first commits an edit to .github/workflows/ci.yml (a path it must not touch)
#   ignore        fix first commits a new #[ignore] above a test
#   mute          fix first commits `|| true` onto a command in a packaging script
#   catalog       fix first commits a new line in the Japanese catalog, leaving its hash stale
# Every call is logged to STUB_LLM_LOG. The real tool is never called: ONCALL_LLM is the stand-in.
mkdir -p "$T/llm"
cat >"$T/llm/oncall-llm" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
# lib.sh asks `fix --help` which flags this version takes; that is not a call worth logging.
case " $* " in *" --help "*) echo "usage: oncall-llm fix --repo DIR --check NAME=CMD [--since REF] [--protect GLOB]"; exit 0 ;; esac
echo "oncall-llm $*" >>"${STUB_LLM_LOG:-/dev/null}"
mode="${STUB_LLM:-ok}"
if [ "$mode" = down ]; then echo "oncall-llm: the server can't be reached" >&2; exit 2; fi
cmd="$1"
shift
repo=. prefix="" checks=() autofixes=()
while [ $# -gt 0 ]; do
  case "$1" in
    --repo) repo="$2"; shift 2 ;;
    --check) checks+=("$2"); shift 2 ;;
    --autofix) autofixes+=("$2"); shift 2 ;;
    --context) cat "$2" >>"${STUB_LLM_LOG:-/dev/null}.context"; shift 2 ;;
    --commit-prefix) prefix="$2"; shift 2 ;;
    --*) shift 2 ;;
    *) shift ;;
  esac
done
cd "$repo"
case "$cmd" in
  resolve)
    for f in $(git diff --name-only --diff-filter=U); do
      git show ":2:$f" >"$f.ours" && git show ":1:$f" >"$f.base" && git show ":3:$f" >"$f.theirs"
      git merge-file --union -p "$f.ours" "$f.base" "$f.theirs" >"$f" || true
      rm -f "$f.ours" "$f.base" "$f.theirs"
      git add "$f"
    done
    ;;
  fix)
    if [ "$mode" = fixfail ]; then echo "oncall-llm: still failing after 8 rounds" >&2; exit 1; fi
    case "$mode" in
      protected) mkdir -p .github/workflows && echo "# the LLM was here" >>.github/workflows/ci.yml ;;
      ignore) printf '#[cfg(test)]\nmod stub_llm {\n    #[test]\n    #[ignore]\n    fn flaky() {}\n}\n' >>crates/geom/src/lib.rs ;;
      mute) echo 'cargo nextest run --workspace || true' >>packaging/macos/test-install.sh ;;
      catalog) printf '\tA string the LLM added\tLLMが追加した文字列\n' >>crates/ui-egui/src/i18n/ja.tsv ;;
    esac
    if [ -n "$(git status --porcelain)" ]; then git add -A && git commit -qm "$prefix: $mode (round 1)"; fi
    for c in "${checks[@]}"; do
      name="${c%%=*}" command="${c#*=}"
      bash -c "$command" >/dev/null 2>&1 && continue
      for a in "${autofixes[@]}"; do
        if [ "${a%%=*}" = "$name" ] && bash -c "${a#*=}" >/dev/null 2>&1 && bash -c "$command" >/dev/null 2>&1; then
          git add -A && git commit -qm "$prefix: $name: mechanical fix"
          continue 2
        fi
      done
      case "$command" in "test -f "*) touch "${command#test -f }" ;; *) exit 1 ;; esac
      bash -c "$command" >/dev/null 2>&1 || exit 1
      git add -A && git commit -qm "$prefix: $name (round 1)"
    done
    ;;
  health) echo ready ;;
esac
EOF
chmod +x "$T/llm/oncall-llm"
export ONCALL_LLM="$T/llm/oncall-llm" SYNC_FIX_CHECKS="always=true"

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
# Usage: sync <name> [fork-side command] [VAR=value…]: dry-run the sync of that upstream in a fresh
# clone at "$T/s-<name>", after running the command there (a fork commit the case needs) when one is
# given, with the variables set (the stand-in LLM's mode, say). The LLM's calls go to "$T/s-<name>.llm".
sync() {
  local n="$1" d="$T/s-$1" rc=0 prep="${2:-}"
  shift
  [ $# = 0 ] || shift
  git clone -q "$T/fork" "$d"
  if [ -n "$prep" ]; then (cd "$d" && eval "$prep"); fi
  : >"$d.llm"
  # In the background: the cases run side by side (`wait` below).
  (
    cd "$d" && env SYNC_DRY_RUN=1 SYNC_FORK_REF=HEAD SYNC_UPSTREAM_URL="$T/up-$n" STUB_LLM_LOG="$d.llm" "$@" \
      bash packaging/fork/sync-upstream.sh >"$d.out" 2>&1 || rc=$?
    echo "$rc" >"$d.rc"
  ) &
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

# Every sync case is set up and started first; they run side by side, then are checked.
# 1. Nothing new upstream.
upstream none
sync none

# 2. An ordinary upstream change.
upstream plain
echo "upstream note" >>"$T/up-plain/NOTICE"
up plain commit -qam "upstream: an ordinary change"
sync plain

# 3. Upstream adds a workflow for another platform or GitHub's runners.
upstream wf
printf 'name: X\non: push\njobs:\n  x:\n    runs-on: ubuntu-latest\n    steps:\n      - run: true\n' >"$T/up-wf/.github/workflows/x.yml"
up wf add -A
up wf commit -qm "upstream: a workflow the fork doesn't run"
sync wf

# 4. Upstream moves its version: the fork's line restarts at <new base>-p1.
upstream bump
# As `cargo xtask version set 9.9.0` does upstream: the workspace version and every internal crate's.
old=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$T/up-bump/Cargo.toml" | head -1)
sed -i "s/\"$old\"/\"9.9.0\"/g" "$T/up-bump/Cargo.toml"
up bump commit -qam "upstream: release 9.9.0"
sync bump

# 5. Upstream adds to a translation catalog the fork also changed: the hash lines conflict.
upstream cat
printf '\tAn upstream string\tアップストリームの文字列\n' >>"$T/up-cat/crates/ui-egui/src/i18n/ja.tsv"
h=$(sha256sum "$T/up-cat/crates/ui-egui/src/i18n/ja.tsv" | cut -d' ' -f1)
awk -v h="$h" '$0 == "path = \"crates/ui-egui/src/i18n/ja.tsv\"" { w = 1 } w && /^sha256 = "/ { $0 = "sha256 = \"" h "\""; w = 0 } { print }' \
  "$T/up-cat/ATTRIBUTION.toml" >"$T/a" && mv "$T/a" "$T/up-cat/ATTRIBUTION.toml"
up cat commit -qam "upstream: a new string"
sync cat

# 5b. Upstream adds to deny.toml where the fork did: upstream's policy is taken if the tree passes it.
upstream deny
sed -i '/RUSTSEC-2024-0370/a\    { id = "RUSTSEC-2099-0001", reason = "upstream test entry" },' "$T/up-deny/deny.toml"
up deny commit -qam "upstream: a dependency policy change"
# The fork adds an entry at the same place, so the two always conflict, whatever both files hold.
sync deny "sed -i '/RUSTSEC-2024-0370/a\\    { id = \"RUSTSEC-2099-0002\", reason = \"fork test entry\" },' deny.toml && git commit -qam 'Fork: a deny entry'"

# 6. Upstream edits a line of code the fork changed. With the on-call LLM it resolves the
#    conflict; without it (SYNC_NO_LLM=1, or the server down) a person decides.
code_change() {
  upstream "$1"
  sed -i 's#^pub const RELEASES_PAGE: &str = .*#pub const RELEASES_PAGE: \&str = "https://example.com/upstream";#' "$T/up-$1/crates/ui-egui/src/updates.rs"
  up "$1" commit -qam "upstream: change the releases page"
}
code_change code
sync code "" SYNC_NO_LLM=1
code_change llm
sync llm
code_change llmdown
sync llmdown "" STUB_LLM=down

# 6b. The merged tree fails a check: the LLM's fix is committed before the release commit; a fix
#     that doesn't make the checks pass stops the sync before anything is pushed.
upstream fixed
echo "upstream note" >>"$T/up-fixed/NOTICE"
up fixed commit -qam "upstream: an ordinary change"
sync fixed "" SYNC_FIX_CHECKS="marker=test -f SYNC_FIXED"
upstream fixfail
echo "upstream note" >>"$T/up-fixfail/NOTICE"
up fixfail commit -qam "upstream: an ordinary change"
sync fixfail "" STUB_LLM=fixfail

# 6b. The merged assets don't check out (upstream changed an icon, not its hash; a merge that
#     breaks xtask fails the same way). Without the LLM a person decides, shown the check's output;
#     with it, the sync goes on to the fix loop, which runs that check.
asset_change() {
  upstream "$1"
  printf '<!-- changed upstream -->\n' >>"$T/up-$1/assets/icons/accessibility.svg"
  up "$1" commit -qam "upstream: an icon changed without its hash"
}
asset_change assetbad
sync assetbad "" SYNC_NO_LLM=1
asset_change assetllm
sync assetllm

# 7. Upstream edits the fork's own workflow where the fork changed it.
upstream ownwf
sed -i 's/^name: CI$/name: CI (upstream)/; s/runs-on: \[self-hosted, linux\]/runs-on: ubuntu-latest/' "$T/up-ownwf/.github/workflows/ci.yml"
up ownwf commit -qam "upstream: CI on GitHub's runners"
sync ownwf

# 6c. What the LLM's fixes may never do: touch what CI runs or the fork's automation, disable a test,
#     mute a command. Each stops the sync before anything is pushed. A catalog it edits is re-hashed
#     by the assets check's mechanical fix, so the sync goes on.
for m in protected ignore mute catalog; do
  upstream "g$m"
  echo "upstream note" >>"$T/up-g$m/NOTICE"
  up "g$m" commit -qam "upstream: an ordinary change"
done
sync gprotected "" STUB_LLM=protected
sync gignore "" STUB_LLM=ignore
sync gmute "" STUB_LLM=mute
sync gcatalog "" STUB_LLM=catalog SYNC_FIX_CHECKS="assets=cargo xtask assets"

wait

check "nothing new: does nothing" ok none
check "  and says so" said none "nothing to do"
check "an upstream change is merged" ok plain
check "  with the change in the tree" grep -q "upstream note" "$T/s-plain/NOTICE"
check "  the fork's next patch version, $next" version_is plain "$next"
check "  and its record, which passes, naming the commit" notes_ok plain "$next"
check "  the record lists the upstream commit" grep -q "upstream: an ordinary change" "$T/s-plain/docs/fork/releases/v$next.md"
check "  assets still check out" assets_ok plain
check "a new upstream workflow: merged" ok wf
check "  and removed" bash -c "! test -e '$T/s-wf/.github/workflows/x.yml'"
check "  the fork's workflows stay" in_tree wf .github/workflows/sync-upstream.yml
check "an upstream version bump: merged" ok bump
check "  the fork version restarts at 9.9.0-p1" version_is bump "9.9.0-p1"
check "  and its record passes" notes_ok bump "9.9.0-p1"
check "catalog hash conflict: merged" ok cat
check "  with upstream's string" grep -q "An upstream string" "$T/s-cat/crates/ui-egui/src/i18n/ja.tsv"
check "  and the fork's strings" grep -q "Install and relaunch" "$T/s-cat/crates/ui-egui/src/i18n/ja.tsv"
check "  and every hash right" assets_ok cat
check "deny.toml conflict: merged" ok deny
check "  with upstream's entry" grep -q "RUSTSEC-2099-0001" "$T/s-deny/deny.toml"
check "  and the fork's entry is kept or upstream's policy passes alone" grep -q "RUSTSEC-2026-0371" "$T/s-deny/deny.toml"
check "  checked by cargo deny" said deny "cargo deny passes"
check "a code conflict without the LLM (SYNC_NO_LLM=1) stops the sync" stopped code "conflicts that need a person"
check "  naming the file" said code "crates/ui-egui/src/updates.rs"
check "  without asking the LLM anything" bash -c "! test -s '$T/s-code.llm'"
check "a code conflict is resolved by the on-call LLM and merged" ok llm
check "  which got the conflicted repo and the fork's merge rules" \
  grep -q "^oncall-llm resolve --repo $T/s-llm --instructions $T/s-llm/packaging/fork/oncall-instructions.md" "$T/s-llm.llm"
check "  both sides' line in the file, no markers left" bash -c \
  "grep -q example.com/upstream '$T/s-llm/crates/ui-egui/src/updates.rs' && ! grep -q '^<<<<<<<' '$T/s-llm/crates/ui-egui/src/updates.rs'"
check "  the merge commit has it" bash -c "git -C '$T/s-llm' log --merges --format=%s | grep -q 'Merge storytold/pdfcraft main'"
check "  and the record says the LLM resolved it" grep -q "The on-call LLM resolved the merge conflicts in: \`crates/ui-egui/src/updates.rs\`" "$T/s-llm/docs/fork/releases/v$next.md"
check "  then CI's checks ran through the LLM's fix loop" grep -q "^oncall-llm fix --repo $T/s-llm --check always=true .*--commit-prefix Sync fix" "$T/s-llm.llm"
check "the LLM's server down: the code conflict stops the sync" stopped llmdown "conflicts that need a person"
check "  after asking it" grep -q "^oncall-llm resolve" "$T/s-llmdown.llm"
check "a check the merge broke is fixed by the LLM" ok fixed
check "  its fix committed on the branch before the release commit" bash -c \
  "[ \"\$(git -C '$T/s-fixed' log --format=%s -2 | tail -1)\" = 'Sync fix: marker (round 1)' ] && git -C '$T/s-fixed' log -1 --format=%s | grep -q '^Release: PdfCraft'"
check "  the fixed tree is what is released" in_tree fixed SYNC_FIXED
check "  and the record lists the fix" grep -q "  - Sync fix: marker (round 1)" "$T/s-fixed/docs/fork/releases/v$next.md"
check "checks still failing after the LLM's rounds stop the sync" stopped fixfail "still fails its checks"

check "merged assets that don't check out stop a sync without the LLM" stopped assetbad "don't check out"
check "  showing what the check said" said assetbad "accessibility.svg"
check "  with the LLM, the sync goes on to its fix loop" said assetllm "the on-call LLM's fix loop runs that check"
check "  and the fix loop ran" grep -q "^oncall-llm fix " "$T/s-assetllm.llm"

# rehash_catalogs rewrites only a catalog's own [[asset]] block. Upstream 0.13.0 lists catalogs
# without a sha256 line; the old rewrite then put the catalog's hash on the next asset's line.
rh="$T/rehash"
mkdir -p "$rh/crates/ui-egui/src/i18n" "$rh/assets"
printf 'key\tvalue\n' >"$rh/crates/ui-egui/src/i18n/xx.tsv"
printf '[[asset]]\npath = "crates/ui-egui/src/i18n/xx.tsv"\nkind = "translation"\n\n[[asset]]\npath = "assets/icon.svg"\nsha256 = "1111"\n\n[[asset]]\npath = "crates/ui-egui/src/i18n/yy.tsv"\nsha256 = "2222"\n' >"$rh/ATTRIBUTION.toml"
printf 'other\tstring\n' >"$rh/crates/ui-egui/src/i18n/yy.tsv"
(cd "$rh" && . "$SRC/packaging/fork/lib.sh" && rehash_catalogs)
check "rehash leaves the next asset's hash alone when a catalog has none" grep -q '^sha256 = "1111"$' "$rh/ATTRIBUTION.toml"
check "  and still re-hashes a catalog that has one" grep -q "^sha256 = \"$(sha256sum "$rh/crates/ui-egui/src/i18n/yy.tsv" | cut -d' ' -f1)\"$" "$rh/ATTRIBUTION.toml"
check "  before anything is pushed" bash -c "! grep -q 'stopping before the push' '$T/s-fixfail.out'"
check "the LLM's fix touching a protected path stops the sync" stopped gprotected "protected path(s): .github/workflows/ci.yml"
check "  before anything is pushed" bash -c "! grep -q 'stopping before the push' '$T/s-gprotected.out'"
check "  and the LLM was told what it may not touch" grep -q -- "--protect .github/\*\* --protect packaging/fork/\*\*" "$T/s-gprotected.llm"
check "  and which files the merge changed" grep -q -- "--since sync-base" "$T/s-gprotected.llm"
check "the LLM's fix adding #[ignore] stops the sync" stopped gignore "disables a test or hides a failure"
check "  naming the line" said gignore "#\[ignore\]"
check "the LLM's fix muting a command with || true stops the sync" stopped gmute "disables a test or hides a failure"
check "the LLM's catalog edit is re-hashed by the assets fix, and the sync goes on" ok gcatalog
check "  with the LLM's string" grep -q "A string the LLM added" "$T/s-gcatalog/crates/ui-egui/src/i18n/ja.tsv"
check "  every hash right" assets_ok gcatalog
check "  through the mechanical fix" bash -c "git -C '$T/s-gcatalog' log --format=%s | grep -q '^Sync fix: assets: mechanical fix$'"
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
  "run list") echo 4242 ;;
  "run view")
    if [ -n "${GH_BIGLOG:-}" ]; then
      # As `gh run view --log-failed` prints a failed Test step: job, step and time on every line,
      # colour codes, thousands of PASS lines, then the failure.
      pre=$'test (shared code)\tRun tests\t2026-10-25T00:36:03.9092598Z '
      for i in $(seq 1 6000); do printf '%s\033[32m        PASS\033[0m [   0.010s] pdfcraft-cos tests::case_%d\n' "$pre" "$i"; done
      printf "%s    Compiling pdfcraft-x v0.1.0\n" "$pre"
      printf "%sthread 'tests::page' panicked at crates/x/src/lib.rs:3:5:\n%sassertion \`left == right\` failed\n" "$pre" "$pre"
      printf '%s        \033[31mFAIL\033[0m [   0.020s] pdfcraft-x tests::page\n' "$pre"
    else
      echo "test (shared code): thread 'tests::page' panicked at crates/x/src/lib.rs:3"
    fi ;;
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
  # shellcheck disable=SC2030 # each run sets its environment in its own subshell, on purpose
  (export PATH="$T/gh:$PATH" GH_LOG="$T/ghlog" GH_STEP="$T/step" GH_STATES="$T/states" REPO=x/y
    sleep() { :; }
    # shellcheck source=lib.sh
    . "$SRC/packaging/fork/lib.sh"
    merge_when_green 7 "${MW_TIMEOUT:-60}") >/dev/null 2>&1
}
pending='{"check_runs":[{"name":"test (shared code)","status":"in_progress","conclusion":null}]}'
green='{"check_runs":[{"name":"test (shared code)","status":"completed","conclusion":"success"},{"name":"lint","status":"completed","conclusion":"success"}]}'
red='{"check_runs":[{"name":"test (shared code)","status":"completed","conclusion":"failure"}]}'
other_only='{"check_runs":[{"name":"lint","status":"completed","conclusion":"success"}]}'
check "merge: waits through pending, merges when green" bash -c "$(declare -f run_merge); T='$T' SRC='$SRC'; run_merge '$pending' '$pending' '$green' && grep -q 'merged 7' '$T/ghlog'"
check "merge: a failing check leaves the PR open" bash -c "$(declare -f run_merge); T='$T' SRC='$SRC'; ! run_merge '$pending' '$red' && ! grep -q merged '$T/ghlog'"
check "merge: green without CI's test job never merges" bash -c "$(declare -f run_merge); T='$T' SRC='$SRC'; ! MW_TIMEOUT=2 run_merge '$other_only' && ! grep -q merged '$T/ghlog'"

self_pending='{"check_runs":[{"name":"test (shared code)","status":"completed","conclusion":"success"},{"name":"merge when green","status":"in_progress","conclusion":null}]}'
check "auto-merge: its own running check doesn't block the merge" bash -c "$(declare -f run_merge); T='$T' SRC='$SRC'; MERGE_IGNORE_CHECK='merge when green' run_merge '$self_pending' && grep -q 'merged 7' '$T/ghlog'"
check "  but without that exemption it never counts as green" bash -c "$(declare -f run_merge); T='$T' SRC='$SRC'; ! MW_TIMEOUT=2 run_merge '$self_pending' && ! grep -q merged '$T/ghlog'"

# 10. merge_when_green_or_fix: a failing check goes to the LLM with the failed job's log, its fix is
#     pushed and the new head's checks awaited, at most SYNC_CI_FIX_RETRIES (2) times.
run_fixing() {
  printf '%s\n' "$@" >"$T/states"
  echo 0 >"$T/step"
  : >"$T/ghlog"
  : >"$T/llmlog"
  rm -f "$T/llmlog.context"
  rm -rf "$T/pr"
  git init -q "$T/pr"
  git -C "$T/pr" commit -q --allow-empty -m base
  # shellcheck disable=SC2030,SC2031 # as in run_merge: this run's environment, in its own subshell
  (export PATH="$T/gh:$PATH" GH_LOG="$T/ghlog" GH_STEP="$T/step" GH_STATES="$T/states" REPO=x/y ROOT="$T/pr" \
    STUB_LLM_LOG="$T/llmlog" SYNC_FIX_CHECKS="ci=test -f CI_FIXED"
    sleep() { :; }
    # shellcheck source=lib.sh
    . "$SRC/packaging/fork/lib.sh"
    push_as_owner() { echo "pushed $1 at $(git -C "$ROOT" log -1 --format=%s)" >>"$GH_LOG"; }
    merge_when_green_or_fix 7 60 sync/x) >/dev/null 2>&1
}
fixes() { grep -c "^oncall-llm fix " "$T/llmlog" || true; }
check "CI red: the LLM fixes it with the failed log, pushes, then merges" bash -c "$(declare -f run_fixing fixes); T='$T' SRC='$SRC'
  run_fixing '$red' '$pending' '$green' && grep -q 'merged 7' '$T/ghlog' && [ \"\$(fixes)\" = 1 ] \
    && grep -q 'pushed sync/x at Sync fix (CI, attempt 1): ci (round 1)' '$T/ghlog' \
    && grep -q \"panicked at crates/x/src/lib.rs:3\" '$T/llmlog.context'"
check "  red after every fix: two attempts, then left open" bash -c "$(declare -f run_fixing fixes); T='$T' SRC='$SRC'
  ! run_fixing '$red' && [ \"\$(fixes)\" = 2 ] && ! grep -q merged '$T/ghlog'"
check "  without the LLM: left open at once" bash -c "$(declare -f run_fixing fixes); T='$T' SRC='$SRC'
  ! SYNC_NO_LLM=1 run_fixing '$red' && [ \"\$(fixes)\" = 0 ] && ! grep -q merged '$T/ghlog'"
check "  a CI fix touching a protected path is never pushed" bash -c "$(declare -f run_fixing fixes); T='$T' SRC='$SRC'
  ! STUB_LLM=protected run_fixing '$red' && [ \"\$(fixes)\" = 1 ] && ! grep -q '^pushed' '$T/ghlog' && ! grep -q merged '$T/ghlog'"
check "  the LLM reads a digest of a 900 KB-class failed log, not the log" bash -c "$(declare -f run_fixing fixes); T='$T' SRC='$SRC'
  GH_BIGLOG=1 run_fixing '$red' '$pending' '$green' \
    && grep -q \"panicked at crates/x/src/lib.rs:3:5\" '$T/llmlog.context' \
    && grep -q 'FAIL \[   0.020s\] pdfcraft-x tests::page' '$T/llmlog.context' \
    && ! grep -q 'PASS \[' '$T/llmlog.context' && ! grep -q \$'\\t' '$T/llmlog.context' \
    && ! grep -q \$'\\033' '$T/llmlog.context' && [ \"\$(wc -c <'$T/llmlog.context')\" -le 40000 ]"

echo "fork automation: $pass passed, $fail failed"
[ "$fail" = 0 ]
