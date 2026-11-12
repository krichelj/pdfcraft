//! Runs the installer inside PdfCraft.app and turns its output into [`Local`] progress.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use crate::args::Args;
use crate::stages::Local;

/// The installer's command for `version` (`0.21.1-p1`); Install and relaunch also passes the
/// process to wait for. Only the version (checked by [`Args::parse`]) and paths reach it.
pub fn command(args: &Args, version: &str) -> Command {
    let mut c = Command::new("/bin/bash");
    c.arg(&args.installer)
        .env("PDFCRAFT_VERSION", version)
        .env("PDFCRAFT_APPS", &args.apps)
        .env("PDFCRAFT_LOCK", std::env::temp_dir().join(crate::gh::INSTALLER_LOCK))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(pid) = args.wait_pid {
        c.env("PDFCRAFT_WAIT_PID", pid.to_string());
    }
    c
}

/// Split output into the pieces the installer writes: lines, and curl's `\r`-separated progress.
/// `rest` keeps an unfinished piece for the next call.
pub fn pieces(rest: &mut String, bytes: &[u8]) -> Vec<String> {
    rest.push_str(&String::from_utf8_lossy(bytes));
    let mut out = Vec::new();
    while let Some(i) = rest.find(['\n', '\r']) {
        let piece: String = rest.drain(..=i).collect();
        let piece = piece.trim_end_matches(['\n', '\r']);
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
    }
    // A piece that never ends (no newline for a long time) is cut rather than kept growing.
    if rest.len() > 4096 {
        out.push(std::mem::take(rest));
    }
    out
}

/// Run `command` to its end, giving `progress` every change, and keeping the output in `log`.
pub fn run(mut command: Command, log: &std::path::Path, mut progress: impl FnMut(&Local)) -> Local {
    let mut local = Local { log: Some(log.display().to_string()), ..Local::default() };
    let mut log_file = log
        .parent()
        .and_then(|d| std::fs::create_dir_all(d).ok())
        .and_then(|()| std::fs::OpenOptions::new().create(true).append(true).open(log).ok());
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            local.error = Some(format!("couldn't start the installer ({e})"));
            progress(&local);
            return local;
        }
    };
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>)]
        .into_iter()
        .flatten()
    {
        let tx = tx.clone();
        let mut pipe = pipe;
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 || tx.send(buf.get(..n).unwrap_or_default().to_vec()).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);
    let mut rest = String::new();
    // Ends when both pipes close (the installer and anything holding its output have exited).
    for bytes in rx {
        if let Some(f) = log_file.as_mut() {
            let _ = f.write_all(&bytes);
        }
        for piece in pieces(&mut rest, &bytes) {
            local.feed(&piece);
            progress(&local);
        }
    }
    match child.wait() {
        Ok(status) => local.exited(status.success(), &status.to_string()),
        Err(e) => local.exited(false, &e.to_string()),
    }
    progress(&local);
    local
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stages::Step;

    #[test]
    fn curl_progress_and_lines_are_split_and_a_partial_piece_waits() {
        let mut rest = String::new();
        assert_eq!(pieces(&mut rest, b"==> PdfCraft 1: downloading x.zip\n####   1"), ["==> PdfCraft 1: downloading x.zip"]);
        assert_eq!(rest, "####   1");
        assert_eq!(pieces(&mut rest, b"2.5%\r#######  40.0%\r"), ["####   12.5%", "#######  40.0%"]);
        assert!(rest.is_empty());
        // Invalid UTF-8 and a piece that never ends don't stop it.
        assert_eq!(pieces(&mut rest, &[0xff, b'\n']).len(), 1);
        assert_eq!(pieces(&mut rest, &[b'x'; 5000]).len(), 1);
    }

    /// The real installer's output, from a run of packaging/macos/install.sh (with this
    /// change's stage lines), drives the steps in order.
    #[test]
    fn the_installer_output_drives_the_steps() {
        let mut l = Local::default();
        let script = [
            ("==> PdfCraft 0.21.0-p1: downloading pdfcraft-0.21.0-p1-macos-aarch64.zip", Step::Download),
            ("#####                                                                      7.3%", Step::Download),
            ("######################################################################## 100.0%", Step::Download),
            ("==> verifying pdfcraft-0.21.0-p1-macos-aarch64.zip against SHA256SUMS.txt and its signature", Step::Verify),
            ("==> Waiting for PdfCraft (pid 4242) to quit", Step::Install),
            ("==> installing PdfCraft 0.21.0-p1 in /Applications", Step::Install),
            ("==> PdfCraft 0.21.0-p1 is installed in /Applications/PdfCraft.app", Step::Relaunch),
            ("==> relaunching PdfCraft", Step::Relaunch),
        ];
        for (line, step) in script {
            l.feed(line);
            assert_eq!(l.step, step, "{line}");
        }
        assert_eq!(l.percent, Some(100.0));
        l.exited(true, "exit status: 0");
        assert_eq!(l.step, Step::Finished);
        assert_eq!(l.error, None);
    }

    #[test]
    fn an_installer_error_is_kept_and_a_silent_failure_says_how_it_ended() {
        let mut l = Local::default();
        l.feed("==> PdfCraft 1: downloading a.zip");
        l.feed("error: a.zip doesn't match SHA256SUMS.txt (expected 1, got 2)");
        l.exited(false, "exit status: 1");
        assert_eq!(l.error.as_deref(), Some("a.zip doesn't match SHA256SUMS.txt (expected 1, got 2)"));
        let mut quiet = Local::default();
        quiet.exited(false, "signal: 9");
        assert_eq!(quiet.error.as_deref(), Some("the installer stopped (signal: 9)"));
        // A percentage outside the download, or nonsense, is not progress.
        let mut p = Local::default();
        p.feed("==> PdfCraft 1: downloading a.zip");
        for junk in ["NaN%", "inf%", "-3%", "120%"] {
            p.feed(junk);
        }
        assert_eq!(p.percent, None);
    }

    #[cfg(unix)]
    #[test]
    fn a_run_reports_every_step_and_keeps_the_log() {
        let dir = std::env::temp_dir().join(format!("pdfcraft-updater-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Command::new("/bin/sh");
        c.args(["-c", "echo '==> PdfCraft 1: downloading a.zip'; printf '##  50.0%%\\r' >&2; echo '==> verifying a.zip'; echo '==> PdfCraft 1 is installed in /A/PdfCraft.app'"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let log = dir.join("logs").join("update.log");
        let mut seen = Vec::new();
        let l = run(c, &log, |l| seen.push(l.step));
        assert_eq!(l.step, Step::Finished);
        assert!(seen.contains(&Step::Download), "{seen:?}");
        assert!(std::fs::read_to_string(&log).unwrap().contains("is installed in"));
        let mut missing = Command::new(dir.join("no-such-installer"));
        missing.stdout(Stdio::piped());
        assert!(run(missing, &log, |_| {}).error.unwrap().starts_with("couldn't start the installer"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
