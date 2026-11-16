//! Browsing history: the pages you visited, when, and their titles.
//!
//! Stored as one line of JSON per visit in `~/.local/share/stales/history.jsonl`,
//! readable only by you (like all of Stales' files). Turn it off, or choose how many
//! days to keep, in Settings → Privacy (`[history]` in config.toml). See it with
//! Ctrl+H (stales://history); matching pages also show up in the command bar.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Visit {
    pub url: String,
    pub title: String,
    /// Seconds since 1970.
    pub time: u64,
}

#[derive(Default)]
pub struct History {
    /// Oldest first.
    visits: Vec<Visit>,
}

fn path() -> PathBuf {
    gtk::glib::user_data_dir().join("stales").join("history.jsonl")
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Pages that don't belong in history: Stales' own pages, blank pages, …
fn worth_keeping(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://") || url.starts_with("file://")
}

/// Add one visit at the end of the history file.
fn append(visit: &Visit) {
    let line = serde_json::to_string(visit).unwrap_or_default();
    if let Some(dir) = path().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path()) {
        let _ = writeln!(file, "{line}");
    }
}

/// A visit whose title changed is written again (same address and time, new title).
/// Keep only the last version of each. Returns the visits, and whether any were merged.
fn merge_title_updates(lines: impl Iterator<Item = Visit>) -> (Vec<Visit>, bool) {
    let mut visits: Vec<Visit> = Vec::new();
    let mut merged = false;
    for visit in lines {
        match visits.last_mut() {
            Some(last) if last.url == visit.url && last.time == visit.time => {
                last.title = visit.title;
                merged = true;
            }
            _ => visits.push(visit),
        }
    }
    (visits, merged)
}

impl History {
    /// Read the history file, dropping visits older than `keep_days` (0: keep all).
    pub fn load(keep_days: u32) -> Self {
        let text = std::fs::read_to_string(path()).unwrap_or_default();
        let (visits, merged) = merge_title_updates(text.lines().filter_map(|l| serde_json::from_str(l).ok()));
        let mut history = Self { visits };
        // Write the file again only if it got shorter (old visits or title updates gone).
        if history.prune(keep_days) || merged {
            history.write_all();
        }
        history
    }

    /// Drop old visits. Returns true if something was dropped.
    fn prune(&mut self, keep_days: u32) -> bool {
        if keep_days == 0 {
            return false;
        }
        let oldest = now().saturating_sub(u64::from(keep_days) * 24 * 60 * 60);
        let before = self.visits.len();
        self.visits.retain(|v| v.time >= oldest);
        self.visits.len() != before
    }

    /// Remember a visit. Visiting the same page again right away only updates its title.
    pub fn add(&mut self, url: &str, title: &str) {
        if !worth_keeping(url) {
            return;
        }
        let title = if title.trim().is_empty() { url } else { title.trim() };
        if let Some(last) = self.visits.last_mut()
            && last.url == url
            && now().saturating_sub(last.time) < 30 * 60
        {
            if last.title != title {
                last.title = title.to_string();
                // The same visit again with the new title. Rewriting the whole file
                // instead took about 10 ms with a few months of history, and some
                // sites change their title every few seconds.
                let updated = last.clone();
                append(&updated);
            }
            return;
        }
        let visit = Visit { url: url.to_string(), title: title.to_string(), time: now() };
        // Append one line instead of rewriting the whole file.
        append(&visit);
        self.visits.push(visit);
    }

    /// Visits matching `query` (title or address, any words), newest first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Visit> {
        let words: Vec<String> = query.to_lowercase().split_whitespace().map(String::from).collect();
        self.visits
            .iter()
            .rev()
            .filter(|v| {
                let haystack = format!("{} {}", v.title.to_lowercase(), v.url.to_lowercase());
                words.iter().all(|w| haystack.contains(w))
            })
            .take(limit)
            .collect()
    }

    /// For the command bar: one suggestion per address, most visited first.
    pub fn suggestions(&self, query: &str, limit: usize) -> Vec<Visit> {
        let mut counts: std::collections::HashMap<&str, (usize, &Visit)> = std::collections::HashMap::new();
        for visit in self.search(query, 2000) {
            counts.entry(visit.url.as_str()).and_modify(|e| e.0 += 1).or_insert((1, visit));
        }
        let mut list: Vec<(usize, &Visit)> = counts.into_values().collect();
        list.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.time.cmp(&a.1.time)));
        list.into_iter().take(limit).map(|(_, v)| v.clone()).collect()
    }

    /// Delete one visit (by address and time).
    pub fn delete(&mut self, url: &str, time: u64) {
        self.visits.retain(|v| !(v.url == url && v.time == time));
        self.write_all();
    }

    /// Delete visits from the last `seconds` (None: everything).
    pub fn clear(&mut self, seconds: Option<u64>) {
        match seconds {
            Some(s) => {
                let since = now().saturating_sub(s);
                self.visits.retain(|v| v.time < since);
            }
            None => self.visits.clear(),
        }
        self.write_all();
    }

    pub fn len(&self) -> usize {
        self.visits.len()
    }

    fn write_all(&self) {
        let text: String = self.visits.iter().filter_map(|v| serde_json::to_string(v).ok()).map(|l| l + "\n").collect();
        let temp = path().with_extension("jsonl.tmp");
        let _ = std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, path()));
    }

    #[cfg(test)]
    fn in_memory(visits: Vec<Visit>) -> Self {
        Self { visits }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visit(url: &str, title: &str, time: u64) -> Visit {
        Visit { url: url.into(), title: title.into(), time }
    }

    #[test]
    fn search_matches_all_words_newest_first() {
        let h = History::in_memory(vec![
            visit("https://wiki.archlinux.org/title/Niri", "Niri - ArchWiki", 1),
            visit("https://github.com/YaLTeR/niri", "GitHub - niri", 2),
            visit("https://www.rust-lang.org", "Rust", 3),
        ]);
        let found: Vec<_> = h.search("niri", 10).iter().map(|v| v.time).collect();
        assert_eq!(found, vec![2, 1]);
        assert_eq!(h.search("niri arch", 10).len(), 1);
        assert_eq!(h.search("", 10).len(), 3);
    }

    #[test]
    fn suggestions_prefer_often_visited_pages() {
        let h = History::in_memory(vec![
            visit("https://a.example", "A", 1),
            visit("https://b.example", "B", 2),
            visit("https://a.example", "A", 3),
        ]);
        let s = h.suggestions("example", 5);
        assert_eq!(s[0].url, "https://a.example");
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn own_pages_are_not_recorded() {
        assert!(!worth_keeping("stales://settings"));
        assert!(!worth_keeping("about:blank"));
        assert!(worth_keeping("https://example.com"));
    }

    #[test]
    fn title_updates_are_merged_when_loading() {
        let lines = vec![
            visit("https://a.example", "Loading…", 5),
            visit("https://a.example", "A page", 5),
            visit("https://a.example", "A page again", 9),
        ];
        let (visits, merged) = merge_title_updates(lines.into_iter());
        assert!(merged);
        assert_eq!(
            visits,
            vec![visit("https://a.example", "A page", 5), visit("https://a.example", "A page again", 9)]
        );
    }

    #[test]
    fn old_visits_are_pruned() {
        let mut h = History::in_memory(vec![
            visit("https://old.example", "old", 1),
            visit("https://new.example", "new", now()),
        ]);
        assert!(h.prune(30));
        assert_eq!(h.len(), 1);
        assert!(!h.prune(0));
    }
}
