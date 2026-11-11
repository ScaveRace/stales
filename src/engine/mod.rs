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
    /// Turn the ad blocker on or off for this tab (off for sites you allowed).
    fn set_blocking(&self, on: bool);
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
    /// Load the ad blocker rules compiled last time. `done(false)` if there are none yet.
    /// `parts`: how many rule lists were compiled last time.
    fn load_saved_blocklist(&self, parts: usize, done: Box<dyn FnOnce(bool)>);
    /// Compile new ad blocker rules (content-blocker JSON), keep them for next time,
    /// and use them in every tab.
    /// `parts`: one or more content-blocker JSON lists (WebKit limits the size of each).
    fn install_blocklist(&self, parts: Vec<String>, done: Box<dyn FnOnce(Result<(), String>)>);
    /// Stop blocking in every tab (the blocker was turned off in the settings).
    fn clear_blocklist(&self);
    /// New single-key settings: used by new pages, and by open tabs after they reload.
    fn set_page_keys(&self, keys: &PageKeys);
    /// Delete cookies, cache and site data (logs you out everywhere).
    fn clear_site_data(&self, done: Box<dyn FnOnce(Result<(), String>)>);
}
