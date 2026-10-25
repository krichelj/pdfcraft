//! Asks GitHub for the latest PdfCraft releases, installs the fork's, and starts the fork's merge
//! of a newer original PdfCraft (Help ▸ Check for updates, issue #28).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pdfcraft_ui_egui::updates::{Install, Latest, RELEASES_PAGE, Release, UPSTREAM_RELEASES_PAGE};

// The fork's releases (krichelj/pdfcraft), not upstream's: see `RELEASES_PAGE`. The page, not
// the REST API: GitHub allows 60 anonymous API requests an hour per address, which everyone on a
// campus or office network shares and runs out of; the page's redirect to the latest release's tag
// has no such quota. packaging/macos/install.sh finds the release the same way.
const LATEST: &str = "https://github.com/krichelj/pdfcraft/releases/latest";
/// The original PdfCraft's latest release: the fork follows it, so a newer one there is merged in.
const UPSTREAM_LATEST: &str = "https://github.com/storytold/pdfcraft/releases/latest";

/// The latest release of the fork and of the original repository (Help ▸ Check for updates).
/// One repository not answering still gives the other's; both failing gives the fork's error.
pub fn latest_release() -> Result<Latest, String> {
    match (latest_from(LATEST, RELEASES_PAGE), latest_from(UPSTREAM_LATEST, UPSTREAM_RELEASES_PAGE)) {
        (Err(e), Err(_)) => Err(e),
        (fork, upstream) => Ok(Latest { fork: fork.ok(), upstream: upstream.ok() }),
    }
}

/// The release `latest` (a `releases/latest` page) redirects to. The answer is untrusted: only a
/// tag page under `page` counts, and anything else is "no release found".
fn latest_from(latest: &str, page: &str) -> Result<Release, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(os_roots()?).build())
        // The redirect is the answer: read it instead of following it.
        .max_redirects(0)
        .build()
        .new_agent();
    let response = agent
        .get(latest)
        .header("User-Agent", concat!("PdfCraft/", env!("CARGO_PKG_VERSION")))
        // Read error statuses ourselves: a 403 or 429 means GitHub is limiting requests, and the
        // retry time may ride along in a response header.
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|e| format!("couldn't reach GitHub ({e})"))?;
    let status = response.status().as_u16();
    if status == 403 || status == 429 {
        let reset = response.headers().get("x-ratelimit-reset").and_then(|v| v.to_str().ok());
        return Err(rate_limited(reset, now_unix()));
    }
    if !(300..400).contains(&status) {
        return Err(format!("couldn't reach GitHub (http status: {status})"));
    }
    let location = response.headers().get("location").and_then(|v| v.to_str().ok()).ok_or("no release found")?;
    from_location(location, page)
}

/// The release a `releases/latest` redirect points at, `<page>/tag/<tag>`. Anything else (the
/// release list, when there is no release yet, or another site) is no release.
fn from_location(location: &str, page: &str) -> Result<Release, String> {
    let tag = location
        .strip_prefix(page)
        .and_then(|rest| rest.strip_prefix("/tag/"))
        .filter(|t| !t.is_empty() && t.len() <= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
        .ok_or("no release found")?;
    Ok(Release { version: tag.to_string(), url: location.to_string() })
}

/// The certificate authorities the operating system trusts.
fn os_roots() -> Result<ureq::tls::RootCerts, String> {
    let found = rustls_native_certs::load_native_certs();
    let certs: Vec<ureq::tls::Certificate<'static>> = found.certs.iter().map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned()).collect();
    if certs.is_empty() {
        return Err("no trusted certificates found on this system".into());
    }
    Ok(ureq::tls::RootCerts::new_with_certs(&certs))
}

/// The current Unix time in seconds (0 when the clock gives nothing usable).
fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// What to tell the user when GitHub answers 403: its anonymous quota (60 checks an hour,
/// shared by everyone behind the same address) ran out. `reset` is the `x-ratelimit-reset`
/// header, the Unix time the quota refills; `now` is the current Unix time. Anything
/// surprising (no header, not a number, already past) falls back to asking for patience.
fn rate_limited(reset: Option<&str>, now: u64) -> String {
    const FALLBACK: &str = "GitHub is limiting update checks for now (shared hourly quota); try again later";
    let wait = reset.and_then(|r| r.trim().parse::<u64>().ok()).and_then(|reset| reset.checked_sub(now)).filter(|&wait| wait > 0);
    match wait {
        Some(wait) => {
            let minutes = wait.div_ceil(60).max(1);
            let plural = if minutes == 1 { "" } else { "s" };
            format!("GitHub is limiting update checks for now (shared hourly quota); try again in about {minutes} minute{plural}")
        }
        None => FALLBACK.into(),
    }
}

/// The version a release tag names (`v0.8.0-p1` → `0.8.0-p1`), or why it isn't one. Only
/// characters a version uses get through to the installer's environment.
fn tag_version(tag: &str) -> Result<&str, String> {
    let version = tag.trim().trim_start_matches(['v', 'V']);
    if version.is_empty() || version.len() > 64 || !version.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') {
        return Err(format!("`{tag}` is not a release version"));
    }
    Ok(version)
}

/// The `X.Y.Z` an original PdfCraft release tag names (`v0.12.0` → `0.12.0`): what the fork's
/// release must be built on. Each part is a number of at most nine digits, as install.sh expects.
fn upstream_base(tag: &str) -> Result<&str, String> {
    let version = tag_version(tag)?;
    let base = version.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<&str> = base.split('.').collect();
    let number = |p: &&str| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()) && (*p == "0" || !p.starts_with('0'));
    if parts.len() != 3 || !parts.iter().all(number) {
        return Err(format!("`{tag}` is not a version like v0.12.0"));
    }
    Ok(base)
}

/// Help ▸ Check for updates ▸ Install and relaunch, and Merge and update: the installer that ships
/// inside the app bundle (`PdfCraft.app/Contents/Resources/install.sh`, from
/// packaging/macos/install.sh). It downloads the fork's release with curl (so macOS doesn't
/// quarantine it), checks it against the release's SHA256SUMS.txt and its signature, and swaps the
/// bundle. It never installs the original PdfCraft's build, which would drop the fork's changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installer {
    script: PathBuf,
    /// The folder that holds `PdfCraft.app` (usually /Applications).
    apps: PathBuf,
    /// Where the installer's output goes (`update.log`).
    log: PathBuf,
}

impl Installer {
    /// The bundled installer, when `exe` (the running binary) is `…/PdfCraft.app/Contents/MacOS/…`
    /// and the bundle has it. Anything else (a `cargo run`, a renamed bundle) has none, and the
    /// Updates dialog offers the release page instead. `logs` is the app's log folder.
    pub fn bundled(exe: &Path, logs: &Path) -> Option<Installer> {
        let macos = exe.parent().filter(|d| d.file_name().is_some_and(|n| n == "MacOS"))?;
        let contents = macos.parent().filter(|d| d.file_name().is_some_and(|n| n == "Contents"))?;
        let bundle = contents.parent().filter(|d| d.file_name().is_some_and(|n| n == "PdfCraft.app"))?;
        let script = contents.join("Resources").join("install.sh");
        script.is_file().then(|| Installer {
            script,
            apps: bundle.parent().map_or_else(PathBuf::new, Path::to_path_buf),
            log: logs.join("update.log"),
        })
    }

    /// The installer's command. Install and relaunch passes the release and this app's `pid`,
    /// whose exit the installer waits for before it replaces anything. Merge and update passes
    /// the original PdfCraft's version the fork's next release must be built on; the installer
    /// waits for that release and asks before quitting the app.
    fn command(&self, what: &Install, pid: u32) -> Result<std::process::Command, String> {
        let mut c = std::process::Command::new("/bin/bash");
        c.arg(&self.script).env("PDFCRAFT_APPS", &self.apps).stdin(std::process::Stdio::null());
        match what {
            Install::Release(tag) => c.env("PDFCRAFT_VERSION", tag_version(tag)?).env("PDFCRAFT_WAIT_PID", pid.to_string()),
            Install::AfterMerge(tag) => c.env("PDFCRAFT_AWAIT_BASE", upstream_base(tag)?),
        };
        Ok(c)
    }

    /// Start the installer in the background; it outlives this app.
    pub fn start(&self, what: &Install) -> Result<(), String> {
        let mut c = self.command(what, std::process::id())?;
        if let Some(dir) = self.log.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {} ({e})", dir.display()))?;
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .map_err(|e| format!("couldn't open {} ({e})", self.log.display()))?;
        let err = log.try_clone().map_err(|e| format!("couldn't open {} ({e})", self.log.display()))?;
        c.stdout(log).stderr(err);
        // Its own process group: quitting the app (or a Ctrl-C in its terminal) must not stop it.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut c, 0);
        let child = c.spawn().map_err(|e| format!("couldn't start the installer ({e})"))?;
        log::info!("update ({what:?}): installer started (pid {}), output in {}", child.id(), self.log.display());
        Ok(())
    }
}

/// The fork whose Sync upstream workflow merges the original PdfCraft in
/// (`.github/workflows/sync-upstream.yml`).
const FORK_REPO: &str = "krichelj/pdfcraft";
const SYNC_WORKFLOW: &str = "sync-upstream.yml";
/// How long the GitHub CLI may take to find itself or start the workflow.
const GH_TIMEOUT: Duration = Duration::from_secs(60);

/// Merge and update: start the fork's Sync upstream workflow with the GitHub CLI and the user's
/// own sign-in (`gh auth login`). Runs on the dialog's background thread.
pub fn trigger_sync() -> Result<(), String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    GitHubCli::find(&GitHubCli::usual_places(home.as_deref()), Path::new("/bin/zsh"))?.run_sync(GH_TIMEOUT)
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
    fn find(places: &[PathBuf], shell: &Path) -> Result<GitHubCli, String> {
        if let Some(program) = places.iter().find(|p| executable(p)) {
            return Ok(GitHubCli { program: program.clone() });
        }
        let mut lookup = std::process::Command::new(shell);
        lookup.args(["-lc", "command -v gh"]);
        if let Ok(out) = run_for(lookup, GH_TIMEOUT)
            && out.status.success()
        {
            let found = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if found.is_absolute() && executable(&found) {
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

    fn run_sync(&self, timeout: Duration) -> Result<(), String> {
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

/// Run `command` to its end, or kill it after `timeout`. Its output is read while it runs, so a
/// chatty command can't fill the pipe and stall.
fn run_for(mut command: std::process::Command, timeout: Duration) -> Result<std::process::Output, String> {
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run {} ({e})", command.get_program().to_string_lossy()))?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                // At most 64 KiB is kept: enough for any message gh prints.
                let _ = pipe.by_ref().take(64 * 1024).read_to_end(&mut bytes);
                let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            }
            bytes
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} didn't finish within {}s", command.get_program().to_string_lossy(), timeout.as_secs()));
            }
            Err(e) => return Err(format!("couldn't wait for {} ({e})", command.get_program().to_string_lossy())),
        }
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    Ok(std::process::Output { status, stdout, stderr })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_release_is_read_from_the_redirect_and_only_from_our_tag_pages() {
        let r = from_location("https://github.com/krichelj/pdfcraft/releases/tag/v0.8.0-p1", RELEASES_PAGE).unwrap();
        assert_eq!(r, Release { version: "v0.8.0-p1".into(), url: "https://github.com/krichelj/pdfcraft/releases/tag/v0.8.0-p1".into() });
        for elsewhere in [
            "https://github.com/krichelj/pdfcraft/releases",
            "https://github.com/krichelj/pdfcraft/releases/tag/",
            "https://github.com/krichelj/pdfcraft/releases/tag/v1?x=1",
            "https://github.com/krichelj/pdfcraft/releases/tag/v1/../../evil",
            "https://github.com/krichelj/pdfcraft/releases.evil/tag/v1",
            "https://github.com/storytold/pdfcraft/releases/tag/v9.9.9",
            "https://example.com/pdfcraft.exe",
            "javascript:alert(1)",
            "",
        ] {
            assert_eq!(from_location(elsewhere, RELEASES_PAGE), Err("no release found".to_string()), "{elsewhere}");
        }
        let r = from_location("https://github.com/storytold/pdfcraft/releases/tag/v0.9.0", UPSTREAM_RELEASES_PAGE).unwrap();
        assert_eq!(r.version, "v0.9.0");
        assert!(
            from_location("https://github.com/storytold/pdfcraft/releases/tag/v0.9.0", RELEASES_PAGE).is_err(),
            "each page only from its own repository"
        );
    }

    /// A temporary folder holding `PdfCraft.app/Contents/{MacOS/PdfCraft, Resources/install.sh}`.
    fn bundle(name: &str, with_script: bool) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("pdfcraft-installer-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let contents = root.join("Applications").join("PdfCraft.app").join("Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::create_dir_all(contents.join("Resources")).unwrap();
        std::fs::write(contents.join("MacOS").join("PdfCraft"), b"").unwrap();
        if with_script {
            std::fs::write(contents.join("Resources").join("install.sh"), b"").unwrap();
        }
        (root, contents.join("MacOS").join("PdfCraft"))
    }

    #[test]
    fn the_bundled_installer_is_found_only_inside_an_installed_bundle() {
        let logs = Path::new("/logs");
        let (root, exe) = bundle("found", true);
        let i = Installer::bundled(&exe, logs).expect("inside PdfCraft.app with its installer");
        assert_eq!(i.apps, root.join("Applications"));
        assert_eq!(i.log, logs.join("update.log"));
        let (root2, exe2) = bundle("no-script", false);
        assert_eq!(Installer::bundled(&exe2, logs), None, "a bundle without install.sh");
        assert_eq!(Installer::bundled(Path::new("/usr/local/bin/pdfcraft"), logs), None, "not a bundle");
        assert_eq!(Installer::bundled(Path::new("pdfcraft"), logs), None, "no parent at all");
        let renamed = root.join("Applications").join("Other.app").join("Contents").join("MacOS").join("PdfCraft");
        assert_eq!(Installer::bundled(&renamed, logs), None, "a renamed bundle would be installed beside itself");
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(root2);
    }

    fn env_of(c: &std::process::Command) -> std::collections::HashMap<String, String> {
        c.get_envs().filter_map(|(k, v)| Some((k.to_str()?.to_string(), v?.to_str()?.to_string()))).collect()
    }

    fn installer() -> Installer {
        Installer { script: "/A/PdfCraft.app/Contents/Resources/install.sh".into(), apps: "/A".into(), log: "/l/update.log".into() }
    }

    #[test]
    fn the_installer_gets_the_version_folder_and_pid_and_nothing_untrusted() {
        let i = installer();
        let c = i.command(&Install::Release("v0.8.0-p2".into()), 4242).unwrap();
        assert_eq!(c.get_program(), "/bin/bash");
        assert_eq!(c.get_args().collect::<Vec<_>>(), ["/A/PdfCraft.app/Contents/Resources/install.sh"]);
        let env = env_of(&c);
        assert_eq!(env["PDFCRAFT_VERSION"], "0.8.0-p2");
        assert_eq!(env["PDFCRAFT_APPS"], "/A");
        assert_eq!(env["PDFCRAFT_WAIT_PID"], "4242");
        assert!(!env.contains_key("PDFCRAFT_REPO"), "only the fork's builds are installed: {env:?}");
        assert!(!env.contains_key("PDFCRAFT_AWAIT_BASE"));
        for bad in ["", "v", "0.8.0; rm -rf ~", "$(id)", "0.8.0\nx", "../../x", &"9".repeat(65)] {
            assert!(i.command(&Install::Release(bad.into()), 1).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn merge_and_update_waits_for_the_fork_release_built_on_the_upstream_version() {
        let c = installer().command(&Install::AfterMerge("v0.12.0".into()), 4242).unwrap();
        let env = env_of(&c);
        assert_eq!(env["PDFCRAFT_AWAIT_BASE"], "0.12.0");
        assert_eq!(env["PDFCRAFT_APPS"], "/A");
        // The app keeps running while the release is made: nothing waits for it to quit.
        assert!(!env.contains_key("PDFCRAFT_WAIT_PID"), "{env:?}");
        assert!(!env.contains_key("PDFCRAFT_VERSION") && !env.contains_key("PDFCRAFT_REPO"), "{env:?}");
        assert_eq!(upstream_base("0.12.0-rc1"), Ok("0.12.0"));
        for bad in ["", "v0.12", "v0.12.0.1", "v0.012.0", "v1234567890.0.0", "nightly", "v0.12.0; id", "v0.x.0"] {
            assert!(installer().command(&Install::AfterMerge(bad.into()), 1).is_err(), "{bad:?}");
        }
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

    #[test]
    fn rate_limit_messages_name_the_wait() {
        let limited = "GitHub is limiting update checks for now (shared hourly quota); try again in about 20 minutes";
        assert_eq!(rate_limited(Some("1800001200"), 1800000000), limited);
        // Rounds up: 61 seconds out reads as 2 minutes.
        assert_eq!(
            rate_limited(Some("1800000061"), 1800000000),
            "GitHub is limiting update checks for now (shared hourly quota); try again in about 2 minutes"
        );
        assert_eq!(
            rate_limited(Some("1800000060"), 1800000000),
            "GitHub is limiting update checks for now (shared hourly quota); try again in about 1 minute"
        );
        // No header, not a number, or already past: ask for patience instead of a time.
        let later = "GitHub is limiting update checks for now (shared hourly quota); try again later";
        for bad in [None, Some(""), Some("soon"), Some("1799999999"), Some("1800000000")] {
            assert_eq!(rate_limited(bad, 1800000000), later);
        }
    }

    /// Live: asks GitHub over TLS with the OS's roots (`cargo test -p pdfcraft -- --ignored`).
    #[test]
    #[ignore = "needs network access"]
    fn github_answers_with_the_latest_releases() {
        let l = latest_release().unwrap();
        let fork = l.fork.expect("the fork answered");
        assert!(pdfcraft_ui_egui::updates::is_newer(&fork.version, "0.0.0"), "{fork:?}");
        assert!(fork.url.starts_with(RELEASES_PAGE), "{fork:?}");
        let up = l.upstream.expect("the original answered");
        assert!(up.url.starts_with(UPSTREAM_RELEASES_PAGE), "{up:?}");
    }
}
