//! The GitHub CLI (`gh`) with the user's own sign-in, and running a program with a deadline.
//! Shared by PdfCraft (Merge and update starts the Sync upstream workflow) and the updater (which
//! follows the workflow, its pull request, checks and release).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The fork whose Sync upstream workflow merges the original PdfCraft in
/// (`.github/workflows/sync-upstream.yml`).
pub const FORK_REPO: &str = "krichelj/pdfcraft";
pub const SYNC_WORKFLOW: &str = "sync-upstream.yml";
/// How long the GitHub CLI may take to find itself or start the workflow.
const GH_TIMEOUT: Duration = Duration::from_secs(60);

/// Merge and update: start the fork's Sync upstream workflow with the GitHub CLI and the user's
/// own sign-in (`gh auth login`). Runs on the dialog's background thread.
pub fn trigger_sync() -> Result<(), String> {
    GitHubCli::locate()?.run_sync(GH_TIMEOUT)
}

/// The GitHub CLI (`gh`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubCli {
    program: PathBuf,
}

impl GitHubCli {
    /// Where `gh` usually is on a Mac: Homebrew on Apple silicon and on Intel, and a per-user
    /// install. An app opened from the Dock or Finder doesn't get the shell's PATH.
    pub fn usual_places(home: Option<&Path>) -> Vec<PathBuf> {
        let mut places = vec![PathBuf::from("/opt/homebrew/bin/gh"), PathBuf::from("/usr/local/bin/gh")];
        places.extend(home.map(|h| h.join(".local").join("bin").join("gh")));
        places
    }

    /// The first executable in `places`, else whatever the user's login `shell` finds on its PATH.
    pub fn find(places: &[PathBuf], shell: &Path) -> Result<GitHubCli, String> {
        if let Some(program) = places.iter().find(|p| executable(p)) {
            return Ok(GitHubCli { program: program.clone() });
        }
        let mut lookup = std::process::Command::new(shell);
        lookup.args(["-lc", "command -v gh"]);
        if let Ok(out) = run_for(lookup, GH_TIMEOUT)
            && out.status.success()
        {
            // A login profile may print first (a banner, a fortune): the answer is the last line.
            let text = String::from_utf8_lossy(&out.stdout);
            let found = text.lines().rev().map(str::trim).find(|l| !l.is_empty()).map(PathBuf::from);
            if let Some(found) = found
                && found.is_absolute()
                && executable(&found)
            {
                return Ok(GitHubCli { program: found });
            }
        }
        Err("the GitHub CLI (gh) isn't installed; install it with `brew install gh` and sign in with `gh auth login`".into())
    }

    /// `gh workflow run sync-upstream.yml -R krichelj/pdfcraft`.
    fn sync_command(&self) -> std::process::Command {
        let mut c = std::process::Command::new(&self.program);
        c.args(["workflow", "run", SYNC_WORKFLOW, "-R", FORK_REPO]).stdin(std::process::Stdio::null());
        c
    }

    /// Where `gh` is on this Mac (see [`GitHubCli::usual_places`]).
    pub fn locate() -> Result<GitHubCli, String> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        GitHubCli::find(&GitHubCli::usual_places(home.as_deref()), Path::new("/bin/zsh"))
    }

    /// `gh api <path>`: GitHub's REST API with the user's sign-in, parsed as JSON. `path` is
    /// built by this crate (never from a response), relative to `https://api.github.com/`.
    pub fn api(&self, path: &str, timeout: Duration) -> Result<serde_json::Value, String> {
        let mut c = std::process::Command::new(&self.program);
        c.args(["api", "-H", "Accept: application/vnd.github+json", path]).stdin(std::process::Stdio::null());
        let out = run_for(c, timeout)?;
        if !out.status.success() {
            let said: String = String::from_utf8_lossy(&out.stderr).trim().chars().take(300).collect();
            return Err(if said.is_empty() { format!("gh api {path} exited with {}", out.status) } else { format!("gh: {said}") });
        }
        serde_json::from_slice(&out.stdout).map_err(|e| format!("GitHub's answer to {path} isn't JSON ({e})"))
    }

    pub fn run_sync(&self, timeout: Duration) -> Result<(), String> {
        let out = run_for(self.sync_command(), timeout)?;
        if out.status.success() {
            log::info!("update: started {FORK_REPO}'s {SYNC_WORKFLOW} with {}", self.program.display());
            return Ok(());
        }
        let said = String::from_utf8_lossy(&out.stderr);
        let said = said.trim();
        let said: String = said.chars().take(300).collect();
        Err(if said.is_empty() { format!("gh exited with {}", out.status) } else { format!("gh: {said}") })
    }
}

/// The installers' lock, in the temporary folder (install.sh's default `PDFCRAFT_LOCK`).
pub const INSTALLER_LOCK: &str = "pdfcraft-installer.lock";

/// Whether process `pid` is running (`kill -0`, which signals nothing).
pub fn alive(pid: u32) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Whether `path` is a file this user may run.
fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// What a finished program wrote is still in its pipes when it exits; this is how long to wait for
/// it, past the exit, before giving up on anything it started that keeps the pipes open.
const OUTPUT_GRACE: Duration = Duration::from_secs(2);
/// The most of each stream kept: enough for any message gh prints.
const OUTPUT_LIMIT: usize = 64 * 1024;

/// One read from a child's stdout (`false`) or stderr (`true`); `None` when that stream closed.
type Chunk = (bool, Option<Vec<u8>>);

/// Run `command` to its end, or kill it after `timeout`. Its output is read while it runs, so a
/// chatty command can't fill the pipe and stall. The timeout bounds the whole call: a program
/// that exits while something it started still holds its output open (a login shell's background
/// job) is waited on for at most [`OUTPUT_GRACE`] more, never for that job.
pub fn run_for(mut command: std::process::Command, timeout: Duration) -> Result<std::process::Output, String> {
    let name = command.get_program().to_string_lossy().into_owned();
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| format!("a timeout of {timeout:?} is too long"))?;
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run {name} ({e})"))?;
    let (tx, rx) = std::sync::mpsc::channel::<Chunk>();
    read_into(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), false, tx.clone());
    read_into(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>), true, tx);
    let mut collected = Collected::default();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                // Waits at most 20 ms: the pause between looks, spent reading what arrives.
                collected.take(&rx, Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{name} didn't finish within {}s", timeout.as_secs()));
            }
            Err(e) => return Err(format!("couldn't wait for {name} ({e})")),
        }
    };
    let grace_end = Instant::now().checked_add(OUTPUT_GRACE).map_or(deadline, |g| deadline.min(g));
    while !collected.both_closed() {
        let Some(left) = grace_end.checked_duration_since(Instant::now()) else { break };
        if left.is_zero() || !collected.take(&rx, left) {
            break;
        }
    }
    Ok(std::process::Output { status, stdout: collected.out, stderr: collected.err })
}

/// Read `pipe` on its own thread, sending each chunk and then `None` when it closes. The thread
/// ends with the pipe; nobody waits for it.
fn read_into(pipe: Option<Box<dyn Read + Send>>, is_err: bool, tx: std::sync::mpsc::Sender<Chunk>) {
    let Some(mut pipe) = pipe else {
        let _ = tx.send((is_err, None));
        return;
    };
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send((is_err, Some(buf.get(..n).unwrap_or_default().to_vec()))).is_err() {
                        return; // the caller stopped listening
                    }
                }
            }
        }
        let _ = tx.send((is_err, None));
    });
}

/// A child's output as it arrives, each stream kept up to [`OUTPUT_LIMIT`].
#[derive(Default)]
struct Collected {
    out: Vec<u8>,
    err: Vec<u8>,
    closed: u8,
}

impl Collected {
    /// Take one chunk, waiting at most `wait`. False when none came (or no reader is left).
    fn take(&mut self, rx: &std::sync::mpsc::Receiver<Chunk>, wait: Duration) -> bool {
        match rx.recv_timeout(wait) {
            Ok((is_err, Some(bytes))) => {
                let into = if is_err { &mut self.err } else { &mut self.out };
                let room = OUTPUT_LIMIT.saturating_sub(into.len());
                into.extend(bytes.iter().take(room));
                true
            }
            Ok((_, None)) => {
                self.closed = self.closed.saturating_add(1);
                true
            }
            Err(_) => false,
        }
    }

    fn both_closed(&self) -> bool {
        self.closed >= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Held while a test writes fake programs and runs them. A child forked by another test thread
    /// while a script is still open for writing makes its exec fail with "Text file busy".
    #[cfg(unix)]
    static EXEC: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(unix)]
    fn exec_lock() -> std::sync::MutexGuard<'static, ()> {
        EXEC.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A temporary folder for fake programs.
    #[cfg(unix)]
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pdfcraft-gh-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An executable shell script at `path`.
    #[cfg(unix)]
    fn script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_github_cli_is_found_where_macs_keep_it_then_on_the_login_path() {
        let _exec = exec_lock();
        let dir = scratch("find");
        let missing = dir.join("missing-gh");
        let not_executable = dir.join("plain-gh");
        std::fs::write(&not_executable, "").unwrap();
        let gh = dir.join("gh");
        script(&gh, "exit 0");
        let no_shell = dir.join("no-shell");
        let found = GitHubCli::find(&[missing.clone(), not_executable.clone(), gh.clone()], &no_shell).unwrap();
        assert_eq!(found.program, gh, "the first executable place");
        // Nowhere usual: the login shell's PATH.
        let shell = dir.join("zsh");
        script(&shell, &format!("[ \"$1\" = -lc ] && [ \"$2\" = 'command -v gh' ] && echo '{}'", gh.display()));
        assert_eq!(GitHubCli::find(std::slice::from_ref(&missing), &shell).unwrap().program, gh);
        // A login profile that prints a banner first: the answer is the last line.
        let chatty_shell = dir.join("chatty-zsh");
        script(&chatty_shell, &format!("echo 'Welcome back'; echo; echo '{}'", gh.display()));
        assert_eq!(GitHubCli::find(std::slice::from_ref(&missing), &chatty_shell).unwrap().program, gh);
        // A shell that finds nothing, or names something that isn't a program.
        let empty_shell = dir.join("empty-zsh");
        script(&empty_shell, "exit 1");
        let liar_shell = dir.join("liar-zsh");
        script(&liar_shell, &format!("echo '{}'", not_executable.display()));
        for shell in [&empty_shell, &liar_shell, &no_shell] {
            let err = GitHubCli::find(std::slice::from_ref(&missing), shell).unwrap_err();
            assert!(err.contains("isn't installed") && err.contains("gh auth login"), "{err}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_sync_workflow_is_started_on_the_fork_and_failures_say_why() {
        let _exec = exec_lock();
        let dir = scratch("run");
        let args = dir.join("args");
        let gh = dir.join("gh");
        script(&gh, &format!("printf '%s\\n' \"$@\" > '{}'", args.display()));
        let cli = GitHubCli { program: gh.clone() };
        cli.run_sync(Duration::from_secs(20)).unwrap();
        assert_eq!(std::fs::read_to_string(&args).unwrap(), "workflow\nrun\nsync-upstream.yml\n-R\nkrichelj/pdfcraft\n");

        script(&gh, "echo 'HTTP 403: Must have admin rights to Repository.' >&2; exit 1");
        assert_eq!(cli.run_sync(Duration::from_secs(20)), Err("gh: HTTP 403: Must have admin rights to Repository.".into()));
        script(&gh, "exit 4");
        let err = cli.run_sync(Duration::from_secs(20)).unwrap_err();
        assert!(err.starts_with("gh exited with"), "{err}");
        // A gh that hangs (no network, a login prompt) is stopped, not waited on forever.
        script(&gh, "sleep 30");
        let started = Instant::now();
        let err = cli.run_sync(Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("didn't finish"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(10), "killed at the deadline");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A login shell's background job keeps the output pipe open after the shell exits: the
    /// answer still comes back within the timeout, not when the job ends.
    #[cfg(unix)]
    #[test]
    fn a_job_left_holding_the_output_does_not_hold_up_the_answer() {
        let _exec = exec_lock();
        let mut c = std::process::Command::new("/bin/sh");
        c.args(["-c", "sleep 8 & echo /opt/homebrew/bin/gh"]);
        let started = Instant::now();
        let out = run_for(c, Duration::from_secs(30)).unwrap();
        assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "/opt/homebrew/bin/gh");
        // And the timeout still bounds a job that runs past it while holding the pipe.
        let mut c = std::process::Command::new("/bin/sh");
        c.args(["-c", "sleep 8 & sleep 8"]);
        let started = Instant::now();
        assert!(run_for(c, Duration::from_millis(300)).unwrap_err().contains("didn't finish"));
        assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
    }
}
