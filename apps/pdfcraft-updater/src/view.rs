//! The updater's window, laid out like a macOS updater (Sparkle, Software Update): the app icon
//! and a one-line headline, the step under way with one progress bar, a Details disclosure with
//! every stage, and the buttons on the right with the default one last.

use egui::{Align, Color32, CornerRadius, Layout, RichText, Stroke, vec2};

use crate::stages::{self, Mode, Stage, State};

/// The window's width; its height follows the content.
pub const WIDTH: f32 = 480.0;
const MARGIN: f32 = 20.0;
const ICON: f32 = 64.0;

/// PdfCraft's icon (assets/app-icon), shown in the header and as the window's icon.
pub const ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.pdfcraft.png");

/// The icon as a texture, or nothing if it doesn't decode.
pub fn icon(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let d = eframe::icon_data::from_png_bytes(ICON_PNG).ok()?;
    let size = [usize::try_from(d.width).ok()?, usize::try_from(d.height).ok()?];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &d.rgba);
    Some(ctx.load_texture("pdfcraft-icon", image, egui::TextureOptions::LINEAR))
}

/// What the user asked for in this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Open(String),
    Minimize,
    Close,
}

/// What the window shows besides the stages.
#[derive(Default)]
pub struct Look<'a> {
    pub icon: Option<&'a egui::TextureHandle>,
    /// Whether Details is open.
    pub details: bool,
    /// Seconds until a finished window closes on its own.
    pub closes_in: Option<u64>,
    /// The download's time left ("about 20 seconds left").
    pub eta: Option<String>,
    /// The fork release being installed, once known (`0.21.1-p3`).
    pub release: Option<String>,
}

/// The type sizes and spacing (macOS: 13 pt body, 11 pt secondary).
pub fn style(ctx: &egui::Context) {
    use egui::{FontFamily::Proportional, FontId, TextStyle};
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Heading, FontId::new(17.0, Proportional)),
            (TextStyle::Body, FontId::new(13.0, Proportional)),
            (TextStyle::Button, FontId::new(13.0, Proportional)),
            (TextStyle::Small, FontId::new(11.0, Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, egui::FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(14.0, 4.0);
        s.spacing.interact_size.y = 24.0;
        // Secondary text at macOS's secondary-label contrast (above 4.5:1 on the panel).
        let dark = s.visuals.dark_mode;
        s.visuals.weak_text_color = Some(if dark { Color32::from_gray(152) } else { Color32::from_gray(110) });
        // Primary text near white or black, so it stands apart from the secondary grey.
        s.visuals.widgets.noninteractive.fg_stroke.color = if dark { Color32::from_gray(232) } else { Color32::from_gray(28) };
    });
}

fn accent(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(10, 132, 255) } else { Color32::from_rgb(0, 122, 255) }
}
fn green(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(48, 209, 88) } else { Color32::from_rgb(40, 167, 69) }
}
fn red(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(255, 105, 97) } else { Color32::from_rgb(215, 0, 21) }
}

/// What a stage is doing, said as the step under way.
pub fn doing(title: &str) -> &str {
    match title {
        "Sync with upstream" => "Syncing with the original PdfCraft",
        "Open sync pull request" => "Opening the sync pull request",
        "Run checks" => "Running the checks",
        "Merge pull request" => "Waiting for the pull request to merge",
        "Build release" => "Building the release",
        "Publish release" => "Publishing the release",
        "Download" => "Downloading",
        "Verify download" => "Checking the signature and version",
        "Install" => "Installing",
        "Relaunch PdfCraft" => "Restarting PdfCraft",
        other => other,
    }
}

/// The time a download has left, from how long it took so far, once that says something.
pub fn eta(elapsed_secs: f32, percent: f32) -> Option<String> {
    if !(elapsed_secs.is_finite() && percent.is_finite()) || elapsed_secs < 2.0 || !(2.0..100.0).contains(&percent) {
        return None;
    }
    let left = (elapsed_secs * (100.0 - percent) / percent).round();
    Some(if left < 5.0 {
        "a few seconds left".into()
    } else if left < 60.0 {
        format!("about {} seconds left", ((left / 5.0).round() * 5.0).max(5.0) as u32)
    } else {
        let m = (left / 60.0).round().max(1.0) as u32;
        if m == 1 { "about a minute left".into() } else { format!("about {m} minutes left") }
    })
}

/// A small state mark: a spinner while it runs, a filled check or cross, an empty ring.
fn mark(ui: &mut egui::Ui, state: &State, skipped: bool, size: f32) {
    let dark = ui.visuals().dark_mode;
    if skipped {
        let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
        let (c, k) = (rect.center(), size * 0.3);
        ui.painter().line_segment([c - vec2(k, 0.0), c + vec2(k, 0.0)], Stroke::new(1.5, ui.visuals().weak_text_color()));
        return;
    }
    if *state == State::Active {
        ui.add_sized([size, size], egui::Spinner::new().size(size).color(accent(dark)));
        return;
    }
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    let (p, c, k) = (ui.painter(), rect.center(), size * 0.5);
    let white = Stroke::new(size / 9.0, Color32::WHITE);
    match state {
        State::Done => {
            p.circle_filled(c, k, green(dark));
            p.line_segment([c + vec2(-k * 0.45, 0.0), c + vec2(-k * 0.1, k * 0.35)], white);
            p.line_segment([c + vec2(-k * 0.1, k * 0.35), c + vec2(k * 0.45, -k * 0.35)], white);
        }
        State::Failed => {
            p.circle_filled(c, k, red(dark));
            let s = k * 0.35;
            p.line_segment([c + vec2(-s, -s), c + vec2(s, s)], white);
            p.line_segment([c + vec2(-s, s), c + vec2(s, -s)], white);
        }
        _ => {
            p.circle_stroke(c, k - 1.0, Stroke::new(1.2, ui.visuals().weak_text_color().gamma_multiply(0.6)));
        }
    }
}

/// One line of Details.
fn row(ui: &mut egui::Ui, s: &Stage, size: f32, action: &mut Option<Action>) {
    ui.spacing_mut().item_spacing.y = 4.0;
    let weak = ui.visuals().weak_text_color();
    let dark = ui.visuals().dark_mode;
    // Skipped, or not reached because an earlier stage failed: a dash, not a ring or a tick.
    let skipped = matches!((&s.state, s.detail.as_str()), (State::Done, "skipped" | "neutral") | (State::Pending, "not merged"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        mark(ui, &s.state, skipped, size);
        let title = RichText::new(sentence(&s.title));
        let title = match s.state {
            _ if skipped => title.color(weak),
            State::Pending => title.color(weak),
            State::Failed => title.color(red(dark)),
            _ => title,
        };
        ui.label(title);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(link) = &s.link
                && s.state != State::Pending
                && ui.link(RichText::new("View").small()).on_hover_text(link).clicked()
            {
                *action = Some(Action::Open(link.clone()));
            }
            // The download's bytes are in the line under the bar already.
            // The mark says passed or failed; the detail is for anything more.
            if !s.detail.is_empty() && s.detail != "failure" && !(s.state == State::Active && s.fraction.is_some()) {
                let c = if s.state == State::Failed { red(dark) } else { weak };
                ui.add(egui::Label::new(RichText::new(&s.detail).small().color(c)).truncate()).on_hover_text(&s.detail);
            }
        });
    });
}

/// A stage's detail as a sentence: capitalised.
fn lower(text: &str) -> String {
    let mut c = text.chars();
    c.next().map(|f| f.to_lowercase().chain(c).collect()).unwrap_or_default()
}

fn sentence(detail: &str) -> String {
    let mut c = detail.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// What went wrong, said for someone who didn't watch the stages.
fn reason(f: &Stage, index: usize) -> String {
    let failed_check = f.children.iter().find(|c| c.state == State::Failed);
    match (failed_check, f.detail.is_empty()) {
        (Some(c), _) if c.detail == "failure" => format!("The check \u{201c}{}\u{201d} failed, so no release was built.", sentence(&c.title)),
        (Some(c), _) => format!("The check \u{201c}{}\u{201d} ended as {}, so no release was built.", sentence(&c.title), c.detail.replace('_', " ")),
        (None, true) => format!("Step {} stopped without saying why.", index + 1),
        (None, false) => sentence(&f.detail),
    }
}

/// The overall bar: a rounded track with the accent fill, visible in both themes.
fn bar(ui: &mut egui::Ui, fraction: f32) {
    let dark = ui.visuals().dark_mode;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), egui::Sense::hover());
    let track = if dark { Color32::from_gray(64) } else { Color32::from_gray(222) };
    ui.painter().rect_filled(rect, 3.0, track);
    let f = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
    if f > 0.0 {
        let mut fill = rect;
        fill.set_width((rect.width() * f).max(6.0));
        ui.painter().rect_filled(fill, 3.0, accent(dark));
    }
}

/// The primary (default) button: filled with the accent colour.
fn primary(ui: &mut egui::Ui, text: &str) -> bool {
    let dark = ui.visuals().dark_mode;
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE)).fill(accent(dark)).min_size(vec2(84.0, 26.0)).corner_radius(6.0)).clicked()
}
fn secondary(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(egui::Button::new(text).min_size(vec2(84.0, 26.0)).corner_radius(6.0)).clicked()
}

/// Draw the window and return what the user asked for. `look.details` is the disclosure's state.
pub fn show(ui: &mut egui::Ui, mode: &Mode, list: &[Stage], look: &mut Look) -> Option<Action> {
    let mut action = None;
    let dark = ui.visuals().dark_mode;
    let weak = ui.visuals().weak_text_color();
    let failed = stages::failure(list);
    let done = stages::finished(list);
    let total = list.len();
    let current = list
        .iter()
        .position(|s| s.state == State::Failed)
        .or_else(|| list.iter().position(|s| s.state == State::Active))
        .or_else(|| list.iter().position(|s| s.state == State::Pending))
        .unwrap_or(total.saturating_sub(1));
    let stage = list.get(current);
    // The fork's version is known once its release is out; until then only the original's is.
    let target = match (mode, &look.release) {
        (_, Some(v)) => Some(v.as_str()),
        (Mode::Merge { .. }, None) => None,
        (Mode::Install { version }, None) => Some(version.as_str()),
    };
    let unchanged = "Your installed PdfCraft is unchanged.";
    egui::Frame::new().inner_margin(MARGIN).show(ui, |ui| {
        ui.set_width(WIDTH - 2.0 * MARGIN);
        // Header: icon, headline, one sentence.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            match look.icon {
                Some(t) => {
                    ui.add(egui::Image::new(t).fit_to_exact_size(vec2(ICON, ICON)));
                }
                None => {
                    ui.allocate_exact_size(vec2(ICON, ICON), egui::Sense::hover());
                }
            }
            ui.vertical(|ui| {
                ui.add_space(6.0);
                let (head, sub) = if done {
                    ("PdfCraft is up to date".to_string(), format!("Updated to {} and restarted.", target.unwrap_or("the latest version")))
                } else if let Some(f) = failed {
                    ("Couldn't update PdfCraft".to_string(), format!("Stopped while {} (step {} of {total})", lower(doing(&f.title)), current + 1))
                } else {
                    let what = stage.map_or("Starting", |s| doing(&s.title));
                    let head = match target {
                        Some(t) => format!("Updating PdfCraft to {t}"),
                        None => "Updating PdfCraft".to_string(),
                    };
                    (head, format!("Step {} of {total}: {what}…", current + 1))
                };
                ui.label(RichText::new(head).heading().strong());
                ui.label(RichText::new(sub).color(weak));
            });
        });
        ui.add_space(if done { 6.0 } else { 14.0 });
        // The step under way: one bar for the whole update, the download's bytes and time left.
        if let Some(f) = failed {
            egui::Frame::new()
                .fill(red(dark).gamma_multiply(0.12))
                .stroke(Stroke::new(1.0, red(dark).gamma_multiply(0.5)))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(reason(f, current)).color(red(dark)));
                    // Up to the install, nothing on this Mac has changed.
                    if !matches!(f.title.as_str(), "Install" | "Relaunch PdfCraft") {
                        ui.label(RichText::new(unchanged).small().color(weak));
                    }
                });
        } else if !done {
            bar(ui, stages::overall(list));
            let detail = stage.map(|s| sentence(&s.detail)).unwrap_or_default();
            let line = match (&look.eta, stage.and_then(|s| s.fraction)) {
                (Some(eta), Some(_)) => format!("{detail} - {eta}"),
                _ => detail,
            };
            // A wait is said by the subtitle already.
            if !line.is_empty() && !line.starts_with("Waiting") {
                ui.add(egui::Label::new(RichText::new(line).small().color(weak)).truncate());
            }
        }
        ui.add_space(8.0);
        // Details: every stage, and the checks under Checks.
        let header = egui::CollapsingHeader::new(RichText::new(if look.details { "Hide details" } else { "Show details" }))
            .id_salt("details")
            .open(Some(look.details))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(460.0).auto_shrink([false, true]).show(ui, |ui| {
                    for s in list {
                        row(ui, s, 14.0, &mut action);
                        if !s.children.is_empty() && s.state != State::Done {
                            ui.indent(("checks", &s.title), |ui| {
                                for c in &s.children {
                                    row(ui, c, 11.0, &mut action);
                                }
                            });
                        }
                    }
                });
            });
        if header.header_response.clicked() {
            look.details = !look.details;
        }
        ui.add_space(12.0);
        // Footer: a note on the left, the buttons on the right with the default one last.
        ui.horizontal(|ui| {
            if done {
                let notes = list.iter().find(|s| s.title == "Publish release").and_then(|s| s.link.clone());
                if let Some(url) = notes
                    && ui.link("Release notes").clicked()
                {
                    action = Some(Action::Open(url));
                }
                if let Some(s) = look.closes_in {
                    ui.label(RichText::new(format!("Closes in {s} s")).small().color(weak));
                }
            } else if failed.is_none() {
                ui.label(RichText::new("PdfCraft restarts once the update is installed.").small().color(weak));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if done {
                    if primary(ui, "Done") {
                        action = Some(Action::Close);
                    }
                } else if let Some(f) = failed {
                    match &f.link {
                        Some(link) => {
                            let check = f.children.iter().any(|c| c.state == State::Failed);
                            if primary(ui, if check { "View Failed Check" } else { "View on GitHub" }) {
                                action = Some(Action::Open(link.clone()));
                            }
                            if secondary(ui, "Close") {
                                action = Some(Action::Close);
                            }
                        }
                        None => {
                            if primary(ui, "Close") {
                                action = Some(Action::Close);
                            }
                        }
                    }
                } else if secondary(ui, "Hide") {
                    action = Some(Action::Minimize);
                }
            });
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stage_title_reads_as_a_step_under_way() {
        let local = stages::Local::default();
        let merge = Mode::Merge { base: "0.21.1".into(), since: "2026-11-12T21:13:00Z".into() };
        for s in stages::stages(&merge, &stages::Remote::default(), &local) {
            assert_ne!(doing(&s.title), s.title, "{} has no wording", s.title);
        }
    }

    #[test]
    fn time_left_is_rounded_and_only_given_once_it_means_something() {
        assert_eq!(eta(10.0, 50.0).as_deref(), Some("about 10 seconds left"));
        assert_eq!(eta(12.0, 48.6).as_deref(), Some("about 15 seconds left"));
        assert_eq!(eta(60.0, 25.0).as_deref(), Some("about 3 minutes left"));
        assert_eq!(eta(40.0, 40.0).as_deref(), Some("about a minute left"));
        assert_eq!(eta(10.0, 99.9).as_deref(), Some("a few seconds left"));
        for (t, p) in [(1.0, 50.0), (10.0, 1.0), (10.0, 100.0), (f32::NAN, 50.0), (10.0, f32::INFINITY), (-5.0, 50.0)] {
            assert_eq!(eta(t, p), None, "{t} {p}");
        }
    }
}
