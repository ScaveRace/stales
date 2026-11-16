//! Your settings, from `~/.config/stales/config.toml`.
//!
//! The file is plain text so you can edit it, keep it in your dotfiles and share it.
//! If it doesn't exist, Stales writes one with every option explained (see
//! `default_config.toml` next to this file). Anything missing or misspelled falls
//! back to the default, so a broken config never stops the browser from starting.

use std::collections::HashMap;
use std::path::PathBuf;

use gtk::glib;
use serde::Deserialize;

use crate::search::{Search, SearchEngine, SuggestFormat};

const DEFAULT_CONFIG: &str = include_str!("default_config.toml");

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub search: SearchConfig,
    pub keys: KeysConfig,
    pub appearance: AppearanceConfig,
    pub passwords: PasswordsConfig,
    pub web: WebConfig,
    pub downloads: DownloadsConfig,
    pub sidebar: SidebarConfig,
    pub history: HistoryConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    /// Remember the pages you visit.
    pub enabled: bool,
    /// Forget visits older than this many days (0: keep everything).
    pub keep_days: u32,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self { enabled: true, keep_days: 90 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SidebarConfig {
    /// How many pinned tiles a Space can have.
    pub pinned_max: usize,
    /// Pinned tiles per row in the sidebar.
    pub pinned_per_row: u32,
    /// "small", "medium" or "large".
    pub pinned_size: String,
    /// Folders in the tab list.
    pub folders: bool,
}

impl Default for SidebarConfig {
    fn default() -> Self {
        Self { pinned_max: 9, pinned_per_row: 3, pinned_size: "medium".into(), folders: true }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct DownloadsConfig {
    /// Ask before each download.
    pub ask: bool,
    /// Where downloads go. Empty: your Downloads folder.
    pub folder: String,
}

impl Default for DownloadsConfig {
    fn default() -> Self {
        Self { ask: true, folder: String::new() }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    /// How Stales introduces itself to websites. Empty: a current Safari.
    pub user_agent: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct PasswordsConfig {
    /// Offer to save logins and fill them in (stored in your system keyring).
    pub enabled: bool,
    /// Keep logins in a separate keyring ("Stales passwords") that Stales locks when
    /// not in use, instead of your login keyring (unlocked while you're logged in).
    pub separate_keyring: bool,
    /// Lock that keyring this many minutes after the last use.
    pub lock_minutes: u32,
}

impl Default for PasswordsConfig {
    fn default() -> Self {
        Self { enabled: true, separate_keyring: false, lock_minutes: 5 }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    /// Theme name: midnight, paper, graphite, aurora, ember, or your own file's name.
    pub theme: String,
    /// "arc" (tabs in the sidebar) or "classic" (tabs along the top).
    pub layout: String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self { theme: "midnight".into(), layout: "arc".into() }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    /// Name or keyword of the default engine, e.g. "DuckDuckGo", "Google" or "br".
    pub engine: String,
    /// Your own engines, e.g. a SearXNG instance.
    pub custom: Vec<CustomEngine>,
    /// Ask the search engine for suggestions while you type in the command bar.
    pub suggestions: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self { engine: "DuckDuckGo".into(), custom: Vec::new(), suggestions: true }
    }
}

#[derive(Debug, Deserialize)]
pub struct CustomEngine {
    pub name: String,
    pub keyword: String,
    pub url: String,
    /// Optional address for suggestions while you type, with %s (OpenSearch answers).
    #[serde(default)]
    pub suggest_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct KeysConfig {
    /// Single-key commands on web pages (j/k to scroll, f for link hints, …).
    pub vim: bool,
    /// Your own key combos, replacing the defaults: action name → list of keys.
    pub bindings: HashMap<String, Vec<String>>,
    /// Your own single-key commands on pages: key → action ("none" removes a key).
    pub page: HashMap<String, String>,
    /// Sites where single keys go to the site instead (they have their own shortcuts).
    pub exclude: Vec<String>,
}

impl Default for KeysConfig {
    fn default() -> Self {
        Self { vim: true, bindings: HashMap::new(), page: HashMap::new(), exclude: Vec::new() }
    }
}

impl Config {
    pub fn search(&self) -> Search {
        let custom = self
            .search
            .custom
            .iter()
            .filter(|e| {
                let ok = e.url.contains("%s");
                if !ok {
                    eprintln!("stales: search engine '{}' has no %s in its url, skipping it", e.name);
                }
                ok
            })
            .map(|e| SearchEngine {
                name: e.name.clone(),
                keyword: e.keyword.clone(),
                url: e.url.clone(),
                suggest_url: e.suggest_url.clone().filter(|u| {
                    let ok = u.contains("%s");
                    if !ok && !u.is_empty() {
                        eprintln!("stales: search engine '{}' has no %s in its suggest_url, ignoring it", e.name);
                    }
                    ok
                }),
                suggest_format: SuggestFormat::OpenSearch,
            })
            .collect();
        Search::new(&self.search.engine, custom)
    }
}

pub fn path() -> PathBuf {
    glib::user_config_dir().join("stales").join("config.toml")
}

/// Read the config file, creating it with the defaults the first time.
pub fn load() -> Config {
    let path = path();
    if !path.exists() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(&path, DEFAULT_CONFIG) {
            Ok(()) => eprintln!("stales: wrote default settings to {}", path.display()),
            Err(e) => eprintln!("stales: could not write {}: {e}", path.display()),
        }
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    match toml::from_str(&text) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("stales: error in {}, using defaults:\n{e}", path.display());
            Config::default()
        }
    }
}

// ---------- Changing settings from inside Stales ----------
//
// The settings page and some shortcuts change config.toml. The file is edited in
// place with `toml_edit`, so your comments and layout stay as they were, and it is
// checked before saving: an edit that would break the file is refused.

/// Apply `change` to config.toml, save it, and return the new settings.
pub fn edit(change: impl FnOnce(&mut toml_edit::DocumentMut)) -> Result<Config, String> {
    let path = path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_CONFIG.to_string());
    let (new_text, config) = edit_text(&text, change)?;
    let temp = path.with_extension("toml.tmp");
    std::fs::write(&temp, new_text).and_then(|()| std::fs::rename(&temp, &path)).map_err(|e| e.to_string())?;
    Ok(config)
}

fn edit_text(text: &str, change: impl FnOnce(&mut toml_edit::DocumentMut)) -> Result<(String, Config), String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("config.toml has an error: {e}"))?;
    change(&mut doc);
    let new_text = doc.to_string();
    let config: Config = toml::from_str(&new_text).map_err(|e| format!("that change would break config.toml: {e}"))?;
    Ok((new_text, config))
}

/// The table at `keys` (e.g. ["keys", "bindings"]), created if it doesn't exist yet.
pub fn table<'a>(doc: &'a mut toml_edit::DocumentMut, keys: &[&str]) -> &'a mut toml_edit::Table {
    let mut table = doc.as_table_mut();
    for key in keys {
        let entry = table.entry(key).or_insert_with(toml_edit::table);
        if !entry.is_table() {
            *entry = toml_edit::table();
        }
        table = entry.as_table_mut().expect("just made sure it is a table");
    }
    table
}

/// Set one value, e.g. `set(doc, &["appearance"], "theme", "aurora".into())`.
pub fn set(doc: &mut toml_edit::DocumentMut, tables: &[&str], key: &str, value: toml_edit::Value) {
    let table = table(doc, tables);
    match table.get_mut(key).and_then(|item| item.as_value_mut()) {
        // Keep any comment written after the old value on the same line.
        Some(old) => {
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        None => {
            table.insert(key, toml_edit::Item::Value(value));
        }
    }
}

/// A list of strings as a TOML value.
pub fn string_list(items: &[String]) -> toml_edit::Value {
    toml_edit::Value::Array(items.iter().map(String::as_str).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_keeps_comments_and_checks_the_result() {
        let text = "# my settings\n[appearance]\n# pick one\ntheme = \"midnight\" # dark\n";
        let (new_text, config) = edit_text(text, |doc| set(doc, &["appearance"], "theme", "aurora".into())).unwrap();
        assert_eq!(config.appearance.theme, "aurora");
        assert!(new_text.contains("# my settings"));
        assert!(new_text.contains("# pick one"));
        assert!(new_text.contains("# dark"));

        // New tables are created when needed.
        let (new_text, config) =
            edit_text(text, |doc| set(doc, &["keys", "bindings"], "close-tab", string_list(&["<Ctrl>q".into()])))
                .unwrap();
        assert_eq!(config.keys.bindings["close-tab"], vec!["<Ctrl>q".to_string()]);
        assert!(new_text.contains("[keys.bindings]"));

        // A change that makes the file invalid is refused.
        assert!(edit_text(text, |doc| set(doc, &["history"], "enabled", "yes please".into())).is_err());
    }

    #[test]
    fn the_default_file_is_valid() {
        let config: Config = toml::from_str(DEFAULT_CONFIG).unwrap();
        assert_eq!(config.search().default_engine().name, "DuckDuckGo");
        assert!(config.keys.vim);
    }

    #[test]
    fn custom_engine_from_toml() {
        let config: Config = toml::from_str(
            r#"
            [search]
            engine = "searx"
            [[search.custom]]
            name = "Searx"
            keyword = "sx"
            url = "https://searx.example.org/search?q=%s"
            suggest_url = "https://searx.example.org/autocompleter?q=%s"
            "#,
        )
        .unwrap();
        let search = config.search();
        assert_eq!(search.default_engine().name, "Searx");
        assert_eq!(
            search.default_engine().suggest_url_for("a b").as_deref(),
            Some("https://searx.example.org/autocompleter?q=a%20b")
        );
        assert!(config.search.suggestions);
    }

    #[test]
    fn missing_sections_use_defaults() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.search().default_engine().name, "DuckDuckGo");
        assert!(config.search.suggestions);
    }
}
