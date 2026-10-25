# Rules for merging storytold/pdfcraft into the krichelj/pdfcraft fork

The on-call LLM reads this file when packaging/fork/sync-upstream.sh hands it a merge: the
conflicts git could not resolve, then the checks that fail on the merged tree. OURS is the fork,
THEIRS is upstream (storytold/pdfcraft).

- Keep both sides. The fork's features (the macOS menu bar, Check for updates installing the
  fork's release, the fork button on the welcome screen, the fork's release automation) must
  survive, and upstream's changes must all be taken. When the two cannot both stand as written,
  take upstream's structure and re-apply the fork's change on top of it.
- The fork builds and releases for macOS on Apple silicon only. The fork's own workflows
  (ci.yml, own-runners-guard.yml, packaging-lint.yml, release.yml, sync-upstream.yml,
  fork-automation.yml, auto-merge.yml) are the fork's: keep OURS for them.
- The fork's version is `<upstream version>-p<N>` in the root Cargo.toml; the sync sets it.
  Never edit a version by hand.
- Never delete, skip or weaken a test or an assertion, never add `#[ignore]`, never silence a lint
  (`#[allow(...)]`, `-A`), and never change what CI runs. Fix the code under test.
- When fixing, never edit these: `.github/`, `packaging/fork/`, `xtask/src/workflows.rs`,
  `deny.toml`, `.repo-structure.toml`, `CLAUDE.md`, `AGENTS.md`. The sync refuses a fix that
  touches them, adds `#[ignore]` or `#[cfg(any())]`, or appends `|| true` to a command.
- Never panic on input: no new `unwrap`, `expect`, `panic!` or indexing with input-derived
  positions in non-test code (CLAUDE.md, "Never panic"). No `unsafe`.
- Respect the layering in xtask/src/layers.rs: a crate depends only on lower layers.
- Translation catalogs (crates/ui-egui/src/i18n/*.tsv) keep both sides' lines; ATTRIBUTION.toml's
  hashes are recomputed by the sync (the `assets` check's mechanical fix), never by hand.
- No machine names, host names or server paths in any file: workflows ask for generic labels.
- Make the smallest change that resolves the conflict or fixes the failure. Do not reformat,
  rename or reorganise code you were not asked about.
