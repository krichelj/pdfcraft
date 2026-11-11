# Fork automation

The scripts that keep krichelj/pdfcraft current with storytold/pdfcraft and release it, run by the
fork's workflows on the own runner hosts (see CLAUDE.md, "Fork rule" sections).

| Script | What it does | Run by |
|---|---|---|
| `sync-upstream.sh` | Merges upstream's main on a `sync/upstream-<sha>` branch with the fork's merge rules and [Mergiraf](https://codeberg.org/mergiraf/mergiraf) for the conflicts they don't cover, bumps the fork version, writes the release record, opens the PR and merges it once its checks pass. Run without the on-call LLM (`SYNC_NO_LLM=1`, the owner's choice): a conflict neither resolves, or a failing check, stops it and opens an issue for the owner. While that sync PR is open, later runs put newer upstream commits on it instead of opening another PR and issue, and a run stopped on the same thing again ends quietly. | `sync-upstream.yml` (each new upstream release tag, every new upstream commit while `SYNC_FOLLOW_MAIN=true`, and on demand) |
| `release-bump.sh` | When the tagged version has fork changes after it, opens the next `<base>-p<N+1>` release PR and merges it once green. | `release.yml` |
| `auto-merge.sh` | Merges the owner's pull requests once every check passed. | `auto-merge.yml` |
| `lib.sh` | Shared by the scripts above: pushing as the owner, waiting for checks, merging, the release record's summary, and the on-call LLM's steps (its fix loop, the guard on what its fixes may change, the CI-log digest it reads). | sourced |
| `private-tmp.sh` | Runs a command with its own empty temp directory under /tmp, removed afterwards whatever happens: upstream's tests leave ~300 folders per run in the temp directory, which once filled the runner host's /tmp. | CI's test step, the on-call LLM's test check |
| `oncall-instructions.md` | The fork's merge rules, shown to the on-call LLM on every resolve and fix call. | read by `oncall-llm` |
| `test-sync.sh` | The scenario suite for all of the above, with stand-ins for GitHub. | `fork-automation.yml` |
