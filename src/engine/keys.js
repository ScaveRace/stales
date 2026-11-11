// Stales keyboard control for web pages (like Vimium / qutebrowser).
//
// This runs inside every page, in a separate "isolated world": it sees the page's
// HTML but the page's own JavaScript can't see it, call it or fake its messages.
// `STALES` (the settings and key map) is put in front of this file by Rust.
//
// Scrolling and link hints happen here. Everything else (switch tab, go back, …)
// is sent to the browser as a message with the action's name.

(() => {
  if (window.__stalesLoaded) return;
  window.__stalesLoaded = true;

  const send = (msg) => window.webkit.messageHandlers.stales.postMessage(msg);
  const excluded = STALES.exclude.some((host) => location.hostname === host || location.hostname.endsWith("." + host));

  // ---------- Helpers ----------

  function deepActive() {
    let el = document.activeElement;
    while (el && el.shadowRoot && el.shadowRoot.activeElement) el = el.shadowRoot.activeElement;
    return el;
  }

  function isEditable(el) {
    if (!el) return false;
    if (el.isContentEditable) return true;
    if (el.tagName === "TEXTAREA" || el.tagName === "SELECT") return true;
    if (el.tagName === "INPUT") {
      const t = (el.type || "text").toLowerCase();
      return !["button", "submit", "reset", "checkbox", "radio", "image", "range", "color", "file"].includes(t);
    }
    return false;
  }

  // The element that actually scrolls. Many sites scroll an inner box, not the page.
  let cachedScroller = null;
  let cachedAt = 0;
  function scroller() {
    const now = Date.now();
    if (cachedScroller && cachedScroller.isConnected && now - cachedAt < 1500) return cachedScroller;
    const page = document.scrollingElement || document.documentElement;
    let best = page;
    if (page.scrollHeight <= page.clientHeight + 4) {
      let bestArea = 0;
      for (const el of document.querySelectorAll("body *")) {
        if (el.scrollHeight <= el.clientHeight + 4) continue;
        const overflow = getComputedStyle(el).overflowY;
        if (overflow !== "auto" && overflow !== "scroll") continue;
        const r = el.getBoundingClientRect();
        const area = r.width * r.height;
        if (area > bestArea) {
          bestArea = area;
          best = el;
        }
      }
    }
    cachedScroller = best;
    cachedAt = now;
    return best;
  }

  function scroll(top, left, smooth) {
    scroller().scrollBy({ top, left, behavior: smooth ? "smooth" : "instant" });
  }

  // A small label in the bottom-left corner (the find bar uses the right one), for insert mode and pending keys.
  let badge = null;
  function showBadge(text) {
    if (!badge) {
      badge = document.createElement("div");
      badge.style.cssText =
        "position:fixed;left:12px;bottom:12px;z-index:2147483647;padding:4px 10px;" +
        "border-radius:8px;background:#18181b;color:#e4e4e7;font:600 12px/1.4 system-ui,sans-serif;" +
        "box-shadow:0 4px 16px rgba(0,0,0,.4);pointer-events:none";
    }
    badge.textContent = text;
    if (!badge.isConnected) document.documentElement.appendChild(badge);
  }
  function hideBadge() {
    if (badge) badge.remove();
  }

  // ---------- Link hints ----------

  const HINT_CHARS = "sadfjklewcmpgh";
  const CLICKABLE =
    'a[href], button, input:not([type="hidden"]), textarea, select, summary, label[for], ' +
    '[role="button"], [role="link"], [role="tab"], [role="menuitem"], [role="checkbox"], ' +
    '[role="option"], [role="switch"], [onclick], [contenteditable="true"], [contenteditable=""], ' +
    '[tabindex]:not([tabindex="-1"])';

  function isVisible(el) {
    const r = el.getBoundingClientRect();
    if (r.width < 3 || r.height < 3) return null;
    if (r.bottom < 0 || r.right < 0 || r.top > innerHeight || r.left > innerWidth) return null;
    const style = getComputedStyle(el);
    if (style.visibility === "hidden" || style.opacity === "0") return null;
    // Is it actually on top, not hidden behind something else?
    const points = [
      [r.left + r.width / 2, r.top + r.height / 2],
      [r.left + 3, r.top + 3],
    ];
    for (const [x, y] of points) {
      const hit = document.elementFromPoint(Math.min(Math.max(x, 0), innerWidth - 1), Math.min(Math.max(y, 0), innerHeight - 1));
      if (hit && (hit === el || el.contains(hit) || hit.contains(el))) return r;
    }
    return null;
  }

  function labels(count) {
    const n = HINT_CHARS.length;
    let length = 1;
    while (Math.pow(n, length) < count) length++;
    const out = [];
    for (let i = 0; i < count; i++) {
      let label = "";
      let rest = i;
      for (let j = 0; j < length; j++) {
        label = HINT_CHARS[rest % n] + label;
        rest = Math.floor(rest / n);
      }
      out.push(label);
    }
    return out;
  }

  let hints = null; // { newTab, typed, items: [{el, label, tag}], box }

  function startHints(newTab) {
    const seen = new Set();
    const targets = [];
    for (const el of document.querySelectorAll(CLICKABLE)) {
      if (seen.has(el)) continue;
      seen.add(el);
      if (newTab && !el.href) continue;
      const rect = isVisible(el);
      if (rect) targets.push({ el, rect });
    }
    if (targets.length === 0) {
      showBadge("No links on screen");
      setTimeout(hideBadge, 1200);
      return;
    }
    const box = document.createElement("div");
    box.style.cssText = "position:fixed;inset:0;z-index:2147483647;pointer-events:none";
    const names = labels(targets.length);
    const items = targets.map(({ el, rect }, i) => {
      const tag = document.createElement("span");
      tag.textContent = names[i].toUpperCase();
      tag.style.cssText =
        `position:fixed;left:${Math.max(rect.left, 0)}px;top:${Math.max(rect.top, 0)}px;` +
        "padding:1px 5px;border-radius:5px;background:#f4f4f5;color:#09090b;" +
        "font:700 11px/1.3 ui-monospace,monospace;letter-spacing:.5px;" +
        "box-shadow:0 2px 8px rgba(0,0,0,.45);border:1px solid rgba(0,0,0,.25)";
      box.appendChild(tag);
      return { el, label: names[i], tag };
    });
    document.documentElement.appendChild(box);
    hints = { newTab, typed: "", items, box };
    showBadge(newTab ? "Open link in new tab" : "Follow link");
  }

  function stopHints() {
    if (hints) hints.box.remove();
    hints = null;
    hideBadge();
  }

  function clickElement(el) {
    if (isEditable(el)) {
      el.focus();
      return;
    }
    // Some sites react to mousedown, not click, so send the whole sequence.
    for (const type of ["pointerdown", "mousedown", "pointerup", "mouseup"]) {
      el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, view: window }));
    }
    el.click();
    if (el.tagName === "SELECT" || el.getAttribute("tabindex") !== null) el.focus();
  }

  function hintKey(key) {
    if (key === "Escape") return stopHints();
    if (key === "Backspace") hints.typed = hints.typed.slice(0, -1);
    else if (key.length === 1 && HINT_CHARS.includes(key.toLowerCase())) hints.typed += key.toLowerCase();
    else return stopHints();

    const matching = hints.items.filter((item) => item.label.startsWith(hints.typed));
    for (const item of hints.items) {
      item.tag.style.display = item.label.startsWith(hints.typed) ? "" : "none";
    }
    if (matching.length === 0) return stopHints();
    if (matching.length === 1 && matching[0].label === hints.typed) {
      const { el } = matching[0];
      const newTab = hints.newTab;
      stopHints();
      if (newTab) send("open-new-tab " + el.href);
      else clickElement(el);
    }
  }

  // ---------- Keys ----------

  let insertMode = false;
  let pending = "";
  let pendingTimer = null;

  // Actions that happen inside the page. Everything else goes to the browser.
  const local = {
    "scroll-down": () => scroll(70, 0),
    "scroll-up": () => scroll(-70, 0),
    "scroll-left": () => scroll(0, -70),
    "scroll-right": () => scroll(0, 70),
    "half-page-down": () => scroll(innerHeight / 2, 0, true),
    "half-page-up": () => scroll(-innerHeight / 2, 0, true),
    "page-down": () => scroll(innerHeight * 0.9, 0, true),
    "page-up": () => scroll(-innerHeight * 0.9, 0, true),
    "scroll-top": () => scroller().scrollTo({ top: 0, behavior: "smooth" }),
    "scroll-bottom": () => {
      const el = scroller();
      el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
    },
    hints: () => startHints(false),
    "hints-new-tab": () => startHints(true),
    "insert-mode": () => {
      insertMode = true;
      showBadge("INSERT · keys go to the page · Esc to leave");
    },
    "focus-input": () => {
      const input = [...document.querySelectorAll("input, textarea, [contenteditable]")].find(
        (el) => isEditable(el) && isVisible(el),
      );
      if (input) input.focus();
    },
  };

  function run(action) {
    if (local[action]) local[action]();
    else send(action);
  }

  function handleKey(key) {
    if (hints) {
      hintKey(key);
      return true;
    }
    if (insertMode) {
      if (key !== "Escape") return false;
      insertMode = false;
      hideBadge();
      return true;
    }
    if (key === "Escape") {
      pending = "";
      hideBadge();
      return false;
    }
    const sequence = pending + key;
    const action = STALES.keys[sequence];
    if (action) {
      pending = "";
      clearTimeout(pendingTimer);
      hideBadge();
      run(action);
      return true;
    }
    if (Object.keys(STALES.keys).some((k) => k.startsWith(sequence))) {
      // Part of a longer sequence like "gg": wait for the next key.
      pending = sequence;
      showBadge(sequence);
      clearTimeout(pendingTimer);
      pendingTimer = setTimeout(() => {
        pending = "";
        hideBadge();
      }, 1200);
      return true;
    }
    pending = "";
    hideBadge();
    return false;
  }

  window.addEventListener(
    "keydown",
    (e) => {
      // Ignore events made by the page's own scripts: only real key presses count.
      if (!e.isTrusted || !STALES.vim || excluded) return;
      if (e.ctrlKey || e.altKey || e.metaKey) return;
      const active = deepActive();
      if (isEditable(active) && !hints) {
        // Escape leaves a text field, so the single-key commands work again.
        if (e.key === "Escape") active.blur();
        return;
      }
      if (handleKey(e.key)) {
        e.preventDefault();
        e.stopImmediatePropagation();
      }
    },
    true,
  );

  // Only for automated tests: Stales calls this from the same isolated world.
  // The page's own scripts cannot reach it.
  window.__stalesKey = (key) => handleKey(key);
})();
