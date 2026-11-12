//! Asks GitHub for the latest PdfCraft releases, installs the fork's, and starts the fork's merge
//! of a newer original PdfCraft (Help ▸ Check for updates, issue #28).

use std::path::{Path, PathBuf};
use std::time::Duration;

use pdfcraft_ui_egui::updates::{Install, Latest, RELEASES_PAGE, Release, UPSTREAM_RELEASES_PAGE};
pub use pdfcraft_updater::gh::trigger_sync;
use pdfcraft_updater::gh::{INSTALLER_LOCK, alive};
use pdfcraft_updater::stages::Mode;

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
    /// The installers' lock (`PDFCRAFT_LOCK`): a symbolic link to the running installer's
    /// process id, so one installer runs at a time and this app can tell one is pending.
    lock: PathBuf,
    /// PdfCraft Updater (`Contents/Helpers/PdfCraft Updater.app`), which shows each stage of the
    /// update in its own window. A bundle without it runs the installer hidden, as before.
    updater: Option<PathBuf>,
}

/// How far before Merge and update's click the Sync upstream run it started may be dated:
/// GitHub stamps the run when it queues it, and clocks differ a little.
const SINCE_MARGIN: chrono::TimeDelta = chrono::TimeDelta::seconds(120);

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
            // The same path install.sh uses by default ($TMPDIR is per user on macOS), so the
            // curl one-liner and the app's installers share one lock.
            lock: std::env::temp_dir().join(INSTALLER_LOCK),
            updater: Some(contents.join("Helpers").join(pdfcraft_updater::HELPER_APP))
                .filter(|a| a.join("Contents").join("MacOS").join("pdfcraft-updater").is_file()),
        })
    }

    /// The command that opens PdfCraft Updater for `what` as an app of its own (`open -a`), so it
    /// keeps running when PdfCraft quits. `pid` is this app's process, `now` the click's time.
    fn updater_command(&self, updater: &Path, what: &Install, pid: u32, now: chrono::DateTime<chrono::Utc>) -> Result<std::process::Command, String> {
        let mode = match what {
            Install::Release(tag) => Mode::Install { version: tag_version(tag)?.to_string() },
            Install::AfterMerge(tag) => {
                let since = now.checked_sub_signed(SINCE_MARGIN).unwrap_or(now);
                Mode::Merge { base: upstream_base(tag)?.to_string(), since: since.format("%Y-%m-%dT%H:%M:%SZ").to_string() }
            }
        };
        let wait_pid = matches!(what, Install::Release(_)).then_some(pid);
        let args = pdfcraft_updater::args::Args { mode, wait_pid, installer: self.script.clone(), apps: self.apps.clone(), log: self.log.clone() };
        let mut c = std::process::Command::new("/usr/bin/open");
        c.arg("-a").arg(updater).arg("--args").args(args.to_args()).stdin(std::process::Stdio::null());
        Ok(c)
    }

    /// The installer's command. Install and relaunch passes the release and this app's `pid`,
    /// whose exit the installer waits for before it replaces anything. Merge and update passes
    /// the original PdfCraft's version the fork's next release must be built on; the installer
    /// waits for that release and asks before quitting the app.
    fn command(&self, what: &Install, pid: u32) -> Result<std::process::Command, String> {
        let mut c = std::process::Command::new("/bin/bash");
        c.arg(&self.script).env("PDFCRAFT_APPS", &self.apps).env("PDFCRAFT_LOCK", &self.lock).stdin(std::process::Stdio::null());
        match what {
            Install::Release(tag) => c.env("PDFCRAFT_VERSION", tag_version(tag)?).env("PDFCRAFT_WAIT_PID", pid.to_string()),
            Install::AfterMerge(tag) => c.env("PDFCRAFT_AWAIT_BASE", upstream_base(tag)?),
        };
        Ok(c)
    }

    /// Whether an installer is running: Install and relaunch, or Merge and update's installer
    /// waiting for its release. The Updates dialog offers no second installer meanwhile.
    pub fn busy(&self) -> bool {
        let updater_lock = self.lock.with_file_name(pdfcraft_updater::UPDATER_LOCK);
        [&self.lock, &updater_lock].iter().any(|lock| {
            let Ok(target) = std::fs::read_link(lock) else { return false };
            target.to_string_lossy().parse::<u32>().is_ok_and(alive)
        })
    }

    /// Start the installer in the background; it outlives this app.
    pub fn start(&self, what: &Install) -> Result<(), String> {
        if let Some(updater) = &self.updater {
            let mut c = self.updater_command(updater, what, std::process::id(), chrono::Utc::now())?;
            // `open` hands the app to Launch Services and returns at once.
            let status = c.status().map_err(|e| format!("couldn't open PdfCraft Updater ({e})"))?;
            if !status.success() {
                return Err(format!("couldn't open PdfCraft Updater (open exited with {status})"));
            }
            log::info!("update ({what:?}): PdfCraft Updater opened; installer output in {}", self.log.display());
            return Ok(());
        }
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
        assert_eq!(i.updater, None, "no helper app in this bundle");
        let helper = root.join("Applications/PdfCraft.app/Contents/Helpers/PdfCraft Updater.app/Contents/MacOS");
        std::fs::create_dir_all(&helper).unwrap();
        std::fs::write(helper.join("pdfcraft-updater"), b"").unwrap();
        let with = Installer::bundled(&exe, logs).unwrap();
        assert_eq!(with.updater, Some(root.join("Applications/PdfCraft.app/Contents/Helpers/PdfCraft Updater.app")));
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
        Installer {
            script: "/A/PdfCraft.app/Contents/Resources/install.sh".into(),
            apps: "/A".into(),
            log: "/l/update.log".into(),
            lock: "/t/pdfcraft-installer.lock".into(),
            updater: None,
        }
    }

    #[test]
    fn the_updater_is_opened_as_its_own_app_with_what_to_follow() {
        use chrono::TimeZone;
        let i = installer();
        let app = Path::new("/A/PdfCraft.app/Contents/Helpers/PdfCraft Updater.app");
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 12, 21, 15, 0).unwrap();
        let c = i.updater_command(app, &Install::AfterMerge("v0.21.1".into()), 4242, now).unwrap();
        assert_eq!(c.get_program(), "/usr/bin/open");
        let a: Vec<_> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(a[..3], ["-a", "/A/PdfCraft.app/Contents/Helpers/PdfCraft Updater.app", "--args"]);
        let parsed = pdfcraft_updater::args::Args::parse(c.get_args().skip(3).map(std::ffi::OsStr::to_os_string)).unwrap();
        // The run the click starts is found from two minutes before it; nothing waits for PdfCraft.
        assert_eq!(parsed.mode, Mode::Merge { base: "0.21.1".into(), since: "2026-11-12T21:13:00Z".into() });
        assert_eq!(parsed.wait_pid, None);
        assert_eq!(parsed.installer, Path::new("/A/PdfCraft.app/Contents/Resources/install.sh"));
        let c = i.updater_command(app, &Install::Release("v0.21.1-p1".into()), 4242, now).unwrap();
        let parsed = pdfcraft_updater::args::Args::parse(c.get_args().skip(3).map(std::ffi::OsStr::to_os_string)).unwrap();
        assert_eq!(parsed.mode, Mode::Install { version: "0.21.1-p1".into() });
        assert_eq!(parsed.wait_pid, Some(4242), "Install and relaunch waits for PdfCraft to quit");
        for bad in [Install::Release("$(id)".into()), Install::AfterMerge("v0.21".into())] {
            assert!(i.updater_command(app, &bad, 1, now).is_err(), "{bad:?}");
        }
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
        assert_eq!(env["PDFCRAFT_LOCK"], "/t/pdfcraft-installer.lock", "one lock for every installer");
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
        assert_eq!(env["PDFCRAFT_LOCK"], "/t/pdfcraft-installer.lock");
        // The app keeps running while the release is made: nothing waits for it to quit.
        assert!(!env.contains_key("PDFCRAFT_WAIT_PID"), "{env:?}");
        assert!(!env.contains_key("PDFCRAFT_VERSION") && !env.contains_key("PDFCRAFT_REPO"), "{env:?}");
        assert_eq!(upstream_base("0.12.0-rc1"), Ok("0.12.0"));
        for bad in ["", "v0.12", "v0.12.0.1", "v0.012.0", "v1234567890.0.0", "nightly", "v0.12.0; id", "v0.x.0"] {
            assert!(installer().command(&Install::AfterMerge(bad.into()), 1).is_err(), "{bad:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_running_installer_is_seen_through_its_lock() {
        let dir = std::env::temp_dir().join(format!("pdfcraft-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join(INSTALLER_LOCK);
        let i = Installer { lock: lock.clone(), ..installer() };
        assert!(!i.busy(), "no lock");
        std::os::unix::fs::symlink(std::process::id().to_string(), &lock).unwrap();
        assert!(i.busy(), "a live installer (this process)");
        std::fs::remove_file(&lock).unwrap();
        let mut gone = std::process::Command::new("/bin/sh").args(["-c", "exit 0"]).spawn().unwrap();
        let dead = gone.id();
        gone.wait().unwrap();
        std::os::unix::fs::symlink(dead.to_string(), &lock).unwrap();
        assert!(!i.busy(), "a lock left by an installer that died");
        std::fs::remove_file(&lock).unwrap();
        std::os::unix::fs::symlink("not-a-pid", &lock).unwrap();
        assert!(!i.busy(), "a lock that names no process");
        // PdfCraft Updater following a merge holds its own lock beside the installers'.
        let updater_lock = dir.join(pdfcraft_updater::UPDATER_LOCK);
        std::os::unix::fs::symlink(std::process::id().to_string(), &updater_lock).unwrap();
        assert!(i.busy(), "a live updater");
        std::fs::remove_file(&updater_lock).unwrap();
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
