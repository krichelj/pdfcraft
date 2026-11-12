//! What the update is doing, as a list of stages, worked out from GitHub's answers (the Sync
//! upstream run, its pull request and checks, the Release run, the latest release) and from the
//! installer's output. Pure: every input is a parsed response or a line of output, so the whole
//! machine is tested from recorded answers.

use serde_json::Value;

use crate::gh::FORK_REPO;

/// What the updater was started for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Merge and update: follow the merge of the original PdfCraft `base` (`X.Y.Z`) into the fork,
    /// started at `since` (an ISO 8601 UTC time), then install the fork's release built on it.
    Merge { base: String, since: String },
    /// Install and relaunch: install the fork's release `version` (`0.21.0-p2`).
    Install { version: String },
}

/// One stage's state.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Pending,
    Active,
    Done,
    Failed,
}

/// One line of the progress list.
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub title: String,
    pub state: State,
    /// What is happening, or why it failed.
    pub detail: String,
    /// The page that shows it (a run, a pull request, a check, a release).
    pub link: Option<String>,
    /// The pull request's checks, under "Run checks".
    pub children: Vec<Stage>,
    /// How far along an active stage is, when that is known (the download).
    pub fraction: Option<f32>,
}

impl Stage {
    fn new(title: &str, state: State, detail: impl Into<String>, link: Option<String>) -> Stage {
        Stage { title: title.into(), state, detail: detail.into(), link, children: Vec::new(), fraction: None }
    }
}

/// A workflow run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub status: String,
    pub conclusion: Option<String>,
    pub url: String,
    pub created_at: String,
}

/// The sync pull request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pr {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub merged_at: Option<String>,
    pub head_sha: String,
}

/// One check on the pull request's head commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: String,
}

/// The fork's latest release, when it is built on the version waited for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published {
    pub version: String,
    pub url: String,
    /// The size of the zip the installer downloads, in bytes.
    pub zip_size: Option<u64>,
    pub has_dmg: bool,
}

/// Everything GitHub said, as of the last look.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Remote {
    pub sync_run: Option<Run>,
    pub pr: Option<Pr>,
    pub checks: Vec<Check>,
    pub release_run: Option<Run>,
    pub release: Option<Published>,
    /// Why the last look failed (no network for a moment): shown on the stage waiting for it.
    pub error: Option<String>,
    /// Why the updater stopped following GitHub (no gh, no release in time): the first stage
    /// not done fails with it.
    pub stopped: Option<String>,
}

/// Where the installer is, read from its output.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Eq, Ord)]
pub enum Step {
    #[default]
    NotStarted,
    Download,
    Verify,
    Install,
    Relaunch,
    Finished,
}

/// The installer's progress.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Local {
    pub step: Step,
    /// The download's percentage (0 to 100) as curl reports it.
    pub percent: Option<f32>,
    /// The installer's own error line, or how it ended.
    pub error: Option<String>,
    /// Where the installer's output is kept.
    pub log: Option<String>,
}

impl Local {
    /// Take in one line (or one `\r`-separated piece) of the installer's output.
    pub fn feed(&mut self, line: &str) {
        let line = line.trim();
        if let Some(e) = line.strip_prefix("error: ") {
            self.error = Some(e.chars().take(400).collect());
        } else if line.starts_with("==> ") {
            let next = if line.contains("is already installed") || line.contains("left for later") {
                Step::Finished
            } else if line.contains("is installed in") {
                Step::Relaunch
            } else if line.contains("Waiting for PdfCraft") || line.contains("Quitting PdfCraft") || line.contains("installing") {
                Step::Install
            } else if line.contains("verifying") {
                Step::Verify
            } else if line.contains(": downloading") {
                Step::Download
            } else {
                self.step
            };
            self.step = self.step.max(next);
        } else if self.step == Step::Download
            && let Some(p) = percent(line)
        {
            self.percent = Some(p);
        }
    }

    /// The installer exited, successfully or not.
    pub fn exited(&mut self, success: bool, how: &str) {
        if success {
            self.step = Step::Finished;
        } else if self.error.is_none() {
            self.error = Some(format!("the installer stopped ({how})"));
        }
    }
}

/// The percentage curl's `--progress-bar` ends a piece with (`######   45.2%`).
fn percent(piece: &str) -> Option<f32> {
    let p: f32 = piece.split_whitespace().last()?.strip_suffix('%')?.parse().ok()?;
    (p.is_finite() && (0.0..=100.0).contains(&p)).then_some(p)
}

/// A string field of a JSON object.
fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_string)
}

fn run_of(v: &Value) -> Option<Run> {
    Some(Run { status: text(v, "status")?, conclusion: text(v, "conclusion"), url: text(v, "html_url")?, created_at: text(v, "created_at")? })
}

/// The newest run in a `…/actions/workflows/<file>/runs` answer created at or after `since`.
/// GitHub's times are all `YYYY-MM-DDTHH:MM:SSZ`, so they compare as text.
pub fn newest_run(answer: &Value, since: &str) -> Option<Run> {
    answer
        .get("workflow_runs")?
        .as_array()?
        .iter()
        .filter_map(run_of)
        .filter(|r| r.created_at.as_str() >= since)
        .max_by(|a, b| a.created_at.cmp(&b.created_at))
}

/// The sync pull request in a `…/pulls?state=all` answer: the newest from a `sync/upstream-`
/// branch that is open, or was merged or closed at or after `since`. The workflow reuses an open
/// one, so it may be older than `since`.
pub fn sync_pr(answer: &Value, since: &str) -> Option<Pr> {
    answer.as_array()?.iter().find_map(|p| {
        let head = p.get("head")?;
        if !text(head, "ref")?.starts_with("sync/upstream-") {
            return None;
        }
        let state = text(p, "state")?;
        let ended = text(p, "closed_at").unwrap_or_default();
        if state != "open" && ended.as_str() < since {
            return None;
        }
        Some(Pr {
            number: p.get("number")?.as_u64()?,
            title: text(p, "title").unwrap_or_default(),
            url: text(p, "html_url")?,
            state,
            merged_at: text(p, "merged_at"),
            head_sha: text(head, "sha")?,
        })
    })
}

/// The checks in a `…/commits/<sha>/check-runs` answer, in GitHub's order. GitHub lists only
/// each check's newest attempt (its default `filter=latest`).
pub fn checks(answer: &Value) -> Vec<Check> {
    let runs = answer.get("check_runs").and_then(Value::as_array).into_iter().flatten();
    runs.filter_map(|c| {
        Some(Check { name: text(c, "name")?, status: text(c, "status")?, conclusion: text(c, "conclusion"), url: text(c, "html_url")? })
    })
    .collect()
}

/// `X.Y.Z` as numbers, or nothing.
fn core(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim_start_matches('v').split(['-', '+']).next()?.split('.').map(|p| p.parse::<u64>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

/// A `…/releases/latest` answer, when that release is built on `base` or later.
pub fn published(answer: &Value, base: &str) -> Option<Published> {
    let tag = text(answer, "tag_name")?;
    if answer.get("draft").and_then(Value::as_bool) == Some(true) || core(&tag)? < core(base)? {
        return None;
    }
    let version = tag.trim_start_matches('v').to_string();
    let assets = answer.get("assets").and_then(Value::as_array).cloned().unwrap_or_default();
    let named = |n: &str| assets.iter().find(|a| text(a, "name").as_deref() == Some(n)).cloned();
    let zip = named(&format!("pdfcraft-{version}-macos-aarch64.zip"));
    Some(Published {
        zip_size: zip.as_ref().and_then(|a| a.get("size")?.as_u64()),
        has_dmg: named(&format!("pdfcraft-{version}-macos-aarch64.dmg")).is_some(),
        url: text(answer, "html_url")?,
        version,
    })
}

/// Whether a finished run or check succeeded.
fn succeeded(conclusion: Option<&str>) -> bool {
    matches!(conclusion, Some("success" | "neutral" | "skipped"))
}

fn check_stage(c: &Check) -> Stage {
    let (state, detail) = match (c.status.as_str(), c.conclusion.as_deref()) {
        ("completed", Some(k @ ("skipped" | "neutral"))) => (State::Done, k.to_string()),
        ("completed", k) if succeeded(k) => (State::Done, String::new()),
        ("completed", k) => (State::Failed, k.unwrap_or("failed").to_string()),
        (s, _) => (State::Active, s.replace('_', " ")),
    };
    Stage::new(&c.name, state, detail, Some(c.url.clone()))
}

const ACTIONS: &str = "https://github.com/krichelj/pdfcraft/actions";

/// The stages GitHub's side of Merge and update has reached (1 to 6).
fn remote_stages(base: &str, r: &Remote) -> Vec<Stage> {
    let titles = ["Sync with upstream", "Open sync pull request", "Run checks", "Merge pull request", "Build release", "Publish release"];
    if let Some(p) = &r.release {
        // The release is out: whatever led to it is done (also when it was out before we looked).
        let mut done: Vec<Stage> = titles.iter().map(|t| Stage::new(t, State::Done, "", None)).collect();
        if let (Some(s), Some(run)) = (done.get_mut(0), &r.sync_run) {
            s.link = Some(run.url.clone());
        }
        if let (Some(s), Some(pr)) = (done.get_mut(1), &r.pr) {
            s.detail = format!("#{}", pr.number);
            s.link = Some(pr.url.clone());
        }
        if let (Some(s), Some(pr)) = (done.get_mut(2), &r.pr) {
            s.link = Some(format!("{}/checks", pr.url));
        }
        if let (Some(s), Some(pr)) = (done.get_mut(3), &r.pr) {
            s.link = Some(pr.url.clone());
        }
        if let (Some(s), Some(run)) = (done.get_mut(4), &r.release_run) {
            s.link = Some(run.url.clone());
        }
        if let Some(s) = done.get_mut(5) {
            let what = if p.has_dmg { "DMG and app" } else { "app" };
            s.detail = format!("PdfCraft {} ({what})", p.version);
            s.link = Some(p.url.clone());
        }
        return done;
    }
    let waiting = |detail: &str| r.error.clone().map_or_else(|| detail.to_string(), |e| format!("{detail}; last look failed: {e}"));
    let mut out = Vec::new();
    // 1. The Sync upstream run.
    let sync = match &r.sync_run {
        Some(run) if run.status == "completed" && !succeeded(run.conclusion.as_deref()) => {
            let k = run.conclusion.as_deref().unwrap_or("failed");
            Stage::new(titles[0], State::Failed, format!("the run ended: {k}"), Some(run.url.clone()))
        }
        Some(run) if run.status == "completed" => Stage::new(titles[0], State::Done, "", Some(run.url.clone())),
        Some(run) => Stage::new(titles[0], State::Done, format!("started, {}", run.status.replace('_', " ")), Some(run.url.clone())),
        None if r.pr.is_some() => Stage::new(titles[0], State::Done, "", None),
        None => Stage::new(titles[0], State::Active, waiting("waiting for the run to start"), Some(format!("{ACTIONS}/workflows/sync-upstream.yml"))),
    };
    let sync_failed = sync.state == State::Failed;
    let sync_done = sync.state == State::Done;
    let sync_finished = r.sync_run.as_ref().is_some_and(|run| run.status == "completed");
    out.push(sync);
    // 2. Its pull request.
    let pr_stage = match &r.pr {
        Some(pr) => Stage::new(titles[1], State::Done, format!("#{}", pr.number), Some(pr.url.clone())),
        None if sync_finished && !sync_failed => Stage::new(
            titles[1],
            State::Failed,
            format!("the sync finished without a pull request, and no release is built on PdfCraft {base} yet"),
            r.sync_run.as_ref().map(|run| run.url.clone()),
        ),
        None if sync_done => Stage::new(titles[1], State::Active, waiting("waiting for the workflow to open it"), None),
        None => Stage::new(titles[1], State::Pending, "", None),
    };
    out.push(pr_stage);
    // 3. Its checks.
    let children: Vec<Stage> = r.checks.iter().map(check_stage).collect();
    let merged = r.pr.as_ref().is_some_and(|p| p.merged_at.is_some());
    let mut checks = match &r.pr {
        None => Stage::new(titles[2], State::Pending, "", None),
        Some(pr) => {
            let link = Some(format!("{}/checks", pr.url));
            if let Some(bad) = children.iter().find(|c| c.state == State::Failed) {
                let n = children.iter().filter(|c| c.state == State::Failed).count();
                Stage::new(titles[2], State::Failed, format!("{n} of {} failed", children.len()), bad.link.clone())
            } else if merged || (!children.is_empty() && children.iter().all(|c| c.state == State::Done)) {
                Stage::new(titles[2], State::Done, format!("{} passed", children.len()), link)
            } else {
                let done = children.iter().filter(|c| c.state == State::Done).count();
                Stage::new(titles[2], State::Active, format!("{done} of {} done", children.len()), link)
            }
        }
    };
    let checks_done = checks.state == State::Done;
    let checks_failed = checks.state == State::Failed;
    checks.children = children;
    out.push(checks);
    // 4. The merge.
    out.push(match &r.pr {
        Some(pr) if pr.merged_at.is_some() => Stage::new(titles[3], State::Done, "merged into main", Some(pr.url.clone())),
        // Closed because a check failed: that is the failure, this only follows from it.
        Some(pr) if pr.state == "closed" && checks_failed => Stage::new(titles[3], State::Pending, "not merged", Some(pr.url.clone())),
        Some(pr) if pr.state == "closed" => {
            Stage::new(titles[3], State::Failed, format!("#{} was closed without merging", pr.number), Some(pr.url.clone()))
        }
        Some(pr) if checks_done => Stage::new(titles[3], State::Active, "waiting", Some(pr.url.clone())),
        _ => Stage::new(titles[3], State::Pending, "", None),
    });
    // 5. The Release run that the merge starts.
    out.push(match &r.release_run {
        Some(run) if run.status == "completed" && !succeeded(run.conclusion.as_deref()) => {
            let k = run.conclusion.as_deref().unwrap_or("failed");
            Stage::new(titles[4], State::Failed, format!("the build ended: {k}"), Some(run.url.clone()))
        }
        Some(run) if run.status == "completed" => Stage::new(titles[4], State::Done, "built", Some(run.url.clone())),
        Some(run) => Stage::new(titles[4], State::Active, format!("building ({})", run.status.replace('_', " ")), Some(run.url.clone())),
        None if merged => {
            Stage::new(titles[4], State::Active, waiting("waiting for the build to start"), Some(format!("{ACTIONS}/workflows/release.yml")))
        }
        None => Stage::new(titles[4], State::Pending, "", None),
    });
    // 6. The release page.
    let built = r.release_run.as_ref().is_some_and(|run| run.status == "completed" && succeeded(run.conclusion.as_deref()));
    out.push(if built {
        Stage::new(
            titles[5],
            State::Active,
            waiting(&format!("waiting for PdfCraft {base}-p… to be published")),
            Some(format!("https://github.com/{FORK_REPO}/releases")),
        )
    } else {
        Stage::new(titles[5], State::Pending, "", None)
    });
    out
}

fn megabytes(bytes: f64) -> String {
    format!("{:.1} MB", bytes / 1_000_000.0)
}

/// The installer's stages (7 to 10). `ready` is whether it has something to install yet.
fn local_stages(l: &Local, ready: bool, zip_size: Option<u64>, link: Option<&str>) -> Vec<Stage> {
    let steps = [(Step::Download, "Download"), (Step::Verify, "Verify download"), (Step::Install, "Install"), (Step::Relaunch, "Relaunch PdfCraft")];
    steps
        .iter()
        .map(|&(step, title)| {
            let mut s = if l.step > step {
                Stage::new(title, State::Done, "", None)
            } else if l.step == step || (l.step == Step::NotStarted && step == Step::Download && ready) {
                Stage::new(title, State::Active, "", None)
            } else {
                Stage::new(title, State::Pending, "", None)
            };
            if s.state == State::Active
                && let Some(e) = &l.error
            {
                s.state = State::Failed;
                s.detail = e.clone();
                s.link = l.log.clone().or_else(|| link.map(str::to_string));
            }
            if step == Step::Download && s.state == State::Active {
                let p = l.percent.unwrap_or(0.0);
                s.fraction = Some(p / 100.0);
                s.detail = match zip_size {
                    Some(size) => format!("{} of {} ({p:.0}%)", megabytes(f64::from(p) / 100.0 * size as f64), megabytes(size as f64)),
                    None => format!("{p:.0}%"),
                };
            }
            if step == Step::Relaunch && s.state == State::Done {
                s.detail = "PdfCraft reopened".into();
            }
            s
        })
        .collect()
}

/// Every stage, in order.
pub fn stages(mode: &Mode, remote: &Remote, local: &Local) -> Vec<Stage> {
    let mut all = all_stages(mode, remote, local);
    if let Some(why) = &remote.stopped
        && let Some(s) = all.iter_mut().find(|s| s.state != State::Done)
    {
        s.state = State::Failed;
        s.detail = why.clone();
        s.link = s.link.clone().or_else(|| Some(ACTIONS.to_string()));
    }
    all
}

fn all_stages(mode: &Mode, remote: &Remote, local: &Local) -> Vec<Stage> {
    match mode {
        Mode::Merge { base, .. } => {
            let mut all = remote_stages(base, remote);
            let r = remote.release.as_ref();
            all.extend(local_stages(local, r.is_some(), r.and_then(|p| p.zip_size), r.map(|p| p.url.as_str())));
            all
        }
        Mode::Install { .. } => local_stages(local, true, remote.release.as_ref().and_then(|p| p.zip_size), None),
    }
}

/// How far along the whole update is, 0 to 1: each stage counts one, the download by its share.
pub fn overall(stages: &[Stage]) -> f32 {
    if stages.is_empty() {
        return 0.0;
    }
    let done: f32 = stages.iter().map(|s| if s.state == State::Done { 1.0 } else { s.fraction.unwrap_or(0.0).clamp(0.0, 1.0) }).sum();
    (done / stages.len() as f32).clamp(0.0, 1.0)
}

/// The first failed stage, if any: the update has stopped there.
pub fn failure(stages: &[Stage]) -> Option<&Stage> {
    stages.iter().find(|s| s.state == State::Failed)
}

/// Whether every stage is done.
pub fn finished(stages: &[Stage]) -> bool {
    !stages.is_empty() && stages.iter().all(|s| s.state == State::Done)
}

#[cfg(test)]
mod tests {
    //! Every answer here is a real one from GitHub's API for krichelj/pdfcraft, recorded on
    //! 2026-11-12 with `gh api` and trimmed to the fields read (tests/fixtures).
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name);
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    /// The pull requests answer without the ones numbered in `drop` (to replay an earlier day).
    fn pulls_without(drop: &[u64]) -> Value {
        let mut v = fixture("pulls.json");
        v.as_array_mut().unwrap().retain(|p| !drop.contains(&p.get("number").and_then(Value::as_u64).unwrap()));
        v
    }

    fn states(list: &[Stage]) -> Vec<(&str, State)> {
        list.iter().map(|s| (s.title.as_str(), s.state.clone())).collect()
    }

    fn merge(base: &str, since: &str) -> Mode {
        Mode::Merge { base: base.into(), since: since.into() }
    }

    /// Merge and update for 0.21.1, clicked at 21:15 on 2026-11-12: the workflow ran, reused the
    /// open pull request #134 whose checks have all finished, and it waits to be merged.
    #[test]
    fn the_reused_sync_pr_with_green_checks_waits_for_its_merge() {
        let since = "2026-11-12T21:13:00Z";
        let r = Remote {
            sync_run: newest_run(&fixture("sync-runs.json"), since),
            pr: sync_pr(&fixture("pulls.json"), since),
            checks: checks(&fixture("checks-134.json")),
            release: published(&fixture("release-latest.json"), "0.21.1"),
            ..Remote::default()
        };
        assert_eq!(r.sync_run.as_ref().unwrap().created_at, "2026-11-12T21:15:15Z");
        assert_eq!(r.pr.as_ref().unwrap().number, 134, "the open sync PR, though older than the click");
        assert_eq!(r.release, None, "v0.21.0-p1 is not built on 0.21.1");
        let list = stages(&merge("0.21.1", since), &r, &Local::default());
        use State::{Active, Done, Pending};
        assert_eq!(
            states(&list),
            [
                ("Sync with upstream", Done),
                ("Open sync pull request", Done),
                ("Run checks", Done),
                ("Merge pull request", Active),
                ("Build release", Pending),
                ("Publish release", Pending),
                ("Download", Pending),
                ("Verify download", Pending),
                ("Install", Pending),
                ("Relaunch PdfCraft", Pending),
            ]
        );
        let checks = &list[2];
        assert_eq!(checks.children.len(), 5);
        assert!(checks.children.iter().any(|c| c.title == "test (shared code)"
            && c.state == Done
            && c.link.as_deref().is_some_and(|l| l.starts_with("https://github.com/krichelj/pdfcraft/"))));
        assert_eq!(list[1].link.as_deref(), Some("https://github.com/krichelj/pdfcraft/pull/134"));
        assert!((overall(&list) - 0.3).abs() < 1e-6, "{}", overall(&list));
    }

    /// Sync PR #126 (2026-11-10): `test (shared code)` failed, so the update stops there and
    /// links to the failed check.
    #[test]
    fn a_failed_check_stops_the_update_with_its_name_and_link() {
        let since = "2026-11-01T00:00:00Z";
        let pr = sync_pr(&pulls_without(&[134, 130]), since).unwrap();
        assert_eq!(pr.number, 126);
        let r = Remote { pr: Some(pr), checks: checks(&fixture("checks-126.json")), ..Remote::default() };
        let list = stages(&merge("0.21.0", since), &r, &Local::default());
        let failed = failure(&list).unwrap();
        assert_eq!(failed.title, "Run checks");
        assert_eq!(failed.detail, format!("1 of {} failed", failed.children.len()));
        assert_eq!(
            failed.children.iter().find(|c| c.state == State::Failed).map(|c| (c.title.as_str(), c.detail.as_str())),
            Some(("test (shared code)", "failure"))
        );
        assert!(failed.link.as_deref().unwrap().contains("/runs/"), "{failed:?}");
        // #126 was closed without being merged because of it: not a second failure.
        assert_eq!((list[3].state.clone(), list[3].detail.as_str()), (State::Pending, "not merged"));
        // Closed with every check green (#126 against #134's checks): that is the failure.
        let r = Remote { checks: checks(&fixture("checks-134.json")), ..r };
        let list = stages(&merge("0.21.0", since), &r, &Local::default());
        assert_eq!(failure(&list).map(|s| s.detail.as_str()), Some("#126 was closed without merging"));
    }

    /// Merge and update for 0.21.0, replayed from 2026-11-11 21:16: #130 merged, the Release run
    /// it started built, and v0.21.0-p1 is out, so the download starts.
    #[test]
    fn a_merged_pr_and_its_release_lead_to_the_download() {
        let since = "2026-11-11T21:16:00Z";
        let pr = sync_pr(&pulls_without(&[134]), since).unwrap();
        assert_eq!((pr.number, pr.merged_at.as_deref()), (130, Some("2026-11-11T21:24:30Z")));
        let release_run = newest_run(&fixture("release-runs-130.json"), pr.merged_at.as_deref().unwrap());
        assert_eq!(release_run.as_ref().unwrap().conclusion.as_deref(), Some("success"));
        let mut r = Remote {
            sync_run: newest_run(&fixture("sync-runs.json"), since),
            pr: Some(pr),
            checks: checks(&fixture("checks-130.json")),
            release_run,
            release: None,
            ..Remote::default()
        };
        // Built but not yet on the release page.
        let list = stages(&merge("0.21.0", since), &r, &Local::default());
        assert_eq!(list[4].state, State::Done);
        assert_eq!(list[5].state, State::Active);
        r.release = published(&fixture("release-latest.json"), "0.21.0");
        let p = r.release.clone().unwrap();
        assert_eq!((p.version.as_str(), p.has_dmg), ("0.21.0-p1", true));
        let zip = p.zip_size.unwrap();
        let mut l = Local::default();
        let list = stages(&merge("0.21.0", since), &r, &l);
        assert!(list[..6].iter().all(|s| s.state == State::Done), "{list:#?}");
        assert_eq!(list[6].state, State::Active, "the download is next");
        l.feed("==> PdfCraft 0.21.0-p1: downloading pdfcraft-0.21.0-p1-macos-aarch64.zip");
        l.feed("##########                   50.0%");
        let list = stages(&merge("0.21.0", since), &r, &l);
        let half = format!("{:.1} MB of {:.1} MB (50%)", zip as f64 / 2e6, zip as f64 / 1e6);
        assert_eq!(list[6].detail, half);
        assert_eq!(list[6].fraction, Some(0.5));
        assert!((overall(&list) - 0.65).abs() < 1e-6, "{}", overall(&list));
        l.feed("error: the signature of PdfCraft.app in a.zip doesn't verify");
        let list = stages(&merge("0.21.0", since), &r, &l);
        assert_eq!(failure(&list).map(|s| s.title.as_str()), Some("Download"));
        l.step = Step::Finished;
        l.error = None;
        assert!(finished(&stages(&merge("0.21.0", since), &r, &l)));
    }

    #[test]
    fn before_the_run_starts_and_when_gh_is_missing() {
        let since = "2099-01-01T00:00:00Z";
        let r = Remote { sync_run: newest_run(&fixture("sync-runs.json"), since), pr: sync_pr(&pulls_without(&[134]), since), ..Remote::default() };
        assert_eq!((r.sync_run.is_none(), r.pr.is_none()), (true, true), "nothing that old counts");
        let list = stages(&merge("0.21.1", since), &r, &Local::default());
        assert_eq!(list[0].state, State::Active);
        assert!(list[1..].iter().all(|s| s.state == State::Pending));
        let stopped = Remote { stopped: Some("the GitHub CLI (gh) isn't installed".into()), ..r };
        let list = stages(&merge("0.21.1", since), &stopped, &Local::default());
        assert_eq!(
            failure(&list).map(|s| (s.title.as_str(), s.detail.as_str())),
            Some(("Sync with upstream", "the GitHub CLI (gh) isn't installed"))
        );
        assert!(list[0].link.is_some(), "a failure links somewhere");
    }

    #[test]
    fn install_and_relaunch_has_only_the_local_stages() {
        let mut l = Local::default();
        let list = stages(&Mode::Install { version: "0.21.0-p1".into() }, &Remote::default(), &l);
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].state, State::Active);
        l.feed("==> installing PdfCraft 0.21.0-p1 in /Applications");
        let list = stages(&Mode::Install { version: "0.21.0-p1".into() }, &Remote::default(), &l);
        assert_eq!(states(&list).iter().map(|(_, s)| s.clone()).collect::<Vec<_>>(), [State::Done, State::Done, State::Active, State::Pending]);
    }

    #[test]
    fn malformed_answers_give_nothing_rather_than_a_crash() {
        for junk in [
            Value::Null,
            serde_json::json!({}),
            serde_json::json!([1, "x", null]),
            serde_json::json!({"workflow_runs": 3, "check_runs": [{"id": "x"}]}),
        ] {
            assert_eq!(newest_run(&junk, ""), None);
            assert_eq!(sync_pr(&junk, ""), None);
            assert!(checks(&junk).is_empty());
            assert_eq!(published(&junk, "0.1.0"), None);
        }
        assert_eq!(published(&serde_json::json!({"tag_name": "v9.9.9", "html_url": "u"}), "not.a.version"), None);
        assert_eq!(core("v0.21.1-p3"), Some((0, 21, 1)));
        assert_eq!(core("0.21"), None);
        assert_eq!(core("0.21.1.4"), None);
        assert_eq!(overall(&[]), 0.0);
    }
}
