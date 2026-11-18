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
use libsecret::{
    Collection, CollectionCreateFlags, CollectionFlags, Schema, SchemaAttributeType, SchemaFlags, SearchFlags,
    ServiceFlags,
};
use serde_json::json;

use crate::state::TabId;
use crate::system_auth::{self, Outcome};
use crate::window::{self, Ui};

/// A password in memory. Overwritten with zeros as soon as it is dropped, so it
/// doesn't linger in Stales' memory after use.
pub type Secret = zeroize::Zeroizing<String>;

/// A message from passwords.js. The password field is a `Secret`, so even the copy
/// made while reading the message is wiped.
#[derive(serde::Deserialize)]
struct ScriptMessage {
    #[serde(rename = "type")]
    kind: String,
    origin: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: Secret,
}

/// A login waiting for you to choose "Save" in the bar.
pub struct PendingLogin {
    pub origin: String,
    pub username: String,
    pub password: Secret,
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

/// Usernames saved for a site. With `unlock`, a locked keyring asks for its password;
/// without, locked logins are simply not found.
pub fn accounts(origin: &str, unlock: bool, done: impl FnOnce(Vec<String>) + 'static) {
    let attributes = HashMap::from([("origin", origin)]);
    let flags = if unlock { SearchFlags::ALL | SearchFlags::UNLOCK } else { SearchFlags::ALL };
    libsecret::password_search(Some(&schema()), attributes, flags, None::<&gtk::gio::Cancellable>, move |result| {
        let mut names: Vec<String> =
            result.unwrap_or_default().iter().filter_map(|item| item.attributes().get("username").cloned()).collect();
        names.sort();
        names.dedup();
        done(names);
    });
}

/// Every saved login, for the settings page: (site, username).
pub fn all(unlock: bool, done: impl FnOnce(Vec<(String, String)>) + 'static) {
    let flags = if unlock { SearchFlags::ALL | SearchFlags::UNLOCK } else { SearchFlags::ALL };
    libsecret::password_search(Some(&schema()), HashMap::new(), flags, None::<&gtk::gio::Cancellable>, move |result| {
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
    });
}

pub fn password(origin: &str, username: &str, done: impl FnOnce(Option<Secret>) + 'static) {
    let attributes = HashMap::from([("origin", origin), ("username", username)]);
    libsecret::password_lookup(Some(&schema()), attributes, None::<&gtk::gio::Cancellable>, move |result| {
        done(result.ok().flatten().map(|p| Secret::new(p.to_string())));
    });
}

fn label_for(origin: &str, username: &str) -> String {
    format!("Stales: {} ({username})", origin.split("://").nth(1).unwrap_or(origin))
}

/// Save a login: in Stales' own keyring (`separate`), or in your default keyring.
fn store(login: &PendingLogin, separate: bool, done: impl FnOnce(Result<(), String>) + 'static) {
    let attributes = HashMap::from([("origin", login.origin.as_str()), ("username", login.username.as_str())]);
    let label = label_for(&login.origin, &login.username);
    libsecret::password_store(
        Some(&schema()),
        attributes,
        separate.then_some(VAULT_ALIAS),
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
    let Ok(msg) = serde_json::from_str::<ScriptMessage>(message) else { return };
    let origin = msg.origin.clone();

    // The script runs inside the page, so check its claim: the message must be about
    // the site this tab is showing.
    let tab_origin = ui.state.borrow().active_tab().and_then(|t| origin_of(&t.url));
    if tab_origin.as_deref() != Some(origin.as_str()) || !is_secure(&origin) || !enabled(ui) {
        return;
    }

    match msg.kind.as_str() {
        // A page with a login form loaded: tell it which accounts exist (names only).
        "logins-wanted" => {
            let weak = Rc::downgrade(ui);
            // With the separate keyring locked: no suggestions, and no password dialog
            // on every login page. Ctrl+Shift+L unlocks it when you actually want a login.
            accounts(&origin, !separate(ui), move |names| {
                if let Some(ui) = weak.upgrade()
                    && !names.is_empty()
                {
                    send_to_page(&ui, id, &format!("window.__stalesLogins && window.__stalesLogins({})", json!(names)));
                }
            });
        }
        // You picked an account in the suggestion.
        "fill" => fill(ui, id, &origin, &msg.username),
        // You logged in: maybe offer to save.
        "submitted" => {
            let (username, password) = (msg.username, msg.password);
            if password.is_empty() || ui.saved.borrow().password_never.contains(&origin) {
                return;
            }
            if separate(ui) {
                // Checking whether it's saved already would ask to unlock the keyring
                // after every login; just offer to save (saving the same again is harmless).
                ask_to_save(ui, PendingLogin { origin, username, password, update: false });
                return;
            }
            let weak = Rc::downgrade(ui);
            let (o, u) = (origin.clone(), username.clone());
            self::password(&origin, &username, move |existing| {
                let Some(ui) = weak.upgrade() else { return };
                if existing.as_ref().map(|e| e.as_str()) == Some(password.as_str()) {
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
    let (origin, user) = (origin.to_string(), username.to_string());
    after_confirming(ui, move |ui| fill_now(ui, id, &origin, &user));
}

fn fill_now(ui: &Rc<Ui>, id: TabId, origin: &str, username: &str) {
    let weak = Rc::downgrade(ui);
    let user = username.to_string();
    password(origin, username, move |found| {
        let (Some(ui), Some(password)) = (weak.upgrade(), found) else { return };
        lock_later(&ui);
        let quoted = Secret::new(serde_json::to_string(password.as_str()).unwrap_or_default());
        let script = Secret::new(format!("window.__stalesFill && window.__stalesFill({}, {})", json!(user), *quoted));
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
    accounts(&site, true, move |names| {
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
    let key = (login.origin.clone(), login.username.clone());
    ui.pending_login.replace(Some(login));
    ui.password_bar.root.set_visible(true);
    // Ignored for 2 minutes: forget it (and wipe the password).
    let weak = Rc::downgrade(ui);
    glib::timeout_add_local_once(Duration::from_secs(120), move || {
        if let Some(ui) = weak.upgrade() {
            let same =
                ui.pending_login.borrow().as_ref().is_some_and(|p| (p.origin.clone(), p.username.clone()) == key);
            if same {
                dismiss(&ui);
            }
        }
    });
}

/// The bar's "Save" (or Ctrl+Shift+Enter).
pub fn save_pending(ui: &Rc<Ui>) {
    let Some(login) = ui.pending_login.take() else { return };
    ui.password_bar.root.set_visible(false);
    let weak = Rc::downgrade(ui);
    let separate = separate(ui);
    gtk::glib::spawn_future_local(async move {
        // The separate keyring is made the first time (your system asks for its password).
        if separate && let Err(e) = vault(true).await {
            eprintln!("stales: could not open the Stales keyring: {e}");
            if let Some(ui) = weak.upgrade() {
                window::toast(&ui, "Could not open the Stales keyring");
            }
            return;
        }
        store(&login, separate, move |result| {
            if let Some(ui) = weak.upgrade() {
                lock_later(&ui);
                match result {
                    Ok(()) => window::toast(&ui, "Login saved in your keyring"),
                    Err(e) => {
                        eprintln!("stales: could not save the login: {e}");
                        window::toast(&ui, "Could not save: is a keyring (GNOME Keyring, KWallet, KeePassXC) running?");
                    }
                }
            }
        });
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

/// Send the list of saved logins to the settings page in tab `id`. With `unlock`, a
/// locked Stales keyring asks for its password first.
pub fn send_list(ui: &Rc<Ui>, id: TabId, unlock: bool) {
    let weak = Rc::downgrade(ui);
    let separate = separate(ui);
    gtk::glib::spawn_future_local(async move {
        let locked = separate && vault(false).await.ok().flatten().is_some_and(|c| c.is_locked());
        let unlock = unlock && locked;
        all(unlock, move |logins| {
            let Some(ui) = weak.upgrade() else { return };
            if unlock {
                lock_later(&ui);
            }
            let never = ui.saved.borrow().password_never.clone();
            let data = json!({
                "logins": logins.iter().map(|(o, u)| json!({"origin": o, "username": u})).collect::<Vec<_>>(),
                "never": never,
                "locked": locked && !unlock,
            });
            let page = ui.pages.borrow().get(&id).cloned();
            if let Some(page) = page {
                page.run_script(&format!("window.stalesPasswords && window.stalesPasswords({data})"));
            }
        });
    });
}

/// Copy a saved password, and clear the clipboard again after 30 seconds.
pub fn copy(ui: &Rc<Ui>, origin: &str, username: &str) {
    let (origin, user) = (origin.to_string(), username.to_string());
    after_confirming(ui, move |ui| copy_now(ui, &origin, &user));
}

fn copy_now(ui: &Rc<Ui>, origin: &str, username: &str) {
    let weak = Rc::downgrade(ui);
    password(origin, username, move |found| {
        let (Some(ui), Some(password)) = (weak.upgrade(), found) else { return };
        lock_later(&ui);
        let clipboard = ui.window.clipboard();
        // Marked as secret, so clipboard history tools (cliphist, Klipper, …) don't keep it.
        let text = gtk::glib::Bytes::from(password.as_bytes());
        let content = gtk::gdk::ContentProvider::new_union(&[
            gtk::gdk::ContentProvider::for_bytes("text/plain;charset=utf-8", &text),
            gtk::gdk::ContentProvider::for_bytes("text/plain", &text),
            gtk::gdk::ContentProvider::for_bytes(
                "x-kde-passwordManagerHint",
                &gtk::glib::Bytes::from_static(b"secret"),
            ),
        ]);
        let _ = clipboard.set_content(Some(&content));
        window::toast(&ui, "Password copied; cleared from the clipboard in 30 seconds");
        glib::timeout_add_local_once(Duration::from_secs(30), move || {
            // Only clear it if it still holds the password (you may have copied something else).
            let clip = clipboard.clone();
            clipboard.read_text_async(None::<&gtk::gio::Cancellable>, move |text| {
                let current = text.ok().flatten().map(|t| Secret::new(t.to_string()));
                if current.as_ref().map(|c| c.as_str()) == Some(password.as_str()) {
                    clip.set_text("");
                }
            });
        });
    });
}

// ---------- "Confirm it's you" ----------

/// Run `then` once you've confirmed with your login password or fingerprint (when
/// `ask_system_password` is on; see system_auth.rs). Within `lock_minutes` of the
/// last confirmation it runs at once.
pub fn after_confirming(ui: &Rc<Ui>, then: impl FnOnce(&Rc<Ui>) + 'static) {
    let (ask, minutes) = {
        let config = ui.config.borrow();
        (config.passwords.ask_system_password, config.passwords.lock_minutes.clamp(1, 24 * 60))
    };
    if !ask {
        then(ui);
        return;
    }
    let weak = Rc::downgrade(ui);
    glib::spawn_future_local(async move {
        let outcome = system_auth::confirm(Duration::from_secs(u64::from(minutes) * 60)).await;
        let Some(ui) = weak.upgrade() else { return };
        match outcome {
            Outcome::Allowed => then(&ui),
            Outcome::Refused => window::toast(&ui, "Not confirmed, so the password was not used"),
            Outcome::NotInstalled => window::toast(
                &ui,
                "Asking for your system password needs Stales' polkit file; see Settings → Passwords",
            ),
            Outcome::Failed(e) => {
                eprintln!("stales: could not ask polkit: {e}");
                window::toast(&ui, "Could not ask for your system password (see the terminal)");
            }
        }
    });
}

// ---------- Stales' own keyring, locked when not in use ----------
//
// Your login keyring is unlocked as long as you're logged in, so any program running
// as you can read from it. With `separate_keyring`, logins live in a keyring of their
// own ("Stales passwords", with its own password) that Stales locks a few minutes after
// the last use. While locked, nobody can read it; unlocking shows your system's dialog.

/// The keyring's alias (its short name for programs).
const VAULT_ALIAS: &str = "stales";

thread_local! {
    /// Counts uses, so only the timer after the latest use locks the keyring.
    static LOCK_ROUND: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn separate(ui: &Ui) -> bool {
    ui.config.borrow().passwords.separate_keyring
}

async fn service() -> Result<libsecret::Service, String> {
    libsecret::Service::get_future(ServiceFlags::OPEN_SESSION | ServiceFlags::LOAD_COLLECTIONS)
        .await
        .map_err(|e| e.to_string())
}

/// Stales' own keyring. With `create`, it's made if it doesn't exist yet (your system
/// then asks you to choose a password for it).
async fn vault(create: bool) -> Result<Option<Collection>, String> {
    let service = service().await?;
    let found = Collection::for_alias_future(Some(&service), VAULT_ALIAS, CollectionFlags::NONE)
        .await
        .map_err(|e| e.to_string())?;
    if found.is_some() || !create {
        return Ok(found);
    }
    Collection::create_future(Some(&service), "Stales passwords", Some(VAULT_ALIAS), CollectionCreateFlags::NONE)
        .await
        .map(Some)
        .map_err(|e| e.to_string())
}

/// Lock Stales' keyring now. Ok(false): there is none.
pub async fn lock_vault() -> Result<bool, String> {
    use libsecret::prelude::ServiceExtManual;
    let service = service().await?;
    let Some(vault) = vault(false).await? else { return Ok(false) };
    service.lock_future(&[vault.upcast()]).await.map_err(|e| e.to_string())?;
    Ok(true)
}

/// After using the separate keyring: lock it again `lock_minutes` after this use.
pub fn lock_later(ui: &Rc<Ui>) {
    if !separate(ui) {
        return;
    }
    let minutes = ui.config.borrow().passwords.lock_minutes.clamp(1, 24 * 60);
    let round = LOCK_ROUND.with(|r| {
        r.set(r.get() + 1);
        r.get()
    });
    glib::timeout_add_seconds_local_once(minutes * 60, move || {
        if LOCK_ROUND.with(|r| r.get()) == round {
            glib::spawn_future_local(async {
                if let Err(e) = lock_vault().await {
                    eprintln!("stales: could not lock the Stales keyring: {e}");
                }
            });
        }
    });
}

/// Move all of Stales' logins into its own keyring (`to_vault`) or back into your
/// default keyring. Returns how many moved. Unlocking may ask for passwords.
pub async fn move_logins(to_vault: bool) -> Result<usize, String> {
    use gtk::gio::prelude::DBusProxyExt;
    let vault = vault(to_vault).await?;
    let vault_path = vault.as_ref().map(|c| c.object_path().to_string());
    let found = libsecret::password_search_future(
        Some(&schema()),
        HashMap::new(),
        SearchFlags::ALL | SearchFlags::UNLOCK | SearchFlags::LOAD_SECRETS,
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut moved = 0;
    for retrievable in found {
        let Some(item) = retrievable.downcast_ref::<libsecret::Item>() else { continue };
        let in_vault = vault_path.as_ref().is_some_and(|p| item.object_path().starts_with(&format!("{p}/")));
        if in_vault == to_vault {
            continue;
        }
        let Some(secret) = item.secret().and_then(|v| v.text()).map(|t| Secret::new(t.to_string())) else { continue };
        let attributes = item.attributes();
        let (Some(origin), Some(username)) = (attributes.get("origin"), attributes.get("username")) else { continue };
        let map = HashMap::from([("origin", origin.as_str()), ("username", username.as_str())]);
        libsecret::password_store_future(
            Some(&schema()),
            map,
            to_vault.then_some(VAULT_ALIAS),
            &label_for(origin, username),
            &secret,
        )
        .await
        .map_err(|e| e.to_string())?;
        // Only delete the old copy once the new one is safely stored.
        item.delete_future().await.map_err(|e| e.to_string())?;
        moved += 1;
    }
    Ok(moved)
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
