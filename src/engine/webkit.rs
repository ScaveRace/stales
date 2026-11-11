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

/// What Stales tells websites it is: a current Safari on Linux.
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

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

use super::{DownloadEvent, Engine, Page, PageEvent, PageKeys, Permission, PermissionAsk};

/// The keyboard script runs in its own JavaScript "world", hidden from the page's scripts.
const WORLD: &str = "stales";

pub struct WebKitEngine {
    /// Shared by all tabs: cookies, cache, logins and site data live here.
    session: NetworkSession,
    /// keys.js with its settings in front.
    keys_script: RefCell<String>,
    blocking: Rc<Blocking>,
    /// How Stales introduces itself to websites.
    user_agent: String,
    /// Where compiled ad blocker rules are kept between runs.
    filter_store: UserContentFilterStore,
}

impl WebKitEngine {
    pub fn new(keys: &PageKeys, user_agent: &str) -> Self {
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
        let user_agent = if user_agent.trim().is_empty() { DEFAULT_USER_AGENT } else { user_agent.trim() }.to_string();
        Self { session, keys_script, blocking: Rc::default(), user_agent, filter_store }
    }
}

/// What a WebKit permission request is about. `None`: refuse without asking.
/// An empty list: allow without asking.
fn permission_kinds(request: &webkit6::PermissionRequest) -> Option<Vec<Permission>> {
    use webkit6::*;
    if request.is::<GeolocationPermissionRequest>() {
        return Some(vec![Permission::Location]);
    }
    if request.is::<NotificationPermissionRequest>() {
        return Some(vec![Permission::Notifications]);
    }
    if let Some(media) = request.downcast_ref::<UserMediaPermissionRequest>() {
        let mut kinds = Vec::new();
        if media.is_for_video_device() {
            kinds.push(Permission::Camera);
        }
        if media.is_for_audio_device() {
            kinds.push(Permission::Microphone);
        }
        // Neither camera nor microphone: sharing your screen.
        if kinds.is_empty() {
            kinds.push(Permission::ScreenShare);
        }
        return Some(kinds);
    }
    if request.is::<DeviceInfoPermissionRequest>() {
        return Some(vec![Permission::DeviceList]);
    }
    if request.is::<WebsiteDataAccessPermissionRequest>() {
        return Some(vec![Permission::EmbeddedCookies]);
    }
    if request.is::<PointerLockPermissionRequest>() {
        return Some(Vec::new());
    }
    None
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
            // Tell sites we are a current Safari (which is what WebKit is). WebKitGTK's
            // own string claimed "Safari 60.5", so sites like YouTube served their slow
            // compatibility version for ancient browsers.
            settings.set_user_agent(Some(&self.user_agent));
            // Speed: keep pages you leave ready, so Back is instant.
            settings.set_enable_page_cache(true);
            // Two-finger swipe on the touchpad goes back and forward.
            settings.set_enable_back_forward_navigation_gestures(true);
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

        // Files WebKit can't show (zip, pdf you want to keep, installers, …) or that the
        // server marks as attachments become downloads. WebKit's default is to silently
        // ignore them, so without this nothing would happen when you click.
        view.connect_decide_policy(|_, decision, kind| {
            if kind != webkit6::PolicyDecisionType::Response {
                return false;
            }
            let Some(response) = decision.downcast_ref::<webkit6::ResponsePolicyDecision>() else { return false };
            // Only the page itself becomes a download, never an embedded frame (ads and
            // widgets often send such responses; they must not ask you anything).
            if !response.is_main_frame_main_resource() {
                return false;
            }
            let Some(reply) = response.response() else { return false };
            // WebKit also reports empty replies (status 0, no type), e.g. when a load is
            // interrupted; those are not files. A real download has a 2xx status and a type.
            let real = (200..300).contains(&reply.status_code())
                && reply.mime_type().is_some_and(|m| !m.is_empty())
                && reply.uri().is_some_and(|u| !u.is_empty());
            if !real {
                return false;
            }
            let attachment = reply
                .http_headers()
                .and_then(|h| h.one("Content-Disposition"))
                .is_some_and(|value| value.to_lowercase().starts_with("attachment"));
            if attachment || !response.is_mime_type_supported() {
                decision.download();
                return true;
            }
            false
        });

        // A site asks for your location, camera, … : turn WebKit's request into a
        // question for the browser, and answer WebKit when you have decided.
        let f = on_event.clone();
        view.connect_permission_request(move |_, request| {
            let Some(kinds) = permission_kinds(request) else {
                // Things Stales never offers (e.g. DRM keys) are refused quietly.
                request.deny();
                return true;
            };
            if kinds.is_empty() {
                request.allow(); // harmless, e.g. locking the mouse pointer in a game
                return true;
            }
            let request = request.clone();
            let reply = Box::new(move |allowed: bool| if allowed { request.allow() } else { request.deny() });
            f(PageEvent::Permission(PermissionAsk { kinds, reply }));
            true
        });
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

    fn set_download_handler(&self, on_event: Box<dyn Fn(DownloadEvent)>) {
        let on_event: Rc<dyn Fn(DownloadEvent)> = Rc::from(on_event);
        self.session.connect_download_started(move |_, download| {
            let url = download.request().and_then(|r| r.uri()).map(|u| u.to_string()).unwrap_or_default();
            let f = on_event.clone();
            download.connect_decide_destination(move |download, suggested| {
                let size = download.response().map(|r| r.content_length()).filter(|n| *n > 0);
                let d = download.clone();
                let decide = Box::new(move |path: Option<std::path::PathBuf>| match path {
                    Some(path) => d.set_destination(&path.to_string_lossy()),
                    None => d.cancel(),
                });
                f(DownloadEvent::Ask { file_name: suggested.to_string(), url: url.clone(), size, decide });
                true // we set the destination (or cancel) once you have answered
            });
            let f = on_event.clone();
            download.connect_finished(move |download| {
                if let Some(path) = download.destination() {
                    f(DownloadEvent::Finished(std::path::PathBuf::from(path.as_str())));
                }
            });
            let f = on_event.clone();
            download.connect_failed(move |download, error| {
                // Cancelling counts as failing; that's not worth a message.
                if !error.matches(webkit6::DownloadError::CancelledByUser) {
                    let file_name = download.destination().map(|d| d.to_string()).unwrap_or_default();
                    f(DownloadEvent::Failed { file_name, error: error.to_string() });
                }
            });
        });
    }

    fn set_notification_permissions(&self, allowed: Vec<String>, blocked: Vec<String>) {
        let Some(context) = webkit6::WebContext::default() else { return };
        let allowed: Vec<_> = allowed.iter().map(|o| webkit6::SecurityOrigin::for_uri(o)).collect();
        let blocked: Vec<_> = blocked.iter().map(|o| webkit6::SecurityOrigin::for_uri(o)).collect();
        let allowed: Vec<&webkit6::SecurityOrigin> = allowed.iter().collect();
        let blocked: Vec<&webkit6::SecurityOrigin> = blocked.iter().collect();
        context.initialize_notification_permissions(&allowed, &blocked);
    }

    fn clear_site_data(
        &self,
        cookies: bool,
        cache: bool,
        since_seconds: Option<u64>,
        done: Box<dyn FnOnce(Result<(), String>)>,
    ) {
        use webkit6::WebsiteDataTypes as T;
        let Some(data) = self.session.website_data_manager() else { return };
        let mut types = T::empty();
        if cookies {
            types |= T::COOKIES
                | T::LOCAL_STORAGE
                | T::SESSION_STORAGE
                | T::INDEXEDDB_DATABASES
                | T::OFFLINE_APPLICATION_CACHE
                | T::SERVICE_WORKER_REGISTRATIONS
                | T::HSTS_CACHE
                | T::ITP
                | T::DOM_CACHE;
        }
        if cache {
            types |= T::MEMORY_CACHE | T::DISK_CACHE;
        }
        if types.is_empty() {
            return done(Ok(()));
        }
        // WebKit counts in microseconds; 0 means "everything".
        let timespan = glib::TimeSpan(since_seconds.map_or(0, |s| (s as i64).saturating_mul(1_000_000)));
        // WebKit wants a callback that could run on any thread; it actually runs on
        // ours, and ThreadGuard checks that.
        let done = glib::thread_guard::ThreadGuard::new(done);
        data.clear(types, timespan, None::<&gtk::gio::Cancellable>, move |result| {
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
