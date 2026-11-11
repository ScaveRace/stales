//! Stales: a fast, customisable browser for Linux with an Arc-style sidebar.

mod actions;
mod blocker;
mod command_bar;
mod config;
mod control;
mod engine;
mod find_bar;
mod help;
mod passwords;
mod saved;
mod search;
mod session;
mod settings;
mod sidebar;
mod state;
mod theme;
mod topbar;
mod welcome;
mod window;

use gtk::prelude::*;

const APP_ID: &str = "dev.stales.Stales";

fn main() -> gtk::glib::ExitCode {
    make_files_private();
    // HANDLES_OPEN: `stales https://example.com` (or clicking a link in another app
    // when Stales is your default browser) opens that address. GTK makes Stales a
    // single instance: if it is already running, the address goes to that window.
    let app =
        gtk::Application::builder().application_id(APP_ID).flags(gtk::gio::ApplicationFlags::HANDLES_OPEN).build();
    app.connect_activate(|app| {
        window::show(app);
    });
    app.connect_open(|app, files, _| {
        let ui = window::show(app);
        for file in files {
            window::open_external(&ui, &file.uri());
        }
    });
    app.run()
}

/// Your cookies (= your logins), history and tabs must not be readable by other
/// people using this computer.
///
/// 1. The "umask" makes every file Stales (and WebKit's helper processes, which
///    inherit it) creates readable and writable by you only.
/// 2. Stales' three folders are locked to you, which also protects files created
///    by older versions before this fix.
fn make_files_private() {
    // SAFETY: umask only changes this process's default permissions for new files.
    unsafe {
        libc::umask(0o077);
    }
    use std::os::unix::fs::PermissionsExt;
    for base in [gtk::glib::user_config_dir(), gtk::glib::user_data_dir(), gtk::glib::user_cache_dir()] {
        let dir = base.join("stales");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        // Files directly inside (cookies, tabs, settings) made by older versions.
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if entry.file_type().is_ok_and(|t| t.is_file()) {
                let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o600));
            }
        }
    }
}
