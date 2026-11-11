//! Find in page (Ctrl+F or `/`): a small bar in the bottom-right corner of the page.
//!
//! Type to search, Enter for the next match, Shift+Enter for the previous one,
//! Escape to close. Matches are highlighted by the web engine.

use std::rc::Rc;

use gtk::gdk::{Key, ModifierType};
use gtk::glib::Propagation;
use gtk::prelude::*;

use crate::window::{self, Ui};

pub struct FindBar {
    pub root: gtk::Box,
    pub entry: gtk::Entry,
    pub count: gtk::Label,
}

pub fn build() -> FindBar {
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Find in page"));
    entry.set_width_chars(22);
    let count = gtk::Label::new(None);
    count.add_css_class("st-muted");

    let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    root.add_css_class("st-findbar");
    root.set_halign(gtk::Align::End);
    root.set_valign(gtk::Align::End);
    root.set_margin_end(16);
    root.set_margin_bottom(16);
    root.append(&gtk::Image::from_icon_name("edit-find-symbolic"));
    root.append(&entry);
    root.append(&count);
    root.set_visible(false);
    FindBar { root, entry, count }
}

pub fn connect(ui: &Rc<Ui>) {
    let bar = &ui.find_bar;

    let u = ui.clone();
    bar.entry.connect_changed(move |entry| {
        if let Some(page) = window::active_page(&u) {
            page.find(&entry.text());
        }
        if entry.text().is_empty() {
            u.find_bar.count.set_label("");
        }
    });

    let keys = gtk::EventControllerKey::new();
    let u = ui.clone();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let page = window::active_page(&u);
        match key {
            Key::Escape => {
                close(&u);
                Propagation::Stop
            }
            Key::Return | Key::KP_Enter => {
                if let Some(page) = page {
                    if modifiers.contains(ModifierType::SHIFT_MASK) {
                        page.find_previous();
                    } else {
                        page.find_next();
                    }
                }
                Propagation::Stop
            }
            _ => Propagation::Proceed,
        }
    });
    bar.entry.add_controller(keys);
}

pub fn open(ui: &Rc<Ui>) {
    let bar = &ui.find_bar;
    bar.root.set_visible(true);
    bar.entry.grab_focus();
    bar.entry.select_region(0, -1);
    // Search again in case the page or tab changed since the last time.
    if !bar.entry.text().is_empty()
        && let Some(page) = window::active_page(ui)
    {
        page.find(&bar.entry.text());
    }
}

pub fn close(ui: &Rc<Ui>) {
    ui.find_bar.root.set_visible(false);
    if let Some(page) = window::active_page(ui) {
        page.find("");
    }
    window::focus_page(ui);
}

/// Called when the engine reports how many matches it found.
pub fn show_result(ui: &Rc<Ui>, matches: Option<u32>) {
    let label = match matches {
        Some(1) => "1 match".to_string(),
        Some(n) => format!("{n} matches"),
        None => "No matches".to_string(),
    };
    ui.find_bar.count.set_label(&label);
}
