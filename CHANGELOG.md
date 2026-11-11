# Changelog

## 0.1.0 (2026-11-10)

First public version.

- Arc-style sidebar with Spaces, pinned tabs and a command bar; resizable, with an icons-only mode
- Classic layout with tabs along the top (Ctrl+Alt+L)
- WebKitGTK engine behind an engine interface; each tab loads only when first opened
- Full keyboard control: every browser action has a shortcut that websites can't block;
  Vimium-style page keys with link hints, find in page, insert mode; F1 cheat sheet
- Built-in ad and tracker blocker using EasyList, EasyPrivacy and uBlock filters,
  compiled into WebKit content rules; per-site shield toggle
- Third-party cookies blocked, WebKit tracking prevention on
- Search engine choice, custom engines and @ shortcuts
- Themes (midnight, paper, graphite, aurora, ember), per-Space colours, user themes,
  user.css, live reload; web pages follow the theme's light/dark mode
- Tabs, pins, Spaces and per-tab history restored after restart
- Welcome page that shows your actual shortcuts
- Single instance: opening a link from another app adds a tab to the running window
- Everything configurable in ~/.config/stales/config.toml
- First-start setup (layout, theme, search, keyboard, privacy) and a settings page (Ctrl+,)
  that changes everything live and saves to config.toml without losing comments
- Password manager using your system keyring (GNOME Keyring, KWallet, KeePassXC):
  offers to save after you log in, fills only on the same secure site and only when you
  pick an account or press Ctrl+Shift+L; logins listed in Settings
- Data files are private to your user (cookies, tabs, settings)
- Ad blocker splits very large filter selections into several WebKit rule lists
- Ad blocker uses uBlock Origin's default lists plus optional cookie-banner and pop-up
  lists; the first page after a start waits for the rules
- Fixed: a hiding exception in uBlock's "unbreak" list was converted into "hide every
  SVG", which removed YouTube's logo and icons (and icons elsewhere)
- Identifies as a current Safari (WebKitGTK claimed "Safari 60.5", so sites served
  their slow fallback version); back/forward cache and touchpad swipe navigation
- Permission prompts for location, camera, microphone, notifications, screen sharing,
  embedded-site cookies and downloads, remembered per site; downloads actually work now
- Drag tabs to pin, reorder or move into folders; pinned tile count, per-row and size
  settings; Arc-style folders of saved links; dimmed look for unloaded tabs
- History (Ctrl+H) with search, retention setting and command-bar suggestions;
  delete history, cookies or cache by time range
- Password hardening: passwords wiped from memory after use, no core dumps,
  copied passwords marked secret for clipboard managers
