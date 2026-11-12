//! PdfCraft Updater (see the library's documentation). Started by PdfCraft with the arguments in
//! [`pdfcraft_updater::args::Args`].

use std::sync::{Arc, Mutex, PoisonError};

use pdfcraft_updater::args::Args;
use pdfcraft_updater::stages;
use pdfcraft_updater::view::{self, Action};
use pdfcraft_updater::watch::{self, Model};

struct App {
    args: Args,
    model: Arc<Mutex<Model>>,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let list = {
            let m = self.model.lock().unwrap_or_else(PoisonError::into_inner);
            stages::stages(&self.args.mode, &m.remote, &m.local)
        };
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show(ui, |ui| match view::show(ui, &self.args.mode, &list) {
            Some(Action::Open(url)) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            Some(Action::Minimize) => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            Some(Action::Close) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            None => {}
        });
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
        viewport: egui::ViewportBuilder::default().with_title("PdfCraft Updater").with_inner_size([560.0, 620.0]).with_min_inner_size([420.0, 360.0]),
        ..Default::default()
    };
    let (worker_args, worker_model) = (args.clone(), Arc::clone(&model));
    let result = eframe::run_native(
        "PdfCraft Updater",
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || watch::run(&worker_args, &worker_model, &move || ctx.request_repaint()));
            Ok(Box::new(App { args, model }))
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
