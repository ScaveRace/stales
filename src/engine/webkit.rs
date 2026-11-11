//! The WebKitGTK backend: the same engine Safari and GNOME Web use, built for Linux.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::glib::translate::{FromGlibPtrFull, ToGlibPtr};
use gtk::prelude::*;
use webkit6::prelude::*;
use webkit6::{
    CookieAcceptPolicy, CookiePersistentStorage, FindOptions, NetworkSession, UserContentFilter,
    UserContentFilterStore, UserContentInjectedFrames, UserContentManager, UserScript, UserScriptInjectionTime,
    WebView, WebViewSessionState,
};

/// Name the compiled ad blocker rules are stored under.
const BLOCKLIST_ID: &str = "stales-blocker";

/// A tab's content manager, and whether blocking is on for it.
type BlockedTab = (glib::WeakRef<UserContentManager>, Rc<Cell<bool>>);

/// The ad blocker rules and every tab that uses them. WebKit applies the rules
/// itself, inside the engine, before a request even starts.
#[derive(Default)]
struct Blocking {
    rules: RefCell<Vec<UserContentFilter>>,
    tabs: RefCell<Vec<BlockedTab>>,
}

impl Blocking {
    fn apply(&self, content: &UserContentManager, on: bool) {
        content.remove_all_filters();
        if on {
            for rules in self.rules.borrow().iter() {
                content.add_filter(rules);
            }
        }
    }

    fn apply_everywhere(&self) {
        let mut tabs = self.tabs.borrow_mut();
        tabs.retain(|(content, _)| content.upgrade().is_some());
        for (content, on) in tabs.iter() {
            if let Some(content) = content.upgrade() {
                self.apply(&content, on.get());
            }
        }
    }
}

/// Histories bigger than this aren't saved (keeps the session file small).
const MAX_HISTORY_BYTES: usize = 256 * 1024;

use super::{Engine, Page, PageEvent, PageKeys};

/// The keyboard script runs in its own JavaScript "world", hidden from the page's scripts.
const WORLD: &str = "stales";

pub struct WebKitEngine {
    /// Shared by all tabs: cookies, cache, logins and site data live here.
    session: NetworkSession,
    /// keys.js with its settings in front.
    keys_script: RefCell<String>,
    blocking: Rc<Blocking>,
    /// Where compiled ad blocker rules are kept between runs.
    filter_store: UserContentFilterStore,
}

impl WebKitEngine {
    pub fn new(keys: &PageKeys) -> Self {
        let settings = serde_json::to_string(keys).expect("key settings are always valid JSON");
        let keys_script = RefCell::new(format!("const STALES = {settings};\n{}", include_str!("keys.js")));
        let data_dir = glib::user_data_dir().join("stales");
        let cache_dir = glib::user_cache_dir().join("stales");
        let session = NetworkSession::new(Some(&path_str(&data_dir)), Some(&path_str(&cache_dir)));

        if let Some(cookies) = session.cookie_manager() {
            // Remember logins between restarts…
            let cookie_file = data_dir.join("cookies.sqlite");
            cookies.set_persistent_storage(&path_str(&cookie_file), CookiePersistentStorage::Sqlite);
            // …but refuse cookies from other sites embedded in a page (trackers, ad networks).
            cookies.set_accept_policy(CookieAcceptPolicy::NoThirdParty);
        }
        // Intelligent Tracking Prevention: WebKit's built-in protection against cross-site tracking.
        session.set_itp_enabled(true);
        if let Some(data) = session.website_data_manager() {
            data.set_favicons_enabled(true);
        }

        let filter_store = UserContentFilterStore::new(&path_str(&data_dir.join("filters")));
        Self { session, keys_script, blocking: Rc::default(), filter_store }
    }
}

/// keys.js as a script for every frame of every page, in its own hidden world.
fn keys_user_script(source: &str) -> UserScript {
    UserScript::for_world(source, UserContentInjectedFrames::AllFrames, UserScriptInjectionTime::Start, WORLD, &[], &[])
}

/// Add Stales' page scripts (keyboard control, password manager) to a tab.
fn add_page_scripts(content: &UserContentManager, keys_script: &str) {
    content.add_script(&keys_user_script(keys_script));
    content.add_script(&UserScript::for_world(
        include_str!("passwords.js"),
        UserContentInjectedFrames::TopFrame,
        UserScriptInjectionTime::Start,
        WORLD,
        &[],
        &[],
    ));
}

fn path_str(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

impl Engine for WebKitEngine {
    fn new_page(&self, on_event: Box<dyn Fn(PageEvent)>) -> Rc<dyn Page> {
        let on_event: Rc<dyn Fn(PageEvent)> = Rc::from(on_event);

        // Each page gets the keyboard script, and a private channel to send us messages on.
        let content = UserContentManager::new();
        add_page_scripts(&content, &self.keys_script.borrow());
        content.register_script_message_handler("stales", Some(WORLD));
        let f = on_event.clone();
        content.connect_script_message_received(Some("stales"), move |_, value| {
            f(PageEvent::Script(value.to_str().into()));
        });

        // The settings page talks to us on "stalesSettings". That name is visible to
        // every page, so a message only counts if it comes while this tab shows one of
        // our own stales:// pages, which websites can't open or embed.
        let view_slot: Rc<RefCell<glib::WeakRef<WebView>>> = Rc::default();
        content.register_script_message_handler("stalesSettings", None);
        let (f, slot) = (on_event.clone(), view_slot.clone());
        content.connect_script_message_received(Some("stalesSettings"), move |_, value| {
            let from_own_page =
                slot.borrow().upgrade().and_then(|v| v.uri()).is_some_and(|uri| uri.starts_with("stales://"));
            if from_own_page {
                f(PageEvent::Settings(value.to_str().into()));
            } else {
                eprintln!("stales: ignored a settings message from a website");
            }
        });

        let blocking_on = Rc::new(Cell::new(true));
        self.blocking.apply(&content, true);
        self.blocking.tabs.borrow_mut().push((content.downgrade(), blocking_on.clone()));

        let view = WebView::builder().network_session(&self.session).user_content_manager(&content).build();
        view_slot.borrow().set(Some(&view));
        view.set_hexpand(true);
        view.set_vexpand(true);

        if let Some(settings) = WebViewExt::settings(&view) {
            settings.set_enable_developer_extras(true);
            settings.set_enable_smooth_scrolling(true);
        }

        // Turn WebKit's signals into our own `PageEvent`s.

        let f = on_event.clone();
        view.connect_title_notify(move |v| f(PageEvent::Title(v.title().unwrap_or_default().into())));
        let f = on_event.clone();
        view.connect_uri_notify(move |v| f(PageEvent::Url(v.uri().unwrap_or_default().into())));
        let f = on_event.clone();
        view.connect_estimated_load_progress_notify(move |_| f(PageEvent::LoadState));
        let f = on_event.clone();
        view.connect_is_loading_notify(move |_| f(PageEvent::LoadState));
        let f = on_event.clone();
        view.connect_load_changed(move |_, _| f(PageEvent::LoadState));
        let f = on_event.clone();
        view.connect_favicon_notify(move |v| f(PageEvent::Favicon(v.favicon())));
        let f = on_event.clone();
        view.connect_create(move |_, action| {
            if let Some(uri) = action.request().and_then(|r| r.uri()) {
                f(PageEvent::OpenInNewTab(uri.into()));
            }
            // Returning no view tells WebKit not to make a popup window; we opened a tab instead.
            None
        });
        let f = on_event.clone();
        view.connect_close(move |_| f(PageEvent::CloseRequested));
        if let Some(finder) = view.find_controller() {
            let f = on_event.clone();
            finder.connect_found_text(move |_, count| f(PageEvent::FindResult(Some(count))));
            let f = on_event;
            finder.connect_failed_to_find_text(move |_| f(PageEvent::FindResult(None)));
        }

        Rc::new(WebKitPage { view, content, blocking: self.blocking.clone(), blocking_on })
    }

    fn set_internal_pages(&self, render: super::PageRenderer) {
        let Some(context) = webkit6::WebContext::default() else { return };
        if let Some(security) = context.security_manager() {
            // "Local" means websites can't load or link to stales:// pages; only you can open them.
            security.register_uri_scheme_as_local("stales");
            security.register_uri_scheme_as_secure("stales");
        }
        context.register_uri_scheme("stales", move |request| {
            let uri = request.uri().unwrap_or_default();
            let name = uri.trim_start_matches("stales://").trim_end_matches('/');
            let name = name.split(['?', '#']).next().unwrap_or_default();
            let html = render(name).unwrap_or_else(|| {
                format!("<!doctype html><title>Not found</title><p>There is no page called stales://{name}</p>")
            });
            let bytes = glib::Bytes::from_owned(html.into_bytes());
            let length = bytes.len() as i64;
            let stream = gtk::gio::MemoryInputStream::from_bytes(&bytes);
            request.finish(&stream, length, Some("text/html; charset=utf-8"));
        });
    }

    fn clear_blocklist(&self) {
        self.blocking.rules.replace(Vec::new());
        self.blocking.apply_everywhere();
    }

    fn set_page_keys(&self, keys: &PageKeys) {
        let settings = serde_json::to_string(keys).expect("key settings are always valid JSON");
        let script = format!("const STALES = {settings};\n{}", include_str!("keys.js"));
        for (content, _) in self.blocking.tabs.borrow().iter() {
            if let Some(content) = content.upgrade() {
                content.remove_all_scripts();
                add_page_scripts(&content, &script);
            }
        }
        self.keys_script.replace(script);
    }

    fn clear_site_data(&self, done: Box<dyn FnOnce(Result<(), String>)>) {
        let Some(data) = self.session.website_data_manager() else { return };
        let types = webkit6::WebsiteDataTypes::ALL;
        // WebKit wants a callback that could run on any thread; it actually runs on
        // ours, and ThreadGuard checks that.
        let done = glib::thread_guard::ThreadGuard::new(done);
        data.clear(types, glib::TimeSpan(0), None::<&gtk::gio::Cancellable>, move |result| {
            (done.into_inner())(result.map_err(|e| e.to_string()));
        });
    }

    fn load_saved_blocklist(&self, parts: usize, done: Box<dyn FnOnce(bool)>) {
        if parts == 0 {
            done(false);
            return;
        }
        let (store, blocking) = (self.filter_store.clone(), self.blocking.clone());
        glib::spawn_future_local(async move {
            let mut loaded = Vec::new();
            for i in 0..parts {
                match store.load_future(&format!("{BLOCKLIST_ID}-{i}")).await {
                    Ok(rules) => loaded.push(rules),
                    Err(_) => return done(false),
                }
            }
            blocking.rules.replace(loaded);
            blocking.apply_everywhere();
            done(true);
        });
    }

    fn install_blocklist(&self, parts: Vec<String>, done: Box<dyn FnOnce(Result<(), String>)>) {
        let (store, blocking) = (self.filter_store.clone(), self.blocking.clone());
        glib::spawn_future_local(async move {
            let mut compiled = Vec::new();
            for (i, json) in parts.into_iter().enumerate() {
                let source = glib::Bytes::from_owned(json.into_bytes());
                match store.save_future(&format!("{BLOCKLIST_ID}-{i}"), &source).await {
                    Ok(rules) => compiled.push(rules),
                    Err(e) => return done(Err(e.to_string())),
                }
            }
            let used = compiled.len();
            blocking.rules.replace(compiled);
            blocking.apply_everywhere();
            done(Ok(()));
            // Delete compiled rules nobody uses any more (fewer parts than last time,
            // or the single-list format of older versions). They can be large.
            let _ = store.remove_future(BLOCKLIST_ID).await;
            for i in used..used + 8 {
                let _ = store.remove_future(&format!("{BLOCKLIST_ID}-{i}")).await;
            }
        });
    }

    fn cached_favicon(&self, url: &str, done: Box<dyn FnOnce(gtk::gdk::Texture)>) {
        let Some(icons) = self.session.website_data_manager().and_then(|d| d.favicon_database()) else { return };
        icons.favicon(url, None::<&gtk::gio::Cancellable>, move |result| {
            if let Ok(texture) = result {
                done(texture);
            }
        });
    }
}

struct WebKitPage {
    view: WebView,
    content: UserContentManager,
    blocking: Rc<Blocking>,
    blocking_on: Rc<Cell<bool>>,
}

impl Page for WebKitPage {
    fn widget(&self) -> gtk::Widget {
        self.view.clone().upcast()
    }
    fn load(&self, url: &str) {
        self.view.load_uri(url);
    }
    fn go_back(&self) {
        self.view.go_back();
    }
    fn go_forward(&self) {
        self.view.go_forward();
    }
    fn reload(&self) {
        self.view.reload();
    }
    fn stop(&self) {
        self.view.stop_loading();
    }
    fn can_go_back(&self) -> bool {
        self.view.can_go_back()
    }
    fn can_go_forward(&self) -> bool {
        self.view.can_go_forward()
    }
    fn is_loading(&self) -> bool {
        self.view.is_loading()
    }
    fn progress(&self) -> f64 {
        self.view.estimated_load_progress()
    }
    fn focus(&self) {
        self.view.grab_focus();
    }
    fn find(&self, text: &str) {
        let Some(finder) = self.view.find_controller() else { return };
        if text.is_empty() {
            finder.search_finish();
        } else {
            let options = FindOptions::CASE_INSENSITIVE | FindOptions::WRAP_AROUND;
            finder.search(text, options.bits(), 1000);
        }
    }
    fn find_next(&self) {
        if let Some(finder) = self.view.find_controller() {
            finder.search_next();
        }
    }
    fn find_previous(&self) {
        if let Some(finder) = self.view.find_controller() {
            finder.search_previous();
        }
    }
    fn zoom(&self) -> f64 {
        self.view.zoom_level()
    }
    fn set_zoom(&self, level: f64) {
        self.view.set_zoom_level(level);
    }
    fn run_script(&self, script: &str) {
        self.view.evaluate_javascript(script, None, None, None::<&gtk::gio::Cancellable>, |_| {});
    }
    fn run_hidden_script(&self, script: &str) {
        self.view.evaluate_javascript(script, Some(WORLD), None, None::<&gtk::gio::Cancellable>, |_| {});
    }
    fn test_key(&self, key: &str) {
        let key = serde_json::to_string(key).unwrap_or_default();
        self.run_hidden_script(&format!("window.__stalesKey && window.__stalesKey({key})"));
    }
    fn history(&self) -> Option<Vec<u8>> {
        let bytes = self.view.session_state()?.serialize()?;
        (bytes.len() <= MAX_HISTORY_BYTES).then(|| bytes.to_vec())
    }
    fn restore_history(&self, data: &[u8]) -> bool {
        let bytes = glib::Bytes::from(data);
        // Called directly because WebKit returns nothing for invalid data, and the
        // Rust wrapper doesn't expect that.
        let raw = unsafe { webkit6::ffi::webkit_web_view_session_state_new(bytes.to_glib_none().0) };
        if raw.is_null() {
            return false;
        }
        let state = unsafe { WebViewSessionState::from_glib_full(raw) };
        self.view.restore_session_state(&state);
        // Restoring only fills the history; load the page it was on.
        match self.view.back_forward_list().and_then(|list| list.current_item()) {
            Some(item) => {
                self.view.go_to_back_forward_list_item(&item);
                true
            }
            None => false,
        }
    }
    fn set_blocking(&self, on: bool) {
        if self.blocking_on.replace(on) != on {
            self.blocking.apply(&self.content, on);
        }
    }
    fn close(&self) {
        self.view.stop_loading();
        self.view.terminate_web_process();
    }
    fn show_inspector(&self) {
        if let Some(inspector) = self.view.inspector() {
            inspector.show();
        }
    }
}
