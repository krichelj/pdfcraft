//! The native menu bar's menus (the macOS menu bar), built from the same command registry as
//! the in-window Menu button (`chrome::main_menu`).
//!
//! This module only describes the menus: [`PdfCraftApp::menu_bar_model`] builds them with live
//! labels, shortcuts, enablement and check marks, and [`PdfCraftApp::menu_bar_action`] runs the id
//! of a chosen item. The desktop app shows the model through a [`MenuBar`] (muda on macOS), so
//! this crate stays free of platform menu code and the model is tested headlessly.

use pdfcraft_engine::commands::{self, Shortcut};

use crate::canvas::{DocView, Fit, PageLayout};
use crate::commands::{ACTUAL_SIZE, FIT_WIDTH, PAGE_LEVEL, ROTATE_CCW, ROTATE_CW, ZOOM_IN, ZOOM_OUT};
use crate::theme::ThemePreference;
use crate::{Dialog, PdfCraftApp, RightPanel};

/// The id the app menu's Quit item reports. The desktop app gives its native item the id its
/// quit handling expects (`craft_quit::QUIT_MENU_ID`); this crate never runs it.
pub const QUIT: &str = "app.quit";

/// Commands the app menu shows, so the menus they are registered in leave them out (macOS puts
/// About, updates and Preferences in the app menu).
const IN_APP_MENU: [&str; 3] = ["help.about", "help.check_updates", "app.preferences"];

/// Prefix of a View-menu zoom item ([`ZOOM_ITEMS`]): these act on the view, not the registry.
const ZOOM_PREFIX: &str = "zoom:";
/// Prefix of an Open Recent entry; the rest is its index in `PdfCraftApp::recent`.
const RECENT_PREFIX: &str = "recent:";
/// Prefix of a Side panels entry; the rest is its index in [`SIDE_PANELS`].
const PANEL_PREFIX: &str = "panel:";
/// Theme ▸ Appearance settings… (opens Preferences on its Appearance part).
const APPEARANCE: &str = "dialog.appearance";

/// The View menu's zoom actions, which act on the active view rather than through the registry.
/// The in-window View menu lists the same table.
pub(crate) const ZOOM_ITEMS: [(&str, &str, Option<Shortcut>); 8] = [
    ("actual_size", "Actual size", Some(ACTUAL_SIZE)),
    ("page_level", "Zoom to page level", Some(PAGE_LEVEL)),
    ("fit_width", "Fit to width", Some(FIT_WIDTH)),
    ("fit_height", "Fit to height", None),
    ("zoom_in", "Zoom in", Some(ZOOM_IN)),
    ("zoom_out", "Zoom out", Some(ZOOM_OUT)),
    ("rotate_cw", "Rotate view clockwise", Some(ROTATE_CW)),
    ("rotate_ccw", "Rotate view counterclockwise", Some(ROTATE_CCW)),
];

/// The side panels the View menu can open, in its order.
pub(crate) const SIDE_PANELS: [(RightPanel, &str); 11] = [
    (RightPanel::Comments, "Comments"),
    (RightPanel::Bookmarks, "Bookmarks"),
    (RightPanel::Pages, "Pages"),
    (RightPanel::Fields, "Fields"),
    (RightPanel::Layers, "Layers"),
    (RightPanel::Attachments, "Attachments"),
    (RightPanel::Signatures, "Signatures"),
    (RightPanel::Accessibility, "Accessibility Checker"),
    (RightPanel::Tags, "Tags"),
    (RightPanel::Search, "Search"),
    (RightPanel::Compare, "Compare"),
];

/// Apply the zoom action `key` ([`ZOOM_ITEMS`]) to `view`. `false` for an unknown key.
pub(crate) fn zoom_action(view: &mut DocView, key: &str) -> bool {
    match key {
        "actual_size" => view.set_zoom(1.0),
        "page_level" => view.fit = Fit::Page,
        "fit_width" => view.fit = Fit::Width,
        "fit_height" => {
            view.fit = Fit::Height;
            view.goto = Some((view.current, 0.0));
        }
        "zoom_in" => view.zoom_step(true),
        "zoom_out" => view.zoom_step(false),
        "rotate_cw" => view.rotate_view(true),
        "rotate_ccw" => view.rotate_view(false),
        _ => return false,
    }
    true
}

/// One entry of a menu.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuEntry {
    Item(MenuItem),
    Separator,
    Submenu(Submenu),
    /// An item macOS provides itself, with a translated label.
    Standard(Standard, String),
}

/// The items macOS provides in every app menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standard {
    Services,
    Hide,
    HideOthers,
    ShowAll,
}

/// A titled menu: a top-level menu of the bar, or a submenu.
#[derive(Clone, Debug, PartialEq)]
pub struct Submenu {
    pub title: String,
    pub enabled: bool,
    pub entries: Vec<MenuEntry>,
}

/// An item that runs an action when chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    /// What [`PdfCraftApp::menu_bar_action`] is given when the item is chosen.
    pub id: String,
    pub label: String,
    /// The key that runs it, in muda's accelerator syntax (`Super+Shift+KeyS`).
    pub accelerator: Option<String>,
    pub enabled: bool,
    /// `Some` for an item that shows a check mark (a radio choice or a toggle).
    pub checked: Option<bool>,
}

/// The whole menu bar: the app menu first, then File, Edit, Pages, View and Help.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuBarModel {
    pub menus: Vec<Submenu>,
}

impl MenuBarModel {
    /// Every item, depth first, for lookups and tests.
    pub fn items(&self) -> Vec<&MenuItem> {
        fn walk<'a>(entries: &'a [MenuEntry], out: &mut Vec<&'a MenuItem>) {
            for e in entries {
                match e {
                    MenuEntry::Item(i) => out.push(i),
                    MenuEntry::Submenu(s) => walk(&s.entries, out),
                    MenuEntry::Separator | MenuEntry::Standard(..) => {}
                }
            }
        }
        let mut out = Vec::new();
        for m in &self.menus {
            walk(&m.entries, &mut out);
        }
        out
    }

    /// Whether `other` has the same menus, items and ids in the same places, so a shown menu
    /// can be updated in place (labels, enablement, checks) instead of rebuilt.
    pub fn same_shape(&self, other: &Self) -> bool {
        fn same(a: &[MenuEntry], b: &[MenuEntry]) -> bool {
            a.len() == b.len()
                && a.iter().zip(b).all(|(x, y)| match (x, y) {
                    (MenuEntry::Item(x), MenuEntry::Item(y)) => {
                        x.id == y.id && x.accelerator == y.accelerator && x.checked.is_some() == y.checked.is_some()
                    }
                    (MenuEntry::Separator, MenuEntry::Separator) => true,
                    (MenuEntry::Submenu(x), MenuEntry::Submenu(y)) => same(&x.entries, &y.entries),
                    (MenuEntry::Standard(x, _), MenuEntry::Standard(y, _)) => x == y,
                    _ => false,
                })
        }
        self.menus.len() == other.menus.len() && self.menus.iter().zip(&other.menus).all(|(x, y)| same(&x.entries, &y.entries))
    }
}

/// Shows a [`MenuBarModel`] as the platform's menu bar. Implemented by the desktop app.
pub trait MenuBar {
    /// Show `model`. Called every frame, so implementations update only what changed.
    fn show(&mut self, model: &MenuBarModel);
    /// The ids of the items chosen since the last call, oldest first.
    fn take_chosen(&mut self) -> Vec<String>;
}

/// `s` in muda's accelerator syntax, as macOS writes it (⌘ is `Super`, ⌃ is `Control`). `None`
/// for a key the menu bar can't bind, which is then shown without a shortcut.
pub fn accelerator(s: Shortcut) -> Option<String> {
    let code = match s.key {
        "," => "Comma".to_owned(),
        "[" => "BracketLeft".to_owned(),
        "]" => "BracketRight".to_owned(),
        "+" | "=" => "Equal".to_owned(),
        "−" | "-" => "Minus".to_owned(),
        "←" => "ArrowLeft".to_owned(),
        "→" => "ArrowRight".to_owned(),
        "Tab" => "Tab".to_owned(),
        k => {
            let mut chars = k.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphabetic() => format!("Key{}", c.to_ascii_uppercase()),
                (Some(c), None) if c.is_ascii_digit() => format!("Digit{c}"),
                _ => return None,
            }
        }
    };
    let mut parts = Vec::new();
    if s.mac_ctrl {
        parts.push("Control".to_owned());
    }
    if s.shift {
        parts.push("Shift".to_owned());
    }
    if s.command {
        parts.push("Super".to_owned());
    }
    if parts.is_empty() {
        // A bare key in the menu bar would swallow ordinary typing.
        return None;
    }
    parts.push(code);
    Some(parts.join("+"))
}

fn item(id: impl Into<String>, label: impl Into<String>, shortcut: Option<Shortcut>, enabled: bool) -> MenuEntry {
    MenuEntry::Item(MenuItem { id: id.into(), label: label.into(), accelerator: shortcut.and_then(accelerator), enabled, checked: None })
}

fn check(id: impl Into<String>, label: impl Into<String>, enabled: bool, checked: bool) -> MenuEntry {
    MenuEntry::Item(MenuItem { id: id.into(), label: label.into(), accelerator: None, enabled, checked: Some(checked) })
}

fn submenu(title: &str, entries: Vec<MenuEntry>) -> MenuEntry {
    MenuEntry::Submenu(Submenu { title: title.to_owned(), enabled: true, entries })
}

impl PdfCraftApp {
    /// A registered command as a menu item, with its live, translated label.
    fn registry_item(&self, spec: &commands::CommandSpec) -> MenuEntry {
        let label = commands::current_label(spec, &self.session, self.active_ids().map(|(_, id)| id));
        item(spec.id, crate::i18n::menu_label(spec.id, &label), spec.shortcut, self.command_enabled(spec))
    }

    /// A top-level menu's registered commands, as `commands::registry_menu` shows them.
    fn registry_entries(&self, menu: &str) -> Vec<MenuEntry> {
        let mut out = Vec::new();
        for spec in commands::menu(menu).filter(|s| !IN_APP_MENU.contains(&s.id)) {
            if spec.id == "file.open_recent" {
                let label = crate::i18n::menu_label(spec.id, spec.label);
                let mut entries: Vec<MenuEntry> =
                    self.recent.iter().enumerate().map(|(i, r)| item(format!("{RECENT_PREFIX}{i}"), r.name.clone(), None, true)).collect();
                if !entries.is_empty() {
                    entries.push(MenuEntry::Separator);
                    entries.push(item("file.clear_recent", tl!("Clear Recent Files"), None, true));
                }
                out.push(MenuEntry::Submenu(Submenu { title: label, enabled: !self.recent.is_empty(), entries }));
                continue;
            }
            out.push(self.registry_item(spec));
        }
        out
    }

    /// The View menu: zoom, page navigation and page display for the active view, then the
    /// registered View commands, the theme and the side panels (as `chrome::main_menu`).
    fn view_entries(&self) -> Vec<MenuEntry> {
        let mut out = Vec::new();
        if let Some(v) = self.active.and_then(|i| self.views.get(i)) {
            for (key, label, shortcut) in ZOOM_ITEMS {
                out.push(item(format!("{ZOOM_PREFIX}{key}"), tl!(label), shortcut, true));
                if key == "fit_height" {
                    out.extend(commands::command("view.fit_visible").map(|spec| self.registry_item(spec)));
                }
            }
            out.push(MenuEntry::Separator);
            for id in ["view.previous_view", "view.next_view"] {
                if let Some(spec) = commands::command(id) {
                    out.push(self.registry_item(spec));
                }
            }
            out.push(MenuEntry::Separator);
            for l in PageLayout::ORDER {
                out.push(check(l.command(), tl!(l.label()), true, v.layout == l));
            }
            out.push(MenuEntry::Separator);
            out.push(check("view.continuous", tl!("Continuous scrolling"), true, v.continuous()));
            out.push(check("view.layout.cover", tl!("Show cover page in two-page view"), v.cover_applies(), v.cover));
            out.push(MenuEntry::Separator);
        }
        out.extend(self.registry_entries("View"));
        let mut theme: Vec<MenuEntry> = [
            (ThemePreference::System, "view.theme.system", "Use system setting"),
            (ThemePreference::Light, "view.theme.light", "Light gray"),
            (ThemePreference::Dark, "view.theme.dark", "Dark gray"),
        ]
        .into_iter()
        .map(|(p, id, label)| check(id, tl!(label), true, self.theme_preference == p))
        .collect();
        theme.push(MenuEntry::Separator);
        theme.push(item(APPEARANCE, tl!("Appearance settings…"), None, true));
        out.push(submenu(tl!("Display theme"), theme));
        let panels = SIDE_PANELS
            .iter()
            .enumerate()
            .map(|(i, (p, label))| check(format!("{PANEL_PREFIX}{i}"), tl!(label), true, self.right == Some(*p)))
            .collect();
        out.push(submenu(tl!("Side panels"), panels));
        out
    }

    /// The menu bar as it should show now: the app menu (About, Check for updates, Preferences,
    /// the macOS items and Quit), then the in-window menu's File, Edit, Pages, View and Help.
    pub fn menu_bar_model(&self) -> MenuBarModel {
        let registered = |id: &str| commands::command(id).map(|spec| self.registry_item(spec));
        let mut app = Vec::new();
        app.extend(registered("help.about"));
        app.push(MenuEntry::Separator);
        app.extend(registered("help.check_updates"));
        app.extend(registered("app.preferences"));
        app.push(MenuEntry::Separator);
        app.push(MenuEntry::Standard(Standard::Services, tl!("Services").to_owned()));
        app.push(MenuEntry::Separator);
        app.push(MenuEntry::Standard(Standard::Hide, tl!("Hide PdfCraft").to_owned()));
        app.push(MenuEntry::Standard(Standard::HideOthers, tl!("Hide Others").to_owned()));
        app.push(MenuEntry::Standard(Standard::ShowAll, tl!("Show All").to_owned()));
        app.push(MenuEntry::Separator);
        app.push(item(QUIT, tl!("Quit PdfCraft"), Some(Shortcut::cmd("Q")), true));
        let top = |title: &str, entries| Submenu { title: title.to_owned(), enabled: true, entries };
        MenuBarModel {
            menus: vec![
                top("PdfCraft", app),
                top(tl!("File"), self.registry_entries("File")),
                top(tl!("Edit"), self.registry_entries("Edit")),
                top(tl!("Pages"), self.registry_entries("Pages")),
                top(tl!("View"), self.view_entries()),
                top(tl!("Help"), self.registry_entries("Help")),
            ],
        }
    }

    /// Run the items chosen in the menu bar, then show it as it should be now. Every frame.
    pub(crate) fn sync_menu_bar(&mut self) {
        let Some(mut bar) = self.menu_bar.take() else { return };
        for id in bar.take_chosen() {
            self.menu_bar_choose(&id);
        }
        bar.show(&self.menu_bar_model());
        self.menu_bar = Some(bar);
    }

    /// [`Self::menu_bar_action`], except that Undo and Redo go to a focused text field, as the
    /// keys did before the menu bar took them.
    fn menu_bar_choose(&mut self, id: &str) {
        let text_focus = self.ctx.as_ref().is_some_and(egui::Context::egui_wants_keyboard_input);
        let shift = match id {
            "edit.undo" => false,
            "edit.redo" => true,
            _ => {
                self.menu_bar_action(id);
                return;
            }
        };
        if !text_focus {
            self.menu_bar_action(id);
            return;
        }
        let modifiers = egui::Modifiers { mac_cmd: true, command: true, shift, ..Default::default() };
        for pressed in [true, false] {
            self.menu_bar_keys.push(egui::Event::Key { key: egui::Key::Z, physical_key: Some(egui::Key::Z), pressed, repeat: false, modifiers });
        }
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
    }

    /// Run the menu bar item `id` ([`MenuBarModel`]). Returns `false` when nothing ran: an
    /// unknown or stale id, a disabled command, or [`QUIT`], which the desktop app handles.
    pub fn menu_bar_action(&mut self, id: &str) -> bool {
        if let Some(key) = id.strip_prefix(ZOOM_PREFIX) {
            return self.active.and_then(|i| self.views.get_mut(i)).is_some_and(|v| zoom_action(v, key));
        }
        if let Some(i) = id.strip_prefix(RECENT_PREFIX) {
            let Some(path) = i.parse::<usize>().ok().and_then(|i| self.recent.get(i)).map(|r| r.path.clone()) else { return false };
            self.open_recent(&path);
            return true;
        }
        if let Some(i) = id.strip_prefix(PANEL_PREFIX) {
            let Some((panel, _)) = i.parse::<usize>().ok().and_then(|i| SIDE_PANELS.get(i)) else { return false };
            self.choose_right_panel(Some(*panel));
            return true;
        }
        if id == APPEARANCE {
            self.dialog = Some(Dialog::Preferences);
            return true;
        }
        if id == QUIT {
            return false;
        }
        self.execute(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(m: &'a MenuBarModel, id: &str) -> Option<&'a MenuItem> {
        m.items().into_iter().find(|i| i.id == id)
    }

    /// Every command the in-window menus list is in the menu bar, once, with its shortcut, and
    /// every id the bar reports is one `menu_bar_action` knows.
    #[test]
    fn menu_bar_lists_every_registered_menu_command_with_its_shortcut() {
        let app = PdfCraftApp::new();
        let model = app.menu_bar_model();
        let titles: Vec<&str> = model.menus.iter().map(|m| m.title.as_str()).collect();
        assert_eq!(titles, ["PdfCraft", "File", "Edit", "Pages", "View", "Help"]);
        for menu in ["File", "Edit", "Pages", "View", "Help"] {
            for spec in commands::menu(menu) {
                let found: Vec<_> = model.items().into_iter().filter(|i| i.id == spec.id).collect();
                if spec.id == "file.open_recent" {
                    continue;
                }
                assert_eq!(found.len(), 1, "{} appears once", spec.id);
                assert_eq!(found[0].accelerator, spec.shortcut.and_then(accelerator), "{}", spec.id);
            }
        }
        let app_menu: Vec<&str> = model.menus[0]
            .entries
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item(i) => Some(i.id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(app_menu, ["help.about", "help.check_updates", "app.preferences", QUIT]);
        assert_eq!(find(&model, "app.preferences").and_then(|i| i.accelerator.as_deref()), Some("Super+Comma"));
        assert_eq!(find(&model, QUIT).and_then(|i| i.accelerator.as_deref()), Some("Super+KeyQ"));
        assert_eq!(find(&model, "file.save_as").and_then(|i| i.accelerator.as_deref()), Some("Shift+Super+KeyS"));
        assert_eq!(find(&model, "view.read_mode").and_then(|i| i.accelerator.as_deref()), Some("Control+Super+KeyH"));
        assert_eq!(find(&model, "view.next_tab").and_then(|i| i.accelerator.as_deref()), Some("Control+Tab"));
        for i in model.items() {
            let known = i.id == QUIT
                || commands::command(&i.id).is_some()
                || i.id.starts_with(ZOOM_PREFIX)
                || i.id.starts_with(PANEL_PREFIX)
                || i.id.starts_with(RECENT_PREFIX)
                || i.id == APPEARANCE;
            assert!(known, "unknown id {}", i.id);
        }
        let accels: Vec<&str> = model.items().into_iter().filter_map(|i| i.accelerator.as_deref()).collect();
        let mut unique = accels.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), accels.len(), "no two items share a key: {accels:?}");
    }

    /// Without a document, document commands are disabled and the view items are left out;
    /// commands that always run stay enabled.
    #[test]
    fn menu_bar_enablement_follows_the_registry() {
        let app = PdfCraftApp::new();
        let model = app.menu_bar_model();
        assert_eq!(find(&model, "file.save").map(|i| i.enabled), Some(false));
        assert_eq!(find(&model, "file.open").map(|i| i.enabled), Some(true));
        assert_eq!(find(&model, "edit.undo").map(|i| i.enabled), Some(false));
        assert!(find(&model, "zoom:actual_size").is_none(), "zoom needs a view");
        assert!(find(&model, "panel:0").is_some_and(|i| i.checked == Some(false)));
        assert_eq!(find(&model, "view.theme.system").and_then(|i| i.checked), Some(app.theme_preference == ThemePreference::System));
    }

    /// Choosing an item runs it: a side panel opens, the Appearance item opens Preferences, and
    /// stale, unknown or disabled ids do nothing instead of failing.
    #[test]
    fn menu_bar_actions_run_their_commands() {
        let mut app = PdfCraftApp::new();
        assert!(app.menu_bar_action("panel:1"));
        assert_eq!(app.right, Some(RightPanel::Bookmarks));
        assert!(find(&app.menu_bar_model(), "panel:1").is_some_and(|i| i.checked == Some(true)));
        assert!(app.menu_bar_action(APPEARANCE));
        assert!(matches!(app.dialog, Some(Dialog::Preferences)));
        app.dialog = None;
        assert!(app.menu_bar_action("app.preferences"));
        assert!(app.dialog.is_some(), "Preferences… opens its dialog");
        for stale in ["panel:99", "panel:x", "recent:0", "recent:-1", "zoom:actual_size", "zoom:nope", "no.such.command", QUIT, "file.save"] {
            assert!(!app.menu_bar_action(stale), "{stale} does nothing");
        }
    }

    #[test]
    fn accelerators_cover_every_registered_key() {
        assert_eq!(accelerator(ZOOM_IN).as_deref(), Some("Super+Equal"));
        assert_eq!(accelerator(ZOOM_OUT).as_deref(), Some("Super+Minus"));
        assert_eq!(accelerator(ROTATE_CW).as_deref(), Some("Shift+Super+Equal"));
        assert_eq!(accelerator(Shortcut::cmd("[")).as_deref(), Some("Super+BracketLeft"));
        assert_eq!(accelerator(Shortcut::cmd("3")).as_deref(), Some("Super+Digit3"));
        assert_eq!(accelerator(Shortcut { command: false, shift: false, mac_ctrl: false, key: "K" }), None, "a bare key is never bound");
        assert_eq!(accelerator(Shortcut::cmd("Delete")), None);
        assert_eq!(accelerator(Shortcut::cmd("")), None);
        for spec in commands::COMMANDS.iter().filter(|s| s.menu.is_some()) {
            if let Some(s) = spec.shortcut {
                assert!(accelerator(s).is_some(), "{} has a key the menu bar can bind", spec.id);
            }
        }
    }

    /// An unchanged model keeps its shape; a new recent file changes it, a label does not.
    #[test]
    fn shape_changes_only_with_structure() {
        let mut app = PdfCraftApp::new();
        let a = app.menu_bar_model();
        let mut relabelled = a.clone();
        relabelled.menus[1].title = "Datei".into();
        assert!(a.same_shape(&relabelled));
        app.recent.push(crate::RecentFile { name: "a.pdf".into(), path: "/nonexistent/a.pdf".into(), pages: 1, size: 1 });
        assert!(!a.same_shape(&app.menu_bar_model()));
    }
}
