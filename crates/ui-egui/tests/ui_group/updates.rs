//! Help ▸ Check for updates (issue #28), with stand-in release sources (no network).

use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::updates::{Release, UpdateInstaller, UpdateSource, is_newer};

fn source(answer: Result<&str, &str>) -> UpdateSource {
    let answer = answer.map(str::to_string).map_err(str::to_string);
    Arc::new(move || {
        answer.clone().map(|v| Release { url: format!("https://github.com/krichelj/pdfcraft/releases/tag/{v}"), version: v, from_fork: true })
    })
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
            Ok(Release { version: "v99.0.0".into(), url: "https://github.com/krichelj/pdfcraft/releases/tag/v99.0.0".into(), from_fork: true })
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

/// An installer stand-in: records the versions it was asked for and answers `answer`.
fn installer(answer: Result<(), &str>) -> (UpdateInstaller, Arc<std::sync::Mutex<Vec<String>>>) {
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = asked.clone();
    let answer = answer.map_err(str::to_string);
    let install: UpdateInstaller = Arc::new(move |v: &str| {
        log.lock().unwrap().push(v.to_string());
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
    assert_eq!(*asked.lock().unwrap(), ["v99.0.0"], "the installer gets the release tag");
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
                let r = Ok(Release {
                    url: format!("https://github.com/krichelj/pdfcraft/releases/tag/{latest}"),
                    version: latest.clone(),
                    from_fork: true,
                });
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

#[test]
fn a_newer_upstream_release_is_offered_for_download_not_installed() {
    let (install, asked) = installer(Ok(()));
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.update_source = Some(Arc::new(|| {
            Ok(Release { version: "v99.0.0".into(), url: "https://github.com/storytold/pdfcraft/releases/tag/v99.0.0".into(), from_fork: false })
        }));
        app.update_installer = Some(install);
        app
    });
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("PdfCraft 99.0.0 is available");
    assert!(h.query_by_label("Install and relaunch").is_none(), "installing upstream's build would replace the fork's");
    h.get_by_label("Download");
    assert!(asked.lock().unwrap().is_empty());
}
