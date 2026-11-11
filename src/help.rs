//! The cheat sheet (F1 or `?`): every shortcut, built from the action list, so it
//! always shows the keys you actually set in config.toml.

use std::rc::Rc;

use gtk::gdk::Key;
use gtk::glib::Propagation;
use gtk::prelude::*;

use crate::actions::{self, ACTIONS};
use crate::window::{self, Ui};

pub struct Help {
    pub scrim: gtk::Box,
    /// The sections, side by side; they wrap onto new rows in a narrow window.
    content: gtk::FlowBox,
}

pub fn build() -> Help {
    let content = gtk::FlowBox::new();
    content.set_selection_mode(gtk::SelectionMode::None);
    content.set_max_children_per_line(4);
    content.set_column_spacing(32);
    content.set_row_spacing(16);
    content.set_valign(gtk::Align::Start);
    let scroller =
        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&content).build();

    let title = gtk::Label::new(Some("Keyboard shortcuts"));
    title.add_css_class("st-help-title");
    title.set_xalign(0.0);
    let hint = gtk::Label::new(Some("Change any of them in ~/.config/stales/config.toml · Esc to close"));
    hint.add_css_class("st-muted");
    hint.set_xalign(0.0);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("st-help");
    // The card fills the window except a margin, and scrolls if the list is taller.
    card.set_margin_top(48);
    card.set_margin_bottom(48);
    card.set_margin_start(64);
    card.set_margin_end(64);
    card.set_vexpand(true);
    card.append(&title);
    card.append(&hint);
    card.append(&scroller);

    let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scrim.add_css_class("st-scrim");
    scrim.set_focusable(true);
    scrim.append(&card);
    scrim.set_visible(false);
    Help { scrim, content }
}

pub fn connect(ui: &Rc<Ui>) {
    let keys = gtk::EventControllerKey::new();
    let u = ui.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if matches!(key, Key::Escape | Key::q | Key::question) {
            close(&u);
            Propagation::Stop
        } else {
            Propagation::Proceed
        }
    });
    ui.help.scrim.add_controller(keys);

    let click = gtk::GestureClick::new();
    let u = ui.clone();
    click.connect_pressed(move |_, _, _, _| close(&u));
    ui.help.scrim.add_controller(click);
}

pub fn toggle(ui: &Rc<Ui>) {
    if ui.help.scrim.is_visible() {
        close(ui);
    } else {
        fill(ui);
        ui.help.scrim.set_visible(true);
        ui.help.scrim.grab_focus();
    }
}

fn close(ui: &Rc<Ui>) {
    ui.help.scrim.set_visible(false);
    window::focus_page(ui);
}

fn fill(ui: &Rc<Ui>) {
    let content = &ui.help.content;
    content.remove_all();

    let mut column = None;
    let mut last_group = "";
    for action in ACTIONS {
        // Ctrl+2 … Ctrl+8 and Alt+2 … Alt+5 would make the list long; show the first and last.
        if matches!(action.name, "tab-2" | "tab-3" | "tab-4" | "tab-5" | "tab-6" | "tab-7")
            || matches!(action.name, "space-2" | "space-3" | "space-4")
        {
            continue;
        }
        if action.group != last_group {
            last_group = action.group;
            let grid = section(content, action.group);
            column = Some((grid, 0));
        }
        let keys: Vec<String> = actions::keys_for(&ui.config.borrow().keys, action)
            .iter()
            // GTK needs this second spelling of Shift+Tab, but people don't.
            .filter(|k| !k.contains("ISO_Left_Tab"))
            .map(|k| actions::pretty_key(k))
            .collect();
        let description = match action.name {
            "tab-8" => "Go to tab 2 … 8",
            "space-5" => "Go to Space 2 … 5",
            _ => action.description,
        };
        if let Some((grid, row)) = column.as_mut() {
            add_row(grid, *row, &keys.join("  "), description);
            *row += 1;
        }
    }

    if ui.config.borrow().keys.vim {
        let grid = section(content, "On web pages");
        let page_keys = actions::page_keys(&ui.config.borrow().keys);
        let mut row = 0;
        for (key, action, description) in actions::PAGE_KEYS {
            // Show what the config actually maps the key to.
            if page_keys.keys.get(*key).map(String::as_str) != Some(*action) {
                continue;
            }
            let shown = if *key == " " { "Space".to_string() } else { key.to_string() };
            add_row(&grid, row, &shown, description);
            row += 1;
        }
        add_row(&grid, row, "Esc", "Leave a text field");
    }
}

fn section(content: &gtk::FlowBox, name: &str) -> gtk::Grid {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let heading = gtk::Label::new(Some(name));
    heading.add_css_class("st-help-heading");
    heading.set_xalign(0.0);
    let grid = gtk::Grid::new();
    grid.set_row_spacing(5);
    grid.set_column_spacing(16);
    column.append(&heading);
    column.append(&grid);
    column.set_valign(gtk::Align::Start);
    content.insert(&column, -1);
    grid
}

fn add_row(grid: &gtk::Grid, row: i32, keys: &str, description: &str) {
    let key_label = gtk::Label::new(Some(keys));
    key_label.add_css_class("st-help-key");
    key_label.set_xalign(0.0);
    key_label.set_halign(gtk::Align::Start);
    let text = gtk::Label::new(Some(description));
    text.set_xalign(0.0);
    grid.attach(&key_label, 0, row, 1, 1);
    grid.attach(&text, 1, row, 1, 1);
}
