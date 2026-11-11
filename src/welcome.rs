//! Stales' own pages (`stales://…`). For now: the welcome page shown on first start.
//!
//! The page is filled in when it's opened, from your real settings, so it always
//! shows the keys and search engine you actually use.

use crate::actions::{self, ACTIONS};
use crate::config::{self, Config};

pub const WELCOME_URL: &str = "stales://welcome";

/// HTML for `stales://<name>`, or `None` if there is no such page.
pub fn render(name: &str, config: &Config) -> Option<String> {
    match name {
        "welcome" | "" => Some(welcome(config)),
        _ => None,
    }
}

fn welcome(config: &Config) -> String {
    let path = config::path();
    let vim = if config.keys.vim {
        let rows = [
            ("Click a link with the keyboard", "hints"),
            ("Open a link in the background", "hints-new-tab"),
            ("Scroll", "scroll-down"),
            ("Top / bottom", "scroll-top"),
            ("Back / forward", "back"),
            ("Leave a text field", ""),
            ("Send all keys to the page", "insert-mode"),
        ];
        let mut list = String::from("<ul>");
        for (text, action) in rows {
            let keys = match action {
                "" => kbd("Esc"),
                "scroll-down" => format!("{} {}", page_key(config, "scroll-down"), page_key(config, "scroll-up")),
                "scroll-top" => format!("{} {}", page_key(config, "scroll-top"), page_key(config, "scroll-bottom")),
                "back" => format!("{} {}", page_key(config, "back"), page_key(config, "forward")),
                _ => page_key(config, action),
            };
            list.push_str(&format!("<li><span>{text}</span><span>{keys}</span></li>"));
        }
        list.push_str("</ul>");
        list
    } else {
        "<p class=\"muted\">Single-key commands are turned off (<code>vim = false</code> in the settings).</p>".into()
    };
    let blocker = if config.blocker.enabled {
        format!(
            "<p>Ads and trackers are blocked with the same lists uBlock Origin uses. \
             The shield at the top of the sidebar (or {}) turns it off for one site.</p>",
            key(config, "toggle-blocker")
        )
    } else {
        "<p class=\"muted\">The ad blocker is turned off in the settings.</p>".into()
    };

    let mut html = include_str!("welcome.html")
        .replace("{vim}", &vim)
        .replace("{blocker}", &blocker)
        .replace("{engine}", &escape(&config.search().default_engine().name))
        .replace("{config_url}", &format!("file://{}", escape(&path.to_string_lossy())))
        .replace("{config_path}", &escape(&path.to_string_lossy()));
    fill(&mut html, "{key:", |name| key(config, name));
    fill(&mut html, "{page:", |name| page_key(config, name));
    html
}

/// Replace every `{prefix:name}` in the page with `value(name)`.
fn fill(html: &mut String, prefix: &str, value: impl Fn(&str) -> String) {
    while let Some(start) = html.find(prefix) {
        let Some(len) = html[start..].find('}') else { break };
        let name = html[start + prefix.len()..start + len].to_string();
        html.replace_range(start..=start + len, &value(&name));
    }
}

/// The first key bound to a browser action, as a key cap.
fn key(config: &Config, name: &str) -> String {
    ACTIONS
        .iter()
        .find(|a| a.name == name)
        .and_then(|a| actions::keys_for(&config.keys, a).into_iter().next())
        .map(|k| kbd(&actions::pretty_key(&k)))
        .unwrap_or_else(|| "<span class=\"muted\">no key</span>".into())
}

/// The single key that runs an action on web pages, as a key cap.
fn page_key(config: &Config, action: &str) -> String {
    let keys = actions::page_keys(&config.keys);
    let mut found: Vec<&String> = keys.keys.iter().filter(|(_, a)| a.as_str() == action).map(|(k, _)| k).collect();
    found.sort_by_key(|k| k.len());
    match found.first() {
        Some(k) if k.as_str() == " " => kbd("Space"),
        Some(k) => kbd(k),
        None => "<span class=\"muted\">no key</span>".into(),
    }
}

fn kbd(text: &str) -> String {
    format!("<kbd>{}</kbd>", escape(text))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_placeholder_is_filled() {
        let html = render("welcome", &Config::default()).unwrap();
        assert!(!html.contains("{key:"), "unfilled key placeholder");
        assert!(!html.contains("{page:"), "unfilled page placeholder");
        for placeholder in ["{vim}", "{blocker}", "{engine}", "{config_url}", "{config_path}"] {
            assert!(!html.contains(placeholder), "{placeholder} not filled");
        }
        assert!(html.contains("DuckDuckGo"));
        assert!(html.contains("<kbd>f</kbd>"));
    }

    #[test]
    fn unknown_pages_are_not_found() {
        assert!(render("nope", &Config::default()).is_none());
    }
}
