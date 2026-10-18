//! Asks GitHub for the latest PdfCraft release and installs it (Help ▸ Check for updates, issue #28).

use std::path::{Path, PathBuf};
use std::time::Duration;

use pdfcraft_ui_egui::updates::{RELEASES_PAGE, Release, UPSTREAM_RELEASES_PAGE};

// The fork's releases (krichelj/pdfcraft), not upstream's: see `RELEASES_PAGE`. The page, not
// the REST API: GitHub allows 60 anonymous API requests an hour per address, which everyone on a
// campus or office network shares and runs out of; the page's redirect to the latest release's tag
// has no such quota. packaging/macos/install.sh finds the release the same way.
const LATEST: &str = "https://github.com/krichelj/pdfcraft/releases/latest";
/// The original PdfCraft's latest release: the fork follows it, so a newer one there is news too.
const UPSTREAM_LATEST: &str = "https://github.com/storytold/pdfcraft/releases/latest";

/// The newest release of the fork and of the original repository (Help ▸ Check for updates).
/// One repository not answering still gives the other's; both failing gives the fork's error.
pub fn latest_release() -> Result<Release, String> {
    match (latest_from(LATEST, RELEASES_PAGE, true), latest_from(UPSTREAM_LATEST, UPSTREAM_RELEASES_PAGE, false)) {
        (Ok(fork), Ok(upstream)) => Ok(Release::newest(fork, upstream)),
        (Ok(one), Err(_)) | (Err(_), Ok(one)) => Ok(one),
        (Err(e), Err(_)) => Err(e),
    }
}

/// The release `latest` (a `releases/latest` page) redirects to. The answer is untrusted: only a
/// tag page under `page` counts, and anything else is "no release found".
fn latest_from(latest: &str, page: &str, from_fork: bool) -> Result<Release, String> {
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
    from_location(location, page, from_fork)
}

/// The release a `releases/latest` redirect points at, `RELEASES_PAGE/tag/<tag>`. Anything else
/// (the release list, when there is no release yet, or another site) is no release.
fn from_location(location: &str, page: &str, from_fork: bool) -> Result<Release, String> {
    let tag = location
        .strip_prefix(page)
        .and_then(|rest| rest.strip_prefix("/tag/"))
        .filter(|t| !t.is_empty() && t.len() <= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
        .ok_or("no release found")?;
    Ok(Release { version: tag.to_string(), url: location.to_string(), from_fork })
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

/// Help ▸ Check for updates ▸ Install and relaunch: the installer that ships inside the app bundle
/// (`PdfCraft.app/Contents/Resources/install.sh`, from packaging/macos/install.sh). It downloads
/// the release with curl (so macOS doesn't quarantine it), checks it against the release's
/// SHA256SUMS.txt and its signature, waits for this app to quit, swaps the bundle and opens it.
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

    /// The installer's command for release `tag` (such as `v0.8.0-p1`), waiting for process `pid`
    /// (this app) to exit before it replaces anything.
    fn command(&self, tag: &str, pid: u32) -> Result<std::process::Command, String> {
        let version = tag.trim().trim_start_matches(['v', 'V']);
        if version.is_empty() || version.len() > 64 || !version.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') {
            return Err(format!("`{tag}` is not a release version"));
        }
        let mut c = std::process::Command::new("/bin/bash");
        c.arg(&self.script)
            .env("PDFCRAFT_VERSION", version)
            .env("PDFCRAFT_APPS", &self.apps)
            .env("PDFCRAFT_WAIT_PID", pid.to_string())
            .stdin(std::process::Stdio::null());
        Ok(c)
    }

    /// Start installing release `tag` in the background; it outlives this app.
    pub fn start(&self, tag: &str) -> Result<(), String> {
        let mut c = self.command(tag, std::process::id())?;
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
        log::info!("update to {tag}: installer started (pid {}), output in {}", child.id(), self.log.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_release_is_read_from_the_redirect_and_only_from_our_tag_pages() {
        let r = from_location("https://github.com/krichelj/pdfcraft/releases/tag/v0.8.0-p1", RELEASES_PAGE, true).unwrap();
        assert_eq!(
            r,
            Release { version: "v0.8.0-p1".into(), url: "https://github.com/krichelj/pdfcraft/releases/tag/v0.8.0-p1".into(), from_fork: true }
        );
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
            assert_eq!(from_location(elsewhere, RELEASES_PAGE, true), Err("no release found".to_string()), "{elsewhere}");
        }
    }

    #[test]
    fn the_newer_of_the_two_repositories_is_offered() {
        let fork = |v: &str| Release { version: v.into(), url: format!("{RELEASES_PAGE}/tag/{v}"), from_fork: true };
        let up = |v: &str| Release { version: v.into(), url: format!("{UPSTREAM_RELEASES_PAGE}/tag/{v}"), from_fork: false };
        // Upstream released a new version the fork hasn't synced yet: point at upstream's.
        assert_eq!(Release::newest(fork("v0.8.0-p9"), up("v0.9.0")), up("v0.9.0"));
        // The fork's -p<N> builds on it, so once synced the fork's wins.
        assert_eq!(Release::newest(fork("v0.9.0-p1"), up("v0.9.0")), fork("v0.9.0-p1"));
        assert_eq!(Release::newest(fork("v0.9.0-p1"), up("v0.8.0")), fork("v0.9.0-p1"));
        // An upstream tag that isn't a version never wins.
        assert_eq!(Release::newest(fork("v0.9.0-p1"), up("nightly")), fork("v0.9.0-p1"));
        let r = from_location("https://github.com/storytold/pdfcraft/releases/tag/v0.9.0", UPSTREAM_RELEASES_PAGE, false).unwrap();
        assert!(!r.from_fork && r.version == "v0.9.0");
        assert!(
            from_location("https://github.com/storytold/pdfcraft/releases/tag/v0.9.0", RELEASES_PAGE, true).is_err(),
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

    #[test]
    fn the_installer_gets_the_version_folder_and_pid_and_nothing_untrusted() {
        let i = Installer { script: "/A/PdfCraft.app/Contents/Resources/install.sh".into(), apps: "/A".into(), log: "/l/update.log".into() };
        let c = i.command("v0.8.0-p2", 4242).unwrap();
        assert_eq!(c.get_program(), "/bin/bash");
        assert_eq!(c.get_args().collect::<Vec<_>>(), ["/A/PdfCraft.app/Contents/Resources/install.sh"]);
        let env: std::collections::HashMap<_, _> =
            c.get_envs().filter_map(|(k, v)| Some((k.to_str()?.to_string(), v?.to_str()?.to_string()))).collect();
        assert_eq!(env["PDFCRAFT_VERSION"], "0.8.0-p2");
        assert_eq!(env["PDFCRAFT_APPS"], "/A");
        assert_eq!(env["PDFCRAFT_WAIT_PID"], "4242");
        for bad in ["", "v", "0.8.0; rm -rf ~", "$(id)", "0.8.0\nx", "../../x", &"9".repeat(65)] {
            assert!(i.command(bad, 1).is_err(), "{bad:?}");
        }
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
    fn github_answers_with_the_latest_release() {
        let r = latest_release().unwrap();
        assert!(pdfcraft_ui_egui::updates::is_newer(&r.version, "0.0.0"), "{r:?}");
        let page = if r.from_fork { RELEASES_PAGE } else { UPSTREAM_RELEASES_PAGE };
        assert!(r.url.starts_with(page), "the release page of the repository it came from: {r:?}");
    }
}
