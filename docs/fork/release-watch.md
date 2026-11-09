# Release watch: set it up on a machine

Status: in use on one runner host. Revised 2026-11-04 (first version).

A timer that checks storytold/pdfcraft every hour and, when it has tagged a release (`vX.Y.Z`) the
fork lacks, starts the fork's Sync upstream workflow. Install it on one machine only: two watchers
are harmless (the second sees the run already going) but pointless.

## Needs

- Linux with systemd, `git` and `gh`.
- `gh` logged in as the fork's owner: `gh auth status`.
- [Mergiraf](https://codeberg.org/mergiraf/mergiraf) on the runner host, for the sync's merge
  conflicts: `cargo install --locked mergiraf`, then `mergiraf --version`.
- Linger on, so it runs at boot without a login (needs sudo once):

```bash
sudo loginctl enable-linger "$USER"
```

## Install

```bash
git clone https://github.com/krichelj/pdfcraft.git /tmp/pdfcraft-rw && bash /tmp/pdfcraft-rw/packaging/fork/release-watch.sh install
```

It keeps its own clone in `~/.cache/pdfcraft-release-watch`, updated to `main` on every check, so
later script changes arrive by themselves. Run `install` again only when the timer itself changes.

## Check it

```bash
systemctl --user list-timers pdfcraft-release-watch.timer
```

```bash
journalctl --user -u pdfcraft-release-watch.service -n 20
```

## Debug the sync on upstream's commits

Production syncs only release tags. To test the sync on every new commit of upstream's main:

```bash
gh variable set SYNC_FOLLOW_MAIN --repo krichelj/pdfcraft --body true
```

Back to releases only:

```bash
gh variable delete SYNC_FOLLOW_MAIN --repo krichelj/pdfcraft
```

## Remove it

```bash
systemctl --user disable --now pdfcraft-release-watch.timer
```
