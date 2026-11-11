//! The settings page (stales://settings, Ctrl+,) and the first-start setup (stales://setup).
//!
//! Both are one HTML page (settings.html). It gets the current settings as JSON,
//! and sends back small JSON messages like `{"cmd": "set-theme", "name": "aurora"}`.
//! Each change is applied at once and written to config.toml (keeping your comments),
//! so the file and the browser never disagree.
//!
//! Security: messages are only accepted from Stales' own stales:// pages (checked in
//! the engine), and every value is checked here before it is used.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::actions::{self, ACTIONS, PAGE_KEYS};
use gtk::prelude::*;

use crate::config;
use crate::state::TabId;
use crate::window::{self, Layout, Ui};
use crate::{blocker, theme};

pub const SETTINGS_URL: &str = "stales://settings";
pub const SETUP_URL: &str = "stales://setup";

/// HTML for a stales:// page.
pub fn render_page(ui: &Ui, name: &str) -> Option<String> {
    match name {
        "settings" | "setup" => {
            let state = serde_json::to_string(&state(ui)).unwrap_or_default().replace("</", "<\\/");
            Some(
                include_str!("settings.html")
                    .replace("{mode}", name)
                    .replace("{state}", &state)
                    .replace("{title}", if name == "setup" { "Set up Stales" } else { "Settings" }),
            )
        }
        other => crate::welcome::render(other, &ui.config.borrow()),
    }
}

/// Ctrl+, : show the settings tab (reusing an open one).
pub fn open(ui: &Rc<Ui>) {
    let existing = {
        let state = ui.state.borrow();
        let space = state.space();
        let pinned = space.pinned.iter().position(|t| t.url.starts_with(SETTINGS_URL));
        let tab = space.tabs.iter().position(|t| t.url.starts_with(SETTINGS_URL));
        pinned.map(crate::state::Selected::Pinned).or(tab.map(crate::state::Selected::Tab))
    };
    match existing {
        Some(selected) => {
            ui.state.borrow_mut().select(selected);
        }
        None => ui.state.borrow_mut().open_tab(SETTINGS_URL),
    }
    window::refresh(ui);
    window::focus_page(ui);
}

/// Change config.toml and use the new settings. Shows a message if it didn't work.
pub fn save(ui: &Rc<Ui>, change: impl FnOnce(&mut toml_edit::DocumentMut)) -> bool {
    match config::edit(change) {
        Ok(new) => {
            ui.config.replace(new);
            true
        }
        Err(e) => {
            eprintln!("stales: {e}");
            window::toast(ui, "Could not save the setting (see the terminal)");
            false
        }
    }
}

/// Older versions kept theme and layout in ui.toml; move them into config.toml once.
pub fn move_old_choices_to_config(ui: &Rc<Ui>) {
    let (theme, layout) = {
        let mut saved = ui.saved.borrow_mut();
        (saved.theme.take(), saved.layout.take())
    };
    if theme.is_none() && layout.is_none() {
        return;
    }
    save(ui, |doc| {
        if let Some(theme) = &theme {
            config::set(doc, &["appearance"], "theme", theme.as_str().into());
        }
        if let Some(layout) = &layout {
            config::set(doc, &["appearance"], "layout", layout.as_str().into());
        }
    });
    crate::saved::store(&ui.saved.borrow());
}

/// Everything the page shows.
fn state(ui: &Ui) -> Value {
    let config = ui.config.borrow();
    let state = ui.state.borrow();
    let saved = ui.saved.borrow();
    let custom: Vec<&str> = config.search.custom.iter().map(|e| e.keyword.as_str()).collect();
    let page_keys = actions::page_keys(&config.keys);

    json!({
        "theme": ui.themes.current.borrow().clone(),
        "themes": theme::names().iter().map(|n| theme::preview(n)).collect::<Vec<_>>(),
        "layout": ui.layout.get().name(),
        "engine": state.search.default_engine().name,
        "engines": state.search.engines.iter().map(|e| json!({
            "name": e.name, "keyword": e.keyword, "url": e.url, "custom": custom.contains(&e.keyword.as_str()),
        })).collect::<Vec<_>>(),
        "vim": config.keys.vim,
        "exclude": config.keys.exclude,
        "actions": ACTIONS.iter().map(|a| json!({
            "name": a.name, "group": a.group, "description": a.description,
            "keys": actions::keys_for(&config.keys, a),
            "defaults": a.keys,
            "pretty": actions::keys_for(&config.keys, a).iter().map(|k| actions::pretty_key(k)).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "page_actions": PAGE_KEYS.iter().map(|(key, action, description)| json!({
            "action": action, "description": description, "default": key,
            "key": page_keys.keys.iter().find(|(_, a)| a == action).map(|(k, _)| k.clone()),
        })).collect::<Vec<_>>(),
        "blocker": {
            "enabled": config.blocker.enabled,
            "lists": config.blocker.lists,
            "status": ui.blocker_status.borrow().describe(),
            "allowed": config.blocker.allow.iter().map(|s| json!({"site": s, "from": "config"}))
                .chain(saved.blocker_allow.iter().map(|s| json!({"site": s, "from": "shield"})))
                .collect::<Vec<_>>(),
        },
        "passwords": {"enabled": config.passwords.enabled},
        "spaces": state.spaces.iter().map(|s| json!({"name": s.name, "icon": s.icon, "color": s.color})).collect::<Vec<_>>(),
        "current_space": state.current,
        "colors": theme::SPACE_COLORS,
        "sidebar": {"width": saved.sidebar_width, "compact": saved.sidebar_compact},
        "paths": {
            "config": config::path().to_string_lossy(),
            "user_css": gtk::glib::user_config_dir().join("stales/user.css").to_string_lossy(),
            "themes": gtk::glib::user_config_dir().join("stales/themes").to_string_lossy(),
            "data": gtk::glib::user_data_dir().join("stales").to_string_lossy(),
        },
    })
}

/// A message from the settings page in tab `id`.
pub fn handle(ui: &Rc<Ui>, id: TabId, message: &str) {
    let Ok(msg) = serde_json::from_str::<Value>(message) else { return };
    let text = |key: &str| msg.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let flag = |key: &str| msg.get(key).and_then(Value::as_bool).unwrap_or(false);
    let list = |key: &str| -> Vec<String> {
        msg.get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
            })
            .unwrap_or_default()
    };

    let mut note: Option<String> = None;
    match msg.get("cmd").and_then(Value::as_str).unwrap_or_default() {
        "get" => {}
        "set-theme" => {
            theme::set(ui, &text("name"));
        }
        "set-layout" => {
            if let Some(layout) = Layout::from_name(&text("name")) {
                window::set_layout(ui, layout);
            }
        }
        "set-engine" => {
            let name = text("name");
            if ui.state.borrow().search.engines.iter().any(|e| e.name == name)
                && save(ui, |doc| config::set(doc, &["search"], "engine", name.as_str().into()))
            {
                reload_search(ui);
            }
        }
        "set-custom-engines" => {
            let engines: Vec<(String, String, String)> = msg
                .get("engines")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|e| {
                            let field =
                                |k: &str| e.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_string();
                            (field("name"), field("keyword"), field("url"))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if let Some((name, ..)) = engines.iter().find(|(n, k, u)| n.is_empty() || k.is_empty() || !u.contains("%s"))
            {
                note = Some(format!("“{name}” needs a name, a shortcut and %s in its address"));
            } else if save(ui, |doc| {
                let mut array = toml_edit::ArrayOfTables::new();
                for (name, keyword, url) in &engines {
                    let mut table = toml_edit::Table::new();
                    table.insert("name", toml_edit::value(name.as_str()));
                    table.insert("keyword", toml_edit::value(keyword.as_str()));
                    table.insert("url", toml_edit::value(url.as_str()));
                    array.push(table);
                }
                config::table(doc, &["search"]).insert("custom", toml_edit::Item::ArrayOfTables(array));
            }) {
                reload_search(ui);
            }
        }
        "set-vim" => {
            let on = flag("on");
            if save(ui, |doc| config::set(doc, &["keys"], "vim", on.into())) {
                reload_page_keys(ui);
                note = Some("Open tabs use this after you reload them".into());
            }
        }
        "set-exclude" => {
            let sites = list("sites");
            if save(ui, |doc| config::set(doc, &["keys"], "exclude", config::string_list(&sites))) {
                reload_page_keys(ui);
            }
        }
        "record" => actions::pause(ui, flag("on")),
        "set-binding" => {
            let name = text("action");
            let mut keys = list("keys");
            let Some(action) = ACTIONS.iter().find(|a| a.name == name) else { return };
            if let Some(bad) = keys.iter().find(|k| !valid_key(k)) {
                note = Some(format!("“{bad}” isn't a key GTK understands"));
            } else {
                // GTK knows Shift+Tab under a second name; add it so it always works.
                let extra: Vec<String> = keys
                    .iter()
                    .filter(|k| k.contains("<Shift>") && k.ends_with("Tab"))
                    .map(|k| k.replace("Tab", "ISO_Left_Tab"))
                    .filter(|k| !keys.contains(k))
                    .collect();
                keys.extend(extra);
                let is_default = keys.iter().map(String::as_str).eq(action.keys.iter().copied());
                if save(ui, |doc| {
                    let table = config::table(doc, &["keys", "bindings"]);
                    if is_default {
                        table.remove(&name);
                    } else {
                        table.insert(&name, toml_edit::Item::Value(config::string_list(&keys)));
                    }
                }) {
                    actions::install(ui);
                }
            }
        }
        "set-page-key" => {
            // One single-key command: `action` gets `key` ("" turns it off).
            let (action, key) =
                (text("action"), msg.get("key").and_then(Value::as_str).unwrap_or_default().to_string());
            if !PAGE_KEYS.iter().any(|(_, a, _)| *a == action) {
                return;
            }
            let mut map = actions::page_keys(&ui.config.borrow().keys).keys;
            map.retain(|_, a| *a != action);
            if !key.is_empty() {
                map.insert(key, action);
            }
            if save(ui, |doc| write_page_keys(doc, &map)) {
                reload_page_keys(ui);
                note = Some("Open tabs use this after you reload them".into());
            }
        }
        "set-blocker" => {
            let on = flag("on");
            if save(ui, |doc| config::set(doc, &["blocker"], "enabled", on.into())) {
                if on {
                    blocker::start(ui);
                } else {
                    ui.engine.clear_blocklist();
                    blocker::start(ui); // shows "off"
                }
            }
        }
        "set-lists" => {
            let lists = list("lists");
            if save(ui, |doc| config::set(doc, &["blocker"], "lists", config::string_list(&lists))) {
                blocker::update(ui);
                note = Some("Downloading the new lists…".into());
            }
        }
        "update-lists" => {
            blocker::update(ui);
            note = Some("Downloading the lists…".into());
        }
        "remove-allowed" => {
            let site = text("site");
            if text("from") == "config" {
                let mut allow = ui.config.borrow().blocker.allow.clone();
                allow.retain(|s| *s != site);
                save(ui, |doc| config::set(doc, &["blocker"], "allow", config::string_list(&allow)));
            } else {
                let mut saved = ui.saved.borrow_mut();
                saved.blocker_allow.retain(|s| *s != site);
                crate::saved::store(&saved);
            }
        }
        "get-logins" => {
            // The list comes from the keyring (it may ask to unlock it first).
            crate::passwords::send_list(ui, id);
            return;
        }
        "copy-password" => {
            crate::passwords::copy(ui, &text("origin"), &text("username"));
            return;
        }
        "delete-login" => {
            let weak = Rc::downgrade(ui);
            crate::passwords::delete(&text("origin"), &text("username"), move || {
                if let Some(ui) = weak.upgrade() {
                    crate::passwords::send_list(&ui, id);
                    window::toast(&ui, "Login deleted");
                }
            });
            return;
        }
        "remove-never" => {
            let site = text("site");
            ui.saved.borrow_mut().password_never.retain(|s| *s != site);
            crate::saved::store(&ui.saved.borrow());
            crate::passwords::send_list(ui, id);
            return;
        }
        "set-passwords" => {
            let on = flag("on");
            save(ui, |doc| config::set(doc, &["passwords"], "enabled", on.into()));
        }
        "clear-data" => {
            let weak = Rc::downgrade(ui);
            ui.engine.clear_site_data(Box::new(move |result| {
                if let Some(ui) = weak.upgrade() {
                    let text = match result {
                        Ok(()) => "Cookies, cache and site data deleted".to_string(),
                        Err(e) => format!("Could not delete everything: {e}"),
                    };
                    window::toast(&ui, &text);
                }
            }));
            note = Some("Deleting…".into());
        }
        "set-space" => {
            let index = msg.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX) as usize;
            let (name, icon, color) = (text("name"), text("icon"), text("color"));
            let mut state = ui.state.borrow_mut();
            if let Some(space) = state.spaces.get_mut(index) {
                if !name.is_empty() {
                    space.name = name.chars().take(40).collect();
                }
                if !icon.is_empty() {
                    space.icon = icon.chars().take(2).collect();
                }
                space.color = match color.as_str() {
                    "" | "none" => None,
                    c if theme::is_space_color(c) => Some(c.to_string()),
                    _ => space.color.clone(),
                };
            }
            drop(state);
            window::refresh(ui);
        }
        "set-sidebar" => {
            if let Some(width) = msg.get("width").and_then(Value::as_i64) {
                window::set_sidebar_width(ui, width as i32);
            }
        }
        "open-folder" => {
            let path = match text("which").as_str() {
                "themes" => gtk::glib::user_config_dir().join("stales/themes"),
                "data" => gtk::glib::user_data_dir().join("stales"),
                _ => gtk::glib::user_config_dir().join("stales"),
            };
            let uri = gtk::gio::File::for_path(&path).uri();
            let _ = gtk::gio::AppInfo::launch_default_for_uri(&uri, None::<&gtk::gio::AppLaunchContext>);
        }
        "finish-setup" => {
            ui.state.borrow_mut().navigate_active(crate::welcome::WELCOME_URL);
            window::load_active(ui);
            return;
        }
        _ => return,
    }
    reply(ui, id, note);
}

/// Send the page the new state (and maybe a short note to show).
fn reply(ui: &Rc<Ui>, id: TabId, note: Option<String>) {
    let mut data = state(ui);
    data["note"] = note.map_or(Value::Null, Value::String);
    let script = format!("window.stalesUpdate && window.stalesUpdate({data})");
    let page = ui.pages.borrow().get(&id).cloned();
    if let Some(page) = page {
        page.run_script(&script);
    }
}

fn reload_search(ui: &Rc<Ui>) {
    let search = ui.config.borrow().search();
    ui.state.borrow_mut().search = search;
}

fn reload_page_keys(ui: &Rc<Ui>) {
    ui.engine.set_page_keys(&actions::page_keys(&ui.config.borrow().keys));
}

/// Write the single-key map as differences from the defaults, so config.toml stays short.
fn write_page_keys(doc: &mut toml_edit::DocumentMut, map: &std::collections::HashMap<String, String>) {
    let table = config::table(doc, &["keys", "page"]);
    table.clear();
    for (key, action, _) in PAGE_KEYS {
        if map.get(*key).map(String::as_str) != Some(*action) && !map.contains_key(*key) {
            table.insert(key, toml_edit::value("none"));
        }
    }
    let mut changed: Vec<(&String, &String)> =
        map.iter().filter(|(k, a)| !PAGE_KEYS.iter().any(|(dk, da, _)| dk == k && da == a)).collect();
    changed.sort();
    for (key, action) in changed {
        table.insert(key, toml_edit::value(action.as_str()));
    }
}

/// Does GTK understand this key combo?
fn valid_key(key: &str) -> bool {
    gtk::ShortcutTrigger::parse_string(key).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_keys_are_written_as_differences() {
        let mut doc: toml_edit::DocumentMut = "".parse().unwrap();
        let mut map: std::collections::HashMap<String, String> =
            PAGE_KEYS.iter().map(|(k, a, _)| (k.to_string(), a.to_string())).collect();
        // Nothing changed: nothing written.
        write_page_keys(&mut doc, &map);
        assert!(!doc.to_string().contains('='));

        // Close tab moves from x to q.
        map.remove("x");
        map.insert("q".into(), "close-tab".into());
        write_page_keys(&mut doc, &map);
        let text = doc.to_string();
        assert!(text.contains("x = \"none\""));
        assert!(text.contains("q = \"close-tab\""));

        let config: crate::config::Config = toml::from_str(&text).unwrap();
        let keys = actions::page_keys(&config.keys).keys;
        assert_eq!(keys.get("q").map(String::as_str), Some("close-tab"));
        assert!(!keys.contains_key("x"));
    }
}
