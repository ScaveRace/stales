//! Every browser action in one list, with its default keys.
//!
//! Keyboard shortcuts, the single-key commands on pages, the F1 cheat sheet and the
//! remote control all use this list, so an action only has to be written once.
//! You can change any key in `~/.config/stales/config.toml` under `[keys.bindings]`.

use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;

use crate::command_bar::{self, Mode};
use crate::config::KeysConfig;
use crate::engine::PageKeys;
use crate::saved::{SIDEBAR_COMPACT, SIDEBAR_MIN};
use crate::window::{self, Ui};
use crate::{blocker, find_bar, help, theme};

pub struct Action {
    pub name: &'static str,
    pub group: &'static str,
    pub description: &'static str,
    /// GTK key names: `<Ctrl>t`, `<Ctrl><Shift>Tab`, `F5`, …
    pub keys: &'static [&'static str],
    run: fn(&Rc<Ui>),
}

const fn action(
    name: &'static str,
    group: &'static str,
    description: &'static str,
    keys: &'static [&'static str],
    run: fn(&Rc<Ui>),
) -> Action {
    Action { name, group, description, keys, run }
}

pub const ACTIONS: &[Action] = &[
    // Tabs
    action("new-tab", "Tabs", "New tab", &["<Ctrl>t"], |ui| command_bar::open(ui, Mode::NewTab)),
    action("open-location", "Tabs", "Change this tab's address", &["<Ctrl>l", "F6"], |ui| {
        command_bar::open(ui, Mode::EditCurrent)
    }),
    action("close-tab", "Tabs", "Close tab", &["<Ctrl>w"], |ui| change(ui, |s| s.close_active())),
    action("reopen-tab", "Tabs", "Reopen closed tab", &["<Ctrl><Shift>t"], |ui| {
        if !ui.state.borrow_mut().reopen_closed() {
            window::toast(ui, "No closed tabs");
        }
        window::refresh(ui);
    }),
    action("tab-next", "Tabs", "Next tab", &["<Ctrl>Tab", "<Ctrl>Page_Down"], |ui| {
        change(ui, |s| s.select_relative(1))
    }),
    action(
        "tab-prev",
        "Tabs",
        "Previous tab",
        &["<Ctrl><Shift>Tab", "<Ctrl><Shift>ISO_Left_Tab", "<Ctrl>Page_Up"],
        |ui| change(ui, |s| s.select_relative(-1)),
    ),
    action("tab-1", "Tabs", "Go to tab 1 (pinned first)", &["<Ctrl>1"], |ui| change(ui, |s| s.select_nth(Some(0)))),
    action("tab-2", "Tabs", "Go to tab 2", &["<Ctrl>2"], |ui| change(ui, |s| s.select_nth(Some(1)))),
    action("tab-3", "Tabs", "Go to tab 3", &["<Ctrl>3"], |ui| change(ui, |s| s.select_nth(Some(2)))),
    action("tab-4", "Tabs", "Go to tab 4", &["<Ctrl>4"], |ui| change(ui, |s| s.select_nth(Some(3)))),
    action("tab-5", "Tabs", "Go to tab 5", &["<Ctrl>5"], |ui| change(ui, |s| s.select_nth(Some(4)))),
    action("tab-6", "Tabs", "Go to tab 6", &["<Ctrl>6"], |ui| change(ui, |s| s.select_nth(Some(5)))),
    action("tab-7", "Tabs", "Go to tab 7", &["<Ctrl>7"], |ui| change(ui, |s| s.select_nth(Some(6)))),
    action("tab-8", "Tabs", "Go to tab 8", &["<Ctrl>8"], |ui| change(ui, |s| s.select_nth(Some(7)))),
    action("tab-last", "Tabs", "Go to last tab", &["<Ctrl>9"], |ui| change(ui, |s| s.select_nth(None))),
    action("pin-tab", "Tabs", "Pin / unpin tab", &["<Ctrl>d"], |ui| change(ui, |s| s.toggle_pin())),
    action("move-tab-up", "Tabs", "Move tab up", &["<Ctrl><Shift>Up"], |ui| change(ui, |s| s.move_active(-1))),
    action("move-tab-down", "Tabs", "Move tab down", &["<Ctrl><Shift>Down"], |ui| change(ui, |s| s.move_active(1))),
    action("copy-url", "Tabs", "Copy address", &["<Ctrl><Shift>c"], copy_url),
    action("fill-login", "Page", "Fill saved login", &["<Ctrl><Shift>l"], crate::passwords::fill_shortcut),
    action("save-login", "Page", "Save login (when asked)", &["<Ctrl><Shift>Return"], crate::passwords::save_pending),
    // Page
    action("back", "Page", "Back", &["<Alt>Left"], |ui| with_page(ui, |p| p.go_back())),
    action("forward", "Page", "Forward", &["<Alt>Right"], |ui| with_page(ui, |p| p.go_forward())),
    action("reload", "Page", "Reload / stop", &["<Ctrl>r", "F5"], window::reload_or_stop),
    action("find", "Page", "Find in page", &["<Ctrl>f"], find_bar::open),
    action("find-next", "Page", "Next match", &["<Ctrl>g", "F3"], |ui| with_page(ui, |p| p.find_next())),
    action("find-prev", "Page", "Previous match", &["<Ctrl><Shift>g", "<Shift>F3"], |ui| {
        with_page(ui, |p| p.find_previous())
    }),
    action("zoom-in", "Page", "Zoom in", &["<Ctrl>plus", "<Ctrl>equal"], |ui| zoom(ui, Some(0.1))),
    action("zoom-out", "Page", "Zoom out", &["<Ctrl>minus"], |ui| zoom(ui, Some(-0.1))),
    action("zoom-reset", "Page", "Reset zoom", &["<Ctrl>0"], |ui| zoom(ui, None)),
    action("toggle-blocker", "Page", "Ad blocker off / on for this site", &["<Ctrl><Shift>b"], blocker::toggle_site),
    action("inspector", "Page", "Developer tools", &["<Ctrl><Shift>i", "F12"], |ui| {
        with_page(ui, |p| p.show_inspector())
    }),
    // Window
    action("toggle-sidebar", "Window", "Show / hide sidebar", &["<Ctrl>s"], window::toggle_sidebar),
    action(
        "compact-sidebar",
        "Window",
        "Sidebar: icons only / full",
        &["<Ctrl><Shift>s"],
        window::toggle_compact_sidebar,
    ),
    action("sidebar-narrower", "Window", "Sidebar narrower", &["<Ctrl>bracketleft"], |ui| resize_sidebar(ui, -20)),
    action("sidebar-wider", "Window", "Sidebar wider", &["<Ctrl>bracketright"], |ui| resize_sidebar(ui, 20)),
    action("space-next", "Window", "Next Space", &["<Ctrl><Alt>Right"], |ui| {
        change(ui, |s| s.switch_space_relative(1))
    }),
    action("space-prev", "Window", "Previous Space", &["<Ctrl><Alt>Left"], |ui| {
        change(ui, |s| s.switch_space_relative(-1))
    }),
    action("space-1", "Window", "Go to Space 1", &["<Alt>1"], |ui| space(ui, 0)),
    action("space-2", "Window", "Go to Space 2", &["<Alt>2"], |ui| space(ui, 1)),
    action("space-3", "Window", "Go to Space 3", &["<Alt>3"], |ui| space(ui, 2)),
    action("space-4", "Window", "Go to Space 4", &["<Alt>4"], |ui| space(ui, 3)),
    action("space-5", "Window", "Go to Space 5", &["<Alt>5"], |ui| space(ui, 4)),
    action("toggle-layout", "Window", "Tabs in sidebar / on top", &["<Ctrl><Alt>l"], window::toggle_layout),
    action("next-theme", "Window", "Next theme", &["<Ctrl><Alt>t"], theme::cycle),
    action("space-color", "Window", "Change this Space's colour", &["<Ctrl><Alt>c"], theme::cycle_space_color),
    action("new-space", "Window", "New Space", &["<Ctrl><Alt>n"], |ui| change(ui, |s| s.add_space())),
    action("settings", "Window", "Settings", &["<Ctrl>comma"], crate::settings::open),
    action("help", "Window", "Show all shortcuts", &["F1"], help::toggle),
];

/// Single-key commands on web pages. Actions in keys.js (scrolling, hints) run
/// inside the page; the others are the browser actions above.
pub const PAGE_KEYS: &[(&str, &str, &str)] = &[
    ("j", "scroll-down", "Scroll down"),
    ("k", "scroll-up", "Scroll up"),
    ("h", "scroll-left", "Scroll left"),
    ("l", "scroll-right", "Scroll right"),
    ("d", "half-page-down", "Half a page down"),
    ("u", "half-page-up", "Half a page up"),
    (" ", "page-down", "Page down (Space)"),
    ("gg", "scroll-top", "Top of page"),
    ("G", "scroll-bottom", "Bottom of page"),
    ("f", "hints", "Click a link: shows letters, type one"),
    ("F", "hints-new-tab", "Open a link in a background tab"),
    ("gi", "focus-input", "Jump to the first text field"),
    ("i", "insert-mode", "Send all keys to the page until Esc"),
    ("H", "back", "Back"),
    ("L", "forward", "Forward"),
    ("r", "reload", "Reload"),
    ("J", "tab-next", "Next tab"),
    ("K", "tab-prev", "Previous tab"),
    ("x", "close-tab", "Close tab"),
    ("X", "reopen-tab", "Reopen closed tab"),
    ("o", "open-location", "Change address"),
    ("t", "new-tab", "New tab"),
    ("yy", "copy-url", "Copy address"),
    ("/", "find", "Find in page"),
    ("n", "find-next", "Next match"),
    ("N", "find-prev", "Previous match"),
    ("s", "toggle-sidebar", "Show / hide sidebar"),
    ("S", "compact-sidebar", "Sidebar: icons only / full"),
    ("?", "help", "Show all shortcuts"),
];

/// The keys for each action: your config's choice, or the default.
pub fn keys_for(config: &KeysConfig, action: &Action) -> Vec<String> {
    match config.bindings.get(action.name) {
        Some(keys) => keys.clone(),
        None => action.keys.iter().map(|k| k.to_string()).collect(),
    }
}

/// The page key map: defaults, then your `[keys.page]` changes on top.
pub fn page_keys(config: &KeysConfig) -> PageKeys {
    let mut keys: HashMap<String, String> = PAGE_KEYS.iter().map(|(k, a, _)| (k.to_string(), a.to_string())).collect();
    for (key, action) in &config.page {
        if action.is_empty() || action == "none" {
            keys.remove(key);
        } else {
            keys.insert(key.clone(), action.clone());
        }
    }
    PageKeys { vim: config.vim, keys, exclude: config.exclude.clone() }
}

/// Run an action by name. Returns false if there is no such action.
pub fn run(ui: &Rc<Ui>, name: &str) -> bool {
    match ACTIONS.iter().find(|a| a.name == name) {
        Some(action) => {
            (action.run)(ui);
            true
        }
        None => false,
    }
}

/// Make the keyboard shortcuts work. They are checked before the page sees the key
/// (the "capture" phase), so a website can't swallow Ctrl+T or Ctrl+L.
/// Call again after the keys change: the old shortcuts are replaced.
pub fn install(ui: &Rc<Ui>) {
    if let Some(old) = ui.shortcuts.take() {
        ui.window.remove_controller(&old);
    }
    let controller = gtk::ShortcutController::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    controller.set_scope(gtk::ShortcutScope::Global);

    let config = ui.config.borrow();
    for action in ACTIONS {
        for key in keys_for(&config.keys, action) {
            let Some(trigger) = gtk::ShortcutTrigger::parse_string(&key) else {
                eprintln!("stales: can't understand the key '{key}' for '{}' in config.toml", action.name);
                continue;
            };
            let weak = Rc::downgrade(ui);
            let run = action.run;
            let callback = gtk::CallbackAction::new(move |_, _| {
                if let Some(ui) = weak.upgrade() {
                    run(&ui);
                }
                gtk::glib::Propagation::Stop
            });
            controller.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(callback)));
        }
    }
    for name in config.keys.bindings.keys() {
        if !ACTIONS.iter().any(|a| a.name == name) {
            eprintln!("stales: there is no action called '{name}' (in [keys.bindings] in config.toml)");
        }
    }
    ui.window.add_controller(controller.clone());
    ui.shortcuts.replace(Some(controller));
}

/// Turn the shortcuts off while the settings page records a new key combo,
/// so pressing e.g. Ctrl+T sets that key instead of opening a tab.
pub fn pause(ui: &Ui, paused: bool) {
    if let Some(controller) = ui.shortcuts.borrow().as_ref() {
        let phase = if paused { gtk::PropagationPhase::None } else { gtk::PropagationPhase::Capture };
        controller.set_propagation_phase(phase);
    }
}

/// Turn "<Ctrl><Shift>t" into "Ctrl+Shift+T" for showing to people.
/// Plain Rust (no GTK), so it also works in tests without a screen.
pub fn pretty_key(key: &str) -> String {
    let mut parts = Vec::new();
    let mut rest = key;
    while let Some(after) = rest.strip_prefix('<') {
        let Some(end) = after.find('>') else { break };
        parts.push(match after[..end].to_lowercase().as_str() {
            "ctrl" | "control" | "primary" => "Ctrl".to_string(),
            "shift" => "Shift".into(),
            "alt" | "mod1" => "Alt".into(),
            "super" | "meta" => "Super".into(),
            other => other.to_string(),
        });
        rest = &after[end + 1..];
    }
    let name = match rest {
        "plus" => "+".to_string(),
        "minus" => "-".into(),
        "equal" => "=".into(),
        "bracketleft" => "[".into(),
        "bracketright" => "]".into(),
        "Page_Down" => "Page Down".into(),
        "Page_Up" => "Page Up".into(),
        "Left" => "←".into(),
        "Right" => "→".into(),
        "Up" => "↑".into(),
        "Down" => "↓".into(),
        "ISO_Left_Tab" => "Tab".into(),
        other if other.chars().count() == 1 => other.to_uppercase(),
        other => other.replace('_', " "),
    };
    parts.push(name);
    parts.join("+")
}

// ---------- Small helpers used by the actions ----------

fn change(ui: &Rc<Ui>, f: impl FnOnce(&mut crate::state::State)) {
    f(&mut ui.state.borrow_mut());
    window::refresh(ui);
}

fn with_page(ui: &Rc<Ui>, f: impl FnOnce(&dyn crate::engine::Page)) {
    if let Some(page) = window::active_page(ui) {
        f(page.as_ref());
    }
}

fn space(ui: &Rc<Ui>, index: usize) {
    if ui.state.borrow_mut().switch_space(index) {
        window::refresh(ui);
    }
}

fn copy_url(ui: &Rc<Ui>) {
    let url = ui.state.borrow().active_tab().map(|t| t.url.clone());
    if let Some(url) = url {
        ui.window.clipboard().set_text(&url);
        window::toast(ui, "Address copied");
    }
}

fn zoom(ui: &Rc<Ui>, step: Option<f64>) {
    if let Some(page) = window::active_page(ui) {
        let level = match step {
            Some(step) => (page.zoom() + step).clamp(0.3, 3.0),
            None => 1.0,
        };
        page.set_zoom(level);
        window::toast(ui, &format!("Zoom {:.0} %", level * 100.0));
    }
}

fn resize_sidebar(ui: &Rc<Ui>, delta: i32) {
    let current = ui.sidebar.root.width_request();
    let width = if ui.sidebar.compact.get() {
        // Growing out of icons-only mode starts at the minimum width.
        if delta > 0 { SIDEBAR_MIN } else { current }
    } else if current + delta < SIDEBAR_MIN {
        // Shrinking past the minimum switches to icons only.
        SIDEBAR_COMPACT
    } else {
        current + delta
    };
    window::set_sidebar_width(ui, width);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_names_are_unique() {
        let mut names: Vec<_> = ACTIONS.iter().map(|a| a.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), ACTIONS.len());
    }

    #[test]
    fn page_keys_point_to_real_actions() {
        let in_page = [
            "scroll-down",
            "scroll-up",
            "scroll-left",
            "scroll-right",
            "half-page-down",
            "half-page-up",
            "page-down",
            "scroll-top",
            "scroll-bottom",
            "hints",
            "hints-new-tab",
            "focus-input",
            "insert-mode",
        ];
        for (key, action, _) in PAGE_KEYS {
            assert!(
                in_page.contains(action) || ACTIONS.iter().any(|a| a.name == *action),
                "page key '{key}' points to unknown action '{action}'"
            );
        }
    }

    #[test]
    fn keys_are_shown_nicely() {
        assert_eq!(pretty_key("<Ctrl><Shift>t"), "Ctrl+Shift+T");
        assert_eq!(pretty_key("<Alt>Left"), "Alt+←");
        assert_eq!(pretty_key("<Ctrl>bracketright"), "Ctrl+]");
        assert_eq!(pretty_key("F5"), "F5");
    }

    #[test]
    fn config_can_change_and_remove_page_keys() {
        let mut config = KeysConfig::default();
        config.page.insert("j".into(), "scroll-up".into());
        config.page.insert("x".into(), "none".into());
        let keys = page_keys(&config);
        assert_eq!(keys.keys["j"], "scroll-up");
        assert!(!keys.keys.contains_key("x"));
    }
}
