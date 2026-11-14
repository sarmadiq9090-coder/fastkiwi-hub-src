// SPDX-License-Identifier: LicenseRef-Proprietary
// SPDX-FileCopyrightText: 2026 FastKiwi
//
// fastkiwi-embed 1.0.0: the adapter a FastKiwi app's web UI loads so it can be hosted inside
// FastKiwi Hub. Contract: kiwi-hub/docs/HUB-APP-CONTRACT.md §6 and docs/MODULE-CONTRACT.md
// (fastkiwi-module@1).
//
// Copy this file byte for byte (LF line endings, no re-formatting, no minifying) and serve it at
// /__fastkiwi/embed.js. Its SHA-256 is pinned in the contract; the conformance harness checks it.
//
// The page declares itself with two meta tags and loads the adapter as a module:
//   <meta name="fastkiwi-module" content="lemon">
//   <meta name="fastkiwi-module-version" content="2.3.0">
//   <script type="module" src="/__fastkiwi/embed.js"></script>
//
// Outside the Hub (a normal browser, or the app's own window, even if that is WebView2) the
// adapter does nothing at all: the page keeps its own settings and never waits for the Hub.
// Inside the Hub it hides every [data-fk-brand] element, follows the Hub's theme, language and
// digits, and sends `ready`. It opens no connection, stores nothing and uses no eval.

export const VERSION = "1.0.0";
export const PROTO = "fastkiwi-module@1";

const MAX_BYTES = 16 * 1024;
const TONE = { light: "light", kiwi: "light", dark: "dark", midnight: "dark" };
const DIR = { ar: "rtl", en: "ltr" };
const DIGITS = ["arab", "latn"];
const STATES = ["checking", "downloading", "installing", "done", "restart-needed", "failed"];
const FIELDS = {
  theme: ["theme", "tone"],
  lang: ["lang", "dir"],
  digits: ["digits"],
  "update-progress": ["state", "progress"],
};

const meta = (name) => document.querySelector(`meta[name="${name}"]`)?.getAttribute("content") ?? "";
const MODULE = meta("fastkiwi-module");
const MODULE_VERSION = meta("fastkiwi-module-version");
const bridge = window.chrome?.webview;
// The Hub sets this marker with an initialization script in the WebViews it owns, and only
// there. An app's own WebView2 window has chrome.webview but not the marker.
const marker = window.__FASTKIWI_HUB__;
const hosted =
  !!bridge &&
  typeof marker === "object" &&
  marker !== null &&
  marker.proto === PROTO &&
  marker.module === MODULE &&
  /^[a-z0-9-]{1,32}$/.test(MODULE);

let current = null;
const listeners = { settings: new Set(), update: new Set() };

/** True only inside a FastKiwi Hub WebView that hosts this module. */
export function isHosted() {
  return hosted;
}

/** The Hub's current settings `{theme, tone, lang, dir, digits}`, or null (not hosted yet). */
export function settings() {
  return current ? { ...current } : null;
}

/**
 * Subscribe to "settings" (`{theme, tone, lang, dir, digits}`, after each change) or "update"
 * (`{state, progress?}`). Returns an unsubscribe function. Never called outside the Hub.
 */
export function on(type, callback) {
  const set = listeners[type];
  if (!set || typeof callback !== "function") throw new TypeError("on(\"settings\" | \"update\", fn)");
  set.add(callback);
  return () => set.delete(callback);
}

/** Ask the Hub to open the installed app in its own window. Does nothing outside the Hub. */
export function openOwnWindow() {
  send({ type: "open-own-window" });
}

/** Format a number for display with the Hub's digits (or the page's language outside it). */
export function formatNumber(value, options) {
  const lang = current?.lang ?? (document.documentElement.lang || "ar");
  const nu = current?.digits ? `-u-nu-${current.digits}` : "";
  return new Intl.NumberFormat(`${lang}${nu}`, options).format(value);
}

function send(m) {
  if (hosted) bridge.postMessage(JSON.stringify({ proto: PROTO, ...m }));
}

function valid(m) {
  if (typeof m !== "object" || m === null || Array.isArray(m)) return false;
  if (m.proto !== PROTO || !Object.hasOwn(FIELDS, m.type)) return false;
  const allowed = FIELDS[m.type];
  for (const k of Object.keys(m)) {
    if (k !== "proto" && k !== "type" && !allowed.includes(k)) return false;
  }
  switch (m.type) {
    case "theme":
      return Object.hasOwn(TONE, m.theme) && TONE[m.theme] === m.tone;
    case "lang":
      return Object.hasOwn(DIR, m.lang) && DIR[m.lang] === m.dir;
    case "digits":
      return DIGITS.includes(m.digits);
    case "update-progress": {
      if (!STATES.includes(m.state)) return false;
      const has = Object.hasOwn(m, "progress");
      if (has && !(typeof m.progress === "number" && m.progress >= 0 && m.progress <= 1)) return false;
      return has || (m.state !== "downloading" && m.state !== "installing");
    }
  }
  return false;
}

function hideBrand(root) {
  const hide = (el) => el.style.setProperty("display", "none", "important");
  if (root.matches?.("[data-fk-brand]")) hide(root);
  root.querySelectorAll?.("[data-fk-brand]").forEach(hide);
}

function emit(type, detail) {
  for (const cb of listeners[type]) {
    try {
      cb({ ...detail });
    } catch (e) {
      console.error(e);
    }
  }
  window.dispatchEvent(new CustomEvent(`fastkiwi:${type}`, { detail: { ...detail } }));
}

function onMessage(e) {
  if (typeof e.data !== "string" || e.data.length > MAX_BYTES) return;
  let m;
  try {
    m = JSON.parse(e.data);
  } catch {
    return;
  }
  if (!valid(m)) return; // drop anything outside the contract, silently
  const html = document.documentElement;
  if (m.type === "update-progress") {
    const u = { state: m.state };
    if (Object.hasOwn(m, "progress")) u.progress = m.progress;
    emit("update", u);
    return;
  }
  current = { theme: "light", tone: "light", lang: "ar", dir: "rtl", digits: "arab", ...current };
  if (m.type === "theme") {
    current.theme = m.theme;
    current.tone = m.tone;
    html.dataset.theme = m.theme;
    html.dataset.tone = m.tone;
  } else if (m.type === "lang") {
    current.lang = m.lang;
    current.dir = m.dir;
    html.lang = m.lang;
    html.dir = m.dir;
  } else if (m.type === "digits") {
    current.digits = m.digits;
    html.dataset.digits = m.digits;
  }
  emit("settings", current);
}

if (hosted) {
  document.documentElement.dataset.fkHosted = "";
  const start = () => {
    hideBrand(document);
    new MutationObserver((records) => {
      for (const r of records) for (const n of r.addedNodes) if (n.nodeType === 1) hideBrand(n);
    }).observe(document.documentElement, { childList: true, subtree: true });
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start, { once: true });
  else start();
  bridge.addEventListener("message", onMessage);
  const ready = { type: "ready", module: MODULE };
  if (/^\d+\.\d+\.\d+([-+][0-9A-Za-z.+-]*)?$/.test(MODULE_VERSION)) ready.version = MODULE_VERSION;
  send(ready);
}
