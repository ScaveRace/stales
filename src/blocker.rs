//! The built-in ad and tracker blocker.
//!
//! 1. Filter lists (EasyList, EasyPrivacy, uBlock filters: the same ones uBlock
//!    Origin uses) are downloaded on the first start and refreshed every week.
//! 2. Brave's blocker library (`adblock`) turns them into WebKit's "content blocker"
//!    format. That happens on a background thread, so the window never freezes.
//! 3. WebKit compiles the rules once and keeps them on disk. From then on it blocks
//!    ads inside the engine, before a request even starts: faster than any extension.
//!
//! Sites you allow (shield button or Ctrl+Shift+B, or `allow` in config.toml) get no blocking.

use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gtk::glib;
use serde::{Deserialize, Serialize};

use crate::search::host_of;
use crate::window::{self, Ui};

/// What the blocker is doing right now, shown in the shield button's tooltip.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Off,
    /// Loading or updating the lists.
    Working(String),
    Ready {
        rules: Option<usize>,
    },
    Failed(String),
}

impl Status {
    pub fn describe(&self) -> String {
        match self {
            Status::Off => "Ad blocker is turned off in config.toml".into(),
            Status::Working(what) => format!("Ad blocker: {what}"),
            Status::Ready { rules: Some(n) } => format!("Ad blocker on ({n} rules)"),
            Status::Ready { rules: None } => "Ad blocker on".into(),
            Status::Failed(e) => format!("Ad blocker problem: {e}"),
        }
    }
}

/// When the lists were last downloaded, and which lists they were.
#[derive(Serialize, Deserialize, Default)]
struct Stamp {
    updated: u64,
    lists: Vec<String>,
    rules: usize,
    /// How many rule lists WebKit compiled (see `convert`).
    #[serde(default)]
    parts: usize,
}

/// WebKit refuses rule lists with more than about 150 000 rules.
const MAX_RULES_PER_PART: usize = 140_000;

fn stamp_path() -> std::path::PathBuf {
    glib::user_data_dir().join("stales").join("filters").join("stamp.toml")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn read_stamp() -> Stamp {
    std::fs::read_to_string(stamp_path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

/// Start the blocker: use the rules from last time right away, and download
/// fresh lists in the background if they are old (or the list choice changed).
pub fn start(ui: &Rc<Ui>) {
    let config = ui.config.borrow().blocker.clone();
    if !config.enabled {
        set_status(ui, Status::Off);
        return;
    }
    let stamp = read_stamp();
    let max_age = u64::from(config.update_days.max(1)) * 24 * 60 * 60;
    let outdated = stamp.lists != config.lists || now().saturating_sub(stamp.updated) > max_age;

    set_status(ui, Status::Working("loading rules".into()));
    // Rules from last time load in well under a second; let pages wait for them so
    // the first page after a start is blocked too. (No saved rules yet, on the very
    // first start, means a download: pages don't wait for that.) Never wait longer
    // than 3 seconds, whatever happens.
    if stamp.parts > 0 {
        ui.hold_pages.set(true);
        let weak = Rc::downgrade(ui);
        glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
            if let Some(ui) = weak.upgrade() {
                window::release_held_pages(&ui);
            }
        });
    }
    let weak = Rc::downgrade(ui);
    let rules = (stamp.rules > 0).then_some(stamp.rules);
    ui.engine.load_saved_blocklist(
        stamp.parts,
        Box::new(move |found| {
            let Some(ui) = weak.upgrade() else { return };
            window::release_held_pages(&ui);
            if found {
                set_status(&ui, Status::Ready { rules });
            }
            if outdated || !found {
                update(&ui);
            }
        }),
    );
}

/// Download the lists, convert them and install the new rules.
pub fn update(ui: &Rc<Ui>) {
    let lists = ui.config.borrow().blocker.lists.clone();
    let weak = Rc::downgrade(ui);
    glib::spawn_future_local(async move {
        let progress = |status: Status| {
            if let Some(ui) = weak.upgrade() {
                set_status(&ui, status);
            }
        };
        let was_ready = weak.upgrade().is_some_and(|ui| matches!(*ui.blocker_status.borrow(), Status::Ready { .. }));
        if !was_ready {
            progress(Status::Working(format!("downloading {} filter lists", lists.len())));
        }

        use soup::prelude::*;
        let session = soup::Session::new();
        session.set_timeout(30);
        let mut texts = Vec::new();
        for url in &lists {
            match download(&session, url).await {
                Ok(text) => texts.push(text),
                Err(e) => eprintln!("stales: could not download filter list {url}: {e}"),
            }
        }
        if texts.is_empty() {
            if !was_ready {
                progress(Status::Failed("could not download any filter list (offline?)".into()));
            }
            return;
        }

        if !was_ready {
            progress(Status::Working("preparing rules".into()));
        }
        // Converting ~100 000 rules takes a moment: do it on another thread.
        let (sender, receiver) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = sender.send_blocking(convert(texts));
        });
        let Ok((parts, count)) = receiver.recv().await else { return };
        let part_count = parts.len();

        let Some(ui) = weak.upgrade() else { return };
        if !was_ready {
            set_status(&ui, Status::Working(format!("compiling {count} rules")));
        }
        let weak = Rc::downgrade(&ui);
        ui.engine.install_blocklist(
            parts,
            Box::new(move |result| {
                let Some(ui) = weak.upgrade() else { return };
                match result {
                    Ok(()) => {
                        let stamp = Stamp { updated: now(), lists, rules: count, parts: part_count };
                        if let Ok(text) = toml::to_string(&stamp) {
                            let _ = std::fs::write(stamp_path(), text);
                        }
                        set_status(&ui, Status::Ready { rules: Some(count) });
                        eprintln!("stales: ad blocker ready with {count} rules");
                    }
                    Err(e) => {
                        eprintln!("stales: WebKit could not compile the ad blocker rules: {e}");
                        if !was_ready {
                            set_status(&ui, Status::Failed(e));
                        }
                    }
                }
            }),
        );
    });
}

async fn download(session: &soup::Session, url: &str) -> Result<String, String> {
    use soup::prelude::*;
    let message = soup::Message::new("GET", url).map_err(|e| e.to_string())?;
    let bytes = session.send_and_read_future(&message, glib::Priority::DEFAULT).await.map_err(|e| e.to_string())?;
    let status = message.status();
    if status != soup::Status::Ok {
        return Err(format!("server answered {status:?}"));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Filter lists (Adblock Plus / uBlock syntax) → WebKit content-blocker JSON.
/// Rules WebKit can't express (scriptlets, procedural filters) are skipped.
///
/// WebKit takes at most ~150 000 rules per list, so big selections are split into
/// several parts. Exception rules ("allow this on that site") only work inside their
/// own list, so every part ends with all of them.
fn convert(lists: Vec<String>) -> (Vec<String>, usize) {
    use adblock::content_blocking::CbType;
    let mut set = adblock::lists::FilterSet::new(true);
    for text in lists {
        set.add_filter_list(text, adblock::lists::ParseOptions::default());
    }
    let rules = set.into_content_blocking().map(|(rules, _)| rules).unwrap_or_default();
    let count = rules.len();
    let (exceptions, blocks): (Vec<_>, Vec<_>) =
        rules.into_iter().partition(|rule| matches!(rule.action.typ, CbType::IgnorePreviousRules));
    let per_part = MAX_RULES_PER_PART.saturating_sub(exceptions.len()).max(1000);
    let parts = blocks
        .chunks(per_part)
        .map(|chunk| {
            let mut part = chunk.to_vec();
            part.extend(exceptions.iter().cloned());
            serde_json::to_string(&part).unwrap_or_else(|_| "[]".into())
        })
        .collect();
    (parts, count)
}

fn set_status(ui: &Rc<Ui>, status: Status) {
    ui.blocker_status.replace(status);
    window::refresh_later(ui);
}

fn host_matches(host: &str, entry: &str) -> bool {
    let entry = entry.trim().trim_start_matches("www.");
    !entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}")))
}

/// Has the user turned blocking off for this page's site?
pub fn is_allowed(ui: &Ui, url: &str) -> bool {
    let Some(host) = host_of(url) else { return false };
    ui.config.borrow().blocker.allow.iter().any(|e| host_matches(&host, e))
        || ui.saved.borrow().blocker_allow.iter().any(|e| host_matches(&host, e))
}

/// Show the blocker's state on a shield button: filled while it blocks this site.
pub fn show_shield(ui: &Ui, shield: &gtk::Button, url: Option<&str>) {
    use gtk::prelude::*;
    let status = ui.blocker_status.borrow().clone();
    let allowed = url.is_some_and(|u| is_allowed(ui, u));
    let (icon, on, tip) = match (&status, allowed) {
        (Status::Off, _) => ("security-low-symbolic", false, status.describe()),
        (_, true) => {
            ("security-low-symbolic", false, "Ad blocker is off for this site · click to turn it on".to_string())
        }
        (Status::Ready { .. }, false) => {
            ("security-high-symbolic", true, format!("{} · click to allow this site", status.describe()))
        }
        _ => ("security-medium-symbolic", false, status.describe()),
    };
    shield.set_icon_name(icon);
    shield.set_tooltip_text(Some(&format!("{tip} (Ctrl+Shift+B)")));
    if on {
        shield.add_css_class("on");
    } else {
        shield.remove_css_class("on");
    }
}

/// Shield button / Ctrl+Shift+B: turn blocking off (or back on) for the current site.
pub fn toggle_site(ui: &Rc<Ui>) {
    if !ui.config.borrow().blocker.enabled {
        window::toast(ui, "The ad blocker is turned off in config.toml");
        return;
    }
    let Some(url) = ui.state.borrow().active_tab().map(|t| t.url.clone()) else { return };
    // Local files and internal pages have no site name; they are always blocked.
    let Some(host) = host_of(&url).filter(|h| !h.is_empty()) else {
        window::toast(ui, "This page has no site to allow");
        return;
    };
    if ui.config.borrow().blocker.allow.iter().any(|e| host_matches(&host, e)) {
        window::toast(ui, &format!("{host} is allowed in config.toml"));
        return;
    }
    let now_allowed = {
        let mut saved = ui.saved.borrow_mut();
        let before = saved.blocker_allow.len();
        saved.blocker_allow.retain(|e| !host_matches(&host, e));
        let removed = saved.blocker_allow.len() != before;
        if !removed {
            saved.blocker_allow.push(host.clone());
        }
        crate::saved::store(&saved);
        !removed
    };
    window::toast(
        ui,
        &if now_allowed { format!("Ad blocker off for {host}") } else { format!("Ad blocker on for {host}") },
    );
    if let Some(page) = window::active_page(ui) {
        page.set_blocking(!now_allowed);
        page.reload();
    }
    window::refresh(ui);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_common_rules() {
        let list = "||doubleclick.net^\n##.ad-banner\nexample.com##.sponsored\n@@||good.example^\n".to_string();
        let (parts, count) = convert(vec![list]);
        assert!(count >= 3, "only {count} rules");
        assert_eq!(parts.len(), 1);
        assert!(parts[0].contains("doubleclick"));
        assert!(parts[0].contains("css-display-none"));
    }

    #[test]
    fn big_lists_are_split_and_every_part_keeps_the_exceptions() {
        let mut list = String::from("@@||good.example^\n");
        for i in 0..(MAX_RULES_PER_PART + 5000) {
            list.push_str(&format!("||ads{i}.example^\n"));
        }
        let (parts, _) = convert(vec![list]);
        assert_eq!(parts.len(), 2);
        for part in &parts {
            let rules: Vec<serde_json::Value> = serde_json::from_str(part).unwrap();
            assert!(rules.len() <= MAX_RULES_PER_PART);
            assert!(part.contains("good"), "a part lost the exception rule");
        }
    }

    #[test]
    fn site_matching_includes_subdomains() {
        assert!(host_matches("www.example.com", "example.com"));
        assert!(host_matches("example.com", "www.example.com"));
        assert!(!host_matches("notexample.com", "example.com"));
        assert!(!host_matches("example.com", ""));
    }
}
