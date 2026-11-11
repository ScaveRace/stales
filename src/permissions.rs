//! When a site wants something (your location, camera, microphone, notifications,
//! screen, its cookies inside another site) or a file wants to download, Stales asks
//! in a bar at the top of the page: "Allow" / "Block". Your answer is remembered per
//! site, and you can change it in Settings → Site permissions.
//!
//! Several questions at once wait in line; the bar shows one at a time.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use serde::{Deserialize, Serialize};

use crate::engine::{DownloadEvent, Permission, PermissionAsk};
use crate::passwords::origin_of;
use crate::state::TabId;
use crate::window::{self, Ui};

/// A remembered answer: `origin` may (or may not) use `kind`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SitePermission {
    pub origin: String,
    pub kind: Permission,
    pub allow: bool,
}

pub fn describe(kind: Permission) -> &'static str {
    match kind {
        Permission::Location => "your location",
        Permission::Notifications => "to show notifications",
        Permission::Camera => "your camera",
        Permission::Microphone => "your microphone",
        Permission::ScreenShare => "to see your screen",
        Permission::EmbeddedCookies => "its cookies inside another site",
        Permission::DeviceList => "the list of your cameras and microphones",
    }
}

fn icon(kind: Permission) -> &'static str {
    match kind {
        Permission::Location => "find-location-symbolic",
        Permission::Notifications => "preferences-system-notifications-symbolic",
        Permission::Camera | Permission::DeviceList => "camera-web-symbolic",
        Permission::Microphone => "audio-input-microphone-symbolic",
        Permission::ScreenShare => "video-display-symbolic",
        Permission::EmbeddedCookies => "network-server-symbolic",
    }
}

/// The remembered answer for one thing, if any.
fn remembered(ui: &Ui, origin: &str, kind: Permission) -> Option<bool> {
    ui.saved.borrow().site_permissions.iter().find(|p| p.origin == origin && p.kind == kind).map(|p| p.allow)
}

fn remember(ui: &Ui, origin: &str, kinds: &[Permission], allow: bool) {
    let mut saved = ui.saved.borrow_mut();
    saved.site_permissions.retain(|p| !(p.origin == origin && kinds.contains(&p.kind)));
    for kind in kinds {
        saved.site_permissions.push(SitePermission { origin: origin.to_string(), kind: *kind, allow });
    }
    crate::saved::store(&saved);
}

/// A site in tab `id` asks for something.
pub fn ask(ui: &Rc<Ui>, id: TabId, ask: PermissionAsk) {
    let origin = {
        let mut state = ui.state.borrow_mut();
        state.tab_mut(id).and_then(|t| origin_of(&t.url))
    };
    let Some(origin) = origin else { return (ask.reply)(false) };

    // Listing devices is part of using a camera or microphone: allowed where one of
    // them is, refused quietly elsewhere (no extra question).
    if ask.kinds == [Permission::DeviceList] {
        let ok = remembered(ui, &origin, Permission::Camera) == Some(true)
            || remembered(ui, &origin, Permission::Microphone) == Some(true);
        return (ask.reply)(ok);
    }

    // Already answered for everything asked? Then answer at once.
    let answers: Vec<Option<bool>> = ask.kinds.iter().map(|k| remembered(ui, &origin, *k)).collect();
    if answers.iter().all(Option::is_some) {
        return (ask.reply)(answers.iter().all(|a| *a == Some(true)));
    }

    let site = origin.split("://").nth(1).unwrap_or(&origin).to_string();
    let wants: Vec<&str> = ask.kinds.iter().map(|k| describe(*k)).collect();
    let text = format!("{site} wants {}", wants.join(" and "));
    let kinds = ask.kinds.clone();
    let weak = Rc::downgrade(ui);
    let reply = ask.reply;
    question(
        ui,
        Question {
            icon: icon(kinds[0]),
            text,
            yes: "Allow",
            no: "Block",
            answer: Box::new(move |allow| {
                if let Some(ui) = weak.upgrade() {
                    remember(&ui, &origin, &kinds, allow);
                    if kinds.contains(&Permission::Notifications) {
                        send_notification_permissions(&ui);
                    }
                }
                reply(allow);
            }),
        },
    );
}

/// Tell the engine which sites may show notifications (at startup and after changes).
pub fn send_notification_permissions(ui: &Ui) {
    let (allowed, blocked): (Vec<_>, Vec<_>) = ui
        .saved
        .borrow()
        .site_permissions
        .iter()
        .filter(|p| p.kind == Permission::Notifications)
        .map(|p| (p.origin.clone(), p.allow))
        .partition(|(_, allow)| *allow);
    ui.engine.set_notification_permissions(
        allowed.into_iter().map(|(o, _)| o).collect(),
        blocked.into_iter().map(|(o, _)| o).collect(),
    );
}

// ---------- Downloads ----------

pub fn connect_downloads(ui: &Rc<Ui>) {
    let weak = Rc::downgrade(ui);
    ui.engine.set_download_handler(Box::new(move |event| {
        let Some(ui) = weak.upgrade() else { return };
        match event {
            DownloadEvent::Ask { file_name, url, size, decide } => {
                let folder = download_folder(&ui);
                if !ui.config.borrow().downloads.ask {
                    return decide(Some(free_name(&folder, &file_name)));
                }
                let site = origin_of(&url).map(|o| o.split("://").nth(1).unwrap_or("").to_string()).unwrap_or_default();
                let size = size.map(|n| format!(" ({})", human_size(n))).unwrap_or_default();
                let from = if site.is_empty() { String::new() } else { format!(" from {site}") };
                question(
                    &ui,
                    Question {
                        icon: "folder-download-symbolic",
                        text: format!("Download “{file_name}”{size}{from}?"),
                        yes: "Save",
                        no: "Cancel",
                        answer: Box::new(move |save| decide(save.then(|| free_name(&folder, &file_name)))),
                    },
                );
            }
            DownloadEvent::Finished(path) => {
                // Stales makes its own files private; a download should be a normal file
                // that other programs (file manager, image viewer, …) can open.
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                window::toast(&ui, &format!("Downloaded {name}"));
            }
            DownloadEvent::Failed { file_name, error } => {
                eprintln!("stales: download {file_name} failed: {error}");
                window::toast(&ui, "Download failed");
            }
        }
    }));
}

fn download_folder(ui: &Ui) -> PathBuf {
    let configured = ui.config.borrow().downloads.folder.trim().to_string();
    if !configured.is_empty() {
        let path = configured.strip_prefix("~/").map(|rest| gtk::glib::home_dir().join(rest));
        return path.unwrap_or_else(|| PathBuf::from(configured));
    }
    gtk::glib::user_special_dir(gtk::glib::UserDirectory::Downloads)
        .unwrap_or_else(|| gtk::glib::home_dir().join("Downloads"))
}

/// `folder/name`, or `folder/name (2).ext` etc. if that file exists already.
fn free_name(folder: &std::path::Path, name: &str) -> PathBuf {
    let _ = std::fs::create_dir_all(folder);
    // Never let a site choose a path outside the folder ("../../.bashrc").
    let name = std::path::Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned());
    let name = name.filter(|n| !n.is_empty() && n != "." && n != "..").unwrap_or_else(|| "download".into());
    let first = folder.join(&name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (name.clone(), String::new()),
    };
    (2..).map(|n| folder.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).expect("some number is free")
}

fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} bytes"),
    }
}

// ---------- The question bar ----------

pub struct Question {
    pub icon: &'static str,
    pub text: String,
    pub yes: &'static str,
    pub no: &'static str,
    pub answer: Box<dyn FnOnce(bool)>,
}

pub struct QuestionBar {
    pub root: gtk::Box,
    icon: gtk::Image,
    label: gtk::Label,
    yes: gtk::Button,
    no: gtk::Button,
    /// The question on screen, then the ones waiting.
    queue: RefCell<VecDeque<Question>>,
}

pub fn build_bar() -> QuestionBar {
    let icon = gtk::Image::from_icon_name("dialog-question-symbolic");
    let label = gtk::Label::new(None);
    label.set_hexpand(true);
    label.set_xalign(0.0);
    label.set_wrap(true);
    let yes = gtk::Button::with_label("Allow");
    yes.add_css_class("st-prompt-main");
    yes.set_tooltip_text(Some("Ctrl+Shift+Y"));
    let no = gtk::Button::with_label("Block");
    no.set_tooltip_text(Some("Ctrl+Shift+N"));
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    root.add_css_class("st-prompt");
    root.set_valign(gtk::Align::Start);
    root.set_halign(gtk::Align::Center);
    root.set_margin_top(12);
    root.set_width_request(560);
    for widget in [icon.upcast_ref::<gtk::Widget>(), label.upcast_ref(), no.upcast_ref(), yes.upcast_ref()] {
        root.append(widget);
    }
    root.set_visible(false);
    QuestionBar { root, icon, label, yes, no, queue: RefCell::new(VecDeque::new()) }
}

pub fn connect_bar(ui: &Rc<Ui>) {
    let u = ui.clone();
    ui.question_bar.yes.connect_clicked(move |_| answer(&u, true));
    let u = ui.clone();
    ui.question_bar.no.connect_clicked(move |_| answer(&u, false));
}

/// Ask something; it waits in line behind questions already showing.
pub fn question(ui: &Rc<Ui>, question: Question) {
    let bar = &ui.question_bar;
    bar.queue.borrow_mut().push_back(question);
    if bar.queue.borrow().len() == 1 {
        show_first(ui);
    }
}

fn show_first(ui: &Ui) {
    let bar = &ui.question_bar;
    let queue = bar.queue.borrow();
    match queue.front() {
        Some(q) => {
            bar.icon.set_icon_name(Some(q.icon));
            bar.label.set_label(&q.text);
            bar.yes.set_label(q.yes);
            bar.no.set_label(q.no);
            bar.root.set_visible(true);
        }
        None => bar.root.set_visible(false),
    }
}

/// The answer to the question on screen (buttons, Ctrl+Shift+Y / Ctrl+Shift+N).
pub fn answer(ui: &Rc<Ui>, yes: bool) {
    let Some(question) = ui.question_bar.queue.borrow_mut().pop_front() else { return };
    show_first(ui);
    (question.answer)(yes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_names_stay_inside_the_folder_and_do_not_overwrite() {
        let dir = std::env::temp_dir().join(format!("stales-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(free_name(&dir, "../../.bashrc"), dir.join(".bashrc"));
        assert_eq!(free_name(&dir, ""), dir.join("download"));
        std::fs::write(dir.join("photo.jpg"), b"x").unwrap();
        assert_eq!(free_name(&dir, "photo.jpg"), dir.join("photo (2).jpg"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(human_size(500), "500 bytes");
        assert_eq!(human_size(12 * 1024 * 1024), "12.0 MB");
    }
}
