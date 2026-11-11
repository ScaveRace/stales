//! The command bar: a box that floats over the window when you press Ctrl+T or Ctrl+L.
//!
//! Type an address or a search and press Enter. While you type, matching open tabs
//! show up underneath; click one to switch to it. Escape or a click outside closes it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk::Key;
use gtk::glib::Propagation;
use gtk::pango::EllipsizeMode;
use gtk::prelude::*;

use crate::state::Selected;
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
}

/// A row in the results list.
#[derive(Clone, Copy)]
enum Pick {
    /// The first row: open what was typed (an address, or a search).
    Typed,
    /// Switch to an open tab.
    Tab(Selected),
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

    CommandBar { scrim, entry, results, mode: Cell::new(Mode::NewTab), matches: RefCell::new(Vec::new()) }
}

pub fn connect(ui: &Rc<Ui>) {
    let cmd = &ui.command;

    // Enter
    let u = ui.clone();
    cmd.entry.connect_activate(move |_| submit(&u));

    // Typing updates the list of matching tabs
    let u = ui.clone();
    cmd.entry.connect_changed(move |_| update_results(&u));

    // Escape
    let keys = gtk::EventControllerKey::new();
    let u = ui.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == Key::Escape {
            close(&u);
            Propagation::Stop
        } else {
            Propagation::Proceed
        }
    });
    cmd.entry.add_controller(keys);

    // Clicking a row
    let u = ui.clone();
    cmd.results.connect_row_activated(move |_, row| {
        let picked = usize::try_from(row.index()).ok().and_then(|i| u.command.matches.borrow().get(i).copied());
        match picked {
            Some(Pick::Typed) => submit(&u),
            Some(Pick::Tab(selected)) => {
                u.state.borrow_mut().select(selected);
                close(&u);
                window::refresh_later(&u);
            }
            None => {}
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

/// Open (or search for) what was typed.
fn submit(ui: &Rc<Ui>) {
    let text = ui.command.entry.text().to_string();
    close(ui);
    if text.trim().is_empty() {
        return;
    }
    match ui.command.mode.get() {
        Mode::NewTab => {
            ui.state.borrow_mut().open_tab(&text);
            window::refresh(ui);
            window::focus_page(ui);
        }
        Mode::EditCurrent => {
            ui.state.borrow_mut().navigate_active(&text);
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
    ui.command.scrim.set_visible(false);
    window::focus_page(ui);
}

fn update_results(ui: &Rc<Ui>) {
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
    let label = gtk::Label::new(Some(&action));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(EllipsizeMode::End);
    label.set_max_width_chars(1);
    let enter = gtk::Label::new(Some("Enter"));
    enter.add_css_class("st-muted");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&label);
    content.append(&enter);
    let row = gtk::ListBoxRow::new();
    row.add_css_class("st-result");
    row.add_css_class("st-result-primary");
    row.set_child(Some(&content));
    cmd.results.append(&row);
    matches.push(Pick::Typed);

    let space = state.space();
    let pinned = space.pinned.iter().enumerate().map(|(i, t)| (Selected::Pinned(i), t));
    let tabs = space.tabs.iter().enumerate().map(|(i, t)| (Selected::Tab(i), t));
    for (selected, tab) in pinned.chain(tabs) {
        if matches.len() >= 7 {
            break;
        }
        if !tab.title.to_lowercase().contains(&query) && !tab.url.to_lowercase().contains(&query) {
            continue;
        }
        let title = gtk::Label::new(Some(&tab.title));
        title.set_xalign(0.0);
        title.set_ellipsize(EllipsizeMode::End);
        title.set_max_width_chars(40);
        let url = gtk::Label::new(Some(&tab.display_url()));
        url.add_css_class("st-muted");
        url.set_xalign(1.0);
        url.set_hexpand(true);
        url.set_ellipsize(EllipsizeMode::Start);
        url.set_max_width_chars(1);
        let hint = gtk::Label::new(Some("Switch to tab"));
        hint.add_css_class("st-muted");

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        content.append(&title);
        content.append(&url);
        content.append(&hint);
        let row = gtk::ListBoxRow::new();
        row.add_css_class("st-result");
        row.set_child(Some(&content));
        cmd.results.append(&row);
        matches.push(Pick::Tab(selected));
    }
    cmd.results.set_visible(!matches.is_empty());
}
