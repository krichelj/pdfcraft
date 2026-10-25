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
#   comment       fix first commits packaging/macos/package.sh's CLI_ZIP line commented out
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
    # As the real tool: committing fixes over uncommitted tracked work is refused.
    if [ -n "$prefix" ] && ! git diff HEAD --quiet; then
      echo "oncall-llm: tracked files have uncommitted changes: commit or stash them before fixing with commits" >&2
      exit 2
    fi
    if [ "$mode" = fixfail ]; then echo "oncall-llm: still failing after 8 rounds" >&2; exit 1; fi
    case "$mode" in
      protected) mkdir -p .github/workflows && echo "# the LLM was here" >>.github/workflows/ci.yml ;;
      ignore) printf '#[cfg(test)]\nmod stub_llm {\n    #[test]\n    #[ignore]\n    fn flaky() {}\n}\n' >>crates/geom/src/lib.rs ;;
      mute) echo 'cargo nextest run --workspace || true' >>packaging/macos/test-install.sh ;;
      catalog) printf '\tA string the LLM added\tLLMが追加した文字列\n' >>crates/ui-egui/src/i18n/ja.tsv ;;
      # As the real fix loop did to quiet shellcheck's "appears unused" in the 0.13.0 sync.
      comment) sed -i 's/^CLI_ZIP=/###CLI_ZIP=/' packaging/macos/package.sh ;;
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
# A stand-in gh that lists no PRs: the scenarios never read the real repository, whose open PRs
# change under them (the real Release workflow's release/v<next> PR, open for three minutes, once
# sent the bump case down the existing-PR path). A case that needs a PR brings its own gh.
mkdir -p "$T/ghnone"
# shellcheck disable=SC2016 # the stand-in's own $*, written as it is
printf '#!/usr/bin/env bash\necho "gh $*" >>"%s/ghnone/calls"\n' "$T" >"$T/ghnone/gh"
chmod +x "$T/ghnone/gh"
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
    cd "$d" && env PATH="$T/ghnone:$PATH" SYNC_DRY_RUN=1 SYNC_FORK_REF=HEAD SYNC_UPSTREAM_URL="$T/up-$n" STUB_LLM_LOG="$d.llm" "$@" \
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

# 2b. A long upstream history (the 0.13.0 sync brought 1,602 commits): the record lists the newest
#     40 and counts the rest. Their subjects outgrow a pipe buffer, so a reader that stops early must
#     not kill the writer (git log | head under pipefail died of SIGPIPE, exit 141).
upstream many
tip=$(up many rev-parse HEAD)
{
  for i in $(seq 1 1500); do
    printf 'commit refs/heads/main\ncommitter T <t@example.com> %d +0000\n' $((1700000000 + i))
    printf 'data <<EOM\nupstream: change %04d, with a subject as long as a real one so the list outgrows a pipe buffer\nEOM\n' "$i"
    if [ "$i" = 1 ]; then printf 'from %s\n' "$tip"; fi
    printf '\n'
  done
} | up many fast-import --quiet
up many reset -q --hard main
sync many

# 2c. A sync PR is already open (a stand-in gh lists it as #77). One behind main (opened before a
#     fix to this automation landed, as #42 was) is rebuilt from today's main; one on today's main is
#     worked on in place, which a dry run never does: it pushes nothing and merges nothing.
mkdir -p "$T/ghpr"
# shellcheck disable=SC2016 # the stand-in's own $1 $2 $*, written as they are
printf '#!/usr/bin/env bash\ncase "$1 $2" in\n  "pr list") echo 77 ;;\n  *) echo "gh $*" >>"%s/ghpr/calls" ;;\nesac\n' "$T" >"$T/ghpr/gh"
chmod +x "$T/ghpr/gh"
for n in stalepr currentpr; do
  upstream "$n"
  echo "upstream note" >>"$T/up-$n/NOTICE"
  up "$n" commit -qam "upstream: an ordinary change"
done
# The open PR's branch: before main's last commit (stale), or on it (current).
git -C "$T/fork" branch -f "sync/upstream-$(git -C "$T/up-stalepr" rev-parse --short=12 HEAD)" HEAD~1
git -C "$T/fork" branch -f "sync/upstream-$(git -C "$T/up-currentpr" rev-parse --short=12 HEAD)" HEAD
sync stalepr "" PATH="$T/ghpr:$PATH"
sync currentpr "" PATH="$T/ghpr:$PATH"
# The rebuild for real (no dry run): the branch is replaced on its origin and the PR's title and
# record updated, through the REST API (`gh pr edit` fails on GitHub's retired classic projects).
# The stand-in gh reports no checks, so the sync then stops waiting for them.
upstream rebuilt
echo "upstream note" >>"$T/up-rebuilt/NOTICE"
up rebuilt commit -qam "upstream: an ordinary change"
rebuilt_branch="sync/upstream-$(git -C "$T/up-rebuilt" rev-parse --short=12 HEAD)"
git -C "$T/fork" branch -f "$rebuilt_branch" HEAD~1
mkdir -p "$T/ghrb"
# shellcheck disable=SC2016 # the stand-in's own $1 $2 $*, written as they are
printf '#!/usr/bin/env bash\necho "gh $*" >>"%s/ghrb/calls"\ncase "$1 $2" in\n  "pr list") echo 77 ;;\n  "pr edit") echo "GraphQL: Projects (classic) is being deprecated" >&2; exit 1 ;;\nesac\n' "$T" >"$T/ghrb/gh"
chmod +x "$T/ghrb/gh"
sync rebuilt "" PATH="$T/ghrb:$PATH" SYNC_DRY_RUN= SYNC_CHECKS_TIMEOUT=1

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
awk -v h="$h" '/^\[\[asset\]\]/ { w = 0 } $0 == "path = \"crates/ui-egui/src/i18n/ja.tsv\"" { w = 1 } w && /^sha256 = "/ { $0 = "sha256 = \"" h "\""; w = 0 } { print }' \
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
# The same conflict for real (no dry run): the sync stops before any push and opens an issue; with
# an issue already open for that branch (a stand-in gh lists #5), it comments there instead.
code_change codeissue
sync codeissue "" SYNC_NO_LLM=1 SYNC_DRY_RUN=
code_change codeissue2
mkdir -p "$T/ghissue"
# shellcheck disable=SC2016 # the stand-in's own $1 $2 $*, written as they are
printf '#!/usr/bin/env bash\necho "gh $*" >>"%s/ghissue/calls"\ncase "$1 $2" in "issue list") echo 5 ;; esac\n' "$T" >"$T/ghissue/gh"
chmod +x "$T/ghissue/gh"
sync codeissue2 "" SYNC_NO_LLM=1 SYNC_DRY_RUN= PATH="$T/ghissue:$PATH"
# With issues disabled (a stand-in gh whose issue commands fail), the sync still says what stopped it.
code_change codenoissue
mkdir -p "$T/ghnoissue"
# shellcheck disable=SC2016 # the stand-in's own $1, written as it is
printf '#!/usr/bin/env bash\ncase "$1" in issue) echo "the repository has disabled issues" >&2; exit 1 ;; esac\n' >"$T/ghnoissue/gh"
chmod +x "$T/ghnoissue/gh"
sync codenoissue "" SYNC_NO_LLM=1 SYNC_DRY_RUN= PATH="$T/ghnoissue:$PATH"
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

# 6c. The merged Cargo.lock is not what cargo writes for the merged manifests (upstream's lock,
#     taken on a conflict, lacks the fork's own dependencies, e.g. muda for the menu bar): the
#     first build rewrites it, and the sync commits that before the fix loop, which refuses to
#     commit over uncommitted tracked work.
upstream lock
# Upstream adds a dependency to the app where the fork added muda (its menu bar), so Cargo.lock
# conflicts and the sync takes upstream's, which lacks muda: cargo adds it back on the first build.
awk '/^name = "pdfcraft"$/ { app = 1 } app && $0 == " \"log\"," { print; print " \"memchr\","; app = 0; next } { print }' \
  "$T/up-lock/Cargo.lock" >"$T/lock.tmp" && mv "$T/lock.tmp" "$T/up-lock/Cargo.lock"
up lock commit -qam "upstream: the app gains a dependency"
sync lock

# 7. Upstream edits the fork's own workflow where the fork changed it.
upstream ownwf
sed -i 's/^name: CI$/name: CI (upstream)/; s/runs-on: \[self-hosted, linux\]/runs-on: ubuntu-latest/' "$T/up-ownwf/.github/workflows/ci.yml"
up ownwf commit -qam "upstream: CI on GitHub's runners"
sync ownwf

# 6c. What the LLM's fixes may never do: touch what CI runs or the fork's automation, disable a test,
#     mute a command. Each stops the sync before anything is pushed. A catalog it edits leaves the
#     assets checking out (re-hashed by the assets check's mechanical fix while catalogs had hashes),
#     so the sync goes on.
for m in protected ignore mute catalog comment; do
  upstream "g$m"
  echo "upstream note" >>"$T/up-g$m/NOTICE"
  up "g$m" commit -qam "upstream: an ordinary change"
done
sync gprotected "" STUB_LLM=protected
sync gignore "" STUB_LLM=ignore
sync gmute "" STUB_LLM=mute
sync gcomment "" STUB_LLM=comment
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
check "an open sync PR behind main is rebuilt from today's main" said stalepr "PR #77 is behind main; rebuilding it"
check "  through the merge to its dry-run stop" said stalepr "dry run; stopping before the push"
check "an open sync PR on today's main: a dry run stops there" said currentpr "dry run; PR #77 is open on the current main"
check "  without asking the LLM or touching the PR" bash -c "[ ! -s '$T/s-currentpr.llm' ] && [ ! -s '$T/ghpr/calls' ]"
check "a rebuilt sync PR's branch is replaced on its origin" bash -c \
  "git -C '$T/fork' merge-base --is-ancestor \"\$(git -C '$T/fork' rev-parse HEAD)\" '$rebuilt_branch' && git -C '$T/fork' log -1 --format=%s '$rebuilt_branch' | grep -q '^Release: PdfCraft'"
check "  and the PR's title and record updated through the REST API" grep -q "api -X PATCH repos/krichelj/pdfcraft/pulls/77 -f title=Sync upstream" "$T/ghrb/calls"
check "  then the sync waits for the PR's checks" said rebuilt "checks didn't finish in time"
check "a long upstream history is merged" ok many
check "  its record lists the newest 40 commits and counts the rest" bash -c \
  "grep -q 'upstream: change 1461,' '$T/s-many/docs/fork/releases/v$next.md' && grep -q 'and 1460 more' '$T/s-many/docs/fork/releases/v$next.md' && ! grep -q 'upstream: change 1460,' '$T/s-many/docs/fork/releases/v$next.md'"
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
check "  saying it would ask a person (a dry run opens no issue)" said code "dry run; would ask a person: upstream merge conflicts"
check "  for real: an issue for the owner" grep -q "issue create --repo krichelj/pdfcraft --title sync/upstream-$(git -C "$T/up-codeissue" rev-parse --short=12 HEAD): upstream merge conflicts" "$T/ghnone/calls"
# shellcheck disable=SC2016 # literal backticks, the issue body's Markdown
check "  naming the conflicted file" grep -q '^- `crates/ui-egui/src/updates.rs`$' "$T/ghnone/calls"
check "  before anything is pushed" bash -c "! grep -q 'git-credential\\|stopping before the push' '$T/s-codeissue.out' && ! git -C '$T/fork' rev-parse -q --verify 'sync/upstream-$(git -C "$T/up-codeissue" rev-parse --short=12 HEAD)' >/dev/null"
check "  with issues disabled, it still names the conflict" stopped codenoissue "conflicts that need a person: crates/ui-egui/src/updates.rs"
check "  and warns that it couldn't open the issue" said codenoissue "couldn't open an issue to ask a person"
check "  and once per sync branch: an open issue gets a comment" grep -q "^gh issue comment 5 " "$T/ghissue/calls"
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

check "a lockfile cargo rewrites after the merge: merged" ok lock
check "  the rewrite committed before the fix loop" bash -c "git -C '$T/s-lock' log --format=%s | grep -q '^Fork: Cargo.lock as cargo writes it'"
check "  and nothing left uncommitted" bash -c "git -C '$T/s-lock' diff HEAD --quiet"
# Upstream's lock, taken on the conflict, has the tokio of the base (an advisory's version, 1.53.1);
# the fork had moved past it. The merged lock keeps the fork's.
check "  with the fork's newer tokio, not upstream's older one" bash -c \
  "[ \"\$(grep -A1 '^name = \"tokio\"$' '$T/s-lock/Cargo.lock' | sed -n 's/^version = //p')\" = \"\$(grep -A1 '^name = \"tokio\"$' '$T/fork/Cargo.lock' | sed -n 's/^version = //p')\" ]"

# private-tmp.sh: the command gets an empty temp directory of its own, which is gone afterwards
# however the command ends, and the command's exit status is the script's.
pt="$T/private-tmp"
mkdir -p "$pt"
pt_run() { PRIVATE_TMP_PARENT="$pt" "$SRC/packaging/fork/private-tmp.sh" "$@"; }
check "private-tmp: the command's temp directory is new, empty and under the parent" bash -c \
  "$(declare -f pt_run); pt='$pt' SRC='$SRC'; d=\$(pt_run bash -c 'ls -A \"\$TMPDIR\" | wc -l; echo \"\$TMPDIR\"'); [ \"\$(echo \"\$d\" | head -1)\" = 0 ] && case \"\$(echo \"\$d\" | tail -1)\" in '$pt'/pdfcraft-tests.*) true ;; *) false ;; esac"
check "  what the command leaves there is removed, and its status kept" bash -c \
  "$(declare -f pt_run); pt='$pt' SRC='$SRC'; pt_run bash -c 'mkdir -p \"\$TMPDIR/a/b\"; touch \"\$TMPDIR/a/b/f\"; exit 7' 2>/dev/null; [ \$? = 7 ] && [ -z \"\$(ls -A '$pt')\" ]"
check "  and when the command succeeds" bash -c \
  "$(declare -f pt_run); pt='$pt' SRC='$SRC'; pt_run bash -c 'touch \"\$TMPDIR/f\"' 2>/dev/null && [ -z \"\$(ls -A '$pt')\" ]"
check "  and when the job is cancelled (SIGTERM)" bash -c \
  "$(declare -f pt_run); pt='$pt' SRC='$SRC'; timeout -s TERM 2 bash -c \"\$(declare -f pt_run); pt='$pt' SRC='$SRC'; pt_run bash -c 'touch \\\"\\\$TMPDIR/f\\\"; sleep 30'\" 2>/dev/null; [ -z \"\$(ls -A '$pt')\" ]"
check "  and with no command it refuses" bash -c "! '$SRC/packaging/fork/private-tmp.sh' 2>/dev/null"

# keep_newer_locked with a stand-in cargo: which packages it raises, and to what.
kn="$T/keepnewer"
mkdir -p "$kn/bin"
printf '#!/usr/bin/env bash\necho "cargo $*" >>"%s/calls"\n' "$kn" >"$kn/bin/cargo"
chmod +x "$kn/bin/cargo"
pkg() { printf '[[package]]\nname = "%s"\nversion = "%s"\n%s\n' "$1" "$2" "${3:-source = \"registry+https://github.com/rust-lang/crates.io-index\"}"; }
{ pkg tokio 1.54.1; pkg serde 1.0.200; pkg foo 0.3.5; pkg bar 0.4.0; pkg pdfcraft 9.0.0 ""; } >"$kn/fork.lock"
{ pkg tokio 1.53.1; pkg serde 1.0.210; pkg foo 0.3.2; pkg bar 0.3.9; pkg pdfcraft 1.0.0 ""; } >"$kn/Cargo.lock"
: >"$kn/calls"
(cd "$kn" && . "$SRC/packaging/fork/lib.sh" && cargo() { "$kn/bin/cargo" "$@"; } && keep_newer_locked fork.lock >/dev/null)
check "the merged lock is raised to the fork's newer versions on the same semver line" bash -c \
  "[ \"\$(sort '$kn/calls')\" = \"\$(printf 'cargo update --quiet -p foo@0.3.2 --precise 0.3.5\ncargo update --quiet -p tokio@1.53.1 --precise 1.54.1')\" ]"

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
check "the LLM's fix commenting a line out stops the sync" stopped gcomment "disables a test or hides a failure"
check "  naming the line" said gcomment "###CLI_ZIP="
# Before upstream 0.13.0 a catalog's hash went stale and the assets check's mechanical fix re-hashed it;
# from 0.13.0 catalogs carry no hash. Either way the edit must leave every asset checking out.
check "the LLM's catalog edit leaves the assets checking out, and the sync goes on" ok gcatalog
check "  with the LLM's string" grep -q "A string the LLM added" "$T/s-gcatalog/crates/ui-egui/src/i18n/ja.tsv"
check "  every hash right" assets_ok gcatalog
check "an upstream edit of the fork's own workflow: merged" ok ownwf
check "  keeping the fork's version" bash -c "grep -q '^name: CI$' '$T/s-ownwf/.github/workflows/ci.yml' && ! grep -q ubuntu-latest '$T/s-ownwf/.github/workflows/ci.yml'"
check "  and the record says so" grep -q "upstream's changes to ci.yml were not taken" "$T/s-ownwf/docs/fork/releases/v$next.md"

# 8. release-bump: tagged and unchanged, tagged and changed, untagged.
bump_case() {
  local d="$T/b-$1" rc=0
  git clone -q "$T/fork" "$d"
  (cd "$d" && eval "$2" && PATH="$T/ghnone:$PATH" BUMP_DRY_RUN=1 BUMP_SKIP_UPSTREAM_CHECK=1 bash packaging/fork/release-bump.sh) >"$d.out" 2>&1 || rc=$?
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
  "pr view")
    case " $* " in
      # GH_PR_STATE, or one state per call from the list GH_PR_STATES (the last one repeats).
      *" state "*)
        if [ -n "${GH_PR_STATES:-}" ]; then
          n=$(($(cat "$GH_STEP.state" 2>/dev/null || echo 0) + 1)); echo "$n" >"$GH_STEP.state"
          read -r -a states <<<"$GH_PR_STATES"
          echo "${states[n - 1]:-${states[-1]}}"
        else
          echo "${GH_PR_STATE:-OPEN}"
        fi ;;
      *) echo deadbeef ;;
    esac ;;
  "pr merge")
    echo "merge args: $*" >>"$GH_LOG"
    # GH_MERGE_FAILS: as when another run merged it first, or a push moved the head.
    [ -z "${GH_MERGE_FAILS:-}" ] || exit 1
    echo "merged $3" >>"$GH_LOG" ;;
  "run list") echo 4242 ;;
  "issue list") ;;
  "issue "*) echo "$*" >>"$GH_LOG" ;;
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

check "merge: only the commit whose checks passed is merged" bash -c "$(declare -f run_merge); T='$T' SRC='$SRC'; run_merge '$green' && grep -q 'match-head-commit deadbeef' '$T/ghlog'"
check "merge: a PR another run merged first counts as merged" bash -c "$(declare -f run_merge); T='$T' SRC='$SRC'; GH_MERGE_FAILS=1 GH_PR_STATE=MERGED run_merge '$green'"
check "merge: a merge refused on an open PR (a newer push) fails" bash -c "$(declare -f run_merge); T='$T' SRC='$SRC'; ! GH_MERGE_FAILS=1 GH_PR_STATE=OPEN MERGE_SETTLE_SECONDS=10 run_merge '$green'"
check "merge: another run's merge still under way is waited for" bash -c "$(declare -f run_merge); T='$T' SRC='$SRC'; rm -f '$T/step.state'; GH_MERGE_FAILS=1 GH_PR_STATES='OPEN OPEN MERGED' run_merge '$green'"

# auto-merge.sh as the workflow runs it. A run for a finished check never waits and never fails for
# one (it used to cancel the PR's own run, leaving every merged PR with a red check).
run_auto() {
  printf '%s\n' "${@:2}" >"$T/states"
  echo 0 >"$T/step"
  : >"$T/ghlog"
  # shellcheck disable=SC2030,SC2031 # each run sets its environment in its own subshell, on purpose
  (export PATH="$T/gh:$PATH" GH_LOG="$T/ghlog" GH_STEP="$T/step" GH_STATES="$T/states" AUTO_MERGE_EVENT="$1"
    # A wait would sleep 30 s a poll: 20 s is time for the decision and none for a wait.
    timeout 20 bash "$SRC/packaging/fork/auto-merge.sh" 7) >"$T/autolog" 2>&1
}
check "auto-merge: a PR merged already is left alone, without failing" bash -c "$(declare -f run_auto); T='$T' SRC='$SRC'; GH_PR_STATE=MERGED run_auto pull_request '$green' && ! grep -q 'merge args' '$T/ghlog'"
check "auto-merge: after a check finishes, a green PR is merged" bash -c "$(declare -f run_auto); T='$T' SRC='$SRC'; run_auto workflow_run '$self_pending' && grep -q 'merged 7' '$T/ghlog'"
check "  a pending PR is left to its own run, without waiting or failing" bash -c "$(declare -f run_auto); T='$T' SRC='$SRC'; run_auto workflow_run '$pending' && ! grep -q 'merge args' '$T/ghlog'"
check "  a failing PR too: its own run reports the failure" bash -c "$(declare -f run_auto); T='$T' SRC='$SRC'; run_auto workflow_run '$red' && ! grep -q 'merge args' '$T/ghlog'"
check "auto-merge: the PR's own run fails on a failing check" bash -c "$(declare -f run_auto); T='$T' SRC='$SRC'; ! run_auto pull_request '$red' && ! grep -q 'merge args' '$T/ghlog'"

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
check "  and an issue tells the owner" bash -c "$(declare -f run_fixing fixes); T='$T' SRC='$SRC'
  ! SYNC_NO_LLM=1 run_fixing '$red'; grep -q '^issue create --repo x/y --title sync/x: PR #7 has failing checks' '$T/ghlog'"
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
