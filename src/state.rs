//! The browser's data: which Spaces exist, which tabs are open, which one is active.
//!
//! This file has no GTK in it on purpose. The UI reads from `State` and calls its
//! methods when the user clicks something; then it redraws. Keeping the data
//! separate from the widgets makes it easy to test and later save to disk.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::search::Search;

/// A number that identifies a tab for its whole life, even when tabs move around.
/// The window uses it to find the web view that belongs to a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

impl TabId {
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

/// One open page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    /// Not saved: every run hands out fresh ids.
    #[serde(skip, default = "TabId::next")]
    pub id: TabId,
    pub title: String,
    pub url: String,
    /// Back/forward history from the engine, base64 text so it fits in the session
    /// file. Filled in when saving; used once when the tab's page is first created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<String>,
    /// The folder this tab is saved in (see `Folder`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<u64>,
}

/// A folder in a Space's tab list (like Arc): a named group of saved links. Closing a
/// tab in a folder only unloads it; the link stays until you drag it out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: u64,
    pub name: String,
    /// Showing its tabs (true) or collapsed.
    pub open: bool,
}

impl Tab {
    /// Build a tab from whatever the user typed: an address ("archlinux.org") or a
    /// search ("rust gtk4"), which goes to the chosen search engine.
    pub fn from_input(input: &str, search: &Search) -> Self {
        let resolved = search.resolve(input);
        Self { id: TabId::next(), title: resolved.title, url: resolved.url, history: None, folder: None }
    }

    /// The URL without "https://" and the trailing slash, for showing in the sidebar.
    pub fn display_url(&self) -> String {
        let without_scheme = self.url.split_once("://").map_or(self.url.as_str(), |(_, rest)| rest);
        without_scheme.trim_end_matches('/').to_string()
    }

    /// One letter used as a placeholder icon until real favicons exist.
    pub fn initial(&self) -> String {
        // Stales' own pages (stales://…) get an S.
        if self.url.starts_with("stales://") {
            return "S".into();
        }
        self.title
            .trim_start_matches("www.")
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".into())
    }
}

/// Which tab is selected: one of the pinned ones at the top, or one in the normal list.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Selected {
    Pinned(usize),
    Tab(usize),
}

/// A Space is a separate set of tabs, like Arc's Spaces (e.g. "Personal", "Work").
#[derive(Debug, Serialize, Deserialize)]
pub struct Space {
    pub name: String,
    pub icon: String,
    /// The Space's colour, tinting the window ("blue", "#3b82f6", …). `None`: the theme's colours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub pinned: Vec<Tab>,
    pub tabs: Vec<Tab>,
    pub active: Option<Selected>,
    /// Folders, in the order they appear at the top of the tab list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<Folder>,
}

impl Space {
    pub fn new(name: &str, icon: &str) -> Self {
        Self {
            name: name.into(),
            icon: icon.into(),
            color: None,
            pinned: Vec::new(),
            tabs: Vec::new(),
            active: None,
            folders: Vec::new(),
        }
    }

    /// The tab list in the order it is shown: each folder's tabs (folders in order),
    /// then the tabs outside folders. Indexes into `tabs`.
    pub fn list_order(&self) -> Vec<usize> {
        let mut order = Vec::new();
        for folder in &self.folders {
            order.extend(self.tabs.iter().enumerate().filter(|(_, t)| t.folder == Some(folder.id)).map(|(i, _)| i));
        }
        let known = |f: Option<u64>| f.is_some_and(|id| self.folders.iter().any(|folder| folder.id == id));
        order.extend(self.tabs.iter().enumerate().filter(|(_, t)| !known(t.folder)).map(|(i, _)| i));
        order
    }
}

#[derive(Debug)]
pub struct State {
    pub spaces: Vec<Space>,
    pub current: usize,
    /// Search engines, from the config file.
    pub search: Search,
    /// Recently closed tabs, newest last, for Ctrl+Shift+T.
    pub closed: Vec<Tab>,
}

impl State {
    /// The very first start: one Space with the welcome page.
    pub fn first_run(search: Search) -> Self {
        let mut personal = Space::new("Personal", "◐");
        // First thing you see: the setup, where you choose how Stales looks and works.
        personal.tabs.push(Tab::from_input(crate::settings::SETUP_URL, &search));
        personal.tabs[0].title = "Set up Stales".into();
        personal.active = Some(Selected::Tab(0));
        Self { spaces: vec![personal], current: 0, search, closed: Vec::new() }
    }

    /// Example tabs used by the tests.
    #[cfg(test)]
    pub fn demo(search: Search) -> Self {
        let tab = |input: &str| Tab::from_input(input, &search);
        let mut personal = Space::new("Personal", "◐");
        personal.pinned = ["github.com", "youtube.com", "mail.proton.me"].map(tab).to_vec();
        personal.tabs = ["archlinux.org/news", "wiki.archlinux.org", "github.com/YaLTeR/niri"].map(tab).to_vec();
        personal.active = Some(Selected::Tab(0));

        let mut dev = Space::new("Dev", "◆");
        dev.pinned = vec![tab("docs.rs")];
        dev.tabs = ["doc.rust-lang.org/book", "gtk-rs.org"].map(tab).to_vec();
        dev.active = Some(Selected::Tab(0));

        Self { spaces: vec![personal, dev], current: 0, search, closed: Vec::new() }
    }

    pub fn space(&self) -> &Space {
        &self.spaces[self.current]
    }

    fn space_mut(&mut self) -> &mut Space {
        &mut self.spaces[self.current]
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        let space = self.space();
        match space.active? {
            Selected::Pinned(i) => space.pinned.get(i),
            Selected::Tab(i) => space.tabs.get(i),
        }
    }

    pub fn active_id(&self) -> Option<TabId> {
        self.active_tab().map(|t| t.id)
    }

    /// Find a tab in any Space (pages keep loading in the background).
    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.spaces.iter_mut().flat_map(|s| s.pinned.iter_mut().chain(s.tabs.iter_mut())).find(|t| t.id == id)
    }

    /// Find a tab anywhere, including recently closed ones.
    pub fn any_tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.spaces
            .iter_mut()
            .flat_map(|s| s.pinned.iter_mut().chain(s.tabs.iter_mut()))
            .chain(self.closed.iter_mut())
            .find(|t| t.id == id)
    }

    /// Every tab that still exists, so the window can drop web views of closed tabs.
    pub fn all_ids(&self) -> Vec<TabId> {
        self.spaces.iter().flat_map(|s| s.pinned.iter().chain(&s.tabs)).map(|t| t.id).collect()
    }

    /// Close a tab by id, for when a page closes itself (`window.close()`).
    pub fn close_id(&mut self, id: TabId) {
        for space_index in 0..self.spaces.len() {
            if let Some(i) = self.spaces[space_index].tabs.iter().position(|t| t.id == id) {
                let current = self.current;
                self.current = space_index;
                self.close_tab(i);
                self.current = current;
                return;
            }
        }
    }

    /// Select a tab. Returns false (and changes nothing) if that tab doesn't exist.
    pub fn select(&mut self, selected: Selected) -> bool {
        let space = self.space_mut();
        let exists = match selected {
            Selected::Pinned(i) => i < space.pinned.len(),
            Selected::Tab(i) => i < space.tabs.len(),
        };
        if exists {
            space.active = Some(selected);
        }
        exists
    }

    /// New tabs go to the top of the list, like in Arc.
    pub fn open_tab(&mut self, input: &str) {
        let tab = Tab::from_input(input, &self.search);
        let space = self.space_mut();
        space.tabs.insert(0, tab);
        space.active = Some(Selected::Tab(0));
    }

    /// Open a tab without switching to it (link hints with `F`). It goes to the
    /// top of the list, so the active tab moves down by one.
    pub fn open_tab_background(&mut self, input: &str) {
        let tab = Tab::from_input(input, &self.search);
        let space = self.space_mut();
        space.tabs.insert(0, tab);
        if let Some(Selected::Tab(i)) = space.active {
            space.active = Some(Selected::Tab(i + 1));
        }
    }

    /// Load something new in the current tab (what typing in the address bar does).
    pub fn navigate_active(&mut self, input: &str) {
        let new = Tab::from_input(input, &self.search);
        let space = self.space_mut();
        let tab = match space.active {
            Some(Selected::Pinned(i)) => space.pinned.get_mut(i),
            Some(Selected::Tab(i)) => space.tabs.get_mut(i),
            None => None,
        };
        match tab {
            // Keep the same id so the tab keeps its web view (and its back/forward history).
            Some(tab) => {
                let id = tab.id;
                *tab = Tab { id, ..new };
            }
            None => self.open_tab(input),
        }
    }

    pub fn close_tab(&mut self, index: usize) {
        let space = self.space_mut();
        if index >= space.tabs.len() {
            return;
        }
        let tab = space.tabs.remove(index);
        self.closed.push(tab);
        if self.closed.len() > 50 {
            self.closed.remove(0);
        }
        let space = self.space_mut();
        space.active = match space.active {
            // Closed the active tab: select the one that moved into its place, or the one above.
            Some(Selected::Tab(i)) if i == index => {
                if space.tabs.is_empty() {
                    None
                } else {
                    Some(Selected::Tab(index.min(space.tabs.len() - 1)))
                }
            }
            // A tab above the active one went away, so the active one moved up by one.
            Some(Selected::Tab(i)) if i > index => Some(Selected::Tab(i - 1)),
            other => other,
        };
    }

    /// Ctrl+W: close the active tab. Pinned tabs are never closed this way, like in Arc.
    pub fn close_active(&mut self) {
        if let Some(Selected::Tab(i)) = self.space().active {
            self.close_tab(i);
        }
    }

    /// Returns false (and changes nothing) if that Space doesn't exist.
    pub fn switch_space(&mut self, index: usize) -> bool {
        let exists = index < self.spaces.len();
        if exists {
            self.current = index;
        }
        exists
    }

    /// All tabs of the current Space in sidebar order: pinned first, then the list.
    fn order(&self) -> Vec<Selected> {
        let space = self.space();
        (0..space.pinned.len()).map(Selected::Pinned).chain(space.list_order().into_iter().map(Selected::Tab)).collect()
    }

    /// Ctrl+Tab / Ctrl+Shift+Tab: the next (1) or previous (-1) tab, wrapping around.
    pub fn select_relative(&mut self, delta: isize) {
        let order = self.order();
        if order.is_empty() {
            return;
        }
        let current = self.space().active.and_then(|a| order.iter().position(|s| *s == a));
        let next = match current {
            Some(i) => (i as isize + delta).rem_euclid(order.len() as isize) as usize,
            None => 0,
        };
        self.select(order[next]);
    }

    /// Ctrl+1 … Ctrl+8: the nth tab in sidebar order (0-based). `None` means the last one (Ctrl+9).
    pub fn select_nth(&mut self, n: Option<usize>) {
        let order = self.order();
        let pick = match n {
            Some(n) => order.get(n),
            None => order.last(),
        };
        if let Some(&selected) = pick {
            self.select(selected);
        }
    }

    /// Ctrl+Shift+T: bring back the most recently closed tab.
    pub fn reopen_closed(&mut self) -> bool {
        let Some(tab) = self.closed.pop() else { return false };
        let space = self.space_mut();
        space.tabs.insert(0, tab);
        space.active = Some(Selected::Tab(0));
        true
    }

    /// Ctrl+D: move the active tab to the pinned tiles, or back to the list.
    #[cfg(test)]
    pub fn toggle_pin(&mut self) {
        let space = self.space_mut();
        match space.active {
            Some(Selected::Tab(i)) if i < space.tabs.len() => {
                let tab = space.tabs.remove(i);
                space.pinned.push(tab);
                space.active = Some(Selected::Pinned(space.pinned.len() - 1));
            }
            Some(Selected::Pinned(i)) if i < space.pinned.len() => {
                let tab = space.pinned.remove(i);
                space.tabs.insert(0, tab);
                space.active = Some(Selected::Tab(0));
            }
            _ => {}
        }
    }

    /// Drag and drop: move a tab or pinned tile to the pinned tiles (`to_pinned`) or the
    /// tab list, before position `index` (past the end = at the end). The moved tab
    /// stays selected if it was. Returns false if nothing moved.
    pub fn move_tab(&mut self, from: Selected, to_pinned: bool, index: usize) -> bool {
        self.move_tab_into(from, to_pinned, index, None)
    }

    /// Like `move_tab`, and when it lands in the tab list, puts it in `folder` (or out of
    /// any folder with `None`).
    pub fn move_tab_into(&mut self, from: Selected, to_pinned: bool, index: usize, folder: Option<u64>) -> bool {
        let space = self.space_mut();
        let was_active = space.active == Some(from);
        let active_id = space.active.and_then(|a| match a {
            Selected::Pinned(i) => space.pinned.get(i).map(|t| t.id),
            Selected::Tab(i) => space.tabs.get(i).map(|t| t.id),
        });
        let tab = match from {
            Selected::Pinned(i) if i < space.pinned.len() => space.pinned.remove(i),
            Selected::Tab(i) if i < space.tabs.len() => space.tabs.remove(i),
            _ => return false,
        };
        let mut tab = tab;
        tab.folder = if to_pinned { None } else { folder };
        let moved_id = tab.id;
        // Removing an item before the drop spot shifts the spot by one.
        let same_list = matches!(from, Selected::Pinned(_)) == to_pinned;
        let from_index = match from {
            Selected::Pinned(i) | Selected::Tab(i) => i,
        };
        let index = if same_list && from_index < index { index - 1 } else { index };
        let list = if to_pinned { &mut space.pinned } else { &mut space.tabs };
        let index = index.min(list.len());
        list.insert(index, tab);
        // Keep the same tab selected, wherever it ended up.
        let target = if was_active { Some(moved_id) } else { active_id };
        space.active = target.and_then(|id| {
            space
                .pinned
                .iter()
                .position(|t| t.id == id)
                .map(Selected::Pinned)
                .or_else(|| space.tabs.iter().position(|t| t.id == id).map(Selected::Tab))
        });
        true
    }

    /// A new folder in the current Space, holding `tab` if given. Returns its id.
    pub fn add_folder(&mut self, tab: Option<Selected>) -> u64 {
        let space = self.space_mut();
        let id = space.folders.iter().map(|f| f.id).max().unwrap_or(0) + 1;
        space.folders.push(Folder { id, name: format!("Folder {id}"), open: true });
        if let Some(from) = tab {
            let index = space.tabs.len();
            self.move_tab_into(from, false, index, Some(id));
        }
        id
    }

    pub fn rename_folder(&mut self, id: u64, name: &str) {
        let name = name.trim();
        if let Some(folder) = self.space_mut().folders.iter_mut().find(|f| f.id == id)
            && !name.is_empty()
        {
            folder.name = name.chars().take(40).collect();
        }
    }

    pub fn toggle_folder(&mut self, id: u64) {
        if let Some(folder) = self.space_mut().folders.iter_mut().find(|f| f.id == id) {
            folder.open = !folder.open;
        }
    }

    /// Remove a folder; its tabs stay, outside any folder.
    pub fn delete_folder(&mut self, id: u64) {
        let space = self.space_mut();
        space.folders.retain(|f| f.id != id);
        for tab in &mut space.tabs {
            if tab.folder == Some(id) {
                tab.folder = None;
            }
        }
    }

    /// Is this tab saved in a folder (so closing it should only unload it)?
    pub fn in_folder(&self, selected: Selected) -> bool {
        let space = self.space();
        match selected {
            Selected::Tab(i) => {
                space.tabs.get(i).and_then(|t| t.folder).is_some_and(|id| space.folders.iter().any(|f| f.id == id))
            }
            Selected::Pinned(_) => false,
        }
    }

    /// Ctrl+Shift+↑/↓: move the active tab up (-1) or down (1) in the list.
    pub fn move_active(&mut self, delta: isize) {
        let space = self.space_mut();
        if let Some(Selected::Tab(i)) = space.active {
            let target = i as isize + delta;
            if target >= 0 && (target as usize) < space.tabs.len() {
                space.tabs.swap(i, target as usize);
                space.active = Some(Selected::Tab(target as usize));
            }
        }
    }

    /// The next (1) or previous (-1) Space, wrapping around.
    pub fn switch_space_relative(&mut self, delta: isize) {
        let count = self.spaces.len() as isize;
        self.current = (self.current as isize + delta).rem_euclid(count) as usize;
    }

    pub fn add_space(&mut self) {
        let name = format!("Space {}", self.spaces.len() + 1);
        self.spaces.push(Space::new(&name, "●"));
        self.current = self.spaces.len() - 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_the_active_tab_selects_a_neighbour() {
        let mut state = State::demo(Search::default()); // Personal space has 3 tabs, tab 0 active
        state.select(Selected::Tab(2));
        state.close_active();
        assert_eq!(state.space().active, Some(Selected::Tab(1)));
        assert_eq!(state.space().tabs.len(), 2);
    }

    #[test]
    fn closing_a_tab_above_keeps_the_same_tab_active() {
        let mut state = State::demo(Search::default());
        state.select(Selected::Tab(2));
        let before = state.active_tab().cloned();
        state.close_tab(0);
        assert_eq!(state.active_tab().cloned(), before);
    }

    #[test]
    fn selecting_a_missing_tab_changes_nothing() {
        let mut state = State::demo(Search::default());
        assert!(!state.select(Selected::Tab(99)));
        assert!(!state.select(Selected::Pinned(99)));
        assert!(!state.switch_space(9));
        assert_eq!(state.space().active, Some(Selected::Tab(0)));
    }

    #[test]
    fn first_run_opens_the_setup() {
        let state = State::first_run(Search::default());
        assert_eq!(state.active_tab().unwrap().url, "stales://setup");
    }

    #[test]
    fn next_and_previous_tab_wrap_around_through_pinned() {
        let mut state = State::demo(Search::default()); // 3 pinned + 3 tabs, Tab(0) active
        state.select_relative(-1);
        assert_eq!(state.space().active, Some(Selected::Pinned(2)));
        state.select_nth(None);
        assert_eq!(state.space().active, Some(Selected::Tab(2)));
        state.select_relative(1);
        assert_eq!(state.space().active, Some(Selected::Pinned(0)));
    }

    #[test]
    fn reopen_brings_back_the_closed_tab() {
        let mut state = State::demo(Search::default());
        let url = state.active_tab().unwrap().url.clone();
        state.close_active();
        assert!(state.reopen_closed());
        assert_eq!(state.active_tab().unwrap().url, url);
        assert!(!state.reopen_closed());
    }

    #[test]
    fn pin_and_unpin() {
        let mut state = State::demo(Search::default());
        let id = state.active_id();
        state.toggle_pin();
        assert_eq!(state.space().pinned.len(), 4);
        assert_eq!(state.active_id(), id);
        state.toggle_pin();
        assert_eq!(state.space().pinned.len(), 3);
        assert_eq!(state.space().active, Some(Selected::Tab(0)));
    }

    #[test]
    fn dragging_tabs_to_pinned_and_back() {
        let mut state = State::demo(Search::default()); // pinned: 3, tabs: 3, Tab(0) active
        let active = state.active_id();
        let dragged = state.space().tabs[1].id;
        // Drag tab 1 onto the pinned tiles, before the first tile.
        assert!(state.move_tab(Selected::Tab(1), true, 0));
        assert_eq!(state.space().pinned[0].id, dragged);
        assert_eq!(state.space().pinned.len(), 4);
        assert_eq!(state.active_id(), active, "the selected tab must stay selected");
        // Drag it back to the end of the list.
        assert!(state.move_tab(Selected::Pinned(0), false, 99));
        assert_eq!(state.space().tabs.last().unwrap().id, dragged);
        // Reorder within the list: tab 0 to after tab 1.
        let first = state.space().tabs[0].id;
        assert!(state.move_tab(Selected::Tab(0), false, 2));
        assert_eq!(state.space().tabs[1].id, first);
        assert!(!state.move_tab(Selected::Tab(42), true, 0));
    }

    #[test]
    fn folders_group_tabs_and_keep_order() {
        let mut state = State::demo(Search::default()); // tabs: arch news, arch wiki, niri
        let niri = state.space().tabs[2].id;
        let folder = state.add_folder(Some(Selected::Tab(2)));
        assert_eq!(state.space().tabs.iter().find(|t| t.id == niri).unwrap().folder, Some(folder));
        // Folder tabs come first in the shown order.
        let order = state.space().list_order();
        assert_eq!(state.space().tabs[order[0]].id, niri);
        assert!(state.in_folder(Selected::Tab(order[0])));
        state.rename_folder(folder, "  Linux  ");
        assert_eq!(state.space().folders[0].name, "Linux");
        // Deleting the folder keeps the tab.
        state.delete_folder(folder);
        assert!(state.space().tabs.iter().any(|t| t.id == niri && t.folder.is_none()));
        assert!(state.space().folders.is_empty());
    }

    #[test]
    fn moving_a_tab() {
        let mut state = State::demo(Search::default());
        let id = state.active_id();
        state.move_active(1);
        assert_eq!(state.space().active, Some(Selected::Tab(1)));
        assert_eq!(state.active_id(), id);
        state.move_active(-1);
        state.move_active(-1); // already at the top: nothing happens
        assert_eq!(state.space().active, Some(Selected::Tab(0)));
    }

    #[test]
    fn background_tab_keeps_the_active_tab() {
        let mut state = State::demo(Search::default());
        let id = state.active_id();
        state.open_tab_background("example.com");
        assert_eq!(state.active_id(), id);
        assert_eq!(state.space().tabs[0].url, "https://example.com");
    }

    #[test]
    fn navigating_keeps_the_tab_id() {
        let mut state = State::demo(Search::default());
        let id = state.active_id();
        state.navigate_active("example.com");
        assert_eq!(state.active_id(), id);
        assert_eq!(state.active_tab().unwrap().url, "https://example.com");
    }

    #[test]
    fn close_id_works_in_another_space() {
        let mut state = State::demo(Search::default());
        let dev_tab = state.spaces[1].tabs[0].id;
        state.close_id(dev_tab);
        assert_eq!(state.spaces[1].tabs.len(), 1);
        assert_eq!(state.current, 0);
    }

    #[test]
    fn closing_the_last_tab_leaves_nothing_selected() {
        let mut state = State::demo(Search::default());
        state.switch_space(1); // Dev space has 2 tabs
        state.close_active();
        state.close_active();
        assert_eq!(state.space().active, None);
    }
}
