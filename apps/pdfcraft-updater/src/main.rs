//! PdfCraft Updater (see the library's documentation). Started by PdfCraft with the arguments in
//! [`pdfcraft_updater::args::Args`].

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pdfcraft_updater::args::Args;
use pdfcraft_updater::stages;
use pdfcraft_updater::view::{self, Action};
use pdfcraft_updater::watch::{self, Model};

struct App {
    args: Args,
    model: Arc<Mutex<Model>>,
    icon: Option<egui::TextureHandle>,
    details: bool,
    /// When the update finished, for the auto-close; and when the download was first seen.
    finished_at: Option<Instant>,
    download_from: Option<Instant>,
    height: f32,
}

/// A finished window closes on its own after this long, unless Details is open.
const AUTO_CLOSE: u64 = 5;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let (list, percent, release) = {
            let m = self.model.lock().unwrap_or_else(PoisonError::into_inner);
            (stages::stages(&self.args.mode, &m.remote, &m.local), m.local.percent, m.remote.release.as_ref().map(|r| r.version.clone()))
        };
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        let failed = stages::failure(&list).is_some();
        if failed && !self.details && self.finished_at.is_none() {
            // A failure opens Details once, so the failed stage is in view.
            self.details = true;
            self.finished_at = Some(now);
        }
        let done = stages::finished(&list);
        if done && self.finished_at.is_none() {
            self.finished_at = Some(now);
        }
        if percent.is_some() && self.download_from.is_none() {
            self.download_from = Some(now);
        }
        let eta = self.download_from.zip(percent).and_then(|(t, p)| view::eta(now.duration_since(t).as_secs_f32(), p));
        let closes_in =
            (done && !self.details).then(|| self.finished_at.map(|t| AUTO_CLOSE.saturating_sub(now.duration_since(t).as_secs()))).flatten();
        if closes_in == Some(0) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if closes_in.is_some() || eta.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        let mut look = view::Look { icon: self.icon.as_ref(), details: self.details, closes_in, eta, release };
        let mut height = 0.0;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(ui.visuals().panel_fill)).show(ui, |ui| {
            let action = view::show(ui, &self.args.mode, &list, &mut look);
            height = ui.min_rect().height();
            match action {
                Some(Action::Open(url)) => ctx.open_url(egui::OpenUrl::new_tab(url)),
                Some(Action::Minimize) => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                Some(Action::Close) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                None => {}
            }
        });
        self.details = look.details;
        // The window is as tall as what it shows (Details open or closed).
        if height.is_finite() && height > 0.0 && (height - self.height).abs() > 1.0 {
            self.height = height;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(view::WIDTH, height)));
        }
    }
}

/// Hold the updater's lock (a symbolic link to this process) while it runs, so PdfCraft knows
/// an update is under way. A lock left by an updater that died is taken over.
fn take_lock() -> Option<std::path::PathBuf> {
    let lock = std::env::temp_dir().join(pdfcraft_updater::UPDATER_LOCK);
    #[cfg(unix)]
    {
        if let Ok(old) = std::fs::read_link(&lock)
            && old.to_string_lossy().parse::<u32>().is_ok_and(|pid| !pdfcraft_updater::gh::alive(pid))
        {
            let _ = std::fs::remove_file(&lock);
        }
        std::os::unix::fs::symlink(std::process::id().to_string(), &lock).ok().map(|()| lock)
    }
    #[cfg(not(unix))]
    {
        let _ = lock;
        None
    }
}

fn main() -> std::process::ExitCode {
    let args = match Args::parse(std::env::args_os().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("pdfcraft-updater: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    let Some(lock) = take_lock() else {
        eprintln!("pdfcraft-updater: another update is already under way");
        return std::process::ExitCode::SUCCESS;
    };
    let model = Arc::new(Mutex::new(Model::default()));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PdfCraft Updater")
            .with_inner_size([view::WIDTH, 200.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_icon(eframe::icon_data::from_png_bytes(view::ICON_PNG).unwrap_or_default()),
        ..Default::default()
    };
    let (worker_args, worker_model) = (args.clone(), Arc::clone(&model));
    let result = eframe::run_native(
        "PdfCraft Updater",
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            view::style(&ctx);
            let icon = view::icon(&ctx);
            std::thread::spawn(move || watch::run(&worker_args, &worker_model, &move || ctx.request_repaint()));
            Ok(Box::new(App { args, model, icon, details: false, finished_at: None, download_from: None, height: 0.0 }))
        }),
    );
    let _ = std::fs::remove_file(lock);
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pdfcraft-updater: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
