// Stales password manager, inside the page.
//
// Runs in the same hidden "world" as keys.js: the page's own scripts can't see or call
// it. It only ever knows the usernames saved for *this* site; the password arrives
// only after you pick an account, and goes straight into the field.
//
// Messages to Stales (as JSON on the "stales" channel):
//   {type: "logins-wanted", origin}             a login form is on the page
//   {type: "fill", origin, username}            you picked an account
//   {type: "submitted", origin, username, password}   you logged in
(() => {
  if (window.__stalesPasswordsLoaded || window.top !== window) return; // top page only
  window.__stalesPasswordsLoaded = true;

  const send = (msg) => window.webkit.messageHandlers.stales.postMessage(JSON.stringify({ origin: location.origin, ...msg }));
  let accounts = [];
  let asked = false;

  const visible = (el) => {
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== "hidden";
  };
  const passwordFields = () => [...document.querySelectorAll('input[type="password"]')].filter(visible);

  // The username field that belongs to a password field: marked with autocomplete,
  // or the last text/email field before it (in its form, or else in the page).
  function usernameFor(password) {
    const scope = password.form || document;
    const marked = scope.querySelector('input[autocomplete="username"], input[autocomplete="email"]');
    if (marked && visible(marked)) return marked;
    const inputs = [...scope.querySelectorAll('input[type="text"], input[type="email"], input[type="tel"], input:not([type])')];
    const before = inputs.filter((i) => visible(i) && (i.compareDocumentPosition(password) & Node.DOCUMENT_POSITION_FOLLOWING));
    return before[before.length - 1] || null;
  }

  // ---------- Is there a login form? ----------
  function check() {
    if (!asked && passwordFields().length) {
      asked = true;
      send({ type: "logins-wanted" });
    }
  }
  document.addEventListener("DOMContentLoaded", check);
  new MutationObserver(() => check()).observe(document.documentElement, { childList: true, subtree: true });

  // ---------- Suggestion under the field ----------
  let box = null;
  function hideBox() {
    if (box) box.host.remove();
    box = null;
  }
  function showBox(field, names) {
    hideBox();
    const host = document.createElement("div");
    // "closed": the page can't look inside or click our buttons.
    const root = host.attachShadow({ mode: "closed" });
    const r = field.getBoundingClientRect();
    host.style.cssText = `position:fixed;left:${r.left}px;top:${r.bottom + 4}px;z-index:2147483647`;
    root.innerHTML = `<style>
        div{background:#18181b;color:#e4e4e7;border:1px solid rgba(255,255,255,.12);border-radius:10px;
            box-shadow:0 10px 30px rgba(0,0,0,.4);padding:4px;font:13px system-ui,sans-serif;min-width:220px}
        button{display:block;width:100%;text-align:left;background:none;border:0;color:inherit;font:inherit;
               padding:7px 10px;border-radius:7px;cursor:pointer}
        button:hover,button:focus{background:rgba(255,255,255,.1);outline:none}
        small{display:block;color:#8a8a93;padding:4px 10px 2px}
      </style><div><small>Stales · saved logins (Ctrl+Shift+L)</small></div>`;
    const list = root.querySelector("div");
    for (const name of names) {
      const b = document.createElement("button");
      b.textContent = name || "(no username)";
      b.addEventListener("click", (e) => {
        if (!e.isTrusted) return; // only a real click from you
        hideBox();
        send({ type: "fill", username: name });
      });
      list.appendChild(b);
    }
    document.documentElement.appendChild(host);
    box = { host, root };
  }
  document.addEventListener("focusin", (e) => {
    const field = e.target;
    if (!accounts.length || !(field instanceof HTMLInputElement)) return;
    const pw = passwordFields();
    if (field.type === "password" || pw.some((p) => usernameFor(p) === field)) showBox(field, accounts);
  }, true);
  document.addEventListener("mousedown", (e) => { if (box && !e.composedPath().includes(box.host)) hideBox(); }, true);
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") hideBox(); }, true);

  // ---------- Called by Stales ----------
  window.__stalesLogins = (names) => {
    accounts = names;
    const focused = document.activeElement;
    if (focused instanceof HTMLInputElement) focused.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
  };
  window.__stalesChoose = (names) => {
    accounts = names;
    const pw = passwordFields()[0];
    if (pw) showBox(usernameFor(pw) || pw, names);
  };
  window.__stalesFill = (username, password) => {
    const pw = passwordFields()[0];
    if (!pw) return;
    // Set values the way typing would, so sites built with React & co notice.
    const setValue = (input, value) => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
      setter.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.dispatchEvent(new Event("change", { bubbles: true }));
    };
    const user = usernameFor(pw);
    if (user && username) setValue(user, username);
    setValue(pw, password);
    pw.focus();
  };

  // ---------- Did you just log in? ----------
  let lastSent = "";
  function captured(from) {
    const pw = passwordFields().find((p) => p.value) || [...document.querySelectorAll('input[type="password"]')].find((p) => p.value);
    if (!pw) return;
    // Sign-up / change-password forms have several password fields; skip those.
    const scope = pw.form || document;
    if (scope.querySelectorAll('input[type="password"]').length > 1 && from !== "submit") return;
    const user = usernameFor(pw);
    const key = (user ? user.value : "") + "\u0000" + pw.value;
    if (key === lastSent) return;
    lastSent = key;
    send({ type: "submitted", username: user ? user.value.trim() : "", password: pw.value });
  }
  document.addEventListener("submit", () => captured("submit"), true);
  document.addEventListener("click", (e) => {
    const button = e.target.closest && e.target.closest('button, input[type="submit"], [role="button"]');
    if (button && passwordFields().some((p) => p.value)) captured("click");
  }, true);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && e.target instanceof HTMLInputElement && (e.target.type === "password" || passwordFields().some((p) => usernameFor(p) === e.target))) {
      captured("enter");
    }
  }, true);
})();
