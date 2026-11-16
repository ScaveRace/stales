//! Search suggestions: what your search engine thinks you are about to type.
//!
//! While you type words in the command bar, Stales waits a short moment and then asks
//! your search engine (or the `@` engine you picked) for ideas, like every big browser.
//! The answer becomes up to five extra rows under the first one.
//!
//! Privacy: this sends the words you type to the engine. It can be turned off in
//! Settings → Search (`[search] suggestions` in config.toml), and addresses and
//! Stales' own pages are never sent.

use std::cell::OnceCell;
use std::time::Duration;

use gtk::glib;
use serde_json::Value;
use soup::prelude::*;

use crate::search::{self, SearchEngine, SuggestFormat};

/// How many suggestion rows to show at most.
pub const MAX: usize = 5;

/// Wait this long after the last key before asking, so we don't ask for every letter.
pub const DELAY: Duration = Duration::from_millis(120);

/// Give up on an answer after this long.
const TIMEOUT: Duration = Duration::from_secs(2);

/// Text longer than this is not sent (it is probably pasted, not a search).
const MAX_LEN: usize = 200;

/// Should we ask for suggestions for `query` (the words, without any `@shortcut`)?
/// Never for empty or very long text, and never for something that looks like an
/// address: that could be a private link, and the engine has no business seeing it.
pub fn worth_asking(query: &str) -> bool {
    let query = query.trim();
    let address = search::looks_like_url(query) || query.contains("://") || query.starts_with("stales:");
    !query.is_empty() && query.len() <= MAX_LEN && !address
}

/// Read the engine's answer into a list of suggestions.
pub fn parse(format: SuggestFormat, body: &str) -> Vec<String> {
    let Ok(json) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    let items: &[Value] = match format {
        // ["what you typed", ["idea 1", …], …]
        SuggestFormat::OpenSearch => json.get(1).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default(),
        // {"data": {"items": [{"value": "idea 1"}, …]}}
        SuggestFormat::Qwant => {
            json.pointer("/data/items").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
        }
        // ["idea 1", "idea 2"]
        SuggestFormat::List => json.as_array().map(Vec::as_slice).unwrap_or_default(),
    };
    items.iter().filter_map(item_text).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// One suggestion: usually plain text. Some engines (Brave with `rich=true`) send
/// objects instead: `{"q": "idea"}`; Qwant sends `{"value": "idea"}`.
fn item_text(item: &Value) -> Option<&str> {
    item.as_str().or_else(|| ["q", "value", "phrase"].iter().find_map(|key| item.get(*key)?.as_str()))
}

/// Keep at most `MAX` suggestions, drop the one that equals what you typed (the first
/// row already does that) and repeats ("Linux" and "linux").
pub fn tidy(query: &str, suggestions: Vec<String>) -> Vec<String> {
    let mut seen = vec![query.trim().to_lowercase()];
    let mut out = Vec::new();
    for suggestion in suggestions {
        let key = suggestion.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(suggestion);
        if out.len() == MAX {
            break;
        }
    }
    out
}

/// Which of the last suggestions still fit while you keep typing ("rust g" showed
/// "rust gtk4"; after typing "rust gt" it still fits). They stay until new ones
/// arrive, so the rows don't flicker on every key. `asked` is the text they were for:
/// for that same text every suggestion fits (Wikipedia suggests "Linus" for "linux").
pub fn still_fitting(asked: &str, last: &[String], query: &str) -> Vec<String> {
    let start = query.trim().to_lowercase();
    if asked.trim().to_lowercase() == start {
        return tidy(query, last.to_vec());
    }
    tidy(query, last.iter().filter(|s| s.to_lowercase().starts_with(&start)).cloned().collect())
}

/// Ask `engine` for suggestions for `query`. Gives an empty list if anything fails.
pub async fn fetch(engine: &SearchEngine, query: &str) -> Vec<String> {
    let Some(url) = engine.suggest_url_for(query) else { return Vec::new() };
    match glib::future_with_timeout(TIMEOUT, download(&url)).await {
        Ok(Ok(body)) => tidy(query, parse(engine.suggest_format, &body)),
        Ok(Err(e)) => {
            eprintln!("stales: no suggestions from {}: {e}", engine.name);
            Vec::new()
        }
        // Too slow: just show none this time.
        Err(_) => {
            eprintln!("stales: no suggestions from {}: no answer within {TIMEOUT:?}", engine.name);
            Vec::new()
        }
    }
}

thread_local! {
    /// One connection manager for all requests, so connections can be reused.
    static SESSION: OnceCell<soup::Session> = const { OnceCell::new() };
}

fn session() -> soup::Session {
    SESSION.with(|cell| {
        cell.get_or_init(|| {
            let session = soup::Session::new();
            session.set_timeout(TIMEOUT.as_secs() as u32);
            // Some engines (Wikipedia, Qwant, Ecosia) refuse requests that don't say
            // which program is asking. Only the name and version are sent.
            session.set_user_agent(concat!("Stales/", env!("CARGO_PKG_VERSION")));
            session
        })
        .clone()
    })
}

async fn download(url: &str) -> Result<String, String> {
    let message = soup::Message::new("GET", url).map_err(|e| e.to_string())?;
    let bytes = session().send_and_read_future(&message, glib::Priority::DEFAULT).await.map_err(|e| e.to_string())?;
    let status = message.status();
    if status != soup::Status::Ok {
        return Err(format!("server answered {status:?}"));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opensearch_answers() {
        let body = r#"["rust gtk",["rust gtk","rust gtk4","rust gtk-rs"],[],{"google:suggestsubtypes":[]}]"#;
        assert_eq!(parse(SuggestFormat::OpenSearch, body), ["rust gtk", "rust gtk4", "rust gtk-rs"]);
        // Wikipedia adds descriptions and links after the list.
        let body = r#"["linux",["Linux","Linux kernel"],["",""],["https://en.wikipedia.org/wiki/Linux","x"]]"#;
        assert_eq!(parse(SuggestFormat::OpenSearch, body), ["Linux", "Linux kernel"]);
    }

    #[test]
    fn brave_rich_answers() {
        let body =
            r#"["rust",[{"type":"entity","q":"rust","name":"Rust (2024 film)"},{"type":"query","q":"rust game"}]]"#;
        assert_eq!(parse(SuggestFormat::OpenSearch, body), ["rust", "rust game"]);
    }

    #[test]
    fn qwant_and_list_answers() {
        let body =
            r#"{"status":"success","data":{"items":[{"value":"rust gtk","suggestType":0},{"value":"rust gtk4"}]}}"#;
        assert_eq!(parse(SuggestFormat::Qwant, body), ["rust gtk", "rust gtk4"]);
        assert_eq!(parse(SuggestFormat::List, r#"["stalld","stalloc"]"#), ["stalld", "stalloc"]);
    }

    #[test]
    fn broken_answers_give_nothing() {
        assert!(parse(SuggestFormat::OpenSearch, "<html>404</html>").is_empty());
        assert!(parse(SuggestFormat::OpenSearch, r#"["q"]"#).is_empty());
        assert!(parse(SuggestFormat::Qwant, r#"["q",["a"]]"#).is_empty());
        assert!(parse(SuggestFormat::OpenSearch, r#"["q",[1,null,"  ","ok"]]"#) == ["ok"]);
    }

    #[test]
    fn tidy_drops_the_typed_text_repeats_and_extras() {
        let list = ["Rust GTK", "rust gtk4", "Rust GTK4", "a", "b", "c", "d", "e"].map(String::from).to_vec();
        assert_eq!(tidy("rust gtk", list), ["rust gtk4", "a", "b", "c", "d"]);
    }

    #[test]
    fn old_suggestions_are_kept_only_while_they_fit() {
        let last = ["rust gtk", "rust gtk4", "Rust GTK4", "rust tokio"].map(String::from).to_vec();
        assert_eq!(still_fitting("rust g", &last, "rust gt"), ["rust gtk", "rust gtk4"]);
        assert_eq!(still_fitting("rust g", &last, "rust gtk"), ["rust gtk4"]);
        assert!(still_fitting("rust g", &last, "python").is_empty());
        // For the text they were asked for, all of them fit.
        assert_eq!(still_fitting("rust g", &last, "Rust G"), ["rust gtk", "rust gtk4", "rust tokio"]);
    }

    #[test]
    fn addresses_and_own_pages_are_never_sent() {
        assert!(worth_asking("rust gtk"));
        assert!(worth_asking(" linux "));
        assert!(!worth_asking("archlinux.org"));
        assert!(!worth_asking("https://example.org/a b"));
        assert!(!worth_asking("stales://settings"));
        assert!(!worth_asking("stales:help"));
        assert!(!worth_asking("localhost:8080"));
        assert!(!worth_asking("about:blank"));
        assert!(!worth_asking("  "));
        assert!(!worth_asking(&"x ".repeat(150)));
    }
}
