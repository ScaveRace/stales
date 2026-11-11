//! The Arc-style left panel, from top to bottom:
//!
//! - navigation buttons (back, forward, reload) and the button that hides the sidebar
//! - the address pill (click it, or press Ctrl+L, to type a new address)
//! - the name of the current Space
//! - pinned tabs as a grid of tiles
//! - "New Tab" and the list of open tabs
//! - the Space switcher at the bottom
//!
//! In compact mode (Ctrl+Shift+S, or drag it narrow) only the icons are shown.

use std::cell::Cell;
use std::rc::Rc;

use gtk::pango::EllipsizeMode;
use gtk::prelude::*;

use crate::blocker;
use crate::command_bar::{self, Mode};
use crate::state::{Selected, Tab};
use crate::window::{self, Ui};

pub struct Sidebar {
    pub root: gtk::Box,
    nav: gtk::Box,
    nav_spacer: gtk::Box,
    new_tab_label: gtk::Label,
    /// Icons-only mode.
    pub compact: Cell<bool>,
    pub toggle: gtk::Button,
    pub settings: gtk::Button,
    pub back: gtk::Button,
    pub forward: gtk::Button,
    pub reload: gtk::Button,
    /// Ad blocker on/off for the current site.
    pub shield: gtk::Button,
    pub url_button: gtk::Button,
    pub url_label: gtk::Label,
    pub space_label: gtk::Label,
    pub pinned: gtk::FlowBox,
    /// "Drop here to pin" box, shown while you drag a tab and nothing is pinned yet.
    pub pin_hint: gtk::Label,
    pub new_tab: gtk::Button,
    pub tabs: gtk::ListBox,
    pub spaces: gtk::Box,
}

pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("st-icon-btn");
    button.set_tooltip_text(Some(tooltip));
    button
}

/// The site's icon, or the first letter of the site in a small rounded square
/// while the icon hasn't loaded (or the tab was never opened).
pub fn favicon(ui: &Ui, tab: &Tab) -> gtk::Widget {
    if let Some(texture) = ui.favicons.borrow().get(&tab.id) {
        let image = gtk::Image::from_paintable(Some(texture));
        image.set_pixel_size(16);
        image.add_css_class("st-favicon-img");
        return image.upcast();
    }
    let label = gtk::Label::new(Some(&tab.initial()));
    label.add_css_class("st-favicon");
    label.upcast()
}

pub fn build() -> Sidebar {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.add_css_class("st-sidebar");
    // The width is set by the window, from the saved width.

    // Top row. Wrapped in a WindowHandle so you can drag the window by it
    // on desktops that float windows (on niri this simply does nothing).
    let nav = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let toggle = icon_button("sidebar-show-symbolic", "Hide sidebar (Ctrl+S)");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let settings = icon_button("emblem-system-symbolic", "Settings (Ctrl+,)");
    nav.append(&toggle);
    nav.append(&settings);
    nav.append(&spacer);
    let nav_spacer = spacer;
    let back = icon_button("go-previous-symbolic", "Back (Alt+←)");
    let forward = icon_button("go-next-symbolic", "Forward (Alt+→)");
    let reload = icon_button("view-refresh-symbolic", "Reload (Ctrl+R)");
    let shield = icon_button("security-high-symbolic", "Ad blocker (Ctrl+Shift+B)");
    shield.add_css_class("st-shield");
    nav.append(&shield);
    nav.append(&back);
    nav.append(&forward);
    nav.append(&reload);
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&nav));
    root.append(&handle);

    let url_label = gtk::Label::new(None);
    url_label.set_xalign(0.0);
    url_label.set_ellipsize(EllipsizeMode::End);
    // Without this a long URL makes the label (and the whole sidebar) as wide as the text.
    url_label.set_max_width_chars(1);
    url_label.set_hexpand(true);
    let url_button = gtk::Button::new();
    url_button.add_css_class("st-urlbar");
    url_button.set_child(Some(&url_label));
    url_button.set_tooltip_text(Some("Ctrl+L"));
    root.append(&url_button);

    let space_label = gtk::Label::new(None);
    space_label.add_css_class("st-space-name");
    space_label.set_xalign(0.0);
    space_label.set_ellipsize(EllipsizeMode::End);
    space_label.set_max_width_chars(1);
    root.append(&space_label);

    let pinned = gtk::FlowBox::new();
    pinned.add_css_class("st-pinned-grid");
    pinned.set_selection_mode(gtk::SelectionMode::None);
    pinned.set_homogeneous(true);
    // Up to 3 tiles per row; fewer when the sidebar is narrow, so the tiles never
    // stop the sidebar from getting smaller.
    pinned.set_min_children_per_line(1);
    pinned.set_max_children_per_line(3);
    pinned.set_column_spacing(6);
    pinned.set_row_spacing(6);
    root.append(&pinned);
    let pin_hint = gtk::Label::new(Some("Drop here to pin"));
    pin_hint.add_css_class("st-pin-hint");
    pin_hint.set_visible(false);
    root.append(&pin_hint);

    let divider = gtk::Separator::new(gtk::Orientation::Horizontal);
    divider.add_css_class("st-divider");
    root.append(&divider);

    let new_tab_content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    new_tab_content.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    let new_tab_label = gtk::Label::new(Some("New Tab"));
    new_tab_content.append(&new_tab_label);
    let new_tab = gtk::Button::new();
    new_tab.add_css_class("st-newtab");
    new_tab.set_child(Some(&new_tab_content));
    new_tab.set_tooltip_text(Some("Ctrl+T"));
    root.append(&new_tab);

    let tabs = gtk::ListBox::new();
    tabs.add_css_class("st-tabs");
    tabs.set_selection_mode(gtk::SelectionMode::None);
    tabs.set_activate_on_single_click(true);
    let scroller =
        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&tabs).build();
    root.append(&scroller);

    let spaces = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    spaces.add_css_class("st-spaces");
    spaces.set_halign(gtk::Align::Center);
    root.append(&spaces);

    Sidebar {
        root,
        nav,
        nav_spacer,
        new_tab_label,
        compact: Cell::new(false),
        toggle,
        settings,
        back,
        forward,
        reload,
        shield,
        url_button,
        url_label,
        space_label,
        pinned,
        pin_hint,
        new_tab,
        tabs,
        spaces,
    }
}

/// Switch between the full sidebar and icons only. Call `refresh` afterwards.
pub fn set_compact(sb: &Sidebar, compact: bool) {
    sb.compact.set(compact);
    let (direction, per_line) =
        if compact { (gtk::Orientation::Vertical, 1) } else { (gtk::Orientation::Horizontal, 3) };
    sb.nav.set_orientation(direction);
    sb.spaces.set_orientation(direction);
    sb.nav_spacer.set_visible(!compact);
    sb.url_button.set_visible(!compact);
    sb.space_label.set_visible(!compact);
    sb.new_tab_label.set_visible(!compact);
    sb.pinned.set_min_children_per_line(1);
    let _ = per_line; // the width per row comes from the settings (see refresh)
    if compact {
        sb.root.add_css_class("compact");
    } else {
        sb.root.remove_css_class("compact");
    }
}

/// Hook up the buttons that never get rebuilt.
pub fn connect(ui: &Rc<Ui>) {
    let sb = &ui.sidebar;

    let u = ui.clone();
    sb.toggle.connect_clicked(move |_| window::toggle_sidebar(&u));
    let u = ui.clone();
    sb.settings.connect_clicked(move |_| crate::settings::open(&u));

    let u = ui.clone();
    sb.back.connect_clicked(move |_| {
        if let Some(page) = window::active_page(&u) {
            page.go_back();
        }
    });
    let u = ui.clone();
    sb.forward.connect_clicked(move |_| {
        if let Some(page) = window::active_page(&u) {
            page.go_forward();
        }
    });
    let u = ui.clone();
    sb.reload.connect_clicked(move |_| window::reload_or_stop(&u));
    let u = ui.clone();
    sb.shield.connect_clicked(move |_| blocker::toggle_site(&u));

    let u = ui.clone();
    sb.url_button.connect_clicked(move |_| command_bar::open(&u, Mode::EditCurrent));

    let u = ui.clone();
    sb.new_tab.connect_clicked(move |_| command_bar::open(&u, Mode::NewTab));

    // Each row is named "tab:<index>" or "folder:<id>".
    let u = ui.clone();
    sb.tabs.connect_row_activated(move |_, row| {
        let name = row.widget_name();
        if let Some(index) = name.strip_prefix("tab:").and_then(|n| n.parse().ok()) {
            u.state.borrow_mut().select(Selected::Tab(index));
        } else if let Some(id) = name.strip_prefix("folder:").and_then(|n| n.parse().ok()) {
            u.state.borrow_mut().toggle_folder(id);
        }
        window::refresh_later(&u);
    });
    // Dropping below the last tab puts it at the end of the list, outside folders.
    let end = Drop::List { index: usize::MAX, folder: None };
    drop_target(ui, &sb.tabs, end);
    drop_target(ui, &sb.pin_hint, Drop::Pinned(usize::MAX));
    drop_target(ui, &sb.pinned, Drop::Pinned(usize::MAX));
}

/// Rebuild the parts of the sidebar that depend on the state.
pub fn refresh(ui: &Rc<Ui>) {
    let state = ui.state.borrow();
    let space = state.space();
    let sb = &ui.sidebar;

    sb.space_label.set_label(&format!("{}  {}", space.icon, space.name));
    match state.active_tab() {
        Some(tab) => sb.url_label.set_label(&tab.display_url()),
        None => sb.url_label.set_label("Search or enter address"),
    }

    blocker::show_shield(ui, &sb.shield, state.active_tab().map(|t| t.url.as_str()));

    // Pinned tiles (drag a tab here to pin it; drag a tile onto another to reorder)
    let settings = ui.config.borrow().sidebar.clone();
    let compact = sb.compact.get();
    sb.pinned.remove_all();
    sb.pinned.set_visible(!space.pinned.is_empty());
    sb.pinned.set_max_children_per_line(if compact { 1 } else { settings.pinned_per_row.clamp(1, 8) });
    for (i, tab) in space.pinned.iter().enumerate() {
        let tile = gtk::Button::new();
        tile.add_css_class("st-pinned");
        tile.add_css_class(match settings.pinned_size.as_str() {
            "small" => "small",
            "large" => "large",
            _ => "medium",
        });
        if space.active == Some(Selected::Pinned(i)) {
            tile.add_css_class("active");
        }
        if !ui.pages.borrow().contains_key(&tab.id) {
            tile.add_css_class("asleep");
        }
        tile.set_tooltip_text(Some(&tab.title));
        tile.set_child(Some(&favicon(ui, tab)));
        let u = ui.clone();
        tile.connect_clicked(move |_| {
            u.state.borrow_mut().select(Selected::Pinned(i));
            window::refresh_later(&u);
        });
        drag_source(ui, &tile, Selected::Pinned(i));
        drop_target(ui, &tile, Drop::Pinned(i));
        tab_menu(ui, &tile, Selected::Pinned(i));
        sb.pinned.insert(&tile, -1);
    }

    // Tab list: folders first (each with its tabs), then the other tabs.
    sb.tabs.remove_all();
    let folders_on = settings.folders;
    let order = space.list_order();
    if folders_on {
        for folder in &space.folders {
            let inside: Vec<usize> =
                order.iter().copied().filter(|&i| space.tabs[i].folder == Some(folder.id)).collect();
            sb.tabs.append(&folder_row(ui, folder, inside.len(), compact));
            if folder.open {
                for i in inside {
                    sb.tabs.append(&tab_row(ui, space, i, compact, true));
                }
            }
        }
    }
    for i in order {
        let in_known_folder =
            folders_on && space.tabs[i].folder.is_some_and(|id| space.folders.iter().any(|f| f.id == id));
        if !in_known_folder {
            sb.tabs.append(&tab_row(ui, space, i, compact, false));
        }
    }

    // Space switcher
    while let Some(child) = sb.spaces.first_child() {
        sb.spaces.remove(&child);
    }
    for (i, s) in state.spaces.iter().enumerate() {
        let button = gtk::Button::with_label(&s.icon);
        button.add_css_class("st-space-btn");
        if i == state.current {
            button.add_css_class("active");
        }
        button.set_tooltip_text(Some(&format!("{} (Alt+{})", s.name, i + 1)));
        let u = ui.clone();
        button.connect_clicked(move |_| {
            u.state.borrow_mut().switch_space(i);
            window::refresh_later(&u);
        });
        sb.spaces.append(&button);
    }
    let add = icon_button("list-add-symbolic", "New Space");
    add.add_css_class("st-space-btn");
    let u = ui.clone();
    add.connect_clicked(move |_| {
        u.state.borrow_mut().add_space();
        window::refresh_later(&u);
    });
    sb.spaces.append(&add);
}

// ---------- Rows of the tab list ----------

fn tab_row(ui: &Rc<Ui>, space: &crate::state::Space, i: usize, compact: bool, in_folder: bool) -> gtk::ListBoxRow {
    let tab = &space.tabs[i];
    let row = gtk::ListBoxRow::new();
    row.set_widget_name(&format!("tab:{i}"));
    row.add_css_class("st-tab");
    if in_folder {
        row.add_css_class("in-folder");
    }
    if space.active == Some(Selected::Tab(i)) {
        row.add_css_class("active");
    }
    // Not loaded right now (restored, or a folder link you closed): shown dimmed.
    if !ui.pages.borrow().contains_key(&tab.id) {
        row.add_css_class("asleep");
    }
    row.set_tooltip_text(Some(if compact { &tab.title } else { &tab.url }));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    content.append(&favicon(ui, tab));
    row.set_child(Some(&content));
    drag_source(ui, &row, Selected::Tab(i));
    drop_target(ui, &row, Drop::List { index: i, folder: tab.folder });
    tab_menu(ui, &row, Selected::Tab(i));
    if compact {
        return row;
    }
    let title = gtk::Label::new(Some(&tab.title));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(EllipsizeMode::End);
    title.set_max_width_chars(1);
    let tooltip = if in_folder { "Unload (stays in the folder)" } else { "Close tab (Ctrl+W)" };
    let close = icon_button("window-close-symbolic", tooltip);
    close.add_css_class("st-close");
    let u = ui.clone();
    close.connect_clicked(move |_| window::close_tab(&u, Selected::Tab(i)));
    content.append(&title);
    content.append(&close);
    row
}

fn folder_row(ui: &Rc<Ui>, folder: &crate::state::Folder, count: usize, compact: bool) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_widget_name(&format!("folder:{}", folder.id));
    row.add_css_class("st-folder");
    row.set_tooltip_text(Some(&format!("{} ({count}) · click to open or close · double-click to rename", folder.name)));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = if folder.open { "folder-open-symbolic" } else { "folder-symbolic" };
    content.append(&gtk::Image::from_icon_name(icon));
    if !compact {
        let name = gtk::Label::new(Some(&folder.name));
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_ellipsize(EllipsizeMode::End);
        name.set_max_width_chars(1);
        let count = gtk::Label::new(Some(&count.to_string()));
        count.add_css_class("st-muted");
        content.append(&name);
        content.append(&count);
        // Double-click: rename in place.
        let click = gtk::GestureClick::new();
        let (u, id, current) = (ui.clone(), folder.id, folder.name.clone());
        let (content_box, name_label) = (content.clone(), name.clone());
        click.connect_pressed(move |_, presses, _, _| {
            if presses == 2 {
                rename_in_place(&u, id, &current, &content_box, &name_label);
            }
        });
        row.add_controller(click);
    }
    row.set_child(Some(&content));
    drop_target(ui, &row, Drop::Folder(folder.id));
    folder_menu(ui, &row, folder.id);
    row
}

fn rename_in_place(ui: &Rc<Ui>, id: u64, current: &str, content: &gtk::Box, label: &gtk::Label) {
    let entry = gtk::Entry::new();
    entry.set_text(current);
    entry.set_hexpand(true);
    content.insert_child_after(&entry, Some(label));
    label.set_visible(false);
    entry.grab_focus();
    let u = ui.clone();
    entry.connect_activate(move |entry| {
        u.state.borrow_mut().rename_folder(id, &entry.text());
        window::refresh_later(&u);
    });
    let keys = gtk::EventControllerKey::new();
    let u = ui.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            window::refresh_later(&u);
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    entry.add_controller(keys);
}

// ---------- Right-click menus ----------

/// A menu entry: its label and what clicking it does.
type MenuItem = (String, Box<dyn Fn()>);

/// A small menu of buttons, opened by right-clicking `widget`.
fn context_menu(widget: &impl IsA<gtk::Widget>, items: Vec<MenuItem>) {
    let menu_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let popover = gtk::Popover::new();
    popover.add_css_class("st-menu");
    popover.set_child(Some(&menu_box));
    popover.set_parent(widget);
    popover.set_has_arrow(false);
    let items: Rc<Vec<MenuItem>> = Rc::new(items);
    for (n, (label, _)) in items.iter().enumerate() {
        let button = gtk::Button::with_label(label);
        button.add_css_class("st-menu-item");
        let (items, pop) = (items.clone(), popover.clone());
        button.connect_clicked(move |_| {
            pop.popdown();
            (items[n].1)();
        });
        menu_box.append(&button);
    }
    let click = gtk::GestureClick::new();
    click.set_button(3);
    let pop = popover.clone();
    click.connect_pressed(move |_, _, x, y| {
        pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        pop.popup();
    });
    widget.add_controller(click);
    // The popover belongs to the widget; take it away when the widget goes.
    widget.connect_destroy(move |_| popover.unparent());
}

fn tab_menu(ui: &Rc<Ui>, widget: &impl IsA<gtk::Widget>, selected: Selected) {
    let mut items: Vec<MenuItem> = Vec::new();
    let pinned = matches!(selected, Selected::Pinned(_));
    let u = ui.clone();
    items.push((if pinned { "Unpin" } else { "Pin" }.into(), Box::new(move || pin_or_unpin(&u, selected))));
    if ui.config.borrow().sidebar.folders {
        let u = ui.clone();
        items.push((
            "New folder with this tab".into(),
            Box::new(move || {
                u.state.borrow_mut().add_folder(Some(selected));
                window::toast(&u, "New folder · double-click its name to rename it");
                window::refresh_later(&u);
            }),
        ));
        let folders: Vec<(u64, String)> =
            ui.state.borrow().space().folders.iter().map(|f| (f.id, f.name.clone())).collect();
        for (id, name) in folders {
            let u = ui.clone();
            items.push((
                format!("Move to “{name}”"),
                Box::new(move || {
                    u.state.borrow_mut().move_tab_into(selected, false, usize::MAX, Some(id));
                    window::refresh_later(&u);
                }),
            ));
        }
        if ui.state.borrow().in_folder(selected) {
            let u = ui.clone();
            items.push((
                "Take out of folder".into(),
                Box::new(move || {
                    u.state.borrow_mut().move_tab_into(selected, false, usize::MAX, None);
                    window::refresh_later(&u);
                }),
            ));
        }
    }
    let u = ui.clone();
    items.push(("Close".into(), Box::new(move || window::close_tab(&u, selected))));
    context_menu(widget, items);
}

fn folder_menu(ui: &Rc<Ui>, widget: &gtk::ListBoxRow, id: u64) {
    let u = ui.clone();
    let row = widget.clone();
    let rename: Box<dyn Fn()> = Box::new(move || {
        // Same as double-clicking the name.
        let content = row.child().and_downcast::<gtk::Box>();
        let label =
            content.as_ref().and_then(|c| c.first_child()).and_then(|w| w.next_sibling()).and_downcast::<gtk::Label>();
        if let (Some(content), Some(label)) = (content, label) {
            rename_in_place(&u, id, &label.text(), &content, &label);
        }
    });
    let u = ui.clone();
    let delete: Box<dyn Fn()> = Box::new(move || {
        u.state.borrow_mut().delete_folder(id);
        window::toast(&u, "Folder removed; its tabs are still here");
        window::refresh_later(&u);
    });
    context_menu(widget, vec![("Rename".into(), rename), ("Delete folder (keep tabs)".into(), delete)]);
}

// ---------- Drag and drop ----------

/// Where something was dropped.
#[derive(Clone, Copy)]
pub enum Drop {
    /// Onto the pinned tiles, before tile `index` (usize::MAX: at the end).
    Pinned(usize),
    /// Onto the tab list, before tab `index`, into `folder` (or none).
    List { index: usize, folder: Option<u64> },
    /// Onto a folder's name: into that folder.
    Folder(u64),
}

fn drag_value(selected: Selected) -> String {
    match selected {
        Selected::Pinned(i) => format!("stales-pinned:{i}"),
        Selected::Tab(i) => format!("stales-tab:{i}"),
    }
}

fn parse_drag(value: &str) -> Option<Selected> {
    if let Some(i) = value.strip_prefix("stales-pinned:") {
        return i.parse().ok().map(Selected::Pinned);
    }
    value.strip_prefix("stales-tab:").and_then(|i| i.parse().ok()).map(Selected::Tab)
}

pub fn drag_source(ui: &Rc<Ui>, widget: &impl IsA<gtk::Widget>, selected: Selected) {
    let source = gtk::DragSource::new();
    source.set_actions(gtk::gdk::DragAction::MOVE);
    source.connect_prepare(move |_, _, _| Some(gtk::gdk::ContentProvider::for_value(&drag_value(selected).to_value())));
    // While dragging with nothing pinned, show where to drop to pin.
    let u = ui.clone();
    source.connect_drag_begin(move |_, _| {
        let empty = u.state.borrow().space().pinned.is_empty();
        u.sidebar.pin_hint.set_visible(empty && !u.sidebar.compact.get());
    });
    let u = ui.clone();
    source.connect_drag_end(move |_, _, _| u.sidebar.pin_hint.set_visible(false));
    widget.add_controller(source);
}

pub fn drop_target(ui: &Rc<Ui>, widget: &impl IsA<gtk::Widget>, spot: Drop) {
    let target = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
    let w = widget.clone().upcast::<gtk::Widget>();
    target.connect_enter(move |_, _, _| {
        w.add_css_class("drop-here");
        gtk::gdk::DragAction::MOVE
    });
    let w = widget.clone().upcast::<gtk::Widget>();
    target.connect_leave(move |_| w.remove_css_class("drop-here"));
    let u = ui.clone();
    target.connect_drop(move |_, value, _, _| {
        let Some(from) = value.get::<String>().ok().and_then(|v| parse_drag(&v)) else { return false };
        dropped(&u, from, spot)
    });
    widget.add_controller(target);
}

fn dropped(ui: &Rc<Ui>, from: Selected, spot: Drop) -> bool {
    let max = ui.config.borrow().sidebar.pinned_max;
    let moved = {
        let mut state = ui.state.borrow_mut();
        match spot {
            Drop::Pinned(index) => {
                let full = state.space().pinned.len() >= max && !matches!(from, Selected::Pinned(_));
                if full {
                    drop(state);
                    window::toast(ui, &format!("Pinned tiles are full ({max}); change the limit in Settings"));
                    return false;
                }
                state.move_tab(from, true, index)
            }
            Drop::List { index, folder } => state.move_tab_into(from, false, index, folder),
            Drop::Folder(id) => state.move_tab_into(from, false, usize::MAX, Some(id)),
        }
    };
    if moved {
        window::refresh_later(ui);
    }
    moved
}

/// Pin a tab (if there is room) or unpin a tile. Also Ctrl+D.
pub fn pin_or_unpin(ui: &Rc<Ui>, selected: Selected) {
    match selected {
        Selected::Pinned(_) => {
            ui.state.borrow_mut().move_tab(selected, false, 0);
        }
        Selected::Tab(_) => {
            dropped(ui, selected, Drop::Pinned(usize::MAX));
            return;
        }
    }
    window::refresh_later(ui);
}
