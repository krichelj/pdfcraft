//! Help ▸ Check for updates (issue #28): ask for the latest releases, and install the fork's or
//! merge the original PdfCraft's into the fork.
//!
//! The desktop app supplies how to ask ([`PdfCraftApp::update_source`]), how to start the fork's
//! upstream merge ([`PdfCraftApp::sync_trigger`]) and, when it runs from an installed macOS app
//! bundle, how to install ([`PdfCraftApp::update_installer`]), so this crate has no network or
//! process code. Without a source (the web build, tests) the command opens the releases page;
//! without an installer the dialog offers the release page's download.
//!
//! Only the fork's builds are installed: the original PdfCraft's (storytold/pdfcraft) would replace
//! the fork's changes. When the original is ahead of the fork, the dialog offers Merge and update
//! instead: it starts the fork's Sync upstream workflow, which merges the new version into the
//! fork, tests it and releases `<version>-p1`, and starts the installer waiting for that release.
//! The fork also checks once at start, quietly: the dialog opens only when there is something to
//! offer (`check_for_updates_quietly`; the owner's decision for krichelj/pdfcraft).

use std::sync::Arc;

use egui::{Align, Layout};

use crate::{PdfCraftApp, theme, widgets};

/// Where every PdfCraft release is listed: this fork's (krichelj/pdfcraft), whose macOS Apple
/// Silicon builds are the ones this app updates to (CLAUDE.md, fork rules).
pub const RELEASES_PAGE: &str = "https://github.com/krichelj/pdfcraft/releases";

/// Where the original PdfCraft (storytold/pdfcraft) lists its releases. A newer one there is
/// merged into the fork (Merge and update), never installed over it.
pub const UPSTREAM_RELEASES_PAGE: &str = "https://github.com/storytold/pdfcraft/releases";

/// The fork's Sync upstream workflow, where its Run workflow button is (when the GitHub CLI can't
/// start it from here).
pub const SYNC_WORKFLOW_PAGE: &str = "https://github.com/krichelj/pdfcraft/actions/workflows/sync-upstream.yml";

/// A published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Its version tag, such as `v0.2.0`.
    pub version: String,
    /// Its page on [`RELEASES_PAGE`] or [`UPSTREAM_RELEASES_PAGE`], where the downloads are.
    pub url: String,
}

/// What a check found: the fork's latest release and the original PdfCraft's, each when its
/// repository answered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Latest {
    pub fork: Option<Release>,
    pub upstream: Option<Release>,
}

/// What the Updates dialog offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Offer {
    UpToDate,
    /// The fork has a newer release: install it.
    Install(Release),
    /// The original PdfCraft has a version the fork isn't built on yet: merge it into the fork.
    Merge(Release),
}

impl Latest {
    /// What to offer an app at version `current`. The original PdfCraft is ahead when its version
    /// is newer than everything the fork has (the running app and the fork's latest release):
    /// `0.12.0` is ahead of `0.11.0-p2`, `0.11.0` is not (the fork's `-p<N>` builds on it). Then
    /// merging comes first, since the release it produces carries the fork's newer changes too.
    pub fn offer(&self, current: &str) -> Offer {
        let fork_update = self.fork.as_ref().filter(|f| is_newer(&f.version, current));
        let fork_has = fork_update.map_or(current, |f| f.version.as_str());
        match (&self.upstream, fork_update) {
            (Some(up), _) if is_newer(&up.version, fork_has) => Offer::Merge(up.clone()),
            (_, Some(fork)) => Offer::Install(fork.clone()),
            _ => Offer::UpToDate,
        }
    }
}

/// Asks for the latest releases (blocking; it runs on its own thread).
pub type UpdateSource = Arc<dyn Fn() -> Result<Latest, String> + Send + Sync>;

/// What the installer is asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    /// Install and relaunch: the fork's release with this tag (`v0.8.0-p1`). The installer
    /// downloads and checks it, waits for this app to quit, puts it in place and opens it.
    Release(String),
    /// Merge and update: wait, in the background, for the fork's release built on the original
    /// PdfCraft's version with this tag (`v0.12.0`), then ask whether to install it. The app keeps
    /// running meanwhile.
    AfterMerge(String),
}

/// Starts the installer and returns once it has started. After [`Install::Release`] the app quits
/// right away, asking about unsaved work.
pub type UpdateInstaller = Arc<dyn Fn(&Install) -> Result<(), String> + Send + Sync>;

/// Starts the fork's Sync upstream workflow (blocking; it runs on its own thread). Its error says
/// why it couldn't (no GitHub CLI, not signed in, no network).
pub type SyncTrigger = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// Whether release `latest` (a tag such as `v0.2.0`) is newer than version `current` (`0.1.1`).
/// Pre-release and build suffixes are ignored, except the fork's patchlevel `-p<N>`: `0.8.0-p2`
/// is newer than `0.8.0-p1`, which is newer than `0.8.0`. A version that doesn't parse is never
/// newer.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

fn parse(v: &str) -> Option<(u64, u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let v = v.split('+').next()?;
    let (core, suffix) = v.split_once('-').unwrap_or((v, ""));
    let patchlevel = match suffix.strip_prefix('p') {
        Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => n.parse::<u64>().ok()?,
        _ => 0,
    };
    let mut parts = core.split('.');
    let mut next = |required: bool| match parts.next() {
        Some(p) => p.parse::<u64>().ok(),
        None if required => None,
        None => Some(0),
    };
    let version = (next(true)?, next(false)?, next(false)?, patchlevel);
    parts.next().is_none().then_some(version)
}

/// Where a check is.
#[derive(Default)]
pub(crate) enum Check {
    #[default]
    Idle,
    #[cfg(not(target_arch = "wasm32"))]
    Running(std::sync::mpsc::Receiver<Result<Latest, String>>),
    Done(Result<Latest, String>),
}

/// Where Merge and update is.
#[derive(Default)]
pub(crate) enum Merge {
    #[default]
    Idle,
    /// The Sync upstream workflow is being started for this upstream tag.
    #[cfg(not(target_arch = "wasm32"))]
    Starting(String, std::sync::mpsc::Receiver<Result<(), String>>),
    /// Asked for this upstream tag; the error says why the workflow couldn't be started from here.
    Started { upstream: String, trigger_error: Option<String> },
}

#[derive(Default)]
pub(crate) struct Updates {
    pub(crate) check: Check,
    pub(crate) merge: Merge,
    /// The Updates dialog is showing.
    pub(crate) open: bool,
    /// The running check was started at launch: show the dialog only if it finds something.
    pub(crate) quiet: bool,
    /// Why the last start of the installer failed.
    pub(crate) install_error: Option<String>,
}

impl PdfCraftApp {
    /// Help ▸ Check for updates: ask for the latest releases and show the outcome.
    pub fn check_for_updates(&mut self) {
        self.start_update_check(false);
    }

    /// At launch: ask for the latest releases, and open the Updates dialog only if there is an
    /// update to install or a version to merge. Errors and "up to date" stay silent. Does
    /// nothing without a source.
    pub fn check_for_updates_quietly(&mut self) {
        if self.update_source.is_some() {
            self.start_update_check(true);
        }
    }

    fn start_update_check(&mut self, quiet: bool) {
        let Some(source) = self.update_source.clone() else {
            self.open_url(RELEASES_PAGE);
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !quiet {
                self.updates.open = true;
            }
            if matches!(self.updates.check, Check::Running(_)) {
                // A quiet check already running shows its outcome once someone asked.
                self.updates.quiet &= quiet;
                return;
            }
            self.updates.quiet = quiet;
            self.updates.install_error = None;
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                // The receiver may be gone (the app quit): nothing to report to then.
                let _ = tx.send(source());
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
            self.updates.check = Check::Running(rx);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = source;
            if !quiet {
                self.open_url(RELEASES_PAGE);
            }
        }
    }

    /// Pick up a finished check or merge start (each frame).
    pub(crate) fn poll_updates(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Check::Running(rx) = &self.updates.check {
                let result = match rx.try_recv() {
                    Ok(r) => Some(r),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("the update check stopped unexpectedly".into())),
                };
                if let Some(result) = result {
                    let offers = result.as_ref().is_ok_and(|l| l.offer(env!("CARGO_PKG_VERSION")) != Offer::UpToDate);
                    if std::mem::take(&mut self.updates.quiet) && offers {
                        self.updates.open = true;
                    }
                    self.updates.check = Check::Done(result);
                }
            }
            if let Merge::Starting(upstream, rx) = &self.updates.merge {
                let result = match rx.try_recv() {
                    Ok(r) => r,
                    Err(std::sync::mpsc::TryRecvError::Empty) => return,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("starting the merge stopped unexpectedly".into()),
                };
                let upstream = upstream.clone();
                self.merge_started(upstream, result.err());
            }
        }
    }

    /// Install and relaunch: start the installer for the fork's release `version`, then quit
    /// (asking about unsaved work, like the close button). If the installer can't start, the
    /// dialog says why.
    fn install_update(&mut self, version: &str) {
        let Some(install) = self.update_installer.clone() else { return };
        match install(&Install::Release(version.to_string())) {
            Ok(()) => {
                self.updates.open = false;
                self.updates.install_error = None;
                if let Some(ctx) = &self.ctx {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Err(e) => self.updates.install_error = Some(e),
        }
    }

    /// Merge and update: start the fork's Sync upstream workflow for the original PdfCraft's
    /// `upstream` tag (in the background), then the installer that waits for its release.
    fn merge_update(&mut self, upstream: &str) {
        self.updates.install_error = None;
        let Some(trigger) = self.sync_trigger.clone() else {
            self.merge_started(upstream.to_string(), Some("this build can't start the workflow itself".into()));
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(trigger());
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
            self.updates.merge = Merge::Starting(upstream.to_string(), rx);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = trigger;
            self.merge_started(upstream.to_string(), Some("a browser can't start the workflow".into()));
        }
    }

    /// The workflow was asked for (or couldn't be): start the waiting installer either way, since
    /// the daily sync produces the same release when nobody starts it by hand.
    fn merge_started(&mut self, upstream: String, trigger_error: Option<String>) {
        if let Some(install) = self.update_installer.clone()
            && let Err(e) = install(&Install::AfterMerge(upstream.clone()))
        {
            self.updates.install_error = Some(e);
        }
        self.updates.merge = Merge::Started { upstream, trigger_error };
    }
}

/// The Updates dialog.
pub(crate) fn dialog(app: &mut PdfCraftApp, ctx: &egui::Context) {
    if !app.updates.open {
        return;
    }
    let t = theme::Tokens::get(ctx);
    let current = env!("CARGO_PKG_VERSION");
    let mut close = false;
    let mut open: Option<String> = None;
    let mut download: Option<String> = None;
    // The fork's newer release's tag, when this app can install it itself.
    let mut installable: Option<String> = None;
    let mut install: Option<String> = None;
    // The original PdfCraft's tag the dialog offers to merge into the fork.
    let mut mergeable: Option<String> = None;
    let mut merge: Option<String> = None;
    // The workflow couldn't be started from here: offer its page instead.
    let mut workflow_page = false;
    // Merge and update was chosen: the note about what happens before choosing no longer applies.
    let mut merging = false;
    let can_install = app.update_installer.is_some();
    let modal = crate::widgets::show_modal(ctx, "updates", |ui| {
        ui.set_width(420.0);
        ui.horizontal(|ui| {
            ui.add(crate::icons::image("cloud", 22.0, t.accent));
            ui.label(egui::RichText::new(tl!("Check for updates")).font(theme::semibold(16.0)));
        });
        ui.add_space(8.0);
        match &app.updates.check {
            Check::Idle => {
                ui.label(tl!("No check has run yet."));
            }
            #[cfg(not(target_arch = "wasm32"))]
            Check::Running(_) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tl!("Checking for a newer version…"));
                });
            }
            Check::Done(Ok(latest)) => match latest.offer(current) {
                Offer::Install(r) => {
                    let version = r.version.trim_start_matches(['v', 'V']);
                    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("PdfCraft {v} is available."), &[("v", version)])).strong());
                    let how = if can_install {
                        tl!("You have version {c}. Install and relaunch downloads the new version, checks it, and reopens PdfCraft.")
                    } else {
                        tl!("You have version {c}. Download the new version from its release page.")
                    };
                    ui.label(egui::RichText::new(crate::i18n::fmt(how, &[("c", current)])).color(t.text_muted));
                    download = Some(r.url.clone());
                    if can_install {
                        installable = Some(r.version.clone());
                    }
                }
                Offer::Merge(r) => {
                    let version = r.version.trim_start_matches(['v', 'V']);
                    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("The original PdfCraft {v} is available."), &[("v", version)])).strong());
                    match &app.updates.merge {
                        #[cfg(not(target_arch = "wasm32"))]
                        Merge::Starting(..) => {
                            merging = true;
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(tl!("Starting the merge…"));
                            });
                        }
                        Merge::Started { upstream, trigger_error } if *upstream == r.version => {
                            merging = true;
                            if let Some(e) = trigger_error {
                                ui.label(crate::i18n::fmt(tl!("Couldn't start the merge: {e}"), &[("e", e)]));
                                workflow_page = true;
                            } else {
                                ui.label(
                                    egui::RichText::new(crate::i18n::fmt(
                                        tl!("Merging PdfCraft {v} into your fork. When the fork's release is out, PdfCraft offers the update; you can keep working."),
                                        &[("v", version)],
                                    ))
                                    .color(t.text_muted),
                                );
                            }
                        }
                        _ => {
                            ui.label(
                                egui::RichText::new(crate::i18n::fmt(
                                    tl!("You have version {c}. Merge and update merges it into your fork on your build machines and runs the tests; the fork's new release follows."),
                                    &[("c", current)],
                                ))
                                .color(t.text_muted),
                            );
                            mergeable = Some(r.version.clone());
                        }
                    }
                }
                Offer::UpToDate => {
                    ui.label(crate::i18n::fmt(tl!("PdfCraft {c} is up to date."), &[("c", current)]));
                }
            },
            Check::Done(Err(e)) => {
                ui.label(crate::i18n::fmt(tl!("Couldn't check for updates: {e}"), &[("e", &e.to_string())]));
                ui.label(
                    egui::RichText::new(crate::i18n::fmt(
                        tl!("You have version {c}. All releases are listed at {page}."),
                        &[("c", current), ("page", RELEASES_PAGE)],
                    ))
                    .color(t.text_muted),
                );
            }
        }
        if let Some(e) = &app.updates.install_error {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Couldn't start the update: {e}"), &[("e", e)])).color(t.text));
        }
        if !merging {
            ui.add_space(10.0);
            let note = if mergeable.is_some() {
                tl!("Asks GitHub for the latest releases. Nothing is merged or installed until you choose Merge and update.")
            } else if installable.is_some() {
                tl!("Asks GitHub for the latest release. Nothing is downloaded or installed until you choose Install and relaunch.")
            } else {
                tl!("Asks GitHub for the latest release. Nothing is downloaded or installed automatically.")
            };
            ui.label(egui::RichText::new(note).color(t.text_muted).small());
        }
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(upstream) = mergeable.take() {
                let go = widgets::pill_button(ui, tl!("Merge and update"), true).clicked();
                close = widgets::pill_button(ui, tl!("Later"), false).clicked();
                merge = go.then_some(upstream);
            } else if let Some(release) = installable.take() {
                let go = widgets::pill_button(ui, tl!("Install and relaunch"), true).clicked();
                close = widgets::pill_button(ui, tl!("Later"), false).clicked();
                download = None;
                install = go.then_some(release);
            } else if let Some(url) = download.take() {
                let get = widgets::pill_button(ui, tl!("Download"), true).clicked();
                let later = widgets::pill_button(ui, tl!("Later"), false).clicked();
                close = get || later;
                open = get.then_some(url);
            } else if workflow_page {
                let go = widgets::pill_button(ui, tl!("Open the workflow page"), true).clicked();
                close = widgets::pill_button(ui, tl!("Close"), false).clicked() || go;
                open = go.then(|| SYNC_WORKFLOW_PAGE.to_string());
            } else if widgets::pill_button(ui, tl!("Close"), true).clicked() {
                close = true;
            }
        });
    });
    if modal.should_close() {
        close = true;
        open = None;
        install = None;
        merge = None;
    }
    if let Some(version) = install {
        app.install_update(&version);
        return;
    }
    if let Some(upstream) = merge {
        app.merge_update(&upstream);
        return;
    }
    if close {
        app.updates.open = false;
        if let Some(url) = open {
            app.open_url(&url);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fork(v: &str) -> Release {
        Release { version: v.into(), url: format!("{RELEASES_PAGE}/tag/{v}") }
    }
    fn up(v: &str) -> Release {
        Release { version: v.into(), url: format!("{UPSTREAM_RELEASES_PAGE}/tag/{v}") }
    }
    fn latest(f: Option<&str>, u: Option<&str>) -> Latest {
        Latest { fork: f.map(fork), upstream: u.map(up) }
    }

    #[test]
    fn a_newer_fork_release_is_installed() {
        assert_eq!(latest(Some("v0.11.0-p3"), Some("v0.11.0")).offer("0.11.0-p2"), Offer::Install(fork("v0.11.0-p3")));
        assert_eq!(latest(Some("v0.11.0-p1"), Some("v0.10.0")).offer("0.10.0-p4"), Offer::Install(fork("v0.11.0-p1")));
        assert_eq!(latest(Some("v0.11.0-p3"), None).offer("0.11.0-p2"), Offer::Install(fork("v0.11.0-p3")), "upstream unreachable");
    }

    #[test]
    fn the_original_pdfcraft_ahead_of_the_fork_is_merged_never_installed() {
        // The case that lost the fork's patches: upstream 0.11.0 while the fork was on 0.9.0-p3.
        assert_eq!(latest(Some("v0.9.0-p3"), Some("v0.11.0")).offer("0.9.0-p3"), Offer::Merge(up("v0.11.0")));
        // Even when the fork has a newer release too: the merge's release carries it.
        assert_eq!(latest(Some("v0.11.0-p3"), Some("v0.12.0")).offer("0.11.0-p2"), Offer::Merge(up("v0.12.0")));
        // Numbers, not text: 0.10.0 is ahead of 0.9.9.
        assert_eq!(latest(Some("v0.9.9-p7"), Some("v0.10.0")).offer("0.9.9-p7"), Offer::Merge(up("v0.10.0")));
        // The fork unreachable: compared with the running app.
        assert_eq!(latest(None, Some("v0.12.0")).offer("0.11.0-p2"), Offer::Merge(up("v0.12.0")));
    }

    #[test]
    fn the_fork_built_on_the_same_version_is_not_behind() {
        // The fork's -p<N> builds on the original's X.Y.Z.
        assert_eq!(latest(Some("v0.11.0-p2"), Some("v0.11.0")).offer("0.11.0-p2"), Offer::UpToDate);
        assert_eq!(latest(Some("v0.11.0-p1"), Some("v0.11.0")).offer("0.11.0-p2"), Offer::UpToDate, "an older fork release");
        assert_eq!(latest(Some("v0.11.0-p2"), Some("v0.11.0")).offer("0.11.0-p1"), Offer::Install(fork("v0.11.0-p2")));
        // An app already on a newer base than the fork's latest (a local build) is not behind either.
        assert_eq!(latest(Some("v0.11.0-p2"), Some("v0.12.0")).offer("0.12.0-p1"), Offer::UpToDate);
    }

    #[test]
    fn a_tag_that_is_not_a_version_offers_nothing() {
        assert_eq!(latest(Some("v0.11.0-p2"), Some("nightly")).offer("0.11.0-p2"), Offer::UpToDate);
        assert_eq!(latest(Some("latest"), Some("nightly")).offer("0.11.0-p2"), Offer::UpToDate);
        assert_eq!(latest(None, None).offer("0.11.0-p2"), Offer::UpToDate);
    }
}
