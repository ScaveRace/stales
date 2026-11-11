//! Things Stales remembers between runs that aren't settings you edit by hand,
//! like how wide you dragged the sidebar. Stored in `~/.local/share/stales/ui.toml`.

use std::path::PathBuf;

use gtk::glib;
use serde::{Deserialize, Serialize};

pub const SIDEBAR_DEFAULT: i32 = 260;
pub const SIDEBAR_MIN: i32 = 180;
pub const SIDEBAR_MAX: i32 = 520;
/// Width of the icons-only sidebar.
pub const SIDEBAR_COMPACT: i32 = 56;
/// Dragging the sidebar narrower than this switches it to icons only…
pub const SNAP_TO_ICONS: i32 = 120;
/// …and wider than this switches back. The gap between the two stops it from
/// flickering between modes while your mouse is near the edge.
pub const SNAP_TO_FULL: i32 = 160;

/// Where the sidebar ends up for a wanted width: (width, icons only?).
pub fn sidebar_size(wanted: i32, icons_only_now: bool) -> (i32, bool) {
    let icons_only = if icons_only_now { wanted < SNAP_TO_FULL } else { wanted < SNAP_TO_ICONS };
    // Always clamp: GTK aborts the program on a negative width.
    let width = if icons_only { SIDEBAR_COMPACT } else { wanted.clamp(SIDEBAR_MIN, SIDEBAR_MAX) };
    (width, icons_only)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapping_has_a_gap_so_it_does_not_flicker() {
        // Full sidebar dragged to 130: not yet icons only.
        assert_eq!(sidebar_size(130, false), (SIDEBAR_MIN, false));
        // Below 120: icons only.
        assert_eq!(sidebar_size(110, false), (SIDEBAR_COMPACT, true));
        // Icons only, mouse wobbling around 130: stays icons only.
        assert_eq!(sidebar_size(130, true), (SIDEBAR_COMPACT, true));
        assert_eq!(sidebar_size(150, true), (SIDEBAR_COMPACT, true));
        // Past 160: full again.
        assert_eq!(sidebar_size(170, true), (SIDEBAR_MIN, false));
        assert_eq!(sidebar_size(300, true), (300, false));
    }

    #[test]
    fn width_is_always_safe() {
        assert_eq!(sidebar_size(-500, false), (SIDEBAR_COMPACT, true));
        assert_eq!(sidebar_size(99_999, false), (SIDEBAR_MAX, false));
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    /// Width of the full sidebar (kept while it is in icons-only mode).
    pub sidebar_width: i32,
    /// Icons-only sidebar.
    pub sidebar_compact: bool,
    /// Sites where you turned the ad blocker off with the shield button.
    pub blocker_allow: Vec<String>,
    /// Sites where you chose "Never" in the save-login bar.
    pub password_never: Vec<String>,
    /// Old versions kept the theme and layout here; they now live in config.toml.
    /// Read once to move them over, never written again.
    #[serde(skip_serializing)]
    pub theme: Option<String>,
    #[serde(skip_serializing)]
    pub layout: Option<String>,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            sidebar_width: SIDEBAR_DEFAULT,
            sidebar_compact: false,
            blocker_allow: Vec::new(),
            password_never: Vec::new(),
            theme: None,
            layout: None,
        }
    }
}

fn path() -> PathBuf {
    glib::user_data_dir().join("stales").join("ui.toml")
}

pub fn load() -> Saved {
    let text = std::fs::read_to_string(path()).unwrap_or_default();
    let mut saved: Saved = toml::from_str(&text).unwrap_or_default();
    saved.sidebar_width = saved.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
    saved.blocker_allow.retain(|site| !site.trim().is_empty());
    saved
}

pub fn store(saved: &Saved) {
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = toml::to_string(saved) {
        let _ = std::fs::write(path, text);
    }
}
