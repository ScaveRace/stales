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
