//! The updater's worker: looks at GitHub until the release is out, then runs the installer.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::args::Args;
use crate::gh::{FORK_REPO, GitHubCli};
use crate::stages::{self, Local, Mode, Remote};

/// What the window shows, shared with the worker.
#[derive(Clone, Debug, Default)]
pub struct Model {
    pub remote: Remote,
    pub local: Local,
}

/// How often GitHub is asked again. Each look is up to five API calls on the user's own quota
/// (5,000 an hour), so six seconds stays far inside it for the hours a build may take.
const POLL: Duration = Duration::from_secs(6);
/// How long a merge is followed before the updater gives up (the installer's own wait).
const GIVE_UP: Duration = Duration::from_secs(6 * 60 * 60);
const API_TIMEOUT: Duration = Duration::from_secs(30);

/// Ask GitHub everything the stages need, as of now. Errors are kept on the result, not thrown:
/// a look that fails (no network for a moment) leaves the stages where they were.
pub fn look(gh: &GitHubCli, base: &str, since: &str, last: &Remote) -> Remote {
    let api = |path: String| gh.api(&path, API_TIMEOUT);
    let mut r = Remote::default();
    let mut errors = Vec::new();
    match api(format!("repos/{FORK_REPO}/releases/latest")) {
        Ok(v) => r.release = stages::published(&v, base),
        // No release at all answers 404: not an error for the stages.
        Err(e) if e.contains("404") => {}
        Err(e) => errors.push(e),
    }
    match api(format!("repos/{FORK_REPO}/actions/workflows/sync-upstream.yml/runs?per_page=10")) {
        Ok(v) => r.sync_run = stages::newest_run(&v, since),
        Err(e) => errors.push(e),
    }
    match api(format!("repos/{FORK_REPO}/pulls?state=all&sort=created&direction=desc&per_page=20")) {
        Ok(v) => r.pr = stages::sync_pr(&v, since),
        Err(e) => errors.push(e),
    }
    if let Some(pr) = &r.pr {
        match api(format!("repos/{FORK_REPO}/commits/{}/check-runs?per_page=100", pr.head_sha)) {
            Ok(v) => r.checks = stages::checks(&v),
            Err(e) => errors.push(e),
        }
        if let Some(merged) = &pr.merged_at {
            match api(format!("repos/{FORK_REPO}/actions/workflows/release.yml/runs?branch=main&per_page=10")) {
                Ok(v) => r.release_run = stages::newest_run(&v, merged),
                Err(e) => errors.push(e),
            }
        }
    }
    if !errors.is_empty() {
        // Keep what the last good look knew for whatever this one couldn't see.
        r.sync_run = r.sync_run.or_else(|| last.sync_run.clone());
        r.pr = r.pr.or_else(|| last.pr.clone());
        if r.checks.is_empty() {
            r.checks = last.checks.clone();
        }
        r.release_run = r.release_run.or_else(|| last.release_run.clone());
        r.release = r.release.or_else(|| last.release.clone());
        r.error = errors.into_iter().next();
    }
    r
}

fn set(model: &Mutex<Model>, f: impl FnOnce(&mut Model)) {
    f(&mut model.lock().unwrap_or_else(PoisonError::into_inner));
}

/// Follow the update to its end. `repaint` wakes the window after each change.
pub fn run(args: &Args, model: &Arc<Mutex<Model>>, repaint: &(dyn Fn() + Send + Sync)) {
    let version = match &args.mode {
        Mode::Install { version } => version.clone(),
        Mode::Merge { base, since } => {
            let gh = match GitHubCli::locate() {
                Ok(gh) => gh,
                Err(e) => {
                    set(model, |m| m.remote.stopped = Some(e));
                    repaint();
                    return;
                }
            };
            let started = Instant::now();
            loop {
                let last = model.lock().unwrap_or_else(PoisonError::into_inner).remote.clone();
                let remote = look(&gh, base, since, &last);
                let release = remote.release.clone();
                set(model, |m| m.remote = remote);
                repaint();
                if let Some(r) = release {
                    break r.version;
                }
                if started.elapsed() > GIVE_UP {
                    set(model, |m| {
                        m.remote.stopped =
                            Some(format!("no release built on PdfCraft {base} after six hours; Check for updates offers it once it is out"))
                    });
                    repaint();
                    return;
                }
                std::thread::sleep(POLL);
            }
        }
    };
    let model_for_progress = Arc::clone(model);
    crate::install::run(crate::install::command(args, &version), &args.log, |l| {
        let l = l.clone();
        set(&model_for_progress, |m| m.local = l);
        repaint();
    });
}
