//! Optional remote control, for scripting Stales and for automated testing.
//!
//! Off by default. Start Stales with `STALES_CONTROL=1` and it listens on
//! `$XDG_RUNTIME_DIR/stales.sock`, a file only your user can open. Each line you
//! send is one command, and Stales answers with one line. For example:
//!
//! ```text
//! echo "open archlinux.org" | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/stales.sock
//! ```
//!
//! Commands: open <text>, go <text>, select <n>, pin <n>, close, back, forward,
//! reload, space <n>, new-space, sidebar, cmdbar [text], cmdbar-edit, cmdbar-close,
//! search-engine <name>, action <name>, page-key <key>, find <text>, find-count,
//! sidebar-width, blocker, blocker-update, js <code>, settings-msg <json>, theme <name>, themes, layout <arc|classic>, space-color <name>, state

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::command_bar::{self, Mode};
use crate::state::Selected;
use crate::window::{self, Ui};
use crate::{actions, find_bar};

pub fn start(ui: &Rc<Ui>) {
    if std::env::var("STALES_CONTROL").as_deref() != Ok("1") {
        return;
    }
    let path = glib::user_runtime_dir().join("stales.sock");
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("stales: could not open control socket {}: {e}", path.display());
            return;
        }
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    eprintln!("stales: remote control listening on {}", path.display());

    // GTK may only be touched from the main thread. A background thread waits for
    // connections and passes each command over a channel to the main thread.
    let (sender, receiver) = async_channel::unbounded::<(String, UnixStream)>();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let Ok(reader) = stream.try_clone() else { continue };
            for line in BufReader::new(reader).lines().map_while(Result::ok) {
                let Ok(reply_to) = stream.try_clone() else { break };
                if sender.send_blocking((line, reply_to)).is_err() {
                    return;
                }
            }
        }
    });

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        while let Ok((line, mut reply_to)) = receiver.recv().await {
            let reply = run(&ui, line.trim());
            let _ = writeln!(reply_to, "{reply}");
        }
    });
}

fn run(ui: &Rc<Ui>, line: &str) -> String {
    let (command, arg) = line.split_once(' ').unwrap_or((line, ""));
    let number = || arg.trim().parse::<usize>().ok();
    match command {
        "open" => {
            ui.state.borrow_mut().open_tab(arg);
            window::refresh(ui);
        }
        "go" => {
            ui.state.borrow_mut().navigate_active(arg);
            window::load_active(ui);
        }
        "select" | "pin" => {
            let Some(n) = number() else { return "error: needs a number".into() };
            let selected = if command == "pin" { Selected::Pinned(n) } else { Selected::Tab(n) };
            if !ui.state.borrow_mut().select(selected) {
                return format!("error: no {command} {n} in this space");
            }
            window::refresh(ui);
        }
        "close" => {
            ui.state.borrow_mut().close_active();
            window::refresh(ui);
        }
        "back" | "forward" | "reload" => {
            let Some(page) = window::active_page(ui) else { return "error: no page".into() };
            match command {
                "back" => page.go_back(),
                "forward" => page.go_forward(),
                _ => page.reload(),
            }
        }
        "space" => {
            let Some(n) = number() else { return "error: needs a number".into() };
            if !ui.state.borrow_mut().switch_space(n) {
                return format!("error: no space {n}");
            }
            window::refresh(ui);
        }
        "new-space" => {
            ui.state.borrow_mut().add_space();
            window::refresh(ui);
        }
        "sidebar" => window::toggle_sidebar(ui),
        "cmdbar" => {
            command_bar::open(ui, Mode::NewTab);
            ui.command.entry.set_text(arg);
            ui.command.entry.set_position(-1);
        }
        "cmdbar-edit" => command_bar::open(ui, Mode::EditCurrent),
        "cmdbar-close" => command_bar::close(ui),
        "search-engine" => {
            if !ui.state.borrow_mut().search.set_default(arg) {
                return format!("error: no search engine '{arg}'");
            }
        }
        "action" => {
            if !actions::run(ui, arg) {
                return format!("error: no action '{arg}'");
            }
        }
        "page-key" => {
            let Some(page) = window::active_page(ui) else { return "error: no page".into() };
            page.test_key(arg);
        }
        "find" => {
            find_bar::open(ui);
            ui.find_bar.entry.set_text(arg);
        }
        "find-count" => return ui.find_bar.count.label().to_string(),
        "sidebar-width" => return ui.sidebar.root.width_request().to_string(),
        "blocker" => return ui.blocker_status.borrow().describe(),
        "blocker-update" => crate::blocker::update(ui),
        "theme" => {
            if !crate::theme::set(ui, arg) {
                return format!("error: no theme '{arg}'");
            }
        }
        "layout" => match crate::window::Layout::from_name(arg) {
            Some(layout) => window::set_layout(ui, layout),
            None => return format!("error: no layout '{arg}' (arc or classic)"),
        },
        "themes" => return crate::theme::names().join(" "),
        "space-color" => {
            if !crate::theme::set_space_color(ui, arg) {
                return format!("error: no colour '{arg}'");
            }
        }
        // Send a message as if the settings page had sent it (for tests).
        "settings-msg" => {
            let id = ui.state.borrow().active_id();
            if let Some(id) = id {
                crate::settings::handle(ui, id, arg);
            }
        }
        // Run JavaScript in the current page, as the page itself (for tests).
        "js" => {
            let Some(page) = window::active_page(ui) else { return "error: no page".into() };
            page.run_script(arg);
        }
        "state" => return describe(ui),
        _ => return format!("error: unknown command '{command}'"),
    }
    "ok".into()
}

/// One line describing what the browser shows right now.
fn describe(ui: &Rc<Ui>) -> String {
    let state = ui.state.borrow();
    let space = state.space();
    let (title, url) = state.active_tab().map(|t| (t.title.clone(), t.url.clone())).unwrap_or_default();
    let page = window::active_page(ui);
    format!(
        "engine={:?} space={} tabs={} pinned={} active_title={:?} active_url={:?} loading={} can_back={} can_forward={} favicons={} pages_alive={}",
        state.search.default_engine().name,
        space.name,
        space.tabs.len(),
        space.pinned.len(),
        title,
        url,
        page.as_ref().is_some_and(|p| p.is_loading()),
        page.as_ref().is_some_and(|p| p.can_go_back()),
        page.as_ref().is_some_and(|p| p.can_go_forward()),
        ui.favicons.borrow().len(),
        ui.pages.borrow().len(),
    )
}
