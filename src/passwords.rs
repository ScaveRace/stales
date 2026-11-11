//! The password manager.
//!
//! Passwords are stored in your system keyring (the "Secret Service": GNOME Keyring,
//! KWallet, or KeePassXC with its Secret Service option), which keeps them encrypted
//! with your login. Stales never writes a password to its own files.
//!
//! - Saving: passwords.js notices when you log in on a page and asks; a bar offers
//!   "Save" / "Never for this site".
//! - Filling: only when you pick an account from the suggestion under the field, or
//!   press Ctrl+Shift+L. Only on the exact same site (scheme + host + port), only in
//!   the page itself (not in frames from other sites), only over https (or localhost).
//!   A page's own scripts can't trigger filling or see other sites' logins.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use libsecret::prelude::*;
use libsecret::{Schema, SchemaAttributeType, SchemaFlags, SearchFlags};
use serde_json::{Value, json};

use crate::state::TabId;
use crate::window::{self, Ui};

/// A login waiting for you to choose "Save" in the bar.
pub struct PendingLogin {
    pub origin: String,
    pub username: String,
    pub password: String,
    /// The keyring already has this login with a different password.
    pub update: bool,
}

fn schema() -> Schema {
    let attributes =
        HashMap::from([("origin", SchemaAttributeType::String), ("username", SchemaAttributeType::String)]);
    Schema::new("dev.stales.Login", SchemaFlags::NONE, attributes)
}

/// Only offer saving and filling where the connection is encrypted.
pub fn is_secure(origin: &str) -> bool {
    origin.starts_with("https://") || origin.starts_with("http://localhost") || origin.starts_with("http://127.0.0.1")
}

/// "https://github.com:443/login?x" → "https://github.com"; the part that identifies a site.
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if host.is_empty() {
        return None;
    }
    let host = host.strip_suffix(":443").filter(|_| scheme == "https").unwrap_or(host);
    Some(format!("{scheme}://{}", host.to_lowercase()))
}

// ---------- The keyring ----------

/// Usernames saved for a site.
pub fn accounts(origin: &str, done: impl FnOnce(Vec<String>) + 'static) {
    let attributes = HashMap::from([("origin", origin)]);
    libsecret::password_search(
        Some(&schema()),
        attributes,
        SearchFlags::ALL | SearchFlags::UNLOCK,
        None::<&gtk::gio::Cancellable>,
        move |result| {
            let mut names: Vec<String> = result
                .unwrap_or_default()
                .iter()
                .filter_map(|item| item.attributes().get("username").cloned())
                .collect();
            names.sort();
            names.dedup();
            done(names);
        },
    );
}

/// Every saved login, for the settings page: (site, username).
pub fn all(done: impl FnOnce(Vec<(String, String)>) + 'static) {
    libsecret::password_search(
        Some(&schema()),
        HashMap::new(),
        SearchFlags::ALL | SearchFlags::UNLOCK,
        None::<&gtk::gio::Cancellable>,
        move |result| {
            let mut logins: Vec<(String, String)> = result
                .unwrap_or_default()
                .iter()
                .filter_map(|item| {
                    let attributes = item.attributes();
                    Some((attributes.get("origin")?.clone(), attributes.get("username")?.clone()))
                })
                .collect();
            logins.sort();
            done(logins);
        },
    );
}

pub fn password(origin: &str, username: &str, done: impl FnOnce(Option<String>) + 'static) {
    let attributes = HashMap::from([("origin", origin), ("username", username)]);
    libsecret::password_lookup(Some(&schema()), attributes, None::<&gtk::gio::Cancellable>, move |result| {
        done(result.ok().flatten().map(|p| p.to_string()));
    });
}

fn store(login: &PendingLogin, done: impl FnOnce(Result<(), String>) + 'static) {
    let attributes = HashMap::from([("origin", login.origin.as_str()), ("username", login.username.as_str())]);
    let label = format!("Stales: {} ({})", login.origin.split("://").nth(1).unwrap_or(&login.origin), login.username);
    libsecret::password_store(
        Some(&schema()),
        attributes,
        None, // the default keyring
        &label,
        &login.password,
        None::<&gtk::gio::Cancellable>,
        move |result| done(result.map_err(|e| e.to_string())),
    );
}

pub fn delete(origin: &str, username: &str, done: impl FnOnce() + 'static) {
    let attributes = HashMap::from([("origin", origin), ("username", username)]);
    libsecret::password_clear(Some(&schema()), attributes, None::<&gtk::gio::Cancellable>, move |_| done());
}

// ---------- Messages from passwords.js ----------

/// A JSON message from the password script in tab `id`.
pub fn handle(ui: &Rc<Ui>, id: TabId, message: &str) {
    let Ok(msg) = serde_json::from_str::<Value>(message) else { return };
    let text = |key: &str| msg.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let origin = text("origin");

    // The script runs inside the page, so check its claim: the message must be about
    // the site this tab is showing.
    let tab_origin = ui.state.borrow().active_tab().and_then(|t| origin_of(&t.url));
    if tab_origin.as_deref() != Some(origin.as_str()) || !is_secure(&origin) || !enabled(ui) {
        return;
    }

    match text("type").as_str() {
        // A page with a login form loaded: tell it which accounts exist (names only).
        "logins-wanted" => {
            let weak = Rc::downgrade(ui);
            accounts(&origin, move |names| {
                if let Some(ui) = weak.upgrade()
                    && !names.is_empty()
                {
                    send_to_page(&ui, id, &format!("window.__stalesLogins && window.__stalesLogins({})", json!(names)));
                }
            });
        }
        // You picked an account in the suggestion.
        "fill" => fill(ui, id, &origin, &text("username")),
        // You logged in: maybe offer to save.
        "submitted" => {
            let (username, password) = (text("username"), text("password"));
            if password.is_empty() || ui.saved.borrow().password_never.contains(&origin) {
                return;
            }
            let weak = Rc::downgrade(ui);
            let (o, u) = (origin.clone(), username.clone());
            self::password(&origin, &username, move |existing| {
                let Some(ui) = weak.upgrade() else { return };
                if existing.as_deref() == Some(password.as_str()) {
                    return; // already saved, nothing to ask
                }
                let update = existing.is_some();
                ask_to_save(&ui, PendingLogin { origin: o, username: u, password, update });
            });
        }
        _ => {}
    }
}

fn enabled(ui: &Ui) -> bool {
    ui.config.borrow().passwords.enabled
}

/// Look up the password and put it into the page's login fields.
fn fill(ui: &Rc<Ui>, id: TabId, origin: &str, username: &str) {
    let weak = Rc::downgrade(ui);
    let user = username.to_string();
    password(origin, username, move |found| {
        let (Some(ui), Some(password)) = (weak.upgrade(), found) else { return };
        let script = format!("window.__stalesFill && window.__stalesFill({}, {})", json!(user), json!(password));
        send_to_page(&ui, id, &script);
    });
}

/// Ctrl+Shift+L: fill the login for this site (the only one, or let you choose).
pub fn fill_shortcut(ui: &Rc<Ui>) {
    let target = {
        let state = ui.state.borrow();
        state.active_tab().map(|t| (t.id, origin_of(&t.url)))
    };
    let Some((id, Some(origin))) = target else { return };
    if !is_secure(&origin) {
        window::toast(ui, "Logins are only filled on secure (https) sites");
        return;
    }
    let weak = Rc::downgrade(ui);
    let site = origin.clone();
    accounts(&site, move |names| {
        let Some(ui) = weak.upgrade() else { return };
        match names.as_slice() {
            [] => window::toast(&ui, "No saved login for this site"),
            [only] => fill(&ui, id, &origin, only),
            many => send_to_page(&ui, id, &format!("window.__stalesChoose && window.__stalesChoose({})", json!(many))),
        }
    });
}

/// Run code in the hidden script world of a tab (where passwords.js lives).
fn send_to_page(ui: &Ui, id: TabId, script: &str) {
    let page = ui.pages.borrow().get(&id).cloned();
    if let Some(page) = page {
        page.run_hidden_script(script);
    }
}

// ---------- The "Save login?" bar ----------

fn ask_to_save(ui: &Rc<Ui>, login: PendingLogin) {
    let site = login.origin.split("://").nth(1).unwrap_or(&login.origin).to_string();
    let who = if login.username.is_empty() { String::new() } else { format!(" as {}", login.username) };
    let question = if login.update {
        format!("Update the saved password for {site}{who}?")
    } else {
        format!("Save the login for {site}{who}?")
    };
    ui.password_bar.label.set_label(&question);
    ui.password_bar.save.set_label(if login.update { "Update" } else { "Save" });
    ui.pending_login.replace(Some(login));
    ui.password_bar.root.set_visible(true);
}

/// The bar's "Save" (or Ctrl+Shift+Enter).
pub fn save_pending(ui: &Rc<Ui>) {
    let Some(login) = ui.pending_login.take() else { return };
    ui.password_bar.root.set_visible(false);
    let weak = Rc::downgrade(ui);
    store(&login, move |result| {
        if let Some(ui) = weak.upgrade() {
            match result {
                Ok(()) => window::toast(&ui, "Login saved in your keyring"),
                Err(e) => {
                    eprintln!("stales: could not save the login: {e}");
                    window::toast(&ui, "Could not save: is a keyring (GNOME Keyring, KWallet, KeePassXC) running?");
                }
            }
        }
    });
}

/// The bar's "Never for this site".
pub fn never_for_site(ui: &Rc<Ui>) {
    let Some(login) = ui.pending_login.take() else { return };
    ui.password_bar.root.set_visible(false);
    let mut saved = ui.saved.borrow_mut();
    if !saved.password_never.contains(&login.origin) {
        saved.password_never.push(login.origin);
    }
    crate::saved::store(&saved);
}

pub fn dismiss(ui: &Rc<Ui>) {
    ui.pending_login.take();
    ui.password_bar.root.set_visible(false);
}

pub struct PasswordBar {
    pub root: gtk::Box,
    pub label: gtk::Label,
    pub save: gtk::Button,
    never: gtk::Button,
    close: gtk::Button,
}

pub fn build_bar() -> PasswordBar {
    let icon = gtk::Image::from_icon_name("dialog-password-symbolic");
    let label = gtk::Label::new(None);
    label.set_hexpand(true);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(1);
    let save = gtk::Button::with_label("Save");
    save.add_css_class("st-prompt-main");
    save.set_tooltip_text(Some("Ctrl+Shift+Enter"));
    let never = gtk::Button::with_label("Never for this site");
    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.add_css_class("st-icon-btn");
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    root.add_css_class("st-prompt");
    root.set_valign(gtk::Align::Start);
    root.set_halign(gtk::Align::Center);
    root.set_margin_top(12);
    root.set_width_request(680);
    for widget in [
        icon.upcast_ref::<gtk::Widget>(),
        label.upcast_ref(),
        save.upcast_ref(),
        never.upcast_ref(),
        close.upcast_ref(),
    ] {
        root.append(widget);
    }
    root.set_visible(false);
    PasswordBar { root, label, save, never, close }
}

pub fn connect_bar(ui: &Rc<Ui>) {
    let u = ui.clone();
    ui.password_bar.save.connect_clicked(move |_| save_pending(&u));
    let u = ui.clone();
    ui.password_bar.never.connect_clicked(move |_| never_for_site(&u));
    let u = ui.clone();
    ui.password_bar.close.connect_clicked(move |_| dismiss(&u));
}

// ---------- The settings page ----------

/// Send the list of saved logins to the settings page in tab `id`.
pub fn send_list(ui: &Rc<Ui>, id: TabId) {
    let weak = Rc::downgrade(ui);
    all(move |logins| {
        let Some(ui) = weak.upgrade() else { return };
        let never = ui.saved.borrow().password_never.clone();
        let data = json!({
            "logins": logins.iter().map(|(o, u)| json!({"origin": o, "username": u})).collect::<Vec<_>>(),
            "never": never,
        });
        let page = ui.pages.borrow().get(&id).cloned();
        if let Some(page) = page {
            page.run_script(&format!("window.stalesPasswords && window.stalesPasswords({data})"));
        }
    });
}

/// Copy a saved password, and clear the clipboard again after 30 seconds.
pub fn copy(ui: &Rc<Ui>, origin: &str, username: &str) {
    let weak = Rc::downgrade(ui);
    password(origin, username, move |found| {
        let (Some(ui), Some(password)) = (weak.upgrade(), found) else { return };
        let clipboard = ui.window.clipboard();
        clipboard.set_text(&password);
        window::toast(&ui, "Password copied; cleared from the clipboard in 30 seconds");
        glib::timeout_add_local_once(Duration::from_secs(30), move || {
            // Only clear it if it still holds the password (you may have copied something else).
            let clip = clipboard.clone();
            clipboard.read_text_async(None::<&gtk::gio::Cancellable>, move |text| {
                if text.ok().flatten().as_deref() == Some(password.as_str()) {
                    clip.set_text("");
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_identify_a_site_exactly() {
        assert_eq!(origin_of("https://GitHub.com/login?return=1").as_deref(), Some("https://github.com"));
        assert_eq!(origin_of("https://github.com:443/").as_deref(), Some("https://github.com"));
        assert_eq!(origin_of("https://github.com:8443/").as_deref(), Some("https://github.com:8443"));
        assert_eq!(origin_of("http://localhost:8000/a").as_deref(), Some("http://localhost:8000"));
        assert_eq!(origin_of("stales://settings"), Some("stales://settings".into()));
        assert_eq!(origin_of("about:blank"), None);
        // A look-alike is a different site.
        assert_ne!(origin_of("https://github.com.evil.example/"), origin_of("https://github.com/"));
    }

    #[test]
    fn only_secure_sites() {
        assert!(is_secure("https://github.com"));
        assert!(is_secure("http://localhost:8000"));
        assert!(!is_secure("http://example.com"));
        assert!(!is_secure("stales://settings"));
    }
}
