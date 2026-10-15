# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

PdfCraft is a clean-room, open-source, Rust-native PDF application targeting Adobe Acrobat Pro parity. It runs natively on macOS, Windows, Linux and FreeBSD, and on the web via WASM. It is the sibling of `../photocraft` (a Photoshop-class editor) and follows the same conventions.

## Fork rule (krichelj/pdfcraft): macOS on Apple Silicon only
**IRONCLAD (owner, 2026-10-08): this fork builds, packages, releases and debugs for macOS on Apple
Silicon (M1, M4: `aarch64-apple-darwin`) and nothing else.** The owner uses no other platform:
"i dont care about Web and Windows x64 or any other thing this is NOT MAC OS M series". Windows,
Linux, Flatpak, FreeBSD and Web packaging stay exactly as upstream (storytold/pdfcraft) has them:
do not edit, fix, build or run them here, and take upstream's versions on every merge. The fork's
`release.yml` runs only `version`, `macos` and `release`, on the own Linux runners
(own-ci-runners), cross-building with `packaging/macos/cross-package.sh`. Shared code and its tests
(`ci.yml`) still run, because the Mac app is built from them.

## Fork rule: Always sync upstream on every commit
**IRONCLAD (owner, 2026-10-10): Always update from upstream (`git fetch upstream && git merge upstream/main`) every time you commit in this repo.** Because this repository is an active fork, it must stay continually aligned with upstream (`storytold/pdfcraft:main`). Never commit or push changes against a stale upstream base. On every commit:
1. Fetch latest upstream: `git fetch upstream`.
2. Integrate upstream changes: `git merge upstream/main`.
3. Verify quality gates: `cargo check --all-targets && cargo xtask assets`.
4. Push to `origin main`.

## Fork rule: Orthogonal fork versioning (`<upstream>-p<N>`)
**IRONCLAD (owner, 2026-10-10): All releases and builds in this fork must use an orthogonal downstream patchlevel tag extending upstream's base version.**
- **Format**: `<base>-p<N>` (e.g. `0.6.0-p1`, `0.6.0-p2`). Follows standard open-source patchlevel conventions and SemVer 2.0.0 Rule 9 without personal developer handles.
- **Single Source of Truth**: Managed exclusively via `cargo xtask version`:
  - `cargo xtask version`: Displays current workspace version (e.g. `0.6.0-p1`).
  - `cargo xtask version base`: Displays upstream base version (e.g. `0.6.0`).
  - `cargo xtask version fork [tag]`: Initializes fork version to `<base>-<tag>1` (default tag `p` -> `0.6.0-p1`).
  - `cargo xtask version bump-fork [tag]`: Increments fork patchlevel (`0.6.0-p1` -> `0.6.0-p2`).
  - `cargo xtask version set <version>`: Explicit version assignment.
- **Cargo Invariant**: Root `[workspace.package] version` and all internal `pdfcraft-*` path dependencies in `[workspace.dependencies]` are synchronized simultaneously to ensure `cargo update --workspace` resolves cleanly.

## Machine privacy rule: Zero infrastructure leaks
**IRONCLAD (owner, 2026-10-10): Machine names, cluster hostnames, and server topology must NEVER appear in public workflows, code, comments, or PRs.**
- **Anonymous runner labels**: Workflows must specify generic self-hosted labels (`runs-on: [self-hosted, linux]`), never hostnames.
- **Runtime identity via GitHub Secrets**: Host machine names or cluster scratch paths needed at runtime must be accessed exclusively through repository secrets (`${{ secrets.RUNNER_HOST_NAME }}`, `${{ secrets.MACOS_SDKROOT }}`).

## Start every session here
1. Read `plan/STATUS.md`: the current milestone, the next unchecked task and any blockers. `ROADMAP.md` is the one-page summary (stage, numbers, progress log) and `docs/roadmap.md` the milestone table; at the end of every session update the milestone row and add a line to the ROADMAP.md progress log.
   - Read `docs/gaps.md` (the ranked gaps: Acrobat interop, the open-issue backlog, the renderer and a fidelity harness, editing existing content, Pro workflows, 1.0 polish) and `docs/target-app-parity.md` (the numbers and how they are measured) before choosing work. Prefer the gaps over new P2/P3 features, and keep both documents true when things change (`docs/` progress docs follow craftrules `standards/progress-docs.md`: status line, revision history).
   - "Shipped" in `parity/` means "exists and tested", not "as good as Acrobat". Don't mark a feature shipped on a generic test, and say in its notes what is still missing.
2. Read that task in `plan/execution-plan.md` §3, the relevant section of `plan/architecture.md`, and the README of the crate you're touching.
3. Follow the **autonomous operation protocol** in `plan/execution-plan.md` §7 (orient → plan → implement + test → verify → record → commit). Don't stop to ask unless §7 lists the decision as the user's.

`plan/` is gitignored (local-only, like PhotoCraft) and may be absent in a fresh checkout; if so, fall back to `ROADMAP.md` and `parity/`, and say so. The machine-readable parity checklist lives in `parity/` (committed).

## Non-negotiables
- **Assets: read `AGENTS.md` §1 before adding or showing any icon, image, font or document.** No assets from Adobe products, ever. Only openly licensed or contributor-original assets are allowed, each with an entry in `ATTRIBUTION.toml`. `cargo xtask assets` enforces this. `AGENTS.md` overrides this file.
- **Fonts live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts).** Never commit font files here (`AGENTS.md` §1.4; team members: [craftrules `standards/fonts.md`](https://github.com/storytold/craftrules/blob/main/standards/fonts.md), internal). It is the optional build input `CRAFT_FONTS_DIR`: `git clone https://github.com/storytold/craft-fonts ../craft-fonts && CRAFT_FONTS_DIR=../craft-fonts cargo test --workspace` embeds the Japanese fonts (UI fallback, Japanese text in edited PDFs) and runs their tests, which otherwise skip. Code using `pdfcraft_fonts::CRAFT_FONTS` must work when it is empty.
- **Clean-room.**
  - Never read, disassemble or copy anything inside the Acrobat bundle (names and listings only). **Never open `Contents/Resources/JavaScripts/`.**
  - Behaviour comes from public docs, specs (ISO 32000-2, the Arlington model) and black-box observation (`plan/acrobat/`).
  - Never copy GPL/AGPL code. MuPDF, Ghostscript, Poppler, veraPDF and DSS run only as external oracle processes.
  - See `plan/README.md` §Clean-room and `plan/adr/0001`.
- **Privacy.** When observing Acrobat, use synthetic fixtures only. Never capture the Home view, recent files or account info. Capture by window id (`plan/acrobat/tools/`). Never commit Acrobat outputs, corpus files or personal data.
- **Layering.** Nothing below L7 depends on egui/winit/eframe/rfd (`plan/architecture.md` §3). `cos`/`filters`/`crypt`/`arlington` stay standalone.
- **Fidelity.** The PDF object graph is the model. Preserve unknown data. Saves are incremental unless a full rewrite is required. Never silently drop or repair data without recording it.
- **Never panic.** Every PDF, script, MCP call, settings file and keystroke is untrusted input, and none of it may crash the app or lose the user's work. This outranks feature work. Return `Result` (the crate's error enum) and propagate with `?`, or fall back leniently and record it. In non-test code:
  - No `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!` unless it is provably infallible, with a comment saying why.
  - No indexing or slicing with input-derived positions (use `get`, and slice strings only at char boundaries).
  - Use checked or saturating arithmetic on input-derived numbers, and guard against division by zero and NaN/inf casts.
  - Cap allocations sized by input, and bound recursion with depth limits or seen-sets.
  - Handle lock poisoning.
  - No `unsafe` (`unsafe_code = "forbid"`).
  - Every crash fix gets a synthetic regression test. See `AGENTS.md` §4 (and, for team members, the internal `craftrules/standards/never-crash.md`).
- **Rust only** in the product and build (`xtask`). No handwritten JS/TS.
- **Quality gates** before every commit: `cargo xtask ci` (fmt --check, clippy `-D warnings`, test, layers, wasm, assets, deny, parity; stops at the first failure). CI runs clippy with `--all-targets`, so do too.
- **Commits:** one task id per commit (e.g. `M1.4: xref stream reader`). Commit only green states. End messages with the attribution line required by the environment.

## Commands
- Single crate / single test: `cargo test -p pdfcraft-cos`, `cargo test -p pdfcraft-cos <name_filter>`, `cargo test -p pdfcraft-ui-egui --test control`.
- Individual gates: `cargo xtask layers`, `cargo xtask wasm`, `cargo xtask assets [--write]`, `cargo xtask deny` (needs cargo-deny), `cargo check -p pdfcraft-cli --no-default-features` (build without the MCP server).
- Corpus: `cargo xtask corpus` fetches pdf.js test files into `corpus/`; `cargo xtask check` sweeps them against `xtask/baselines`; `cargo xtask text-oracle` compares text extraction with pdftotext.
- `cargo xtask` with no args lists every command (`xtask/src/main.rs`). `cargo xtask version set X.Y.Z` bumps the workspace version.

## Architecture
- Workspace: `crates/*` (libraries, all named `pdfcraft-<dir>`), `apps/pdfcraft` (desktop), `apps/pdfcraft-cli` (headless runner + MCP), `apps/pdfcraft-web` (Trunk/WASM, not a default member), `xtask`. `vendor/` is excluded from the workspace.
- Layers are enforced by `cargo xtask layers` from the table in `xtask/src/layers.rs`: L0-1 standalone (`geom`, `filters`, `crypt`, `arlington`, `cos`) → L2 object model (`model`, `content`, `fonts`) → L3 (`render`, `annot`, `forms`, `js`, `xfa`) → L4 features (`edit`, `organize`, `redact`, `sign`, `ocr`, `export`, ...) → L5 interaction → L6 `engine` → L7 frontends (`ui-egui`, `automation`). A crate may depend only on lower layers; adding a crate means adding it to that table.
- `pdfcraft-engine` is the façade every frontend uses (sessions, edits, undo, save, tool catalogue). The UI and `pdfcraft-automation` both drive it, so engine features are testable headlessly via `pdfcraft-cli run`.

## Running and looking at the app
- `cargo run -p pdfcraft -- <file.pdf>` opens the desktop app.
- `cargo run -p pdfcraft-cli -- run --script steps.json --root DIR` drives the engine headlessly through the automation tools (`pdfcraft-cli tools` lists them). Use it, together with `page_render`, to check engine changes. `pdfcraft-cli mcp` is the opt-in MCP server (AGENTS.md §3).
- `cargo xtask fuzz --time 300` mutation-fuzzes open/render/edit/save in child processes. Findings land in `fuzz-out/findings/` (git-ignored; never commit corpus-derived files). Turn every real finding into a small synthetic regression test before fixing it.
- `cargo xtask parity [--partial]` reports Acrobat-parity progress from `parity/acrobat-features.toml`. Update the entry when a feature ships.
- `cargo xtask demo-pdf` builds `dist/demo/pdfcraft-showcase.pdf` (needs Chrome) for visual checks.
- For UI work, **look at the result**. Either launch `pdfcraft --control FILE doc.pdf` and use `pdfcraft-cli ui --control FILE screenshot --out x.png` (plus `inspect`, `click`, `key`, `type`, `command`, `set`), or take a headless shot with `cargo run -p pdfcraft-ui-egui --example shot`. Compare against `plan/acrobat/02-ui-ux.md`. Control-channel tests use kittest (`crates/ui-egui/tests/control.rs`).
- Parallel agents: use a separate `CARGO_TARGET_DIR` per agent and separate git worktrees.

## Current bootstrap debt (tracked in STATUS.md)
- `pdfcraft-render` renders through the `hayro` crate directly and inspects documents through `lopdf`. Both get replaced by `cos` / `model` / the DisplayList device (M1–M2, ADR-0004).
