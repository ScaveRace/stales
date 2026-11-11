//! The classic layout: tabs along the top, like most browsers.
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │ [G][Y] │ Arch News × │ ArchWiki × │ + │         ◐ ◆ +    │  tabs, Spaces
//! │  ← → ⟳  [ archlinux.org/news                    ]   🛡    │  toolbar
//! ├──────────────────────────────────────────────────────────┤
//! │                        page                              │
//! ```
//!
//! It shows the same tabs and Spaces as the sidebar; switch with Ctrl+Alt+L.
//! Middle-click a tab to close it.

use std::rc::Rc;

use gtk::pango::EllipsizeMode;
use gtk::prelude::*;

use crate::blocker;
use crate::command_bar::{self, Mode};
use crate::sidebar::{favicon, icon_button};
use crate::state::Selected;
use crate::window::{self, Ui};

pub struct TopBar {
    pub root: gtk::Box,
    tabs: gtk::Box,
    scroller: gtk::ScrolledWindow,
    spaces: gtk::Box,
    pub back: gtk::Button,
    pub forward: gtk::Button,
    pub reload: gtk::Button,
    pub shield: gtk::Button,
    settings: gtk::Button,
    url_button: gtk::Button,
    url_label: gtk::Label,
    new_tab: gtk::Button,
}

pub fn build() -> TopBar {
    // Row 1: tabs (scroll sideways when there are many), new tab, Spaces.
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    tabs.add_css_class("st-toptabs");
    let new_tab = icon_button("list-add-symbolic", "New tab (Ctrl+T)");
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .child(&tabs)
        .build();
    // The mouse wheel scrolls the tab row sideways.
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let adjustment = scroller.hadjustment();
    wheel.connect_scroll(move |_, _, dy| {
        adjustment.set_value(adjustment.value() + dy * 60.0);
        gtk::glib::Propagation::Stop
    });
    scroller.add_controller(wheel);
    let spaces = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    spaces.add_css_class("st-spaces");
    let tab_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    tab_row.append(&scroller);
    // "+" stays visible however many tabs there are.
    tab_row.append(&new_tab);
    // A line between "new tab" and the Spaces, so the two + buttons aren't confused.
    let divider = gtk::Separator::new(gtk::Orientation::Vertical);
    divider.add_css_class("st-divider");
    tab_row.append(&divider);
    tab_row.append(&spaces);
    // Drag the window by empty space in the tab row (on floating desktops).
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&tab_row));

    // Row 2: navigation, address, shield.
    let back = icon_button("go-previous-symbolic", "Back (Alt+←)");
    let forward = icon_button("go-next-symbolic", "Forward (Alt+→)");
    let reload = icon_button("view-refresh-symbolic", "Reload (Ctrl+R)");
    let url_label = gtk::Label::new(None);
    url_label.set_xalign(0.0);
    url_label.set_ellipsize(EllipsizeMode::End);
    url_label.set_max_width_chars(1);
    url_label.set_hexpand(true);
    let url_button = gtk::Button::new();
    url_button.add_css_class("st-urlbar");
    url_button.set_hexpand(true);
    url_button.set_child(Some(&url_label));
    url_button.set_tooltip_text(Some("Ctrl+L"));
    let shield = icon_button("security-high-symbolic", "Ad blocker (Ctrl+Shift+B)");
    shield.add_css_class("st-shield");
    let settings = icon_button("emblem-system-symbolic", "Settings (Ctrl+,)");
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    toolbar.add_css_class("st-toolbar");
    for widget in [
        back.upcast_ref::<gtk::Widget>(),
        forward.upcast_ref(),
        reload.upcast_ref(),
        url_button.upcast_ref(),
        shield.upcast_ref(),
        settings.upcast_ref(),
    ] {
        toolbar.append(widget);
    }

    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.add_css_class("st-topbar");
    root.set_hexpand(true);
    root.append(&handle);
    root.append(&toolbar);
    TopBar { root, tabs, scroller, spaces, back, forward, reload, shield, settings, url_button, url_label, new_tab }
}

pub fn connect(ui: &Rc<Ui>) {
    let bar = &ui.topbar;
    let u = ui.clone();
    bar.back.connect_clicked(move |_| {
        if let Some(page) = window::active_page(&u) {
            page.go_back();
        }
    });
    let u = ui.clone();
    bar.forward.connect_clicked(move |_| {
        if let Some(page) = window::active_page(&u) {
            page.go_forward();
        }
    });
    let u = ui.clone();
    bar.reload.connect_clicked(move |_| window::reload_or_stop(&u));
    let u = ui.clone();
    bar.shield.connect_clicked(move |_| blocker::toggle_site(&u));
    let u = ui.clone();
    bar.settings.connect_clicked(move |_| crate::settings::open(&u));
    let u = ui.clone();
    bar.url_button.connect_clicked(move |_| command_bar::open(&u, Mode::EditCurrent));
    let u = ui.clone();
    bar.new_tab.connect_clicked(move |_| command_bar::open(&u, Mode::NewTab));
}

/// One tab in the strip. Pinned tabs show only their icon.
fn tab_widget(ui: &Rc<Ui>, selected: Selected, tab: &crate::state::Tab, active: bool) -> gtk::Box {
    let pinned = matches!(selected, Selected::Pinned(_));
    let item = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    item.add_css_class("st-toptab");
    if pinned {
        item.add_css_class("pinned");
    }
    if active {
        item.add_css_class("active");
    }
    item.set_tooltip_text(Some(&tab.title));
    item.append(&favicon(ui, tab));

    if !pinned {
        let title = gtk::Label::new(Some(&tab.title));
        // Fixed-width tabs, like most browsers; many tabs scroll sideways.
        title.set_xalign(0.0);
        title.set_ellipsize(EllipsizeMode::End);
        title.set_width_chars(16);
        title.set_max_width_chars(16);
        item.append(&title);
        let close = icon_button("window-close-symbolic", "Close tab (Ctrl+W)");
        close.add_css_class("st-close");
        if let Selected::Tab(i) = selected {
            let u = ui.clone();
            close.connect_clicked(move |_| {
                u.state.borrow_mut().close_tab(i);
                window::refresh_later(&u);
            });
        }
        item.append(&close);
    }

    // Left click selects; middle click closes (not pinned tabs).
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let u = ui.clone();
    click.connect_released(move |gesture, _, _, _| {
        match (gesture.current_button(), selected) {
            (2, Selected::Tab(i)) => {
                u.state.borrow_mut().close_tab(i);
            }
            (1, _) => {
                u.state.borrow_mut().select(selected);
            }
            _ => return,
        }
        window::refresh_later(&u);
    });
    item.add_controller(click);
    item
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// Scroll the tab row so the active tab is visible. Waits a moment, because the
/// new tabs only get their positions once GTK has laid them out.
fn scroll_into_view(scroller: gtk::ScrolledWindow, tabs: gtk::Box, tab: gtk::Box) {
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(30), move || {
        let Some(bounds) = tab.compute_bounds(&tabs) else { return };
        let adjustment = scroller.hadjustment();
        let (start, end) = (bounds.x() as f64, (bounds.x() + bounds.width()) as f64);
        let (value, page) = (adjustment.value(), adjustment.page_size());
        if start < value {
            adjustment.set_value(start);
        } else if end > value + page {
            adjustment.set_value(end - page);
        }
    });
}

pub fn refresh(ui: &Rc<Ui>) {
    let state = ui.state.borrow();
    let space = state.space();
    let bar = &ui.topbar;

    match state.active_tab() {
        Some(tab) => bar.url_label.set_label(&tab.display_url()),
        None => bar.url_label.set_label("Search or enter address"),
    }
    blocker::show_shield(ui, &bar.shield, state.active_tab().map(|t| t.url.as_str()));

    clear(&bar.tabs);
    let mut active_widget = None;
    for (i, tab) in space.pinned.iter().enumerate() {
        let selected = Selected::Pinned(i);
        let widget = tab_widget(ui, selected, tab, space.active == Some(selected));
        if space.active == Some(selected) {
            active_widget = Some(widget.clone());
        }
        bar.tabs.append(&widget);
    }
    if !space.pinned.is_empty() && !space.tabs.is_empty() {
        let divider = gtk::Separator::new(gtk::Orientation::Vertical);
        divider.add_css_class("st-divider");
        bar.tabs.append(&divider);
    }
    for (i, tab) in space.tabs.iter().enumerate() {
        let selected = Selected::Tab(i);
        let widget = tab_widget(ui, selected, tab, space.active == Some(selected));
        if space.active == Some(selected) {
            active_widget = Some(widget.clone());
        }
        bar.tabs.append(&widget);
    }

    if let Some(widget) = active_widget {
        scroll_into_view(bar.scroller.clone(), bar.tabs.clone(), widget);
    }

    clear(&bar.spaces);
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
        bar.spaces.append(&button);
    }
    let add = icon_button("list-add-symbolic", "New Space (Ctrl+Alt+N)");
    add.add_css_class("st-space-btn");
    let u = ui.clone();
    add.connect_clicked(move |_| {
        u.state.borrow_mut().add_space();
        window::refresh_later(&u);
    });
    bar.spaces.append(&add);
}
