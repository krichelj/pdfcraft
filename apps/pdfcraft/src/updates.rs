//! Asks GitHub for the latest PdfCraft release (Help ▸ Check for updates, issue #28).

use std::path::{Path, PathBuf};
use std::time::Duration;

use pdfcraft_ui_egui::updates::{RELEASES_PAGE, Release};

// The fork's releases (krichelj/pdfcraft), not upstream's: see `RELEASES_PAGE`.
const LATEST: &str = "https://api.github.com/repos/krichelj/pdfcraft/releases/latest";

/// The latest release. The answer is untrusted: its size is capped, and only a page under
/// [`RELEASES_PAGE`] is ever offered for download (anything else falls back to that list).
pub fn latest_release() -> Result<Release, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(os_roots()?).build())
        .build()
        .new_agent();
    let mut response = agent
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", concat!("PdfCraft/", env!("CARGO_PKG_VERSION")))
        // Read error statuses ourselves: a 403 from GitHub usually means its anonymous
        // rate limit ran out, and the retry time rides along in a response header.
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|e| format!("couldn't reach GitHub ({e})"))?;
    let status = response.status().as_u16();
    if status == 403 {
        let reset = response.headers().get("x-ratelimit-reset").and_then(|v| v.to_str().ok());
        return Err(rate_limited(reset, now_unix()));
    }
    if !(200..300).contains(&status) {
        return Err(format!("couldn't reach GitHub (http status: {status})"));
    }
    let body = response.body_mut().with_config().limit(1 << 20).read_to_string().map_err(|e| format!("unreadable answer ({e})"))?;
    parse(&body)
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

fn parse(body: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("unreadable answer ({e})"))?;
    let version = v["tag_name"].as_str().filter(|t| !t.is_empty() && t.len() <= 64).ok_or("no release found")?.to_string();
    let url = v["html_url"]
        .as_str()
        .filter(|u| u.strip_prefix(RELEASES_PAGE).is_some_and(|rest| rest.starts_with('/') && !rest.contains(['?', '#', '\\'])))
        .unwrap_or(RELEASES_PAGE)
        .to_string();
    Ok(Release { version, url })
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
    fn answers_are_read_and_only_our_release_pages_are_offered() {
        let r = parse(r#"{"tag_name":"v0.2.0","html_url":"https://github.com/krichelj/pdfcraft/releases/tag/v0.2.0"}"#).unwrap();
        assert_eq!(r, Release { version: "v0.2.0".into(), url: "https://github.com/krichelj/pdfcraft/releases/tag/v0.2.0".into() });
        for elsewhere in [
            "https://example.com/pdfcraft.exe",
            "https://github.com/krichelj/pdfcraft/releases.evil/x",
            "https://github.com/storytold/pdfcraft/releases/tag/v9.9.9",
            "javascript:alert(1)",
        ] {
            let r = parse(&format!(r#"{{"tag_name":"v9.9.9","html_url":"{elsewhere}"}}"#)).unwrap();
            assert_eq!(r.url, RELEASES_PAGE, "{elsewhere}");
        }
        assert!(parse(r#"{"message":"Not Found"}"#).is_err());
        assert!(parse("<html>").is_err());
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
        assert!(r.url.starts_with(RELEASES_PAGE), "{r:?}");
    }
}
