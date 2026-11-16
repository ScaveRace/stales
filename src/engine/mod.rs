//! The web engine, behind an interface.
//!
//! The rest of Stales only talks to the `Engine` and `Page` traits below, never to
//! WebKit directly. That way a different engine (Servo, Chromium) can be added later
//! by writing one new file next to `webkit.rs`, without touching the browser UI.

pub mod webkit;

use std::rc::Rc;

use gtk::gdk;

/// Makes the HTML of a stales:// page from its name (`None`: no such page).
pub type PageRenderer = Box<dyn Fn(&str) -> Option<String>>;

/// Something a site can ask to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    Location,
    Notifications,
    Camera,
    Microphone,
    ScreenShare,
    /// An embedded site (e.g. a video player) wants its cookies inside another site.
    EmbeddedCookies,
    /// Listing your cameras and microphones; allowed only where you allowed one of them.
    DeviceList,
}

/// A site asks for permissions; call `reply` with your answer.
pub struct PermissionAsk {
    pub kinds: Vec<Permission>,
    pub reply: Box<dyn FnOnce(bool)>,
}

/// A download wants to start, finished, or failed.
pub enum DownloadEvent {
    /// Call `decide` with where to save it, or `None` to cancel.
    Ask {
        file_name: String,
        url: String,
        size: Option<u64>,
        decide: Box<dyn FnOnce(Option<std::path::PathBuf>)>,
    },
    Finished(std::path::PathBuf),
    Failed {
        file_name: String,
        error: String,
    },
}

/// Things a page tells the browser about while it runs.
pub enum PageEvent {
    Title(String),
    Url(String),
    /// Loading started or stopped, progress moved, or back/forward became possible.
    LoadState,
    /// The site's icon changed. `None` means the new page has no icon.
    Favicon(Option<gdk::Texture>),
    /// The page wants to open a link in a new window (`target="_blank"`, `window.open`).
    OpenInNewTab(String),
    /// The page called `window.close()`.
    CloseRequested,
    /// A message from the keyboard script (keys.js): an action name like "tab-next",
    /// or "open-new-tab <url>".
    Script(String),
    /// A message from one of Stales' own pages (stales://settings, stales://setup).
    /// Never sent for websites.
    Settings(String),
    /// Find in page: how many matches, or `None` if the text wasn't found.
    FindResult(Option<u32>),
    /// The site wants to use your location, camera, … (see `Permission`).
    Permission(PermissionAsk),
}

/// One web page (one tab).
pub trait Page {
    /// The GTK widget that shows the page.
    fn widget(&self) -> gtk::Widget;
    fn load(&self, url: &str);
    fn go_back(&self);
    fn go_forward(&self);
    fn reload(&self);
    fn stop(&self);
    fn can_go_back(&self) -> bool;
    fn can_go_forward(&self) -> bool;
    fn is_loading(&self) -> bool;
    /// From 0.0 to 1.0.
    fn progress(&self) -> f64;
    fn focus(&self);
    /// Developer tools (Ctrl+Shift+I).
    fn show_inspector(&self);
    /// Find in page. An empty text clears the highlights.
    fn find(&self, text: &str);
    fn find_next(&self);
    fn find_previous(&self);
    /// 1.0 is 100 %.
    fn zoom(&self) -> f64;
    fn set_zoom(&self, level: f64);
    /// Press a key in the keyboard script, as if typed. For automated tests.
    fn test_key(&self, key: &str);
    /// Run JavaScript in the page. Used to send updates to Stales' own pages.
    fn run_script(&self, script: &str);
    /// Run JavaScript in Stales' hidden script world (keys.js, passwords.js),
    /// which the page's own scripts can't see.
    fn run_hidden_script(&self, script: &str);
    /// The back/forward history (and scroll positions) as bytes, to save to disk.
    fn history(&self) -> Option<Vec<u8>>;
    /// Bring back a saved history and load its current page. Returns false if the
    /// data isn't valid (then nothing happens and the caller should just load the URL).
    fn restore_history(&self, data: &[u8]) -> bool;
    /// The tab was closed: free everything, including the page's background processes.
    fn close(&self);
}

/// Settings for the keyboard script that runs inside pages (see keys.js).
#[derive(serde::Serialize)]
pub struct PageKeys {
    /// Single-key commands on or off.
    pub vim: bool,
    /// Key sequence → action name, e.g. "j" → "scroll-down", "gg" → "scroll-top".
    pub keys: std::collections::HashMap<String, String>,
    /// Sites where single keys are left to the site (e.g. Gmail has its own).
    pub exclude: Vec<String>,
}

pub trait Engine {
    /// Make a new, empty page. `on_event` is called whenever something about it changes.
    fn new_page(&self, on_event: Box<dyn Fn(PageEvent)>) -> Rc<dyn Page>;
    /// The icon the engine remembers for a URL from an earlier visit, if any.
    /// `done` is only called when an icon was found.
    fn cached_favicon(&self, url: &str, done: Box<dyn FnOnce(gdk::Texture)>);
    /// Serve Stales' own pages at `stales://name` (e.g. stales://welcome).
    /// `render` gets the name and returns the page's HTML, or `None` for "not found".
    fn set_internal_pages(&self, render: PageRenderer);
    /// New single-key settings: used by new pages, and by open tabs after they reload.
    fn set_page_keys(&self, keys: &PageKeys);
    /// Downloads are reported here (all tabs share one handler).
    fn set_download_handler(&self, on_event: Box<dyn Fn(DownloadEvent)>);
    /// Sites you allowed or blocked notifications for, so pages know at once.
    fn set_notification_permissions(&self, allowed: Vec<String>, blocked: Vec<String>);
    /// Delete site data from the last `since_seconds` (None: all of it): cookies and
    /// everything sites store (logs you out), and/or the cache (pages load slower once).
    fn clear_site_data(
        &self,
        cookies: bool,
        cache: bool,
        since_seconds: Option<u64>,
        done: Box<dyn FnOnce(Result<(), String>)>,
    );
}
