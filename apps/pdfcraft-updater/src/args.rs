//! The updater's command line: what PdfCraft passes it, written and read here so both sides agree.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::stages::Mode;

/// What to do, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    /// PdfCraft's process, whose exit Install and relaunch waits for.
    pub wait_pid: Option<u32>,
    /// The installer (`PdfCraft.app/Contents/Resources/install.sh`).
    pub installer: PathBuf,
    /// The folder that holds PdfCraft.app.
    pub apps: PathBuf,
    /// Where the installer's output is kept.
    pub log: PathBuf,
}

/// A version the installer accepts: letters, digits, dots and dashes, at most 64.
fn version_like(v: &str) -> bool {
    !v.is_empty() && v.len() <= 64 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// An ISO 8601 UTC time as GitHub writes it (`2026-11-12T21:15:15Z`).
fn time_like(t: &str) -> bool {
    t.len() == 20 && t.ends_with('Z') && t.bytes().all(|b| b.is_ascii_digit() || b"-:TZ".contains(&b))
}

impl Args {
    /// The command line for these arguments.
    pub fn to_args(&self) -> Vec<OsString> {
        let mut a: Vec<OsString> = Vec::new();
        match &self.mode {
            Mode::Merge { base, since } => a.extend(["--merge".into(), base.into(), "--since".into(), since.into()]),
            Mode::Install { version } => a.extend(["--install".into(), version.into()]),
        }
        if let Some(pid) = self.wait_pid {
            a.extend(["--wait-pid".into(), pid.to_string().into()]);
        }
        a.extend(["--installer".into(), self.installer.clone().into_os_string()]);
        a.extend(["--apps".into(), self.apps.clone().into_os_string()]);
        a.extend(["--log".into(), self.log.clone().into_os_string()]);
        a
    }

    /// Read a command line (without the program name). Anything unexpected is refused.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
        let mut it = args.into_iter();
        let (mut merge, mut since, mut install, mut wait_pid, mut installer, mut apps, mut log) = (None, None, None, None, None, None, None);
        while let Some(flag) = it.next() {
            let flag = flag.to_string_lossy().into_owned();
            let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
            let text = || value.to_str().map(str::to_string).ok_or_else(|| format!("{flag}'s value isn't text"));
            match flag.as_str() {
                "--merge" => merge = Some(text()?),
                "--since" => since = Some(text()?),
                "--install" => install = Some(text()?),
                "--wait-pid" => wait_pid = Some(text()?.parse::<u32>().map_err(|_| "--wait-pid is not a process id".to_string())?),
                "--installer" => installer = Some(PathBuf::from(value)),
                "--apps" => apps = Some(PathBuf::from(value)),
                "--log" => log = Some(PathBuf::from(value)),
                _ => return Err(format!("unknown option {flag}")),
            }
        }
        let mode = match (merge, since, install) {
            (Some(base), Some(since), None) => {
                let parts: Vec<&str> = base.split('.').collect();
                if parts.len() != 3 || !parts.iter().all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit())) {
                    return Err(format!("--merge {base} is not a version like 0.21.1"));
                }
                if !time_like(&since) {
                    return Err(format!("--since {since} is not a time like 2026-11-12T21:15:15Z"));
                }
                Mode::Merge { base, since }
            }
            (None, None, Some(version)) if version_like(&version) => Mode::Install { version },
            (None, None, Some(version)) => return Err(format!("--install {version} is not a version")),
            _ => return Err("give either --merge VERSION --since TIME, or --install VERSION".into()),
        };
        Ok(Args {
            mode,
            wait_pid,
            installer: installer.ok_or("--installer is missing")?,
            apps: apps.ok_or("--apps is missing")?,
            log: log.ok_or("--log is missing")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn what_pdfcraft_writes_the_updater_reads_back() {
        for mode in [Mode::Merge { base: "0.21.1".into(), since: "2026-11-12T21:15:15Z".into() }, Mode::Install { version: "0.21.0-p2".into() }] {
            let a = Args { mode, wait_pid: Some(42), installer: "/A/i.sh".into(), apps: "/A".into(), log: "/l/u.log".into() };
            assert_eq!(Args::parse(a.to_args()), Ok(a.clone()));
            let none = Args { wait_pid: None, ..a };
            assert_eq!(Args::parse(none.to_args()), Ok(none));
        }
    }

    #[test]
    fn anything_unexpected_is_refused() {
        let tail = ["--installer", "/i", "--apps", "/A", "--log", "/l"];
        let with = |head: &[&str]| Args::parse(os(&[head, &tail[..]].concat()));
        assert!(with(&["--merge", "0.21.1", "--since", "2026-11-12T21:15:15Z"]).is_ok());
        for bad in [
            &["--merge", "0.21", "--since", "2026-11-12T21:15:15Z"][..],
            &["--merge", "0.21.1; id", "--since", "2026-11-12T21:15:15Z"],
            &["--merge", "0.21.1", "--since", "yesterday"],
            &["--merge", "0.21.1"],
            &["--install", "0.21.0-p2; rm -rf ~"],
            &["--install", ""],
            &["--install", "1", "--merge", "0.21.1", "--since", "2026-11-12T21:15:15Z"],
            &["--install", "1", "--wait-pid", "-3"],
            &["--install", "1", "--bogus", "x"],
            &["--install"],
        ] {
            assert!(with(bad).is_err(), "{bad:?}");
        }
        assert!(Args::parse(os(&["--install", "1"])).is_err(), "paths are required");
    }
}
