//! Themes and colours.
//!
//! The look is built from four layers of CSS, each one able to change the ones below:
//!
//! 1. `base.css`: layout and shapes (built in)
//! 2. the theme: colours, maybe a gradient (built in, or `~/.config/stales/themes/<name>.css`)
//! 3. the current Space's colour, if you gave it one (like Arc's Spaces)
//! 4. `~/.config/stales/user.css`: your own tweaks, always on top
//!
//! Theme files and user.css are watched: save the file and the browser changes at once.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::window::{self, Ui};

/// Built-in themes: name, CSS.
pub const BUILT_IN: &[(&str, &str)] = &[
    ("midnight", include_str!("theme/themes/midnight.css")),
    ("paper", include_str!("theme/themes/paper.css")),
    ("graphite", include_str!("theme/themes/graphite.css")),
    ("aurora", include_str!("theme/themes/aurora.css")),
    ("ember", include_str!("theme/themes/ember.css")),
];

/// Colours a Space can have (Ctrl+Alt+C cycles through them).
pub const SPACE_COLORS: &[(&str, &str)] = &[
    ("rose", "#f43f5e"),
    ("orange", "#f97316"),
    ("amber", "#f59e0b"),
    ("green", "#22c55e"),
    ("teal", "#14b8a6"),
    ("blue", "#3b82f6"),
    ("violet", "#8b5cf6"),
    ("pink", "#ec4899"),
];

const USER_CSS_TEMPLATE: &str = include_str!("theme/user_template.css");

pub struct Themes {
    theme: gtk::CssProvider,
    space: gtk::CssProvider,
    user: gtk::CssProvider,
    /// The theme in use right now.
    pub current: RefCell<String>,
    /// The Space colour CSS in use, so it is only reloaded when it changes.
    space_css: RefCell<String>,
    /// File watchers; they stop when dropped, so they are kept here.
    monitors: RefCell<Vec<gio::FileMonitor>>,
}

fn config_dir() -> PathBuf {
    glib::user_config_dir().join("stales")
}

fn user_theme_path(name: &str) -> PathBuf {
    config_dir().join("themes").join(format!("{name}.css"))
}

fn user_css_path() -> PathBuf {
    config_dir().join("user.css")
}

/// Every theme you can pick: built-in ones, then your own files.
pub fn names() -> Vec<String> {
    let mut names: Vec<String> = BUILT_IN.iter().map(|(n, _)| n.to_string()).collect();
    if let Ok(entries) = std::fs::read_dir(config_dir().join("themes")) {
        let mut own: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().into_owned()))
            .filter(|n| !names.contains(n))
            .collect();
        own.sort();
        names.extend(own);
    }
    names
}

/// The theme's CSS: your file of that name if there is one, else the built-in one.
fn theme_css(name: &str) -> Option<String> {
    std::fs::read_to_string(user_theme_path(name))
        .ok()
        .or_else(|| BUILT_IN.iter().find(|(n, _)| *n == name).map(|(_, css)| css.to_string()))
}

/// Themes say "stales: light" in a comment if they are light; everything else is dark.
fn is_light(css: &str) -> bool {
    css.contains("stales: light")
}

/// Set up the four layers. Call once at startup, before the window shows.
pub fn init() -> Themes {
    let display = gdk::Display::default().expect("no display found (is Wayland or X11 running?)");
    let base = gtk::CssProvider::new();
    base.load_from_string(include_str!("theme/base.css"));
    let themes = Themes {
        theme: gtk::CssProvider::new(),
        space: gtk::CssProvider::new(),
        user: gtk::CssProvider::new(),
        current: RefCell::new(String::new()),
        space_css: RefCell::new(String::new()),
        monitors: RefCell::new(Vec::new()),
    };
    let layers = [(&base, 0), (&themes.theme, 1), (&themes.space, 2)];
    for (provider, offset) in layers {
        gtk::style_context_add_provider_for_display(
            &display,
            provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + offset,
        );
    }
    gtk::style_context_add_provider_for_display(&display, &themes.user, gtk::STYLE_PROVIDER_PRIORITY_USER);

    // Report mistakes in theme files instead of silently ignoring them.
    for (provider, what) in [(&themes.theme, "theme"), (&themes.user, "user.css")] {
        provider.connect_parsing_error(move |_, section, error| {
            eprintln!("stales: problem in {what} at {}: {error}", section.to_str());
        });
    }

    // A starting point for your own tweaks.
    let user_css = user_css_path();
    if !user_css.exists() {
        let _ = std::fs::create_dir_all(config_dir().join("themes"));
        let _ = std::fs::write(&user_css, USER_CSS_TEMPLATE);
    }
    themes
}

/// Start using the theme chosen in ui.toml or config.toml, plus user.css, and watch them.
pub fn start(ui: &Rc<Ui>) {
    let chosen = ui.config.borrow().appearance.theme.clone();
    if !apply(ui, &chosen) {
        eprintln!("stales: there is no theme called '{chosen}', using midnight");
        apply(ui, "midnight");
    }
    load_user_css(ui);
    watch(ui, user_css_path(), load_user_css);
    apply_space_color(ui);
}

/// Switch to a theme. Returns false if there is no theme with that name.
pub fn apply(ui: &Rc<Ui>, name: &str) -> bool {
    let Some(css) = theme_css(name) else { return false };
    ui.themes.theme.load_from_string(&css);
    // Tell GTK and the web pages whether this is a dark theme, so sites that
    // support dark mode follow along.
    if let Some(settings) = gtk::Settings::default() {
        let light = is_light(&css);
        // GTK 4.20+ has a new setting for this; older GTK (e.g. Ubuntu 24.04) only the old one.
        let nick = if light { "light" } else { "dark" };
        let new_style = settings
            .find_property("gtk-interface-color-scheme")
            .and_then(|spec| glib::EnumClass::with_type(spec.value_type()))
            .and_then(|class| class.to_value_by_nick(nick));
        match new_style {
            Some(value) => settings.set_property_from_value("gtk-interface-color-scheme", &value),
            None => settings.set_gtk_application_prefer_dark_theme(!light),
        }
    }
    let changed = ui.themes.current.replace(name.to_string()) != name;
    if changed && user_theme_path(name).exists() {
        let theme = name.to_string();
        watch(ui, user_theme_path(name), move |ui| {
            if *ui.themes.current.borrow() == theme {
                apply(ui, &theme);
            }
        });
    }
    true
}

fn load_user_css(ui: &Rc<Ui>) {
    let css = std::fs::read_to_string(user_css_path()).unwrap_or_default();
    ui.themes.user.load_from_string(&css);
}

/// Call `reload` whenever the file is saved.
fn watch(ui: &Rc<Ui>, path: PathBuf, reload: impl Fn(&Rc<Ui>) + 'static) {
    let Ok(monitor) = gio::File::for_path(&path).monitor_file(gio::FileMonitorFlags::NONE, None::<&gio::Cancellable>)
    else {
        return;
    };
    let weak = Rc::downgrade(ui);
    monitor.connect_changed(move |_, _, _, event| {
        if matches!(event, gio::FileMonitorEvent::ChangesDoneHint | gio::FileMonitorEvent::Created)
            && let Some(ui) = weak.upgrade()
        {
            reload(&ui);
        }
    });
    ui.themes.monitors.borrow_mut().push(monitor);
}

/// Next theme in the list (Ctrl+Alt+T), remembered for next time.
pub fn cycle(ui: &Rc<Ui>) {
    let names = names();
    let current = ui.themes.current.borrow().clone();
    let next = names.iter().position(|n| *n == current).map_or(0, |i| (i + 1) % names.len());
    set(ui, &names[next]);
}

/// Switch theme and remember the choice.
pub fn set(ui: &Rc<Ui>, name: &str) -> bool {
    if !apply(ui, name) {
        return false;
    }
    crate::settings::save(ui, |doc| crate::config::set(doc, &["appearance"], "theme", name.into()));
    window::toast(ui, &format!("Theme: {name}"));
    true
}

/// Tint the window with the current Space's colour, if it has one.
pub fn apply_space_color(ui: &Rc<Ui>) {
    let color = ui.state.borrow().space().color.clone();
    let css = match color.as_deref().and_then(color_value) {
        Some(value) => format!(
            "@define-color st_space {value};\n\
             window.stales {{ background: mix(@st_bg, @st_space, 0.22); }}\n\
             .st-sidebar row.st-tab.active, .st-pinned.active {{ background: alpha(@st_space, 0.28); }}\n\
             .st-space-btn.active {{ color: @st_space; }}\n"
        ),
        None => String::new(),
    };
    // Reloading CSS restyles the whole window, so only do it when something changed.
    if *ui.themes.space_css.borrow() != css {
        ui.themes.space.load_from_string(&css);
        ui.themes.space_css.replace(css);
    }
}

/// Is this a colour a Space can have (a palette name or #rrggbb)?
pub fn is_space_color(name: &str) -> bool {
    color_value(name).is_some()
}

/// A theme's main colours, for the little previews on the settings page.
pub fn preview(name: &str) -> serde_json::Value {
    let css = theme_css(name).unwrap_or_default();
    let colour = |key: &str, fallback: &str| -> String {
        css.lines()
            .find_map(|line| line.trim().strip_prefix(&format!("@define-color {key}")))
            .map(|rest| rest.trim().trim_end_matches(';').trim().to_string())
            .unwrap_or_else(|| fallback.to_string())
    };
    // A theme may paint the window with a gradient instead of a plain colour.
    let background = css
        .split("window.stales")
        .nth(1)
        .and_then(|rule| rule.split("background:").nth(1))
        .and_then(|value| value.split(';').next())
        .map(|v| v.trim().to_string());
    serde_json::json!({
        "name": name,
        "bg": colour("st_bg", "#111113"),
        "background": background.unwrap_or_else(|| colour("st_bg", "#111113")),
        "page": colour("st_page", "#0a0a0b"),
        "fg": colour("st_fg", "#e4e4e7"),
        "muted": colour("st_muted", "#85858f"),
        "accent": colour("st_accent", "#e4e4e7"),
        "light": is_light(&css),
        "custom": user_theme_path(name).exists(),
    })
}

/// "blue" → "#3b82f6"; a "#rrggbb" value is used as it is.
fn color_value(name: &str) -> Option<String> {
    if let Some((_, hex)) = SPACE_COLORS.iter().find(|(n, _)| *n == name) {
        return Some(hex.to_string());
    }
    let valid = name.len() == 7 && name.starts_with('#') && name[1..].chars().all(|c| c.is_ascii_hexdigit());
    valid.then(|| name.to_string())
}

/// Ctrl+Alt+C: give the current Space the next colour (after the last one: no colour).
pub fn cycle_space_color(ui: &Rc<Ui>) {
    let name = {
        let mut state = ui.state.borrow_mut();
        let current = state.current;
        let space = &mut state.spaces[current];
        let next = match space.color.as_deref().and_then(|c| SPACE_COLORS.iter().position(|(n, _)| *n == c)) {
            None if space.color.is_none() => Some(0),
            Some(i) if i + 1 < SPACE_COLORS.len() => Some(i + 1),
            _ => None,
        };
        space.color = next.map(|i| SPACE_COLORS[i].0.to_string());
        space.color.clone()
    };
    apply_space_color(ui);
    window::refresh(ui);
    window::toast(ui, &format!("Space colour: {}", name.as_deref().unwrap_or("none")));
}

/// Set the current Space's colour by name or #rrggbb; "none" removes it.
pub fn set_space_color(ui: &Rc<Ui>, name: &str) -> bool {
    let color = match name {
        "none" | "" => None,
        other if color_value(other).is_some() => Some(other.to_string()),
        _ => return false,
    };
    {
        let mut state = ui.state.borrow_mut();
        let current = state.current;
        state.spaces[current].color = color;
    }
    apply_space_color(ui);
    window::refresh(ui);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_themes_say_whether_they_are_light() {
        let paper = BUILT_IN.iter().find(|(n, _)| *n == "paper").unwrap().1;
        assert!(is_light(paper));
        let midnight = BUILT_IN.iter().find(|(n, _)| *n == "midnight").unwrap().1;
        assert!(!is_light(midnight));
    }

    #[test]
    fn every_theme_sets_every_colour() {
        let colours =
            ["st_bg", "st_page", "st_surface", "st_fg", "st_muted", "st_hover", "st_active", "st_border", "st_accent"];
        for (name, css) in BUILT_IN {
            for colour in colours {
                assert!(css.contains(&format!("@define-color {colour}")), "theme {name} has no {colour}");
            }
        }
    }

    #[test]
    fn space_colours_by_name_or_hex() {
        assert_eq!(color_value("blue").as_deref(), Some("#3b82f6"));
        assert_eq!(color_value("#abcdef").as_deref(), Some("#abcdef"));
        assert_eq!(color_value("#xyz"), None);
        assert_eq!(color_value("chartreuse"), None);
    }
}
