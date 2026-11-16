//! The command bar: a box that floats over the window when you press Ctrl+T or Ctrl+L.
//!
//! Type an address or a search and press Enter. While you type, rows show up
//! underneath: what Enter will do, search suggestions from your engine (see
//! suggest.rs), matching open tabs and pages from your history.
//!
//! Keyboard: ↓/↑ (or Ctrl+N/Ctrl+P) move the highlight through the rows while you
//! keep typing, Enter opens the highlighted row, Tab copies the highlighted
//! suggestion into the box. Escape or a click outside closes the bar.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk::{Key, ModifierType};
use gtk::glib::{self, Propagation};
use gtk::pango::EllipsizeMode;
use gtk::prelude::*;

use crate::state::Selected;
use crate::suggest;
use crate::window::{self, Ui};

/// What Enter does with the text.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// Ctrl+T: open it in a new tab.
    NewTab,
    /// Ctrl+L: load it in the current tab.
    EditCurrent,
}

pub struct CommandBar {
    /// Dark layer over the whole window; the bar sits inside it.
    pub scrim: gtk::Box,
    pub entry: gtk::Entry,
    pub results: gtk::ListBox,
    pub mode: Cell<Mode>,
    /// What each row in `results` does when picked.
    matches: RefCell<Vec<Pick>>,
    /// Which row is highlighted (Enter opens it). 0 is the first row.
    highlight: Cell<usize>,
    /// The latest suggestions from the search engine.
    suggestions: RefCell<Suggestions>,
    /// The suggestion request that is waiting or running, so a newer one can stop it.
    pending: RefCell<Option<glib::JoinHandle<()>>>,
}

/// Suggestions, which engine sent them, and for which words.
#[derive(Default)]
struct Suggestions {
    engine: String,
    query: String,
    list: Vec<String>,
}

/// A row in the results list.
#[derive(Clone, PartialEq)]
enum Pick {
    /// The first row: open what was typed (an address, or a search).
    Typed,
    /// Search for this suggestion from the search engine.
    Suggestion(String),
    /// Switch to an open tab.
    Tab(Selected),
    /// A page from your history: open this address.
    Visit(String),
}

pub fn build() -> CommandBar {
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Search or enter address"));

    let results = gtk::ListBox::new();
    results.add_css_class("st-results");
    results.set_selection_mode(gtk::SelectionMode::None);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
    card.add_css_class("st-command");
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Start);
    card.set_margin_top(120);
    card.set_width_request(620);
    card.append(&entry);
    card.append(&results);

    let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scrim.add_css_class("st-scrim");
    scrim.append(&card);
    scrim.set_visible(false);

    CommandBar {
        scrim,
        entry,
        results,
        mode: Cell::new(Mode::NewTab),
        matches: RefCell::new(Vec::new()),
        highlight: Cell::new(0),
        suggestions: RefCell::new(Suggestions::default()),
        pending: RefCell::new(None),
    }
}

pub fn connect(ui: &Rc<Ui>) {
    let cmd = &ui.command;

    // Enter opens the highlighted row.
    let u = ui.clone();
    cmd.entry.connect_activate(move |_| pick(&u, u.command.highlight.get()));

    // Typing updates the rows and asks for new suggestions.
    let u = ui.clone();
    cmd.entry.connect_changed(move |_| {
        u.command.highlight.set(0);
        update_results(&u, None);
        ask_for_suggestions(&u);
    });

    // Keys: checked before the text box sees them ("capture"), so ↑/↓ and Tab
    // don't move the keyboard focus out of the box.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let u = ui.clone();
    keys.connect_key_pressed(move |_, key, _, modifiers| on_key(&u, key, modifiers));
    cmd.entry.add_controller(keys);

    // Clicking a row
    let u = ui.clone();
    cmd.results.connect_row_activated(move |_, row| {
        if let Ok(index) = usize::try_from(row.index()) {
            pick(&u, index);
        }
    });

    // Clicking the dark area outside the bar
    let click = gtk::GestureClick::new();
    let u = ui.clone();
    click.connect_pressed(move |_, _, x, y| {
        let scrim = &u.command.scrim;
        let hit = scrim.pick(x, y, gtk::PickFlags::DEFAULT);
        if hit.as_ref() == Some(scrim.upcast_ref()) {
            close(&u);
        }
    });
    cmd.scrim.add_controller(click);
}

fn on_key(ui: &Rc<Ui>, key: Key, modifiers: ModifierType) -> Propagation {
    let ctrl = modifiers.contains(ModifierType::CONTROL_MASK);
    let rows = ui.command.matches.borrow().len();
    match key {
        Key::Escape => close(ui),
        Key::Down | Key::KP_Down if rows > 0 => move_highlight(ui, 1),
        Key::Up | Key::KP_Up if rows > 0 => move_highlight(ui, -1),
        Key::n | Key::N if ctrl && rows > 0 => move_highlight(ui, 1),
        Key::p | Key::P if ctrl && rows > 0 => move_highlight(ui, -1),
        Key::Tab | Key::ISO_Left_Tab if !ctrl => complete(ui),
        _ => return Propagation::Proceed,
    }
    Propagation::Stop
}

/// Move the highlight down (+1) or up (-1), wrapping around at the ends.
fn move_highlight(ui: &Rc<Ui>, step: isize) {
    let cmd = &ui.command;
    let rows = cmd.matches.borrow().len();
    if rows == 0 {
        return;
    }
    let next = (cmd.highlight.get() as isize + step).rem_euclid(rows as isize) as usize;
    set_highlight(cmd, next);
}

/// Show which row is highlighted. It gets the look of the first row ("Enter does this").
fn set_highlight(cmd: &CommandBar, index: usize) {
    cmd.highlight.set(index);
    let mut i = 0;
    while let Some(row) = cmd.results.row_at_index(i) {
        if i as usize == index {
            row.add_css_class("st-result-primary");
        } else {
            row.remove_css_class("st-result-primary");
        }
        i += 1;
    }
}

/// Tab: put the highlighted suggestion (or the first one) into the box, to edit it further.
fn complete(ui: &Rc<Ui>) {
    let cmd = &ui.command;
    let text = {
        let matches = cmd.matches.borrow();
        let highlighted = matches.get(cmd.highlight.get());
        let wanted = match highlighted {
            Some(Pick::Typed) | None => matches.iter().find(|p| matches!(p, Pick::Suggestion(_))),
            other => other,
        };
        match wanted {
            Some(Pick::Suggestion(words)) => Some(format!("{}{words}", shortcut_typed(ui))),
            Some(Pick::Visit(url)) => Some(url.clone()),
            Some(Pick::Tab(selected)) => {
                let state = ui.state.borrow();
                let space = state.space();
                let tab = match selected {
                    Selected::Pinned(i) => space.pinned.get(*i),
                    Selected::Tab(i) => space.tabs.get(*i),
                };
                tab.map(|t| t.url.clone())
            }
            _ => None,
        }
    };
    if let Some(text) = text {
        cmd.entry.set_text(&text);
        cmd.entry.set_position(-1);
    }
}

/// "@w " if the text starts with an engine shortcut, else "".
fn shortcut_typed(ui: &Ui) -> String {
    let typed = ui.command.entry.text().trim().to_string();
    let state = ui.state.borrow();
    let (engine, query) = state.search.pick(&typed);
    if query.len() == typed.len() { String::new() } else { format!("@{} ", engine.keyword) }
}

/// Do what row `index` says.
fn pick(ui: &Rc<Ui>, index: usize) {
    let picked = ui.command.matches.borrow().get(index).cloned();
    match picked {
        Some(Pick::Typed) | None => go(ui, &ui.command.entry.text()),
        Some(Pick::Suggestion(words)) => {
            // Always search (with the engine that suggested it), even if the
            // suggestion happens to look like an address.
            let keyword = {
                let typed = ui.command.entry.text().trim().to_string();
                ui.state.borrow().search.pick(&typed).0.keyword.clone()
            };
            go(ui, &format!("@{keyword} {words}"));
        }
        Some(Pick::Tab(selected)) => {
            ui.state.borrow_mut().select(selected);
            close(ui);
            window::refresh_later(ui);
        }
        Some(Pick::Visit(url)) => go(ui, &url),
    }
}

/// Open (or search for) `text`: in a new tab for Ctrl+T, in this tab for Ctrl+L.
fn go(ui: &Rc<Ui>, text: &str) {
    close(ui);
    if text.trim().is_empty() {
        return;
    }
    match ui.command.mode.get() {
        Mode::NewTab => {
            ui.state.borrow_mut().open_tab(text);
            window::refresh(ui);
            window::focus_page(ui);
        }
        Mode::EditCurrent => {
            ui.state.borrow_mut().navigate_active(text);
            window::load_active(ui);
        }
    }
}

pub fn open(ui: &Rc<Ui>, mode: Mode) {
    let cmd = &ui.command;
    cmd.mode.set(mode);
    let text = match mode {
        Mode::EditCurrent => ui.state.borrow().active_tab().map(|t| t.url.clone()).unwrap_or_default(),
        Mode::NewTab => String::new(),
    };
    cmd.entry.set_text(&text);
    // Only list matches once the user starts typing, not for the pre-filled URL.
    clear_results(cmd);
    cmd.suggestions.replace(Suggestions::default());
    cmd.scrim.set_visible(true);
    cmd.entry.grab_focus();
    cmd.entry.select_region(0, -1);
}

fn clear_results(cmd: &CommandBar) {
    cmd.results.remove_all();
    cmd.results.set_visible(false);
    cmd.matches.borrow_mut().clear();
}

pub fn close(ui: &Rc<Ui>) {
    stop_asking(&ui.command);
    ui.command.scrim.set_visible(false);
    window::focus_page(ui);
}

/// Forget a suggestion request that hasn't answered yet.
fn stop_asking(cmd: &CommandBar) {
    if let Some(old) = cmd.pending.take() {
        old.abort();
    }
}

/// Ask the search engine for suggestions for the current text, a moment after the
/// last key press. Typing again before the answer arrives stops this request.
fn ask_for_suggestions(ui: &Rc<Ui>) {
    let cmd = &ui.command;
    stop_asking(cmd);
    if !ui.config.borrow().search.suggestions {
        return;
    }
    let typed = cmd.entry.text().trim().to_string();
    let (engine, query) = {
        let state = ui.state.borrow();
        let (engine, query) = state.search.pick(&typed);
        (engine.clone(), query.to_string())
    };
    if engine.suggest_url.is_none() || !suggest::worth_asking(&query) {
        return;
    }
    let weak = Rc::downgrade(ui);
    let task = glib::spawn_future_local(async move {
        glib::timeout_future(suggest::DELAY).await;
        let list = suggest::fetch(&engine, &query).await;
        let Some(ui) = weak.upgrade() else { return };
        let cmd = &ui.command;
        // Only use the answer if it is for what is in the box right now.
        if !cmd.scrim.is_visible() || cmd.entry.text().trim() != typed {
            return;
        }
        cmd.suggestions.replace(Suggestions { engine: engine.keyword.clone(), query, list });
        let keep = cmd.matches.borrow().get(cmd.highlight.get()).cloned();
        update_results(&ui, keep);
    });
    cmd.pending.replace(Some(task));
}

/// Rebuild the rows for the current text. `keep` is a row to keep highlighted if it
/// is still there (when suggestions arrive while you move through the rows).
fn update_results(ui: &Rc<Ui>, keep: Option<Pick>) {
    let cmd = &ui.command;
    clear_results(cmd);
    let typed = cmd.entry.text().trim().to_string();
    if typed.is_empty() {
        return;
    }
    let query = typed.to_lowercase();
    let mut matches = cmd.matches.borrow_mut();
    let state = ui.state.borrow();

    // First row: what Enter will do, so you can see which search engine is used.
    let resolved = state.search.resolve(&typed);
    let (icon, action) = match &resolved.engine {
        Some(engine) => ("system-search-symbolic", format!("Search {engine} for “{}”", resolved.title)),
        None => ("web-browser-symbolic", format!("Open {}", resolved.url)),
    };
    cmd.results.append(&row(Some(icon), &action, None, "Enter"));
    matches.push(Pick::Typed);

    // Then suggestions from the search engine, if they still fit what is typed.
    let (engine, words) = state.search.pick(&typed);
    let stored = cmd.suggestions.borrow();
    if ui.config.borrow().search.suggestions && stored.engine == engine.keyword && suggest::worth_asking(words) {
        for words in suggest::still_fitting(&stored.query, &stored.list, words) {
            cmd.results.append(&row(Some("system-search-symbolic"), &words, None, "Search"));
            matches.push(Pick::Suggestion(words));
        }
    }
    let first_tab_row = matches.len();

    // Then open tabs.
    let space = state.space();
    let open_urls: Vec<&str> = space.pinned.iter().chain(&space.tabs).map(|t| t.url.as_str()).collect();
    let pinned = space.pinned.iter().enumerate().map(|(i, t)| (Selected::Pinned(i), t));
    let tabs = space.tabs.iter().enumerate().map(|(i, t)| (Selected::Tab(i), t));
    for (selected, tab) in pinned.chain(tabs) {
        if matches.len() >= first_tab_row + 6 {
            break;
        }
        if !tab.title.to_lowercase().contains(&query) && !tab.url.to_lowercase().contains(&query) {
            continue;
        }
        cmd.results.append(&row(None, &tab.title, Some(&tab.display_url()), "Switch to tab"));
        matches.push(Pick::Tab(selected));
    }

    // Then pages from your history (not already open as a tab).
    for visit in ui.history.borrow().suggestions(&typed, 12) {
        if matches.len() >= (first_tab_row + 9).min(12) {
            break;
        }
        if open_urls.contains(&visit.url.as_str()) {
            continue;
        }
        let url = visit.url.trim_start_matches("https://").trim_end_matches('/');
        cmd.results.append(&row(Some("document-open-recent-symbolic"), &visit.title, Some(url), "History"));
        matches.push(Pick::Visit(visit.url));
    }
    cmd.results.set_visible(!matches.is_empty());

    let highlight = keep.and_then(|keep| matches.iter().position(|p| *p == keep)).unwrap_or(0);
    drop(matches);
    set_highlight(cmd, highlight);
}

/// One row: an optional icon, the main text, an optional grey address, and a grey hint.
fn row(icon: Option<&str>, text: &str, address: Option<&str>, hint: &str) -> gtk::ListBoxRow {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    if let Some(icon) = icon {
        content.append(&gtk::Image::from_icon_name(icon));
    }
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_ellipsize(EllipsizeMode::End);
    content.append(&label);
    match address {
        Some(address) => {
            label.set_max_width_chars(40);
            let url = gtk::Label::new(Some(address));
            url.add_css_class("st-muted");
            url.set_xalign(1.0);
            url.set_hexpand(true);
            url.set_ellipsize(EllipsizeMode::Start);
            url.set_max_width_chars(1);
            content.append(&url);
        }
        None => {
            label.set_hexpand(true);
            label.set_max_width_chars(1);
        }
    }
    let hint = gtk::Label::new(Some(hint));
    hint.add_css_class("st-muted");
    content.append(&hint);
    let row = gtk::ListBoxRow::new();
    row.add_css_class("st-result");
    row.set_child(Some(&content));
    row
}

// ---------- For automated tests (see control.rs) ----------

/// The rows as one line; `>` marks the highlighted row.
pub fn describe(ui: &Ui) -> String {
    let cmd = &ui.command;
    let rows: Vec<String> = cmd
        .matches
        .borrow()
        .iter()
        .enumerate()
        .map(|(i, pick)| {
            let mark = if i == cmd.highlight.get() { ">" } else { " " };
            let what = match pick {
                Pick::Typed => "typed".to_string(),
                Pick::Suggestion(words) => format!("suggestion:{words}"),
                Pick::Tab(selected) => format!("tab:{selected:?}"),
                Pick::Visit(url) => format!("visit:{url}"),
            };
            format!("{mark}{what}")
        })
        .collect();
    format!("text={:?} rows=[{}]", cmd.entry.text(), rows.join(" | "))
}

/// Press a key in the bar: down, up, ctrl-n, ctrl-p, tab or enter.
pub fn press(ui: &Rc<Ui>, name: &str) -> bool {
    let (key, modifiers) = match name {
        "down" => (Key::Down, ModifierType::empty()),
        "up" => (Key::Up, ModifierType::empty()),
        "ctrl-n" => (Key::n, ModifierType::CONTROL_MASK),
        "ctrl-p" => (Key::p, ModifierType::CONTROL_MASK),
        "tab" => (Key::Tab, ModifierType::empty()),
        "enter" => {
            ui.command.entry.emit_activate();
            return true;
        }
        _ => return false,
    };
    on_key(ui, key, modifiers);
    true
}
