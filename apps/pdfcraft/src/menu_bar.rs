//! The macOS menu bar, showing `pdfcraft_ui_egui::menu_bar::MenuBarModel` with muda.
//!
//! It replaces craft-quit's app-only menu (`craft_quit::install_app_menu`) with the app menu plus
//! the in-window menu's File, Edit, Pages, View and Help. Quit keeps craft-quit's id and handling,
//! so ⌘Q still asks about unsaved changes like the close button (#1136); every other item is
//! queued and run by the app on its next frame.

use std::sync::{Arc, Mutex};

use muda::accelerator::Accelerator;
use muda::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use pdfcraft_ui_egui::menu_bar::{self as model, MenuBarModel, MenuEntry, Standard};

/// A shown item, kept so it can be updated in place and outlives the native menu showing it.
enum Shown {
    Item(MenuItem),
    Check(CheckMenuItem),
    Submenu(Submenu, Vec<Shown>),
    Other(PredefinedMenuItem),
}

pub struct NativeMenuBar {
    chosen: Arc<Mutex<Vec<String>>>,
    shown: Option<(Menu, Vec<Shown>, MenuBarModel)>,
    /// Building the menu failed once; it isn't retried every frame.
    gave_up: bool,
}

impl NativeMenuBar {
    /// Take over muda's menu events: Quit goes to craft-quit, everything else to the app.
    /// muda keeps the first handler it is given, so this replaces `craft_quit::install_app_menu`.
    pub fn new(ctx: &egui::Context) -> Self {
        let chosen = Arc::new(Mutex::new(Vec::new()));
        let queue = Arc::clone(&chosen);
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let id = event.id.as_ref();
            if id == craft_quit::QUIT_MENU_ID {
                craft_quit::on_menu_event(&ctx, id);
                return;
            }
            // A poisoned queue still holds plain strings; take it over rather than lose the click.
            queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(id.to_owned());
            ctx.request_repaint();
        }));
        Self { chosen, shown: None, gave_up: false }
    }
}

fn native_id(id: &str) -> &str {
    if id == model::QUIT { craft_quit::QUIT_MENU_ID } else { id }
}

fn accelerator(text: Option<&str>) -> Option<Accelerator> {
    let text = text?;
    match text.parse() {
        Ok(a) => Some(a),
        Err(e) => {
            log::warn!("menu bar: can't bind {text}: {e}");
            None
        }
    }
}

fn build(entries: &[MenuEntry], parent: &Submenu) -> muda::Result<Vec<Shown>> {
    let mut shown = Vec::with_capacity(entries.len());
    for e in entries {
        let s = match e {
            MenuEntry::Item(i) => match i.checked {
                Some(checked) => {
                    Shown::Check(CheckMenuItem::with_id(native_id(&i.id), &i.label, i.enabled, checked, accelerator(i.accelerator.as_deref())))
                }
                None => Shown::Item(MenuItem::with_id(native_id(&i.id), &i.label, i.enabled, accelerator(i.accelerator.as_deref()))),
            },
            MenuEntry::Separator => Shown::Other(PredefinedMenuItem::separator()),
            MenuEntry::Standard(kind, label) => Shown::Other(match kind {
                Standard::Services => PredefinedMenuItem::services(Some(label)),
                Standard::Hide => PredefinedMenuItem::hide(Some(label)),
                Standard::HideOthers => PredefinedMenuItem::hide_others(Some(label)),
                Standard::ShowAll => PredefinedMenuItem::show_all(Some(label)),
            }),
            MenuEntry::Submenu(m) => {
                let sub = Submenu::new(&m.title, m.enabled);
                let children = build(&m.entries, &sub)?;
                Shown::Submenu(sub, children)
            }
        };
        let item: &dyn IsMenuItem = match &s {
            Shown::Item(i) => i,
            Shown::Check(i) => i,
            Shown::Submenu(i, _) => i,
            Shown::Other(i) => i,
        };
        parent.append(item)?;
        shown.push(s);
    }
    Ok(shown)
}

/// Bring the shown items in line with `entries`, which have the same shape as when they were built.
fn update(shown: &[Shown], entries: &[MenuEntry], old: &[MenuEntry]) {
    for ((s, e), o) in shown.iter().zip(entries).zip(old) {
        match (s, e, o) {
            (Shown::Item(n), MenuEntry::Item(i), MenuEntry::Item(was)) => {
                if i.label != was.label {
                    n.set_text(&i.label);
                }
                if i.enabled != was.enabled {
                    n.set_enabled(i.enabled);
                }
            }
            (Shown::Check(n), MenuEntry::Item(i), MenuEntry::Item(was)) => {
                if i.label != was.label {
                    n.set_text(&i.label);
                }
                if i.enabled != was.enabled {
                    n.set_enabled(i.enabled);
                }
                // macOS ticks a check item when it's clicked, whether or not the choice took.
                let checked = i.checked.unwrap_or(false);
                if n.is_checked() != checked {
                    n.set_checked(checked);
                }
            }
            (Shown::Submenu(n, children), MenuEntry::Submenu(m), MenuEntry::Submenu(was)) => {
                if m.title != was.title {
                    n.set_text(&m.title);
                }
                if m.enabled != was.enabled {
                    n.set_enabled(m.enabled);
                }
                update(children, &m.entries, &was.entries);
            }
            _ => {}
        }
    }
}

fn install(bar: &MenuBarModel) -> muda::Result<(Menu, Vec<Shown>)> {
    let menu = Menu::new();
    let mut shown = Vec::with_capacity(bar.menus.len());
    for m in &bar.menus {
        let sub = Submenu::new(&m.title, m.enabled);
        let children = build(&m.entries, &sub)?;
        menu.append(&sub)?;
        shown.push(Shown::Submenu(sub, children));
    }
    menu.init_for_nsapp();
    Ok((menu, shown))
}

impl model::MenuBar for NativeMenuBar {
    fn show(&mut self, bar: &MenuBarModel) {
        if self.gave_up {
            return;
        }
        if let Some((_, shown, old)) = &mut self.shown
            && old.same_shape(bar)
        {
            for ((s, m), was) in shown.iter().zip(&bar.menus).zip(&old.menus) {
                if let Shown::Submenu(n, children) = s {
                    if m.title != was.title {
                        n.set_text(&m.title);
                    }
                    update(children, &m.entries, &was.entries);
                }
            }
            if old != bar {
                *old = bar.clone();
            }
            return;
        }
        match install(bar) {
            // The old menu is dropped only after the new one is the app's.
            Ok((menu, shown)) => self.shown = Some((menu, shown, bar.clone())),
            Err(e) => {
                // Not every frame: the menu shown before (or winit's) stays.
                log::warn!("couldn't show the menu bar: {e}");
                self.gave_up = true;
            }
        }
    }

    fn take_chosen(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.chosen.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}
