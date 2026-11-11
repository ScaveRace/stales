//! The main window: sidebar on the left, page area on the right, command bar on top.
//!
//! ```text
//! ┌───────────┬──────────────────────────────┐
//! │  sidebar  │  ┌────────────────────────┐  │
//! │  (tabs,   │  │   web page of the      │  │
//! │  spaces)  │  │   active tab           │  │
//! │           │  └────────────────────────┘  │
//! └───────────┴──────────────────────────────┘
//!        command bar floats above everything (Ctrl+T / Ctrl+L)
//! ```
//!
//! Each tab gets its own web page, created the first time you open that tab
//! (so 20 restored tabs don't all load at startup). All pages sit in a `gtk::Stack`,
//! which shows one at a time: the active tab's.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::actions;
use crate::command_bar::{self, CommandBar};
use crate::config::{self, Config};
use crate::engine::webkit::WebKitEngine;
use crate::engine::{Engine, Page, PageEvent};
use crate::find_bar::{self, FindBar};
use crate::help::{self, Help};
use crate::saved::{self, SIDEBAR_COMPACT, SIDEBAR_DEFAULT, Saved};
use crate::sidebar::{self, Sidebar};
use crate::state::{State, TabId};
use crate::topbar::{self, TopBar};
use crate::{blocker, session, theme};

/// A page waiting to load until the blocker is ready: the page, its address, its saved history.
type HeldPage = (Rc<dyn Page>, String, Option<String>);

/// Where the tabs are: in the Arc-style sidebar, or along the top (classic).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layout {
    Arc,
    Classic,
}

impl Layout {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "arc" | "sidebar" => Some(Layout::Arc),
            "classic" | "top" | "tabs" => Some(Layout::Classic),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Layout::Arc => "arc",
            Layout::Classic => "classic",
        }
    }
}

/// Everything the UI needs in one place. Shared between all the click handlers
/// through `Rc<Ui>` (a reference-counted pointer, so many closures can hold it).
pub struct Ui {
    /// `RefCell` lets us change the state from inside click handlers.
    pub state: RefCell<State>,
    /// Your settings. A `RefCell` because the settings page changes them while Stales runs.
    pub config: RefCell<Config>,
    pub saved: RefCell<Saved>,
    pub engine: Box<dyn Engine>,
    /// The web page of each tab that has been opened at least once.
    pub pages: RefCell<HashMap<TabId, Rc<dyn Page>>>,
    pub favicons: RefCell<HashMap<TabId, gdk::Texture>>,
    pub window: gtk::ApplicationWindow,
    pub sidebar: Sidebar,
    pub sidebar_revealer: gtk::Revealer,
    pub topbar: TopBar,
    pub topbar_revealer: gtk::Revealer,
    pub layout: Cell<Layout>,
    pub content_frame: gtk::Box,
    pub stack: gtk::Stack,
    pub progress: gtk::ProgressBar,
    pub command: CommandBar,
    pub find_bar: FindBar,
    pub help: Help,
    /// Short messages like "Address copied", shown at the bottom of the page.
    pub toast: gtk::Label,
    pub password_bar: crate::passwords::PasswordBar,
    /// A login waiting for "Save" in the password bar.
    pub pending_login: RefCell<Option<crate::passwords::PendingLogin>>,
    refresh_queued: Cell<bool>,
    /// A session save is already scheduled (see session.rs).
    pub save_queued: Cell<bool>,
    pub blocker_status: RefCell<blocker::Status>,
    /// Pages wait to load while this is set (the blocker's rules are being loaded).
    pub hold_pages: Cell<bool>,
    held_pages: RefCell<Vec<HeldPage>>,
    pub themes: theme::Themes,
    /// The keyboard shortcuts, replaced when you change keys in the settings.
    pub shortcuts: RefCell<Option<gtk::ShortcutController>>,
}

thread_local! {
    /// The open window, so a second `stales <url>` can reach it.
    static UI: RefCell<std::rc::Weak<Ui>> = const { RefCell::new(std::rc::Weak::new()) };
}

/// Bring the window to the front, creating it the first time.
pub fn show(app: &gtk::Application) -> Rc<Ui> {
    if let Some(ui) = UI.with(|cell| cell.borrow().upgrade()) {
        ui.window.present();
        return ui;
    }
    let ui = build(app);
    UI.with(|cell| cell.replace(Rc::downgrade(&ui)));
    ui
}

/// Open an address that came from outside (command line, another app) in a new tab.
pub fn open_external(ui: &Rc<Ui>, uri: &str) {
    ui.state.borrow_mut().open_tab(uri);
    refresh(ui);
    focus_page(ui);
}

fn build(app: &gtk::Application) -> Rc<Ui> {
    let config = config::load();
    let themes = theme::init();
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Stales")
        .default_width(1280)
        .default_height(800)
        .build();
    window.add_css_class("stales");
    // An empty title bar: no GNOME-style header, the sidebar takes its place.
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));

    let saved = saved::load();
    let sidebar = sidebar::build();
    if saved.sidebar_compact {
        sidebar::set_compact(&sidebar, true);
        sidebar.root.set_width_request(SIDEBAR_COMPACT);
    } else {
        sidebar.root.set_width_request(saved.sidebar_width);
    }

    // A thin strip on the sidebar's right edge. Drag it to change the width,
    // double-click it to go back to the default.
    let resize_handle = gtk::Box::new(gtk::Orientation::Vertical, 0);
    resize_handle.add_css_class("st-resize-handle");
    resize_handle.set_cursor_from_name(Some("col-resize"));
    resize_handle.set_tooltip_text(Some(
        "Drag to resize, drag far left for icons only, double-click to reset (Ctrl+[ / Ctrl+] / Ctrl+Shift+S)",
    ));

    let sidebar_with_handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    sidebar_with_handle.append(&sidebar.root);
    sidebar_with_handle.append(&resize_handle);
    let sidebar_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideRight)
        .transition_duration(180)
        .reveal_child(true)
        .child(&sidebar_with_handle)
        .build();
    // Labels inside the sidebar ask to expand; without this that request travels up
    // and the sidebar grabs half of any spare width (very visible in fullscreen).
    // The sidebar is exactly as wide as we say; only the page expands.
    sidebar.root.set_hexpand(false);
    sidebar_with_handle.set_hexpand(false);
    sidebar_revealer.set_hexpand(false);

    // Shown when no tab is open.
    let empty = gtk::Box::new(gtk::Orientation::Vertical, 6);
    empty.set_valign(gtk::Align::Center);
    let empty_title = gtk::Label::new(Some("No tab open"));
    empty_title.add_css_class("st-content-title");
    let empty_hint = gtk::Label::new(Some("Press Ctrl+T to open one"));
    empty_hint.add_css_class("st-content-url");
    empty.append(&empty_title);
    empty.append(&empty_hint);

    let stack = gtk::Stack::new();
    stack.add_named(&empty, Some("empty"));

    // Thin loading bar along the top edge of the page.
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("st-progress");
    progress.set_valign(gtk::Align::Start);
    progress.set_visible(false);

    let find_bar = find_bar::build();
    let toast = gtk::Label::new(None);
    toast.add_css_class("st-toast");
    toast.set_halign(gtk::Align::Center);
    toast.set_valign(gtk::Align::End);
    toast.set_margin_bottom(20);
    toast.set_visible(false);

    let page_overlay = gtk::Overlay::new();
    page_overlay.set_child(Some(&stack));
    page_overlay.add_overlay(&progress);
    page_overlay.add_overlay(&find_bar.root);
    page_overlay.add_overlay(&toast);
    let password_bar = crate::passwords::build_bar();
    page_overlay.add_overlay(&password_bar.root);

    // The page area: a rounded card, like Arc. `Overflow::Hidden` clips the web
    // page to the rounded corners.
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("st-content");
    card.set_overflow(gtk::Overflow::Hidden);
    card.set_hexpand(true);
    card.set_vexpand(true);
    card.append(&page_overlay);
    page_overlay.set_vexpand(true);

    let content_frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content_frame.add_css_class("st-content-frame");
    content_frame.append(&card);

    let main = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    main.append(&sidebar_revealer);
    main.append(&content_frame);

    // The classic layout's tab bar, above everything. Hidden in the Arc layout.
    let topbar = topbar::build();
    let topbar_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(160)
        .reveal_child(true)
        .child(&topbar.root)
        .build();
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.append(&topbar_revealer);
    outer.append(&main);
    main.set_vexpand(true);

    let command = command_bar::build();
    let help = help::build();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&outer));
    overlay.add_overlay(&command.scrim);
    overlay.add_overlay(&help.scrim);
    window.set_child(Some(&overlay));

    let engine = Box::new(WebKitEngine::new(&actions::page_keys(&config.keys)));
    let ui = Rc::new(Ui {
        state: RefCell::new(session::load(config.search())),
        config: RefCell::new(config),
        saved: RefCell::new(saved),
        engine,
        pages: RefCell::new(HashMap::new()),
        favicons: RefCell::new(HashMap::new()),
        window,
        sidebar,
        sidebar_revealer,
        topbar,
        topbar_revealer,
        layout: Cell::new(Layout::Arc),
        content_frame,
        stack,
        progress,
        command,
        find_bar,
        help,
        toast,
        password_bar,
        pending_login: RefCell::new(None),
        refresh_queued: Cell::new(false),
        save_queued: Cell::new(false),
        blocker_status: RefCell::new(blocker::Status::Off),
        hold_pages: Cell::new(false),
        held_pages: RefCell::new(Vec::new()),
        themes,
        shortcuts: RefCell::new(None),
    });

    // stales:// pages (the welcome page), filled in from the current settings.
    let weak = Rc::downgrade(&ui);
    ui.engine.set_internal_pages(Box::new(move |name| {
        let ui = weak.upgrade()?;
        crate::settings::render_page(&ui, name)
    }));

    theme::start(&ui);
    // Before any page loads: pages wait until the blocker's rules are in place.
    blocker::start(&ui);
    topbar::connect(&ui);
    crate::settings::move_old_choices_to_config(&ui);
    let layout_name = ui.config.borrow().appearance.layout.clone();
    let layout = Layout::from_name(&layout_name).unwrap_or_else(|| {
        eprintln!("stales: unknown layout '{layout_name}', using arc");
        Layout::Arc
    });
    apply_layout(&ui, layout);
    connect_resize_handle(&ui, &resize_handle);
    sidebar::connect(&ui);
    command_bar::connect(&ui);
    find_bar::connect(&ui);
    crate::passwords::connect_bar(&ui);
    help::connect(&ui);
    actions::install(&ui);
    crate::control::start(&ui);
    refresh(&ui);
    load_cached_favicons(&ui);

    // Save the tabs one last time when the window closes.
    let u = ui.clone();
    ui.window.connect_close_request(move |_| {
        capture_histories(&u);
        session::store(&u.state.borrow());
        glib::Propagation::Proceed
    });
    ui.window.present();
    ui
}

/// Redraw everything from the current state, and make sure the active tab's page is showing.
pub fn refresh(ui: &Rc<Ui>) {
    ui.refresh_queued.set(false);
    // Everything that changes tabs ends up here, so this is where saving starts.
    session::save_soon(ui);
    theme::apply_space_color(ui);
    match ui.layout.get() {
        Layout::Arc => sidebar::refresh(ui),
        Layout::Classic => topbar::refresh(ui),
    }

    // Copy what we need out of the state first. Creating a page can trigger events
    // that change the state, and we must not be holding a borrow when that happens.
    let (active, existing, title) = {
        let state = ui.state.borrow();
        let active = state.active_tab().map(|t| (t.id, t.url.clone(), t.history.clone()));
        let title = state.active_tab().map(|t| t.title.clone());
        (active, state.all_ids(), title)
    };

    drop_closed_pages(ui, &existing);

    match active {
        Some((id, url, history)) => {
            page_for(ui, id, &url, history.as_deref());
            ui.stack.set_visible_child_name(&id.0.to_string());
        }
        None => ui.stack.set_visible_child_name("empty"),
    }

    let window_title = match title {
        Some(t) => format!("{t} — Stales"),
        None => "Stales".into(),
    };
    ui.window.set_title(Some(&window_title));
    update_load_state(ui);
}

/// Copy the current back/forward history of every open page into the state,
/// so the next save includes it.
pub fn capture_histories(ui: &Rc<Ui>) {
    let histories: Vec<(TabId, Option<String>)> =
        ui.pages.borrow().iter().map(|(id, page)| (*id, page.history().map(|d| BASE64.encode(d)))).collect();
    let mut state = ui.state.borrow_mut();
    for (id, history) in histories {
        if let Some(tab) = state.any_tab_mut(id) {
            tab.history = history;
        }
    }
}

/// Show the real icons of saved tabs right away, from the engine's icon cache,
/// without loading the pages.
fn load_cached_favicons(ui: &Rc<Ui>) {
    let tabs: Vec<(TabId, String)> = ui
        .state
        .borrow()
        .spaces
        .iter()
        .flat_map(|s| s.pinned.iter().chain(&s.tabs))
        .map(|t| (t.id, t.url.clone()))
        .collect();
    for (id, url) in tabs {
        let weak = Rc::downgrade(ui);
        ui.engine.cached_favicon(
            &url,
            Box::new(move |texture| {
                if let Some(ui) = weak.upgrade() {
                    // A page that loaded meanwhile already set a fresher icon.
                    ui.favicons.borrow_mut().entry(id).or_insert(texture);
                    refresh_later(&ui);
                }
            }),
        );
    }
}

/// Redraw soon, after the current event. Many events can arrive at once while a page
/// loads; this collapses them into a single redraw. It also means a button inside a
/// list never deletes itself while it is being clicked.
pub fn refresh_later(ui: &Rc<Ui>) {
    if ui.refresh_queued.replace(true) {
        return;
    }
    let ui = ui.clone();
    glib::idle_add_local_once(move || refresh(&ui));
}

/// The page of a tab, created and loaded the first time it is needed.
fn page_for(ui: &Rc<Ui>, id: TabId, url: &str, history: Option<&str>) -> Rc<dyn Page> {
    if let Some(page) = ui.pages.borrow().get(&id) {
        return page.clone();
    }

    let weak = Rc::downgrade(ui);
    let page = ui.engine.new_page(Box::new(move |event| {
        // Handle it after the current WebKit callback returns, never in the middle of it.
        let weak = weak.clone();
        glib::idle_add_local_once(move || {
            if let Some(ui) = weak.upgrade() {
                handle_page_event(&ui, id, event);
            }
        });
    }));
    ui.stack.add_named(&page.widget(), Some(&id.0.to_string()));
    ui.pages.borrow_mut().insert(id, page.clone());
    page.set_blocking(!blocker::is_allowed(ui, url));
    if ui.hold_pages.get() {
        // The blocker is still loading its rules (a fraction of a second at startup).
        // Loading now would let the first page's ads through, so wait for it.
        ui.held_pages.borrow_mut().push((page.clone(), url.to_string(), history.map(str::to_string)));
    } else {
        start_loading(&page, url, history);
    }
    page
}

fn start_loading(page: &Rc<dyn Page>, url: &str, history: Option<&str>) {
    // A saved tab gets its back/forward history back; otherwise just load the address.
    let restored = history.and_then(|text| BASE64.decode(text).ok()).is_some_and(|data| page.restore_history(&data));
    if !restored {
        page.load(url);
    }
}

/// The blocker is ready (or gave up): load the pages that were waiting for it.
pub fn release_held_pages(ui: &Rc<Ui>) {
    ui.hold_pages.set(false);
    let held = std::mem::take(&mut *ui.held_pages.borrow_mut());
    for (page, url, history) in held {
        start_loading(&page, &url, history.as_deref());
    }
}

fn drop_closed_pages(ui: &Rc<Ui>, existing: &[TabId]) {
    let closed: Vec<TabId> = ui.pages.borrow().keys().filter(|id| !existing.contains(id)).copied().collect();
    for id in closed {
        let page = ui.pages.borrow_mut().remove(&id);
        if let Some(page) = page {
            // Keep the history with the closed tab, so Ctrl+Shift+T brings it back too.
            let history = page.history().map(|data| BASE64.encode(data));
            if let Some(tab) = ui.state.borrow_mut().any_tab_mut(id) {
                tab.history = history;
            }
            ui.stack.remove(&page.widget());
            page.close();
        }
        ui.favicons.borrow_mut().remove(&id);
    }
}

fn handle_page_event(ui: &Rc<Ui>, id: TabId, event: PageEvent) {
    match event {
        PageEvent::Title(title) => {
            if !title.is_empty() {
                if let Some(tab) = ui.state.borrow_mut().tab_mut(id) {
                    tab.title = title;
                }
                refresh_later(ui);
            }
        }
        PageEvent::Url(url) => {
            if !url.is_empty() {
                // The tab went to another site: use that site's blocker setting.
                let page = ui.pages.borrow().get(&id).cloned();
                if let Some(page) = page {
                    page.set_blocking(!blocker::is_allowed(ui, &url));
                }
                if let Some(tab) = ui.state.borrow_mut().tab_mut(id) {
                    tab.url = url;
                }
                refresh_later(ui);
            }
        }
        PageEvent::Favicon(texture) => {
            match texture {
                Some(texture) => ui.favicons.borrow_mut().insert(id, texture),
                None => ui.favicons.borrow_mut().remove(&id),
            };
            refresh_later(ui);
        }
        PageEvent::LoadState => {
            if ui.state.borrow().active_id() == Some(id) {
                update_load_state(ui);
            }
        }
        PageEvent::OpenInNewTab(url) => {
            ui.state.borrow_mut().open_tab(&url);
            refresh(ui);
        }
        PageEvent::CloseRequested => {
            ui.state.borrow_mut().close_id(id);
            refresh(ui);
        }
        PageEvent::Script(message) => {
            // Only the tab you're looking at may run actions (you pressed a key in it).
            if ui.state.borrow().active_id() != Some(id) {
                return;
            }
            if message.starts_with('{') {
                // JSON: a message from the password manager script.
                crate::passwords::handle(ui, id, &message);
            } else if let Some(url) = message.strip_prefix("open-new-tab ") {
                ui.state.borrow_mut().open_tab_background(url);
                toast(ui, "Opened in a new tab");
                refresh(ui);
            } else if !actions::run(ui, &message) {
                eprintln!("stales: page asked for unknown action '{message}'");
            }
        }
        PageEvent::Settings(message) => crate::settings::handle(ui, id, &message),
        PageEvent::FindResult(matches) => {
            if ui.state.borrow().active_id() == Some(id) {
                find_bar::show_result(ui, matches);
            }
        }
    }
}

/// Back/forward buttons, reload/stop button and the loading bar, for the active tab.
pub fn update_load_state(ui: &Rc<Ui>) {
    let page = active_page(ui);
    // The same buttons exist in the sidebar and in the classic top bar.
    let sets = [
        (&ui.sidebar.back, &ui.sidebar.forward, &ui.sidebar.reload),
        (&ui.topbar.back, &ui.topbar.forward, &ui.topbar.reload),
    ];
    for (back, forward, reload) in sets {
        match &page {
            Some(page) => {
                reload.set_sensitive(true);
                back.set_sensitive(page.can_go_back());
                forward.set_sensitive(page.can_go_forward());
                reload.set_icon_name(if page.is_loading() { "process-stop-symbolic" } else { "view-refresh-symbolic" });
            }
            None => {
                reload.set_sensitive(false);
                back.set_sensitive(false);
                forward.set_sensitive(false);
            }
        }
    }
    match page {
        Some(page) => {
            ui.progress.set_visible(page.is_loading());
            ui.progress.set_fraction(page.progress().max(0.05));
        }
        None => ui.progress.set_visible(false),
    }
}

pub fn active_page(ui: &Rc<Ui>) -> Option<Rc<dyn Page>> {
    let id = ui.state.borrow().active_id()?;
    ui.pages.borrow().get(&id).cloned()
}

/// Load the active tab's (new) address in its page. Used after Ctrl+L.
pub fn load_active(ui: &Rc<Ui>) {
    let target = ui.state.borrow().active_tab().map(|t| (t.id, t.url.clone()));
    let Some((id, url)) = target else { return };
    // If the tab has no page yet, `refresh` creates one and loads the address.
    let existing = ui.pages.borrow().get(&id).cloned();
    if let Some(page) = existing {
        page.load(&url);
    }
    refresh(ui);
    focus_page(ui);
}

pub fn focus_page(ui: &Rc<Ui>) {
    if let Some(page) = active_page(ui) {
        page.focus();
    }
}

pub fn reload_or_stop(ui: &Rc<Ui>) {
    if let Some(page) = active_page(ui) {
        if page.is_loading() { page.stop() } else { page.reload() }
    }
}

fn connect_resize_handle(ui: &Rc<Ui>, handle: &gtk::Box) {
    let drag = gtk::GestureDrag::new();
    // Where on the handle you grabbed it, so the edge doesn't jump to the mouse.
    let grab_x = Rc::new(Cell::new(0.0));
    let grab = grab_x.clone();
    drag.connect_drag_begin(move |_, x, _| grab.set(x));

    // The handle moves while the sidebar resizes, so GTK's "distance dragged"
    // (measured on the handle) would keep shifting and make the edge jitter.
    // Instead: where is the mouse in the window, and where does the sidebar start?
    let u = ui.clone();
    drag.connect_drag_update(move |gesture, _, _| {
        let (Some((x, y)), Some(handle)) = (gesture.point(None), gesture.widget()) else { return };
        let pointer = handle.compute_point(&u.window, &gtk::graphene::Point::new(x as f32, y as f32));
        let sidebar_left = u.sidebar.root.compute_point(&u.window, &gtk::graphene::Point::zero());
        if let (Some(pointer), Some(left)) = (pointer, sidebar_left) {
            let wanted = f64::from(pointer.x()) - grab_x.get() - f64::from(left.x());
            apply_sidebar_width(&u, wanted.round() as i32);
        }
    });
    let u = ui.clone();
    drag.connect_drag_end(move |_, _, _| save_sidebar(&u));
    handle.add_controller(drag);

    let double_click = gtk::GestureClick::new();
    let u = ui.clone();
    double_click.connect_pressed(move |_, presses, _, _| {
        if presses == 2 {
            set_sidebar_width(&u, SIDEBAR_DEFAULT);
        }
    });
    handle.add_controller(double_click);
}

/// Resize the sidebar; very narrow switches to icons only (see `saved::sidebar_size`).
fn apply_sidebar_width(ui: &Rc<Ui>, wanted: i32) {
    let (width, compact) = saved::sidebar_size(wanted, ui.sidebar.compact.get());
    if ui.sidebar.root.width_request() != width {
        ui.sidebar.root.set_width_request(width);
    }
    if compact != ui.sidebar.compact.get() {
        sidebar::set_compact(&ui.sidebar, compact);
        refresh_later(ui);
    }
}

/// Remember the sidebar's size for next time.
fn save_sidebar(ui: &Rc<Ui>) {
    let mut saved = ui.saved.borrow_mut();
    saved.sidebar_compact = ui.sidebar.compact.get();
    if !saved.sidebar_compact {
        saved.sidebar_width = ui.sidebar.root.width_request();
    }
    saved::store(&saved);
}

/// Set the sidebar width and remember it.
pub fn set_sidebar_width(ui: &Rc<Ui>, width: i32) {
    apply_sidebar_width(ui, width);
    save_sidebar(ui);
}

/// Ctrl+Shift+S: switch between icons only and the full sidebar (at its last width).
pub fn toggle_compact_sidebar(ui: &Rc<Ui>) {
    if ui.layout.get() == Layout::Classic {
        toast(ui, "Icons only is for the sidebar layout (Ctrl+Alt+L)");
        return;
    }
    if !ui.sidebar_revealer.reveals_child() {
        toggle_sidebar(ui);
    }
    let width = if ui.sidebar.compact.get() { ui.saved.borrow().sidebar_width } else { SIDEBAR_COMPACT };
    set_sidebar_width(ui, width);
}

/// Show the tabs in the sidebar (Arc) or along the top (classic).
pub fn apply_layout(ui: &Rc<Ui>, layout: Layout) {
    ui.layout.set(layout);
    let classic = layout == Layout::Classic;
    ui.topbar_revealer.set_visible(classic);
    ui.topbar_revealer.set_reveal_child(true);
    ui.sidebar_revealer.set_visible(!classic);
    if classic {
        ui.content_frame.add_css_class("classic");
    } else {
        ui.content_frame.remove_css_class("classic");
    }
    refresh(ui);
}

/// Ctrl+Alt+L: switch between the Arc sidebar and classic top tabs, remembered for next time.
pub fn toggle_layout(ui: &Rc<Ui>) {
    let next = match ui.layout.get() {
        Layout::Arc => Layout::Classic,
        Layout::Classic => Layout::Arc,
    };
    set_layout(ui, next);
}

pub fn set_layout(ui: &Rc<Ui>, layout: Layout) {
    apply_layout(ui, layout);
    crate::settings::save(ui, |doc| config::set(doc, &["appearance"], "layout", layout.name().into()));
    let label = match layout {
        Layout::Arc => "Layout: sidebar",
        Layout::Classic => "Layout: tabs on top",
    };
    toast(ui, label);
}

pub fn toggle_sidebar(ui: &Rc<Ui>) {
    // In the classic layout, "hide the sidebar" hides the tab bar instead.
    if ui.layout.get() == Layout::Classic {
        let show = !ui.topbar_revealer.reveals_child();
        ui.topbar_revealer.set_reveal_child(show);
        return;
    }
    let show = !ui.sidebar_revealer.reveals_child();
    ui.sidebar_revealer.set_reveal_child(show);
    if show {
        ui.content_frame.remove_css_class("no-sidebar");
    } else {
        ui.content_frame.add_css_class("no-sidebar");
    }
}

/// Show a short message at the bottom of the page for a moment.
pub fn toast(ui: &Rc<Ui>, text: &str) {
    ui.toast.set_label(text);
    ui.toast.set_visible(true);
    let weak = Rc::downgrade(ui);
    let shown = text.to_string();
    glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
        if let Some(ui) = weak.upgrade() {
            // Only hide it if no newer message replaced it in the meantime.
            if ui.toast.label() == shown {
                ui.toast.set_visible(false);
            }
        }
    });
}
