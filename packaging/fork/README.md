# Fork automation

The scripts that keep krichelj/pdfcraft current with storytold/pdfcraft and release it, run by the
fork's workflows on the own runner hosts (see CLAUDE.md, "Fork rule" sections).

| Script | What it does | Run by |
|---|---|---|
| `sync-upstream.sh` | Merges upstream's main on a `sync/upstream-<sha>` branch with the fork's merge rules, bumps the fork version, writes the release record, opens the PR and merges it once its checks pass. | `sync-upstream.yml` (daily, and on demand) |
| `release-bump.sh` | When the tagged version has fork changes after it, opens the next `<base>-p<N+1>` release PR and merges it once green. | `release.yml` |
| `auto-merge.sh` | Merges the owner's pull requests once every check passed. | `auto-merge.yml` |
| `lib.sh` | Shared by the scripts above: pushing as the owner, waiting for checks, merging, the release record's summary, and the on-call LLM's steps (its fix loop, the guard on what its fixes may change, the CI-log digest it reads). | sourced |
| `oncall-instructions.md` | The fork's merge rules, shown to the on-call LLM on every resolve and fix call. | read by `oncall-llm` |
| `test-sync.sh` | The scenario suite for all of the above, with stand-ins for GitHub. | `fork-automation.yml` |
