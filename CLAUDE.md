@AGENTS.md

# Fork additions (krichelj/pdfcraft)

The rules below are this fork's own and come on top of `AGENTS.md`, which is upstream's.

## Fork rule (krichelj/pdfcraft): macOS on Apple Silicon only
**IRONCLAD (owner, 2026-10-08): this fork builds, packages, releases and debugs for macOS on Apple
Silicon (M1, M4: `aarch64-apple-darwin`) and nothing else.** The owner uses no other platform:
"i dont care about Web and Windows x64 or any other thing this is NOT MAC OS M series". Windows,
Linux, Flatpak, FreeBSD and Web packaging stay exactly as upstream (storytold/pdfcraft) has them:
do not edit, fix, build or run them here, and take upstream's versions on every merge. The fork's
`release.yml` runs only `version`, `macos` and `release`, on the own Linux runners
(own-ci-runners), cross-building with `packaging/macos/cross-package.sh`. Shared code and its tests
(`ci.yml`) still run, because the Mac app is built from them; CI/CD runs no other non-Mac check (no
wasm32/web, Windows, Linux-desktop, Flatpak or Nix checks; upstream workflows for other platforms,
such as `freebsd.yml`, `windows-arm64.yml` and `nix.yml`, are deleted on merge), and `packaging-lint.yml` covers only the macOS
packaging scripts and property lists. The `Draft GitHub Release` job runs in the `release`
environment, which deploys only from the `release` branch.

## Fork rule: Always sync upstream on every commit
**IRONCLAD (owner, 2026-10-10): Always update from upstream (`git fetch upstream && git merge upstream/main`) every time you commit in this repo.** Because this repository is an active fork, it must stay continually aligned with upstream (`storytold/pdfcraft:main`). Never commit or push changes against a stale upstream base. On every commit:
1. Fetch latest upstream: `git fetch upstream`.
2. Integrate upstream changes: `git merge upstream/main`.
3. Verify quality gates: `cargo check --all-targets && cargo xtask assets`.
4. Push to `origin main`.

## Fork rule: upstream syncs and releases run themselves
- **Sync upstream** (`.github/workflows/sync-upstream.yml`, on each upstream release, and on demand) runs
  `packaging/fork/sync-upstream.sh`. A new storytold/pdfcraft release tag (`vX.Y.Z`) is merged within
  an hour: `packaging/fork/release-watch.sh`, a `systemd --user` timer on the runner host (hourly; setup: `docs/fork/release-watch.md`), dispatches the workflow when `upstream-release.sh` names a tag
  the fork lacks, and that run merges the tag's commit (`SYNC_UPSTREAM_TAG`), not main. GitHub's own
  schedules start hours late, so the workflow's 15-minute cron is only the fallback. Upstream's main
  is merged only when the workflow is run by hand with no new release (owner, 2026-11-08: upstream
  releases from a branch of its own, so its main conflicts between releases). It merges on a `sync/upstream-<sha>`
  branch, keeps only the fork's workflows, bumps the version (`<base>-p<N+1>`, or `<new base>-p1`)
  and writes the release record listing the upstream commits. It then opens the PR, waits for its
  checks (CI's `test (shared code)` must pass) and merges it, which deletes the branch.
- **The on-call LLM** (the owner's private `oncall-llm`, configured on the runner hosts, never in
  this repo) takes what a merge breaks: conflicts the sync's rules don't cover (`resolve`, with
  `packaging/fork/oncall-instructions.md`), then CI's test-job checks on the merged tree (`fix`,
  one `Sync fix: ...` commit per round, before the release commit, listed in the record), then a
  failing PR check with a digest of its log (at most twice). Its fixes never reach a push when they
  touch `.github/`, `packaging/fork/`, `xtask/src/workflows.rs`, `deny.toml`, `.repo-structure.toml`,
  `CLAUDE.md` or `AGENTS.md`, or add `#[ignore]`, `#[cfg(any())]` or `|| true` (`llm_guard` in
  lib.sh); an edited catalog is re-hashed by the `assets` check's mechanical fix. The job asks for
  `[self-hosted, linux, primary]` (listed in `.github/actionlint.yaml`):
  the primary runner host at once, the other only when it hasn't answered. Without the LLM
  (`SYNC_NO_LLM=1`, or `oncall-llm health` failing) or when it can't, the sync stops for a person.
  `packaging/fork/test-sync.sh` covers each path with a stand-in.
- It uses the owner's gh login on the runner host: the workflow token can't push workflow files,
  and its pushes start no workflows.
- **Release** runs on every push to main. If the version is already tagged but the fork changed since,
  its `bump` job (`packaging/fork/release-bump.sh`) opens the next patch release's PR
  (`<base>-p<N+1>`, record = the PRs merged since) and merges it once green; that merge releases.
  A version that is already tagged is not rebuilt. Otherwise it
  builds, publishes "PdfCraft v<version>" with the record as notes, then downloads the published DMG
  back and checks it: the app, the Applications link, arm64, the signature and the version.

## Fork rule: Orthogonal fork versioning (`<upstream>-p<N>`)
**Gate:** `cargo xtask version check-upstream` refuses a fork version that is not `<X.Y.Z>-p<N>`
with X.Y.Z at least storytold/pdfcraft's latest release. The sync proves it after merging, and
Release refuses to publish without it.
**IRONCLAD (owner, 2026-10-10): All releases and builds in this fork must use an orthogonal downstream patchlevel tag extending upstream's base version.**
- **Format**: `<base>-p<N>` (e.g. `0.6.0-p1`, `0.6.0-p2`). Follows standard open-source patchlevel conventions and SemVer 2.0.0 Rule 9 without personal developer handles.
- **Single Source of Truth**: Managed exclusively via `cargo xtask version`:
  - `cargo xtask version`: Displays current workspace version (e.g. `0.6.0-p1`).
  - `cargo xtask version base`: Displays upstream base version (e.g. `0.6.0`).
  - `cargo xtask version fork [tag]`: Initializes fork version to `<base>-<tag>1` (default tag `p` -> `0.6.0-p1`).
  - `cargo xtask version bump-fork [tag]`: Increments fork patchlevel (`0.6.0-p1` -> `0.6.0-p2`).
  - `cargo xtask version set <version>`: Explicit version assignment.
- **Cargo Invariant**: Root `[workspace.package] version` and all internal `pdfcraft-*` path dependencies in `[workspace.dependencies]` are synchronized simultaneously to ensure `cargo update --workspace` resolves cleanly.

## Fork rule: every release has a record
**IRONCLAD (owner, 2026-10-16): no fork release ships without `docs/fork/releases/v<version>.md`.**
The record's "## What changed" is written by hand (what the release means for people using it; a
`TODO` or an empty list is refused). Its "Based on" part is generated from git
and is the bookkeeping: the storytold/pdfcraft commit and version the release is based on, and the
previous fork release.
- In the PR that bumps the version: `cargo xtask fork-notes generate <version>`, write the summary,
  commit both. Regenerate after an upstream merge (the summary is kept).
- CI runs `cargo xtask fork-notes pending` (checks the record of a version not yet tagged); the
  Release workflow runs `cargo xtask fork-notes body`, which refuses before anything is built and
  becomes the release page. `xtask/src/workflows.rs` fails if either gate is removed.

## Machine privacy rule: Zero infrastructure leaks
**IRONCLAD (owner, 2026-10-10): Machine names, cluster hostnames, and server topology must NEVER appear in public workflows, code, comments, or PRs.**
- **Anonymous runner labels**: Workflows must specify generic self-hosted labels (`runs-on: [self-hosted, linux]`), never hostnames.
- **Runtime identity via GitHub Secrets**: Host machine names or cluster scratch paths needed at runtime must be accessed exclusively through repository secrets (`${{ secrets.RUNNER_HOST_NAME }}`, `${{ secrets.MACOS_SDKROOT }}`).

## Commands
- Single crate / single test: `cargo test -p pdfcraft-cos -j 2`, `cargo test -p pdfcraft-cos -j 2 <name_filter>`, `cargo test -p pdfcraft-ui-egui -j 2 --test control`.
- Individual gates: `cargo xtask layers`, `cargo xtask wasm`, `cargo xtask assets [--write]`, `cargo xtask deny` (needs cargo-deny), `cargo check -p pdfcraft-cli --no-default-features` (build without the MCP server).
- Corpus: `cargo xtask corpus` fetches pdf.js test files into `corpus/`; `cargo xtask check` sweeps them against `xtask/baselines`; `cargo xtask text-oracle` compares text extraction with pdftotext.
- `cargo xtask` with no args lists every command (`xtask/src/main.rs`). `cargo xtask version set X.Y.Z` bumps the workspace version.

## Architecture
- Workspace: `crates/*` (libraries, all named `pdfcraft-<dir>`), `apps/pdfcraft` (desktop), `apps/pdfcraft-cli` (headless runner + MCP), `apps/pdfcraft-web` (Trunk/WASM, not a default member), `xtask`. `vendor/` is excluded from the workspace.
- Layers are enforced by `cargo xtask layers` from the table in `xtask/src/layers.rs`: L0-1 standalone (`geom`, `filters`, `crypt`, `arlington`, `cos`) → L2 object model (`model`, `content`, `fonts`) → L3 (`render`, `annot`, `forms`, `js`, `xfa`) → L4 features (`edit`, `organize`, `redact`, `sign`, `ocr`, `export`, ...) → L5 interaction → L6 `engine` → L7 frontends (`ui-egui`, `automation`). A crate may depend only on lower layers; `testkit` and `oracle` are test-only (`Class::Testkit`). Adding a crate means adding it to that table.
- `pdfcraft-engine` is the façade every frontend uses (sessions, edits, undo, save, tool catalogue). The UI and `pdfcraft-automation` both drive it, so engine features are testable headlessly via `pdfcraft-cli run`.
