# Structure

How PdfCraft is divided. The layout belongs to upstream (storytold/pdfcraft); this map is the fork's,
in a file upstream does not have, so a daily sync never conflicts with it.

## Structure

| Path | What it holds |
|---|---|
| `apps/` | The programs: `pdfcraft` (the desktop app), `pdfcraft-cli` (the headless runner and opt-in MCP server), `pdfcraft-web` (the WASM build). |
| `assets/` | Openly licensed icons, the app icon, demo inputs and font metadata, each listed in ATTRIBUTION.toml (AGENTS.md §1). |
| `contributors/` | The contributor list the app's About page shows. |
| `crates/` | The libraries, one per layer component (`pdfcraft-<dir>`), from the standalone PDF object layer up to the UI; layers are enforced by `cargo xtask layers`. |
| `docs/` | Design and user documents; `docs/fork/` holds the fork's own records (its release records). |
| `packaging/` | Building and installing the app per platform; `packaging/fork/` is the fork's sync and release automation. |
| `parity/` | The machine-readable Acrobat-parity checklist that `cargo xtask parity` reports from. |
| `vendor/` | Patched third-party crates the workspace builds from, outside the workspace (see vendor/README.md). |
| `xtask/` | Project automation, run as `cargo xtask <command>`: the quality gates, corpus checks, versioning and release records. |
