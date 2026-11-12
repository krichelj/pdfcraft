//! Headless screenshots of the updater window at several stages, drawn from GitHub's recorded
//! answers in tests/fixtures (egui_kittest + wgpu, no window needed):
//!
//! ```text
//! cargo run -p pdfcraft-updater --example shot -- OUT_DIR
//! ```

use egui_kittest::Harness;
use pdfcraft_updater::stages::{self, Local, Mode, Remote, Step};
use serde_json::Value;

fn fixture(name: &str) -> Result<Value, String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn pulls_without(drop: &[u64]) -> Result<Value, String> {
    let mut v = fixture("pulls.json")?;
    if let Some(a) = v.as_array_mut() {
        a.retain(|p| !p.get("number").and_then(Value::as_u64).is_some_and(|n| drop.contains(&n)));
    }
    Ok(v)
}

fn shot(out: &std::path::Path, name: &str, mode: &Mode, remote: &Remote, local: &Local) -> Result<(), String> {
    let list = stages::stages(mode, remote, local);
    let mode = mode.clone();
    let mut harness = Harness::builder().with_size(egui::vec2(560.0, 640.0)).with_pixels_per_point(2.0).build_ui(move |ui| {
        ui.ctx().set_visuals(egui::Visuals::dark());
        egui::Frame::central_panel(ui.style()).inner_margin(12.0).show(ui, |ui| {
            // The window's size less the frame's margins, as eframe's central panel gives it.
            ui.set_width(536.0);
            ui.set_height(616.0);
            let _ = pdfcraft_updater::view::show(ui, &mode, &list);
        });
    });
    harness.run_steps(4);
    let image = harness.render()?;
    let path = out.join(format!("{name}.png"));
    image.save(&path).map_err(|e| e.to_string())?;
    eprintln!("shot: wrote {}", path.display());
    Ok(())
}

fn main() -> Result<(), String> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).ok_or("usage: shot OUT_DIR")?);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;

    // 1. Merge and update for 0.21.1 at 21:15 on 2026-11-12: #134's checks passed, waiting for the merge.
    let since = "2026-11-12T21:13:00Z";
    let merge = Mode::Merge { base: "0.21.1".into(), since: since.into() };
    let waiting = Remote {
        sync_run: stages::newest_run(&fixture("sync-runs.json")?, since),
        pr: stages::sync_pr(&fixture("pulls.json")?, since),
        checks: stages::checks(&fixture("checks-134.json")?),
        release: stages::published(&fixture("release-latest.json")?, "0.21.1"),
        ..Remote::default()
    };
    shot(&out, "1-waiting-for-merge", &merge, &waiting, &Local::default())?;

    // 2. Sync PR #126: a failed check stops the update.
    let early = "2026-11-01T00:00:00Z";
    let failed = Remote {
        pr: stages::sync_pr(&pulls_without(&[134, 130])?, early),
        checks: stages::checks(&fixture("checks-126.json")?),
        ..Remote::default()
    };
    shot(&out, "2-check-failed", &Mode::Merge { base: "0.21.0".into(), since: early.into() }, &failed, &Local::default())?;

    // 3. The 0.21.0 merge replayed: #130 merged, v0.21.0-p1 published, the download half done.
    let since = "2026-11-11T21:16:00Z";
    let pr = stages::sync_pr(&pulls_without(&[134])?, since);
    let merged_at = pr.as_ref().and_then(|p| p.merged_at.clone()).unwrap_or_default();
    let released = Remote {
        sync_run: stages::newest_run(&fixture("sync-runs.json")?, since),
        pr,
        checks: stages::checks(&fixture("checks-130.json")?),
        release_run: stages::newest_run(&fixture("release-runs-130.json")?, &merged_at),
        release: stages::published(&fixture("release-latest.json")?, "0.21.0"),
        ..Remote::default()
    };
    let replay = Mode::Merge { base: "0.21.0".into(), since: since.into() };
    let mut local = Local::default();
    local.feed("==> PdfCraft 0.21.0-p1: downloading pdfcraft-0.21.0-p1-macos-aarch64.zip");
    local.feed("##################################                                   48.6%");
    shot(&out, "3-downloading", &replay, &released, &local)?;

    // 4. Installed and relaunched.
    local.step = Step::Finished;
    shot(&out, "4-done", &replay, &released, &local)
}
