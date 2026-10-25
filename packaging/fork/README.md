# Fork automation

The scripts that keep krichelj/pdfcraft current with storytold/pdfcraft and release it, run by the
fork's workflows on the own runner hosts (see CLAUDE.md, "Fork rule" sections).

| Script | What it does | Run by |
|---|---|---|
| `sync-upstream.sh` | Merges upstream's main on a `sync/upstream-<sha>` branch with the fork's merge rules, bumps the fork version, writes the release record, opens the PR and merges it once its checks pass. | `sync-upstream.yml` (daily, and on demand) |
| `release-bump.sh` | When the tagged version has fork changes after it, opens the next `<base>-p<N+1>` release PR and merges it once green. | `release.yml` |
| `auto-merge.sh` | Merges the owner's pull requests once every check passed. | `auto-merge.yml` |
| `lib.sh` | Shared by the scripts above: pushing as the owner, waiting for checks, merging, the release record's summary. | sourced |
| `test-sync.sh` | The scenario suite for all of the above, with stand-ins for GitHub. | `fork-automation.yml` |
