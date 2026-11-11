//! Saving your Spaces and tabs, so they come back when you start Stales again.
//!
//! Stored in `~/.local/share/stales/session.json`. Saved a moment after every change
//! (not on every tiny change while a page loads) and once more when the window closes.
//! The file is written to a temporary file first and then renamed, so a crash or power
//! cut while saving never leaves you with half a file.
//!
//! Only the active tab loads at startup; the others load when you open them.
//! Each tab also keeps its back/forward history (and scroll position), which the
//! engine gives us as bytes and we store as base64 text.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use serde::{Deserialize, Serialize};

use crate::search::Search;
use crate::state::{Selected, Space, State, Tab};
use crate::window::Ui;

const VERSION: u32 = 1;

#[derive(Serialize)]
struct SessionRef<'a> {
    version: u32,
    current: usize,
    spaces: &'a [Space],
    closed: &'a [Tab],
}

#[derive(Deserialize)]
struct Session {
    #[allow(dead_code)]
    version: u32,
    current: usize,
    spaces: Vec<Space>,
    #[serde(default)]
    closed: Vec<Tab>,
}

pub fn path() -> PathBuf {
    glib::user_data_dir().join("stales").join("session.json")
}

/// Your saved session, or the welcome page the first time.
pub fn load(search: Search) -> State {
    let path = path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return State::first_run(search);
    };
    match from_json(&text, search.clone()) {
        Some(state) => state,
        None => {
            // Keep the broken file instead of overwriting it, so nothing is lost for good.
            let backup = path.with_extension("json.broken");
            let _ = std::fs::rename(&path, &backup);
            eprintln!("stales: could not read {}, kept it as {}", path.display(), backup.display());
            State::first_run(search)
        }
    }
}

fn from_json(text: &str, search: Search) -> Option<State> {
    let session: Session = serde_json::from_str(text).ok()?;
    let mut state = State { spaces: session.spaces, current: session.current, search, closed: session.closed };
    repair(&mut state);
    Some(state)
}

/// Fix anything in a saved file that doesn't make sense (edited by hand, older version, …).
fn repair(state: &mut State) {
    if state.spaces.is_empty() {
        state.spaces.push(Space::new("Personal", "◐"));
    }
    state.current = state.current.min(state.spaces.len() - 1);
    for space in &mut state.spaces {
        let valid = match space.active {
            Some(Selected::Pinned(i)) => i < space.pinned.len(),
            Some(Selected::Tab(i)) => i < space.tabs.len(),
            None => true,
        };
        if !valid {
            space.active = if space.tabs.is_empty() { None } else { Some(Selected::Tab(0)) };
        }
    }
}

fn to_json(state: &State) -> String {
    let session = SessionRef { version: VERSION, current: state.current, spaces: &state.spaces, closed: &state.closed };
    serde_json::to_string_pretty(&session).expect("the session is always valid JSON")
}

/// Save right now. Writes a temporary file, then swaps it in.
pub fn store(state: &State) {
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let temp = path.with_extension("json.tmp");
    let result = std::fs::write(&temp, to_json(state)).and_then(|()| std::fs::rename(&temp, &path));
    if let Err(e) = result {
        eprintln!("stales: could not save tabs to {}: {e}", path.display());
    }
}

/// Save in a moment. Many changes in a row (a page loading, typing) become one save.
pub fn save_soon(ui: &Rc<Ui>) {
    if ui.save_queued.replace(true) {
        return;
    }
    let weak = Rc::downgrade(ui);
    glib::timeout_add_local_once(Duration::from_millis(800), move || {
        if let Some(ui) = weak.upgrade() {
            ui.save_queued.set(false);
            crate::window::capture_histories(&ui);
            store(&ui.state.borrow());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_survives_saving_and_loading() {
        let mut state = State::demo(Search::default());
        state.switch_space(1);
        state.open_tab("example.com");
        state.close_active();
        state.toggle_pin();

        let restored = from_json(&to_json(&state), Search::default()).unwrap();
        assert_eq!(restored.current, 1);
        assert_eq!(restored.spaces.len(), state.spaces.len());
        for (a, b) in restored.spaces.iter().zip(&state.spaces) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.active, b.active);
            let urls = |tabs: &[Tab]| tabs.iter().map(|t| t.url.clone()).collect::<Vec<_>>();
            assert_eq!(urls(&a.tabs), urls(&b.tabs));
            assert_eq!(urls(&a.pinned), urls(&b.pinned));
        }
        assert_eq!(restored.closed.len(), 1);
    }

    #[test]
    fn restored_tabs_get_fresh_unique_ids() {
        let state = State::demo(Search::default());
        let restored = from_json(&to_json(&state), Search::default()).unwrap();
        let mut ids = restored.all_ids();
        let count = ids.len();
        ids.sort_by_key(|id| id.0);
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    #[test]
    fn nonsense_in_the_file_is_repaired() {
        let text = r#"{"version":1,"current":7,"spaces":[
            {"name":"A","icon":"●","pinned":[],"tabs":[{"title":"x","url":"https://x.org"}],"active":{"Tab":5}}
        ]}"#;
        let state = from_json(text, Search::default()).unwrap();
        assert_eq!(state.current, 0);
        assert_eq!(state.space().active, Some(Selected::Tab(0)));
    }

    #[test]
    fn empty_session_gets_a_space() {
        let state = from_json(r#"{"version":1,"current":0,"spaces":[]}"#, Search::default()).unwrap();
        assert_eq!(state.spaces.len(), 1);
    }

    #[test]
    fn history_is_saved_with_the_tab() {
        let mut state = State::demo(Search::default());
        state.spaces[0].tabs[0].history = Some("aGVsbG8=".into());
        let restored = from_json(&to_json(&state), Search::default()).unwrap();
        assert_eq!(restored.spaces[0].tabs[0].history.as_deref(), Some("aGVsbG8="));
        assert_eq!(restored.spaces[0].tabs[1].history, None);
    }

    #[test]
    fn a_broken_file_is_rejected() {
        assert!(from_json("{ not json", Search::default()).is_none());
    }
}
