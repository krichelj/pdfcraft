//! Help ▸ Check for updates (issue #28): ask for the latest release, and install it or offer its
//! download page.
//!
//! The desktop app supplies how to ask ([`PdfCraftApp::update_source`]) and, when it runs from an
//! installed macOS app bundle, how to install ([`PdfCraftApp::update_installer`]), so this crate
//! has no network or process code. Without a source (the web build, tests) the command opens the
//! releases page; without an installer the dialog offers the release page's download.
//! The fork also checks once at start, quietly: the dialog opens only when a newer release exists
//! (`check_for_updates_quietly`; the owner's decision for krichelj/pdfcraft).

use std::sync::Arc;

use egui::{Align, Layout};

use crate::{PdfCraftApp, theme, widgets};

/// Where every PdfCraft release is listed: this fork's (krichelj/pdfcraft), whose macOS Apple
/// Silicon builds are the ones this app updates to (CLAUDE.md, fork rules).
pub const RELEASES_PAGE: &str = "https://github.com/krichelj/pdfcraft/releases";

/// The latest published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Its version tag, such as `v0.2.0`.
    pub version: String,
    /// Its page on [`RELEASES_PAGE`], where the downloads are.
    pub url: String,
}

/// Asks for the latest release (blocking; it runs on its own thread).
pub type UpdateSource = Arc<dyn Fn() -> Result<Release, String> + Send + Sync>;

/// Starts installing release `version` (a tag such as `v0.8.0-p1`) and returns once it has
/// started: the installer downloads and checks the release, waits for this app to quit, puts the
/// new version in its place and opens it. The app quits right after, asking about unsaved work.
pub type UpdateInstaller = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

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
    Running(std::sync::mpsc::Receiver<Result<Release, String>>),
    Done(Result<Release, String>),
}

#[derive(Default)]
pub(crate) struct Updates {
    pub(crate) check: Check,
    /// The Updates dialog is showing.
    pub(crate) open: bool,
    /// The running check was started at launch: show the dialog only if it finds a newer release.
    pub(crate) quiet: bool,
    /// Why the last Install and relaunch couldn't start.
    pub(crate) install_error: Option<String>,
}

impl PdfCraftApp {
    /// Help ▸ Check for updates: ask for the latest release and show the outcome.
    pub fn check_for_updates(&mut self) {
        self.start_update_check(false);
    }

    /// At launch: ask for the latest release, and open the Updates dialog only if it is newer.
    /// Errors and "up to date" stay silent. Does nothing without a source.
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

    /// Pick up a finished check (each frame).
    pub(crate) fn poll_updates(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Check::Running(rx) = &self.updates.check {
            let result = match rx.try_recv() {
                Ok(r) => r,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the update check stopped unexpectedly".into()),
            };
            if std::mem::take(&mut self.updates.quiet) && result.as_ref().is_ok_and(|r| is_newer(&r.version, env!("CARGO_PKG_VERSION"))) {
                self.updates.open = true;
            }
            self.updates.check = Check::Done(result);
        }
    }

    /// Install and relaunch: start the installer for `version`, then quit (asking about unsaved
    /// work, like the close button). If the installer can't start, the dialog says why.
    fn install_update(&mut self, version: &str) {
        let Some(install) = self.update_installer.clone() else { return };
        match install(version) {
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
}

/// The Updates dialog.
pub(crate) fn dialog(app: &mut PdfCraftApp, ctx: &egui::Context) {
    if !app.updates.open {
        return;
    }
    let t = theme::Tokens::get(ctx);
    let current = env!("CARGO_PKG_VERSION");
    let mut close = false;
    let mut download: Option<String> = None;
    // The newer release's tag, when this app can install it itself.
    let mut installable: Option<String> = None;
    let mut install: Option<String> = None;
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
            Check::Done(Ok(r)) if is_newer(&r.version, current) => {
                let version = r.version.trim_start_matches(['v', 'V']);
                ui.label(egui::RichText::new(crate::i18n::fmt(tl!("PdfCraft {v} is available."), &[("v", version)])).strong());
                ui.label(
                    egui::RichText::new(crate::i18n::fmt(
                        tl!("You have version {c}. Download the new version from its release page."),
                        &[("c", current)],
                    ))
                    .color(t.text_muted),
                );
                download = Some(r.url.clone());
                if can_install {
                    installable = Some(r.version.clone());
                }
                if let Some(e) = &app.updates.install_error {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Couldn't start the update: {e}"), &[("e", e)])).color(t.text));
                }
            }
            Check::Done(Ok(_)) => {
                ui.label(crate::i18n::fmt(tl!("PdfCraft {c} is up to date."), &[("c", current)]));
            }
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
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(tl!("Asks GitHub for the latest release. Nothing is downloaded or installed automatically."))
                .color(t.text_muted)
                .small(),
        );
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(version) = installable.take() {
                let go = widgets::pill_button(ui, tl!("Install and relaunch"), true).clicked();
                close = widgets::pill_button(ui, tl!("Later"), false).clicked();
                download = None;
                install = go.then_some(version);
            } else if let Some(url) = download.take() {
                let get = widgets::pill_button(ui, tl!("Download"), true).clicked();
                let later = widgets::pill_button(ui, tl!("Later"), false).clicked();
                close = get || later;
                download = get.then_some(url);
            } else if widgets::pill_button(ui, tl!("Close"), true).clicked() {
                close = true;
            }
        });
    });
    if modal.should_close() {
        close = true;
        download = None;
        install = None;
    }
    if let Some(version) = install {
        app.install_update(&version);
        return;
    }
    if close {
        app.updates.open = false;
        if let Some(url) = download {
            app.open_url(&url);
        }
    }
}
