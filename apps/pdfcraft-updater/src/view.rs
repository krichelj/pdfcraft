//! The updater's window: a heading, the overall progress bar and every stage.

use egui::{Color32, RichText};

use crate::stages::{self, Mode, Stage, State};

/// What the user asked for in this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Open(String),
    Minimize,
    Close,
}

fn color(state: &State, dark: bool) -> Color32 {
    match (state, dark) {
        (State::Done, _) => Color32::from_rgb(52, 168, 83),
        (State::Failed, _) => Color32::from_rgb(220, 60, 50),
        (State::Active, _) => Color32::from_rgb(66, 133, 244),
        (State::Pending, true) => Color32::from_gray(90),
        (State::Pending, false) => Color32::from_gray(190),
    }
}

/// A stage's mark: a spinner while it runs, else a dot in its state's colour.
fn mark(ui: &mut egui::Ui, state: &State, size: f32) {
    if *state == State::Active {
        ui.add(egui::Spinner::new().size(size));
        return;
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let c = color(state, ui.visuals().dark_mode);
    let p = ui.painter();
    match state {
        State::Done => {
            p.circle_filled(rect.center(), size * 0.45, c);
            let s = size * 0.22;
            let k = rect.center();
            let stroke = egui::Stroke::new(1.8, Color32::WHITE);
            p.line_segment([k + egui::vec2(-s, 0.0), k + egui::vec2(-s * 0.3, s * 0.7)], stroke);
            p.line_segment([k + egui::vec2(-s * 0.3, s * 0.7), k + egui::vec2(s, -s * 0.6)], stroke);
        }
        State::Failed => {
            p.circle_filled(rect.center(), size * 0.45, c);
            let s = size * 0.2;
            let k = rect.center();
            let stroke = egui::Stroke::new(1.8, Color32::WHITE);
            p.line_segment([k + egui::vec2(-s, -s), k + egui::vec2(s, s)], stroke);
            p.line_segment([k + egui::vec2(-s, s), k + egui::vec2(s, -s)], stroke);
        }
        _ => {
            p.circle_stroke(rect.center(), size * 0.4, egui::Stroke::new(1.5, c));
        }
    }
}

fn row(ui: &mut egui::Ui, n: Option<usize>, s: &Stage, action: &mut Option<Action>) {
    let weak = ui.visuals().weak_text_color();
    ui.horizontal_wrapped(|ui| {
        mark(ui, &s.state, if n.is_some() { 18.0 } else { 14.0 });
        let title = match n {
            Some(n) => format!("{n}. {}", s.title),
            None => s.title.clone(),
        };
        let title = RichText::new(title);
        let title = match s.state {
            State::Pending => title.color(weak),
            State::Failed => title.color(color(&State::Failed, true)).strong(),
            _ => title.strong(),
        };
        ui.label(title);
        if !s.detail.is_empty() {
            let detail = RichText::new(&s.detail).color(if s.state == State::Failed { color(&State::Failed, true) } else { weak });
            ui.label(detail);
        }
        if let Some(link) = &s.link
            && s.state != State::Pending
            && ui.link(if s.state == State::Failed { "Show why" } else { "View" }).clicked()
        {
            *action = Some(Action::Open(link.clone()));
        }
    });
    if let Some(f) = s.fraction {
        ui.indent(("bar", &s.title), |ui| {
            ui.add(egui::ProgressBar::new(f.clamp(0.0, 1.0)).desired_height(6.0));
        });
    }
}

/// Draw the window's contents and return what the user asked for.
pub fn show(ui: &mut egui::Ui, mode: &Mode, list: &[Stage]) -> Option<Action> {
    let mut action = None;
    let heading = match mode {
        Mode::Merge { base, .. } => format!("Merging PdfCraft {base} into your fork and updating"),
        Mode::Install { version } => format!("Installing PdfCraft {version}"),
    };
    ui.add_space(6.0);
    ui.heading(heading);
    ui.add_space(4.0);
    let overall = stages::overall(list);
    let failed = stages::failure(list);
    let done = stages::finished(list);
    let status = if let Some(f) = failed {
        format!("Stopped at {}", f.title)
    } else if done {
        "PdfCraft is up to date".to_string()
    } else {
        list.iter().find(|s| s.state == State::Active).map_or_else(|| "Starting".to_string(), |s| s.title.clone())
    };
    ui.label(RichText::new(status).color(ui.visuals().weak_text_color()));
    let bar = egui::ProgressBar::new(overall).show_percentage();
    let bar = if failed.is_some() { bar.fill(color(&State::Failed, true)) } else { bar };
    ui.add(bar);
    ui.add_space(8.0);
    egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(ui.available_height() - 44.0).show(ui, |ui| {
        for (i, s) in list.iter().enumerate() {
            row(ui, Some(i + 1), s, &mut action);
            if !s.children.is_empty() {
                ui.indent(("checks", i), |ui| {
                    for c in &s.children {
                        row(ui, None, c, &mut action);
                    }
                });
            }
            ui.add_space(2.0);
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        let text = if done || failed.is_some() { "Close" } else { "Minimize" };
        if ui.button(text).clicked() {
            action = Some(if done || failed.is_some() { Action::Close } else { Action::Minimize });
        }
        ui.label(RichText::new("Closing PdfCraft doesn't stop the update.").small().color(ui.visuals().weak_text_color()));
    });
    action
}
