//! Search engines: which one you search with, and the `@` shortcuts.
//!
//! Your default engine is set in `~/.config/stales/config.toml`. You can also search
//! with any other engine just once by starting with its keyword:
//!
//! ```text
//! rust gtk4        → searches your default engine
//! @g rust gtk4     → searches Google this one time
//! @w Linus Torvalds → searches Wikipedia
//! ```

/// One search engine. `url` contains `%s` where the search words go.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchEngine {
    pub name: String,
    pub keyword: String,
    pub url: String,
}

impl SearchEngine {
    fn new(name: &str, keyword: &str, url: &str) -> Self {
        Self { name: name.into(), keyword: keyword.into(), url: url.into() }
    }

    pub fn url_for(&self, query: &str) -> String {
        self.url.replace("%s", &encode(query))
    }

    /// Does this engine match a name from the config file? "DuckDuckGo", "duckduckgo",
    /// "duck" and the keyword "d" all match DuckDuckGo; "brave" matches Brave Search.
    fn matches(&self, wanted: &str) -> bool {
        let simplify = |s: &str| s.to_lowercase().replace([' ', '-', '_'], "");
        let wanted = simplify(wanted);
        let name = simplify(&self.name);
        name == wanted || self.keyword.to_lowercase() == wanted || (wanted.len() >= 4 && name.starts_with(&wanted))
    }
}

/// The engines that come with Stales.
pub fn builtin() -> Vec<SearchEngine> {
    vec![
        SearchEngine::new("DuckDuckGo", "d", "https://duckduckgo.com/?q=%s"),
        SearchEngine::new("Google", "g", "https://www.google.com/search?q=%s"),
        SearchEngine::new("Brave Search", "br", "https://search.brave.com/search?q=%s"),
        SearchEngine::new("Startpage", "s", "https://www.startpage.com/do/search?q=%s"),
        SearchEngine::new("Bing", "b", "https://www.bing.com/search?q=%s"),
        SearchEngine::new("Ecosia", "e", "https://www.ecosia.org/search?q=%s"),
        SearchEngine::new("Qwant", "q", "https://www.qwant.com/?q=%s"),
        SearchEngine::new("Kagi", "k", "https://kagi.com/search?q=%s"),
        SearchEngine::new("Mojeek", "m", "https://www.mojeek.com/search?q=%s"),
        // Searches on one site, handy with @ shortcuts.
        SearchEngine::new("Wikipedia", "w", "https://en.wikipedia.org/w/index.php?search=%s"),
        SearchEngine::new("YouTube", "y", "https://www.youtube.com/results?search_query=%s"),
        SearchEngine::new("GitHub", "gh", "https://github.com/search?q=%s"),
        SearchEngine::new("Arch Wiki", "aw", "https://wiki.archlinux.org/index.php?search=%s"),
        SearchEngine::new("AUR", "aur", "https://aur.archlinux.org/packages?K=%s"),
        SearchEngine::new("docs.rs", "rs", "https://docs.rs/releases/search?query=%s"),
    ]
}

/// All engines (built-in plus your own) and which one is the default.
#[derive(Clone, Debug)]
pub struct Search {
    pub engines: Vec<SearchEngine>,
    default: usize,
}

impl Default for Search {
    fn default() -> Self {
        Self { engines: builtin(), default: 0 }
    }
}

impl Search {
    /// `custom` engines are added to the list; an engine with the same keyword as a
    /// built-in one replaces it. `default` is a name or keyword; unknown names fall back to DuckDuckGo.
    pub fn new(default: &str, custom: Vec<SearchEngine>) -> Self {
        let mut engines = builtin();
        for engine in custom {
            engines.retain(|e| e.keyword != engine.keyword);
            engines.push(engine);
        }
        let mut search = Self { engines, default: 0 };
        if !search.set_default(default) {
            eprintln!("stales: unknown search engine '{default}', using DuckDuckGo");
        }
        search
    }

    pub fn default_engine(&self) -> &SearchEngine {
        &self.engines[self.default]
    }

    /// Returns false if no engine has that name or keyword.
    pub fn set_default(&mut self, wanted: &str) -> bool {
        let wanted = wanted.trim().trim_matches('"');
        match self.engines.iter().position(|e| e.matches(wanted)) {
            Some(i) => {
                self.default = i;
                true
            }
            None => false,
        }
    }

    /// Turn what the user typed into the address to load.
    pub fn resolve(&self, input: &str) -> Resolved {
        let input = input.trim();
        let (engine, query) = self.pick(input);
        let explicit_shortcut = query.len() != input.len();
        if !explicit_shortcut && looks_like_url(input) {
            let url = if input.contains("://") || input.starts_with("about:") {
                input.to_string()
            } else if input.starts_with("localhost") || input.starts_with("127.0.0.1") {
                format!("http://{input}")
            } else {
                format!("https://{input}")
            };
            let title = host_of(&url).unwrap_or_else(|| url.clone());
            return Resolved { url, title, engine: None };
        }
        Resolved { url: engine.url_for(query), title: query.to_string(), engine: Some(engine.name.clone()) }
    }

    /// Which engine to use for this text, and the words to search for.
    /// "@g rust" → (Google, "rust"). Anything else → (default engine, whole text).
    pub fn pick<'a>(&self, input: &'a str) -> (&SearchEngine, &'a str) {
        if let Some(rest) = input.strip_prefix('@') {
            let (keyword, query) = rest.split_once(' ').unwrap_or((rest, ""));
            if let Some(engine) = self.engines.iter().find(|e| e.keyword.eq_ignore_ascii_case(keyword)) {
                return (engine, query.trim());
            }
        }
        (self.default_engine(), input)
    }
}

/// The result of `Search::resolve`.
#[derive(Debug, PartialEq)]
pub struct Resolved {
    pub url: String,
    /// A first title for the tab, until the page reports its real one.
    pub title: String,
    /// Which engine is searched, or `None` if the text was an address.
    pub engine: Option<String>,
}

fn looks_like_url(input: &str) -> bool {
    !input.contains(' ')
        && (input.contains("://")
            || input.contains('.')
            || input.starts_with("localhost")
            || input.starts_with("about:"))
}

pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    Some(host.trim_start_matches("www.").to_string())
}

/// Make text safe to put in a URL: "c++ & rust" → "c%2B%2B%20%26%20rust".
fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_characters_are_encoded() {
        assert_eq!(encode("c++ & rust"), "c%2B%2B%20%26%20rust");
        assert_eq!(encode("über"), "%C3%BCber");
    }

    #[test]
    fn default_engine_by_name_or_keyword() {
        assert_eq!(Search::new("Google", vec![]).default_engine().name, "Google");
        assert_eq!(Search::new("brave-search", vec![]).default_engine().name, "Brave Search");
        assert_eq!(Search::new("s", vec![]).default_engine().name, "Startpage");
        assert_eq!(Search::new("brave", vec![]).default_engine().name, "Brave Search");
        assert_eq!(Search::new("nonsense", vec![]).default_engine().name, "DuckDuckGo");
    }

    #[test]
    fn at_shortcuts_pick_another_engine_once() {
        let search = Search::default();
        let (engine, query) = search.pick("@w Linus Torvalds");
        assert_eq!(engine.name, "Wikipedia");
        assert_eq!(query, "Linus Torvalds");

        let (engine, query) = search.pick("@nope hello");
        assert_eq!(engine.name, "DuckDuckGo");
        assert_eq!(query, "@nope hello");
    }

    #[test]
    fn addresses_and_searches_are_told_apart() {
        let search = Search::new("Google", vec![]);
        assert_eq!(search.resolve("archlinux.org").url, "https://archlinux.org");
        assert_eq!(search.resolve("localhost:8080").url, "http://localhost:8080");
        let r = search.resolve("rust gtk4");
        assert_eq!(r.url, "https://www.google.com/search?q=rust%20gtk4");
        assert_eq!(r.engine.as_deref(), Some("Google"));
        // A shortcut always searches, even if the words look like an address.
        assert_eq!(search.resolve("@w rust-lang.org").url, "https://en.wikipedia.org/w/index.php?search=rust-lang.org");
    }

    #[test]
    fn custom_engines_can_be_added_and_chosen() {
        let searx = SearchEngine::new("My SearXNG", "sx", "https://searx.example.org/search?q=%s");
        let search = Search::new("My SearXNG", vec![searx]);
        assert_eq!(search.default_engine().url_for("linux"), "https://searx.example.org/search?q=linux");
    }
}
