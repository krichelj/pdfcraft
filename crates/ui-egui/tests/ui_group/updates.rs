//! Help ▸ Check for updates (issue #28), with stand-in release sources (no network).

use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::updates::{Install, Latest, Release, SYNC_WORKFLOW_PAGE, SyncTrigger, UpdateInstaller, UpdateSource, is_newer};

fn fork(v: &str) -> Release {
    Release { url: format!("https://github.com/krichelj/pdfcraft/releases/tag/{v}"), version: v.into() }
}

fn upstream(v: &str) -> Release {
    Release { url: format!("https://github.com/storytold/pdfcraft/releases/tag/{v}"), version: v.into() }
}

/// A source whose fork has release `answer` and whose original PdfCraft is not ahead.
fn source(answer: Result<&str, &str>) -> UpdateSource {
    let answer = answer.map(str::to_string).map_err(str::to_string);
    Arc::new(move || answer.clone().map(|v| Latest { fork: Some(fork(&v)), upstream: None }))
}

fn harness(answer: Result<&str, &str>) -> Harness<'static, PdfCraftApp> {
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.update_source = Some(source(answer));
        app
    })
}

/// Run frames until the background check has reported. By what the dialog shows, with the
/// same 30-second deadline as `support::wait_until`, not for a fixed second: under load the
/// check's thread can take longer (#1277).
fn settle(h: &mut Harness<'static, PdfCraftApp>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        h.run_steps(2);
        if h.query_by_label_contains("Checking for a newer version").is_none() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "timed out waiting for the update check");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.run_steps(2);
}

#[test]
fn versions_compare_by_number() {
    assert!(is_newer("v0.2.0", "0.1.1"));
    assert!(is_newer("v0.1.10", "0.1.9"));
    assert!(is_newer("1", "0.9.9"));
    assert!(!is_newer("v0.1.1", "0.1.1"));
    assert!(!is_newer("v0.1.0", "0.1.1"));
    assert!(!is_newer("v0.1.1-beta.2", "0.1.1"), "suffixes are ignored");
    assert!(!is_newer("nightly", "0.1.1"), "a tag that isn't a version is never newer");
    assert!(!is_newer("v1.2.3.4", "0.1.1"));
    assert!(!is_newer("v99999999999999999999.0.0", "0.1.1"), "out of range");
    // The fork's patchlevel `-p<N>` counts; other suffixes still don't.
    assert!(is_newer("v0.8.0-p2", "0.8.0-p1"));
    assert!(is_newer("v0.8.0-p10", "0.8.0-p9"));
    assert!(is_newer("v0.8.0-p1", "0.8.0"));
    assert!(is_newer("v0.9.0", "0.8.0-p7"));
    assert!(!is_newer("v0.8.0-p1", "0.8.0-p1"));
    assert!(!is_newer("v0.8.0", "0.8.0-p1"));
    assert!(!is_newer("v0.8.0-preview", "0.8.0"), "not a patchlevel");
    assert!(!is_newer("v0.8.0-p99999999999999999999", "0.8.0"), "patchlevel out of range");
}

#[test]
fn a_newer_release_is_offered_for_download() {
    let mut h = harness(Ok("v99.0.0"));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("PdfCraft 99.0.0 is available");
    h.get_by_label("Download");
    h.get_by_label("Later").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("is available").is_none(), "Later closes the dialog");
}

#[test]
fn an_up_to_date_or_failed_check_says_so() {
    let mut h = harness(Ok(concat!("v", env!("CARGO_PKG_VERSION"))));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("is up to date");
    assert!(h.query_by_label("Download").is_none());

    let mut h = harness(Err("couldn't reach GitHub"));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("Couldn't check for updates: couldn't reach GitHub");
}

#[test]
fn nothing_is_asked_until_the_user_checks() {
    // The app model never checks by itself: the desktop app asks once at launch, through
    // `check_for_updates_quietly`, and otherwise only the command asks.
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        // Settings from a build that had the startup option are ignored.
        app.restore(r#"{"check_updates_at_start": true}"#);
        let counted = counted.clone();
        app.update_source = Some(Arc::new(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Latest { fork: Some(fork("v99.0.0")), upstream: None })
        }));
        app
    });
    settle(&mut h);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(h.query_by_label_contains("is available").is_none());
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    h.get_by_label_contains("PdfCraft 99.0.0 is available");
}

/// An installer stand-in: records what it was asked to do and answers `answer`.
fn installer(answer: Result<(), &str>) -> (UpdateInstaller, Arc<std::sync::Mutex<Vec<Install>>>) {
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = asked.clone();
    let answer = answer.map_err(str::to_string);
    let install: UpdateInstaller = Arc::new(move |what: &Install| {
        log.lock().unwrap().push(what.clone());
        answer.clone()
    });
    (install, asked)
}

fn harness_with(answer: Result<&str, &str>, install: Option<UpdateInstaller>) -> Harness<'static, PdfCraftApp> {
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.update_source = Some(source(answer));
        app.update_installer = install;
        app
    })
}

fn closing(h: &Harness<'static, PdfCraftApp>) -> bool {
    h.output().viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::Close)))
}

#[test]
fn a_newer_release_is_installed_and_the_app_quits_to_relaunch() {
    let (install, asked) = installer(Ok(()));
    let mut h = harness_with(Ok("v99.0.0"), Some(install));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("PdfCraft 99.0.0 is available");
    assert!(h.query_by_label("Download").is_none(), "an installable update isn't sent to the browser");
    h.get_by_label_contains("Install and relaunch downloads the new version, checks it, and reopens PdfCraft");
    h.get_by_label_contains("Nothing is downloaded or installed until you choose Install and relaunch");
    assert!(h.query_by_label_contains("from its release page").is_none(), "no stale download advice");
    assert!(h.query_by_label_contains("installed automatically").is_none());
    h.get_by_label("Install and relaunch").click();
    h.step();
    assert_eq!(*asked.lock().unwrap(), [Install::Release("v99.0.0".into())], "the installer gets the release tag");
    assert!(closing(&h), "then the app quits, asking about unsaved work like the close button");
    h.run_steps(2);
    assert!(h.query_by_label_contains("is available").is_none(), "and the dialog is gone");
}

#[test]
fn an_installer_that_cannot_start_says_why_and_the_app_stays() {
    let (install, asked) = installer(Err("couldn't open the log"));
    let mut h = harness_with(Ok("v99.0.0"), Some(install));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label("Install and relaunch").click();
    h.step();
    assert_eq!(asked.lock().unwrap().len(), 1);
    assert!(!closing(&h), "nothing to relaunch into: the app keeps running");
    h.run_steps(2);
    h.get_by_label_contains("Couldn't start the update: couldn't open the log");
    h.get_by_label("Later").click();
    h.run_steps(2);
    assert!(h.query_by_label_contains("is available").is_none());
}

#[test]
fn no_second_installer_is_offered_while_one_runs() {
    // Merge and update's installer is waiting for its release (it holds the installers' lock):
    // the fork's release appearing must not start Install and relaunch beside it.
    let (install, asked) = installer(Ok(()));
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.update_source = Some(source(Ok("v99.0.0")));
        app.update_installer = Some(install);
        app.installer_busy = Some(Arc::new(|| true));
        app
    });
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("PdfCraft 99.0.0 is available");
    h.get_by_label_contains("Another update is already in progress or waiting for its release");
    assert!(h.query_by_label("Install and relaunch").is_none(), "no second installer");
    assert!(asked.lock().unwrap().is_empty());
    // Once nothing runs, the next check offers the install again.
    h.state_mut().installer_busy = Some(Arc::new(|| false));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label("Install and relaunch");
}

#[test]
fn an_up_to_date_app_offers_nothing_to_install() {
    let (install, asked) = installer(Ok(()));
    let mut h = harness_with(Ok(concat!("v", env!("CARGO_PKG_VERSION"))), Some(install));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("is up to date");
    assert!(h.query_by_label("Install and relaunch").is_none());
    assert!(asked.lock().unwrap().is_empty());
}

/// Run frames until `source` has answered `calls` times, then a few more so the answer is read.
fn until_asked(h: &mut Harness<'static, PdfCraftApp>, calls: &std::sync::atomic::AtomicUsize, n: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while calls.load(std::sync::atomic::Ordering::SeqCst) < n {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for the update check");
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.run_steps(4);
}

#[test]
fn the_launch_check_is_silent_unless_a_newer_release_exists() {
    for (latest, shown) in [(concat!("v", env!("CARGO_PKG_VERSION")), false), ("v0.0.1", false), ("v99.0.0", true)] {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        let latest = latest.to_string();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            app.update_source = Some(Arc::new(move || {
                let r = Ok(Latest { fork: Some(fork(&latest)), upstream: None });
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                r
            }));
            app.check_for_updates_quietly();
            app
        });
        until_asked(&mut h, &calls, 1);
        assert_eq!(h.query_by_label_contains("is available").is_some(), shown, "newer: {shown}");
        assert!(h.query_by_label_contains("is up to date").is_none(), "an up-to-date app says nothing at launch");
    }
    // A failed launch check says nothing either.
    let mut h = harness(Err("couldn't reach GitHub"));
    h.state_mut().check_for_updates_quietly();
    h.run_steps(20);
    std::thread::sleep(std::time::Duration::from_millis(50));
    h.run_steps(4);
    assert!(h.query_by_label_contains("Couldn't check for updates").is_none());
}

/// A trigger stand-in: counts the times it was asked and answers `answer`.
fn trigger(answer: Result<(), &str>) -> (SyncTrigger, Arc<std::sync::atomic::AtomicUsize>) {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let answer = answer.map_err(str::to_string);
    let t: SyncTrigger = Arc::new(move || {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        answer.clone()
    });
    (t, calls)
}

/// The original PdfCraft at v99.0.0 while the fork's latest is this app's own version.
fn upstream_ahead(install: Option<UpdateInstaller>, sync: Option<SyncTrigger>) -> Harness<'static, PdfCraftApp> {
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.update_source =
            Some(Arc::new(|| Ok(Latest { fork: Some(fork(concat!("v", env!("CARGO_PKG_VERSION")))), upstream: Some(upstream("v99.0.0")) })));
        app.update_installer = install;
        app.sync_trigger = sync;
        app
    })
}

/// Run frames until the dialog shows `label`.
fn until_shown(h: &mut Harness<'static, PdfCraftApp>, label: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while h.query_by_label_contains(label).is_none() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {label:?}");
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn a_newer_original_pdfcraft_is_merged_into_the_fork_never_installed() {
    let (install, asked) = installer(Ok(()));
    let (sync, calls) = trigger(Ok(()));
    let mut h = upstream_ahead(Some(install), Some(sync));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("The original PdfCraft 99.0.0 is available");
    h.get_by_label_contains("Merge and update merges it into your fork");
    h.get_by_label_contains("Nothing is merged or installed until you choose Merge and update");
    // The original's build is never offered: installing it would drop the fork's changes.
    assert!(h.query_by_label("Install and relaunch").is_none());
    assert!(h.query_by_label("Download").is_none());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "nothing starts before the click");
    let shots = std::env::var("PDFCRAFT_UPDATE_SHOTS").ok();
    if let Some(dir) = &shots {
        h.render().unwrap().save(std::path::Path::new(dir).join("merge-offered.png")).unwrap();
    }
    h.get_by_label("Merge and update").click();
    until_shown(&mut h, "Merging PdfCraft 99.0.0 into your fork");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1, "the Sync upstream workflow is started once");
    assert_eq!(*asked.lock().unwrap(), [Install::AfterMerge("v99.0.0".into())], "then the installer waits for its release");
    h.run_steps(3);
    if let Some(dir) = &shots {
        h.render().unwrap().save(std::path::Path::new(dir).join("merge-started.png")).unwrap();
    }
    assert!(!closing(&h), "the app keeps running while the fork's release is made");
    assert!(h.query_by_label("Merge and update").is_none(), "not offered twice");
    assert!(h.query_by_label_contains("Nothing is").is_none(), "no note about what happens before choosing");
    h.get_by_label("Close").click();
    h.run_steps(2);
    assert!(h.query_by_label_contains("The original PdfCraft").is_none());
}

#[test]
fn a_merge_that_cannot_start_here_offers_the_workflow_page_and_still_waits() {
    let (install, asked) = installer(Ok(()));
    let (sync, calls) = trigger(Err("the GitHub CLI (gh) isn't installed"));
    let mut h = upstream_ahead(Some(install), Some(sync));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label("Merge and update").click();
    until_shown(&mut h, "Couldn't start the merge: the GitHub CLI (gh) isn't installed");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    // The daily sync makes the same release: the installer waits for it anyway.
    assert_eq!(*asked.lock().unwrap(), [Install::AfterMerge("v99.0.0".into())]);
    assert!(!closing(&h));
    h.get_by_label("Open the workflow page").click();
    h.run_steps(2);
    assert_eq!(h.state().last_opened_url.as_deref(), Some(SYNC_WORKFLOW_PAGE));
    assert!(h.query_by_label_contains("Couldn't start the merge").is_none(), "and the dialog closes");
}

#[test]
fn a_waiting_installer_that_cannot_start_says_why() {
    let (install, _) = installer(Err("couldn't open the log"));
    let (sync, _) = trigger(Ok(()));
    let mut h = upstream_ahead(Some(install), Some(sync));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label("Merge and update").click();
    until_shown(&mut h, "Merging PdfCraft 99.0.0 into your fork");
    h.get_by_label_contains("Couldn't start the update: couldn't open the log");
}

#[test]
fn the_launch_check_opens_the_dialog_when_the_original_is_ahead() {
    let (sync, calls) = trigger(Ok(()));
    let mut h = upstream_ahead(None, Some(sync));
    h.state_mut().check_for_updates_quietly();
    until_shown(&mut h, "The original PdfCraft 99.0.0 is available");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "asking is not merging");
}
