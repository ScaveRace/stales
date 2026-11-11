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
    sb.pinned.set_max_children_per_line(per_line);
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

    let u = ui.clone();
    sb.tabs.connect_row_activated(move |_, row| {
        if let Ok(index) = usize::try_from(row.index()) {
            u.state.borrow_mut().select(Selected::Tab(index));
            window::refresh_later(&u);
        }
    });
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

    // Pinned tiles
    sb.pinned.remove_all();
    sb.pinned.set_visible(!space.pinned.is_empty());
    for (i, tab) in space.pinned.iter().enumerate() {
        let tile = gtk::Button::new();
        tile.add_css_class("st-pinned");
        if space.active == Some(Selected::Pinned(i)) {
            tile.add_css_class("active");
        }
        tile.set_tooltip_text(Some(&tab.title));
        tile.set_child(Some(&favicon(ui, tab)));
        let u = ui.clone();
        tile.connect_clicked(move |_| {
            u.state.borrow_mut().select(Selected::Pinned(i));
            window::refresh_later(&u);
        });
        sb.pinned.insert(&tile, -1);
    }

    // Tab list
    sb.tabs.remove_all();
    for (i, tab) in space.tabs.iter().enumerate() {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("st-tab");
        if space.active == Some(Selected::Tab(i)) {
            row.add_css_class("active");
        }
        let compact = sb.compact.get();
        row.set_tooltip_text(Some(if compact { &tab.title } else { &tab.url }));
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.append(&favicon(ui, tab));
        row.set_child(Some(&content));
        sb.tabs.append(&row);
        if compact {
            continue;
        }

        let title = gtk::Label::new(Some(&tab.title));
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.set_ellipsize(EllipsizeMode::End);
        title.set_max_width_chars(1);

        let close = icon_button("window-close-symbolic", "Close tab (Ctrl+W)");
        close.add_css_class("st-close");
        let u = ui.clone();
        close.connect_clicked(move |_| {
            u.state.borrow_mut().close_tab(i);
            window::refresh_later(&u);
        });

        content.append(&title);
        content.append(&close);
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
