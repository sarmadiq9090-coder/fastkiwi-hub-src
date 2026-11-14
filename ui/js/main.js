// FastKiwi Hub UI (Phase C prototype). Vanilla ES module, served over the fastkiwi custom
// protocol. Talks to FastKiwi.exe only through window.ipc.postMessage (WebView2 web messages).
// Everything shown comes from the fixture catalog and the read-only detection; nothing is fetched.
import { icon as I } from "./icons.js";
import { STR } from "./i18n.js";

const S = {
  st: null,          // state from the exe
  view: "home",      // home | app | updates | settings
  open: [],          // app ids with an open tab
  cur: null,         // app id shown in the app view
  mod: {},           // app id -> {status, source, version}
  dlg: null,         // {kind: "add"|"ask"|"priv", app}
  scanning: true,
  scannedAt: null,
  demo: null,        // update demo progress 0..1
  note: null,        // notice shown in the module bar
  shotSent: false,
};

const ipc = (o) => { try { window.ipc.postMessage(JSON.stringify(o)); } catch { /* not in the Hub */ } };
const $ = (sel) => document.querySelector(sel);
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

// ---------- settings: language, theme, digits (BCP 47, CLDR numbering systems) ----------
const darkMq = matchMedia("(prefers-color-scheme: dark)");
function resolved() {
  const st = S.st?.settings ?? { theme: "auto", lang: "auto", digits: "auto" };
  const lang = st.lang === "auto" ? ((navigator.language || "ar").toLowerCase().startsWith("ar") ? "ar" : "en") : st.lang;
  const theme = st.theme === "auto" ? (darkMq.matches ? "dark" : "light") : st.theme;
  const digits = st.digits === "auto" ? (lang === "ar" ? "arab" : "latn") : st.digits;
  return { lang, theme, digits };
}
const T = () => STR[resolved().lang];
function nf(opts) {
  const r = resolved();
  return new Intl.NumberFormat(`${r.lang}-u-nu-${r.digits}`, opts);
}
const n = (x, o) => nf(o).format(x);
const pct = (x) => nf({ style: "percent", maximumFractionDigits: 0 }).format(x);
const mb = (x) => T().mb(n(x, { maximumFractionDigits: x < 10 ? 1 : 0 }));
const bytesMb = (b) => mb(b / (1024 * 1024));
const time = (d) => new Intl.DateTimeFormat(`${resolved().lang}-u-nu-${resolved().digits}`, { hour: "numeric", minute: "2-digit" }).format(d);
const ver = (v) => `<span class="num">${esc(v)}</span>`;

function applySettings(send = true) {
  const r = resolved();
  const html = document.documentElement;
  html.lang = r.lang;
  html.dir = r.lang === "ar" ? "rtl" : "ltr";
  html.dataset.theme = r.theme;
  html.dataset.tone = r.theme === "dark" || r.theme === "midnight" ? "dark" : "light";
  if (send) ipc({ cmd: "settings", settings: S.st.settings, resolved: r });
}
darkMq.addEventListener("change", () => { if (S.st) { applySettings(); render(); } });
function setSetting(k, v) {
  S.st.settings = { ...S.st.settings, [k]: v };
  applySettings();
  render();
}

// ---------- data helpers ----------
const apps = () => S.st?.apps ?? [];
const app = (id) => apps().find((a) => a.id === id);
const det = (id) => (S.st?.detection ?? []).find((d) => d.id === id);
const installs = (id) => det(id)?.installs ?? [];
const isInstalled = (id) => installs(id).length > 0;
const nameOf = (a) => esc(a?.name?.[resolved().lang] ?? a?.id ?? "");
const iconOf = (a) => esc(a?.icon ?? "icons/fastkiwi.svg");
const canHost = (id) => !!det(id)?.module;

// ---------- chrome ----------
function hubbar() {
  const t = T();
  const tab = (v, ic, label, extra = "") => `<a href="#" data-go="${v}" class="${S.view === v ? "on" : ""}" aria-current="${S.view === v ? "page" : "false"}">${I(ic, 20)} ${label}${extra}</a>`;
  const updBadge = `<span class="pin" style="background:var(--accent);color:var(--on-accent)" aria-label="${n(1)}">${n(1)}</span>`;
  const openTabs = S.open.map((id) => {
    const a = app(id); const on = S.view === "app" && S.cur === id;
    return `<span class="tabw${on ? " on" : ""}"><a href="#" data-tab="${esc(id)}" aria-current="${on ? "page" : "false"}"><img src="${iconOf(a)}" alt=""><bdi>${nameOf(a)}</bdi></a><button class="x" data-close="${esc(id)}" aria-label="${esc(t.close(a?.name?.[resolved().lang] ?? id))}" title="${esc(t.close(a?.name?.[resolved().lang] ?? id))}">${I("dismiss", 12)}</button></span>`;
  }).join("");
  return `<header class="appbar">
    <span class="brand"><img src="icons/fastkiwi.svg" alt=""><span>FastKiwi</span></span>
    <nav class="tabs" aria-label="${esc(t.hub)}">${tab("home", "home", t.home)}${tab("updates", "arrow_sync", t.updates, " " + updBadge)}</nav>
    ${S.open.length ? `<span class="vsep" aria-hidden="true"></span><nav class="apptabs" aria-label="${esc(t.openApps)}">${openTabs}</nav>` : ""}
    <span class="grow"></span>
    ${S.st?.dev ? `<span class="chip warn">${I("warning_f")} ${t.devChip}</span>` : ""}
    <button class="pchip" data-dlg="priv">${I("lock_closed")} ${t.onDevice}</button>
    <button class="ibtn" data-dlg="ask" aria-label="${esc(t.ask)}" title="${esc(t.ask)}">${I("question_circle", 20)}</button>
    <button class="ibtn${S.view === "settings" ? " on" : ""}" data-go="settings" aria-label="${esc(t.settings)}" title="${esc(t.settings)}" aria-pressed="${S.view === "settings"}">${I("settings", 20)}</button>
  </header>`;
}

// ---------- home ----------
const tag = (ic, s) => `<span class="tag">${I(ic)}${s}</span>`;
function opensTag(a) {
  const t = T();
  if (canHost(a.id)) return tag("apps", a.kind === "S" ? t.opensHereWeb : t.opensHere);
  if (a.kind === "P") return tag("desktop", t.hostLater);
  return tag("desktop", t.opensWindow);
}
function installedCard(a) {
  const t = T();
  const ins = installs(a.id);
  const main = ins[0];
  const host = canHost(a.id);
  const state = main.launchable
    ? `<span class="state ok">${I("checkmark_circle_f")} ${t.installed}</span>`
    : `<span class="state na" style="color:var(--warn)">${I("warning_f")} ${t.broken}</span>`;
  const meta = [opensTag(a), tag(main.source === "store" ? "shield_checkmark" : "folder", t.src[main.source])];
  if (main.size_bytes) meta.push(tag("arrow_download", bytesMb(main.size_bytes)));
  const acts = host
    ? `${main.launchable ? `<button class="btn" data-launch="${esc(a.id)}" data-i="0">${I("desktop")} ${t.ownWindow}</button>` : ""}<button class="btn primary" data-host="${esc(a.id)}">${t.open}</button>`
    : `<button class="btn${main.launchable ? " primary" : ""}" data-launch="${esc(a.id)}" data-i="0" ${main.launchable ? "" : "disabled"}>${t.open}</button>`;
  const others = ins.slice(1).map((x, i) => `<div class="copy"><span>${t.otherCopy}: ${t.srcShort[x.source]}${x.version ? " " + ver(x.version) : ""}</span><span class="grow"></span><button class="btn subtle" data-launch="${esc(a.id)}" data-i="${i + 1}" ${x.launchable ? "" : "disabled"}>${t.open}</button></div>`).join("");
  return `<article class="app" aria-label="${nameOf(a)}">
    <div class="top"><img src="${iconOf(a)}" alt=""><div class="t"><h3><bdi>${nameOf(a)}</bdi>${main.version ? `<small>${esc(main.version)}</small>` : ""}${a.status !== "released" ? `<span class="chip warn" style="height:20px;font-size:11px">${t.notReleased}</span>` : ""}</h3><p>${esc(a.tagline[resolved().lang])}</p></div></div>
    <div class="meta">${meta.join("")}</div>
    ${others}
    <div class="acts">${state}<span class="grow"></span>${acts}</div>
  </article>`;
}
function addCard(a) {
  const t = T();
  const meta = [tag("apps", a.kind === "L" ? t.opensWindow : t.hereOrAlone)];
  if (a.on_demand?.length) meta.push(tag("cloud", t.modelsLater));
  if (a.needs) meta.push(tag("info", esc(a.needs[resolved().lang])));
  if (det(a.id)?.used_before) meta.push(tag("history", t.usedBefore));
  const dev = canHost(a.id) ? `<button class="btn" data-host="${esc(a.id)}">${I("apps")} ${t.open}</button>` : "";
  return `<article class="app add" aria-label="${nameOf(a)}">
    <div class="top"><img src="${iconOf(a)}" alt=""><div class="t"><h3><bdi>${nameOf(a)}</bdi>${a.status !== "released" ? `<span class="chip warn" style="height:20px;font-size:11px">${t.notReleased}</span>` : ""}</h3><p>${esc(a.tagline[resolved().lang])}</p></div></div>
    <div class="meta">${meta.join("")}</div>
    <div class="acts"><span class="state na">${I("arrow_download")} ${mb(a.size_mb)}</span><span class="grow"></span>${dev}<button class="btn" data-add="${esc(a.id)}">${t.details}</button><button class="btn" data-add="${esc(a.id)}">${I("arrow_download")} ${t.add}</button></div>
  </article>`;
}
function homeView() {
  const t = T();
  const hidden = new Set(S.st.settings.hidden ?? []);
  const shown = apps().filter((a) => !hidden.has(a.id));
  const mine = shown.filter((a) => isInstalled(a.id));
  const rest = shown.filter((a) => !isInstalled(a.id));
  const hour = new Date().getHours();
  const scanLine = S.scanning ? `<span>${I("arrow_sync")} ${t.scanning}</span>` : `<span>${I("shield_checkmark")} ${t.footScan(time(S.scannedAt ?? new Date()))}</span>`;
  return `<div class="scroll"><div class="wrap hub">
    <div class="hello"><div><h1>${hour < 12 ? t.morning : t.evening}</h1><p>${t.lead}</p></div><span class="grow"></span><span class="thin">${I("shield_checkmark")} ${t.noAccount}</span></div>
    <section aria-labelledby="h-mine">
      <div class="sec"><h2 id="h-mine">${t.mine}</h2><span class="faint">${S.scanning ? t.scanning : t.mineCount(n(mine.length))}</span></div>
      ${mine.length ? `<div class="grid">${mine.map(installedCard).join("")}</div>` : `<p class="muted" style="margin:0">${S.scanning ? "" : t.mineNone}</p>`}
    </section>
    <section aria-labelledby="h-add">
      <div class="sec"><h2 id="h-add">${t.addTo}</h2><span class="faint">${t.addHint}</span></div>
      <div class="grid">${rest.map(addCard).join("")}
        <article class="app add" style="justify-content:center;gap:8px"><h3>${I("info", 20)} ${t.honestT}</h3><p class="muted" style="margin:0;font-size:13px;line-height:20px">${t.honestP(mb(6))}</p></article>
      </div>
    </section>
    <div class="foot"><span>${I("lock_closed")} ${t.foot1}</span>${scanLine}<button class="btn subtle" data-rescan style="min-height:32px;height:32px;font-size:12.5px">${I("arrow_sync")} ${t.rescan}</button><span class="grow"></span><span>${t.about(ver(S.st.version))}</span></div>
  </div></div>`;
}

// ---------- app page (module host) ----------
function appView() {
  const t = T();
  const a = app(S.cur);
  const m = S.mod[S.cur] ?? { status: "loading" };
  const statusChip = {
    loading: `<span class="chip">${I("arrow_sync")} ${t.modLoading}</span>`,
    ready: `<span class="chip ok">${I("checkmark_circle_f")} ${t.modReady}</span>`,
    "no-proto": `<span class="chip warn">${I("warning_f")} ${t.modNoProto}</span>`,
    unavailable: `<span class="chip warn">${I("warning_f")} ${t.modUnavailable}</span>`,
    error: `<span class="chip bad">${I("warning_f")} ${t.modError}</span>`,
  }[m.status] ?? "";
  const srcChip = m.source ? `<span class="chip">${{ installed: t.srcInstalled, "dev-dir": t.srcDev, stub: t.srcStub }[m.source]}</span>` : "";
  const launchable = installs(S.cur).some((i) => i.launchable);
  const firstI = installs(S.cur).findIndex((i) => i.launchable);
  return `<div class="modbar">
      <span class="mname"><img src="${iconOf(a)}" alt=""><bdi>${nameOf(a)}</bdi></span>
      ${statusChip}${srcChip}
      <span class="grow"></span>
      ${S.note ? `<span class="chip ${S.note.bad ? "bad" : "ok"}" role="status">${I(S.note.bad ? "warning_f" : "checkmark_circle_f")} ${esc(S.note.msg)}</span>` : `<span class="hosted">${I("lock_closed")} ${t.hosted}</span>`}
      <button class="btn" data-launch="${esc(S.cur)}" data-i="${firstI}" ${launchable ? "" : `disabled title="${esc(t.noCopy)}"`}>${I("desktop")} ${t.openOwnWindow}</button>
    </div>
    <main class="modview modhost" id="modhost" aria-label="${nameOf(a)}"><div class="modwait">${m.status === "unavailable" || m.status === "error" ? (m.status === "error" ? t.modError : t.modUnavailable) : t.modLoading}</div></main>`;
}

// ---------- updates (fixture, UI only) ----------
function updatesView() {
  const t = T();
  const fx = S.st.updates ?? { rows: [] };
  const COLS = "grid-template-columns:minmax(0,1fr) 150px 170px 300px;height:64px";
  const rowsHtml = fx.rows.map((r) => {
    const a = r.app === "hub" ? { name: { ar: t.hubSelf, en: t.hubSelf }, icon: "icons/fastkiwi.svg" } : app(r.app);
    if (!a) return "";
    const v = r.to && r.state !== "done" && r.state !== "latest" ? `${ver(r.from)} → <b style="color:var(--accent)">${ver(r.to)}</b>` : `<b>${ver(r.to ?? r.from)}</b>`;
    const src = r.route === "store-here" ? `<span class="chip accent">${I("shield_checkmark")} ${t.storeHere}</span>` : `<span class="chip">${I("shield_checkmark")} ${t.storeOnly}</span>`;
    let st = "";
    const p = r.state === "updating" ? (S.demo ?? r.progress) : null;
    if (r.state === "updating" && p < 1) {
      st = `<div class="prog"><div class="pl"><b>${t.updating(pct(p))}</b><span class="faint">${t.ofMb(mb(Math.round(r.total_mb * p * 10) / 10), mb(r.total_mb))}</span></div><div class="bar" role="progressbar" aria-valuenow="${Math.round(p * 100)}" aria-valuemin="0" aria-valuemax="100" aria-label="${nameOf(a)}"><i style="width:${p * 100}%"></i></div></div>`;
    } else if (r.state === "done" || r.state === "updating") {
      st = `<div style="display:flex;align-items:center;gap:8px"><span class="state ok">${I("checkmark_circle_f")} ${t.updated}</span><span class="grow"></span><button class="btn" style="min-height:32px;height:32px" disabled>${t.reopen}</button></div>`;
    } else if (r.state === "redirect") {
      st = `<div style="display:flex;align-items:center;gap:8px"><span class="state na">${t.redirect}</span><span class="grow"></span><button class="btn" style="min-height:32px;height:32px" disabled>${I(resolved().lang === "ar" ? "arrow_export_rtl" : "arrow_export_ltr")} ${t.updInStore}</button></div>`;
    } else {
      st = `<span class="state ok">${I("checkmark_circle_f")} ${t.latest}</span>`;
    }
    return `<div class="r" role="row" style="${COLS}"><span class="fn"><img src="${iconOf(a)}" alt="" style="width:28px;height:28px;border-radius:6px"><b style="font-weight:600"><bdi>${nameOf(a)}</bdi></b></span><span>${v}</span><span>${src}</span><span style="justify-self:stretch">${st}</span></div>`;
  }).join("");
  const [hh, mm] = (fx.last_check ?? "09:00").split(":").map(Number);
  const lc = new Date(); lc.setHours(hh, mm, 0, 0);
  return `<div class="scroll"><div class="wrap hub" style="max-width:1060px;gap:14px">
    <div class="hello"><div><h1>${t.updates}</h1><p>${t.updLead}</p></div><span class="grow"></span>
      <span class="faint" style="font-size:12.5px">${t.lastCheck(time(lc))}</span><button class="btn" disabled title="${esc(t.protoNoInstall)}">${I("arrow_sync")} ${t.checkNow}</button></div>
    <div class="note">${I("info")}<span>${t.fixtureNote}</span><span class="grow"></span><button class="btn subtle" data-demo style="min-height:28px;height:28px;font-size:12.5px">${t.playDemo}</button></div>
    <div class="rows" role="table">
      <div class="r h" role="row" style="${COLS.replace("64px", "32px")}"><span role="columnheader">${t.colApp}</span><span role="columnheader">${t.colVer}</span><span role="columnheader">${t.colSrc}</span><span role="columnheader">${t.colState}</span></div>
      ${rowsHtml}
    </div>
    <div class="how">
      <div><b>${I("shield_checkmark")} ${t.howStore}</b><span>${t.howStoreS}</span></div>
      <div><b>${I("arrow_download")} ${t.howDirect}</b><span>${t.howDirectS}</span></div>
      <div><b>${I(resolved().lang === "ar" ? "arrow_export_rtl" : "arrow_export_ltr")} ${t.howRedirect}</b><span>${t.howRedirectS}</span></div>
    </div>
  </div></div>`;
}

// ---------- settings ----------
function seg(name, opts, cur) {
  return `<div class="seg" role="radiogroup" aria-label="${esc(name)}">${opts.map(([v, l]) => `<button type="button" class="segb${v === cur ? " on" : ""}" role="radio" aria-checked="${v === cur}" data-set="${name}" data-v="${v}">${l}</button>`).join("")}</div>`;
}
function settingsView() {
  const t = T();
  const st = S.st.settings;
  const themes = ["auto", "light", "dark", "midnight", "kiwi"];
  const hidden = new Set(st.hidden ?? []);
  const modrow = (a) => {
    const ins = installs(a.id)[0];
    const where = ins ? `${canHost(a.id) ? (a.kind === "S" ? t.opensHereWeb : t.opensHere) : t.opensWindow} · ${ins.size_bytes ? bytesMb(ins.size_bytes) : "—"} · ${t.srcShort[ins.source]}` : t.notInstalledHere;
    const show = !hidden.has(a.id);
    return `<div class="it modrow"><img src="${iconOf(a)}" alt=""><div class="lab"><b><bdi>${nameOf(a)}</bdi></b><small>${where}</small></div>
      <span class="muted" style="font-size:12.5px">${t.showHome}</span><button type="button" class="toggle${show ? "" : " off"}" role="switch" aria-checked="${show}" aria-label="${esc(t.showOn(a.name[resolved().lang]))}" data-hide="${esc(a.id)}"></button>
      <button class="btn" disabled title="${esc(t.removeLater)}">${I("delete")} ${t.remove}</button></div>`;
  };
  return `<div class="scroll"><div class="wrap hub" style="gap:12px">
    <div><h1 style="margin:0;font-size:24px;line-height:32px">${t.settings}</h1><p class="muted" style="margin:2px 0 0;font-size:13px">${t.setLead}</p></div>
    <div class="setgrid">
      <div style="display:flex;flex-direction:column;gap:12px">
        <div class="box">
          <div class="it">${I("globe", 20)}<div class="lab"><b id="l-lang">${t.uiLang}</b></div>${seg("lang", [["auto", t.byWindows], ["ar", "العربية"], ["en", "English"]], st.lang)}</div>
          <div class="it col">${I("desktop", 20)}<div class="lab"><b>${t.look}</b><small>${t.lookS}</small></div>
            <div class="themes" role="radiogroup" aria-label="${esc(t.look)}">${themes.map((k) => `<button type="button" class="thm" role="radio" aria-checked="${k === st.theme}" data-set="theme" data-v="${k}"><span class="swatch" data-swatch="${k}"><i></i><i></i></span>${t.themes[k]}</button>`).join("")}</div></div>
          <div class="it">${I("number_symbol", 20)}<div class="lab"><b>${t.digits}</b><small>${t.example}: <span>${n(1250.75, { minimumFractionDigits: 2 })}</span>. ${t.digitsS}</small></div>${seg("digits", [["auto", t.byLang], ["arab", "٠١٢٣"], ["latn", "0123"]], st.digits)}</div>
        </div>
        <div class="box">
          <div class="it">${I("arrow_sync", 20)}<div class="lab"><b>${t.follow}</b><small>${t.followS}</small></div><button type="button" class="toggle${st.follow ? "" : " off"}" role="switch" aria-checked="${st.follow}" aria-label="${esc(t.follow)}" data-follow></button></div>
        </div>
      </div>
      <div style="display:flex;flex-direction:column;gap:12px">
        <div class="box">
          <div class="it" style="min-height:44px"><div class="lab"><b>${t.myApps}</b><small>${t.myAppsS}</small></div></div>
          ${apps().map(modrow).join("")}
        </div>
        <div class="box">
          <div class="it">${I("shield_checkmark", 20)}<div class="lab"><b>${t.checkUpd}</b><small>${t.checkUpdS}</small></div><button type="button" class="toggle off" role="switch" aria-checked="false" aria-label="${esc(t.checkUpd)}" disabled></button></div>
        </div>
      </div>
    </div>
  </div></div>`;
}

// ---------- dialogs ----------
function addDialog(a) {
  const t = T();
  const od = a.on_demand ?? [];
  const nm = a.name[resolved().lang];
  const store = a.store
    ? `<label class="on"><span class="radio on"></span><div>${t.storeName} <span class="chip accent" style="height:20px;font-size:11px">${t.recommended}</span><small>${t.storeS}${a.store.live ? "" : " " + t.storeNotLive}</small></div><span class="chip ok">${t.free}</span></label>`
    : `<label class="dis"><span class="radio"></span><div>${t.storeName}<small>${t.noStore}</small></div><span class="tag">${t.later}</span></label>`;
  return `<div class="smoke" data-smoke><div class="dlg" role="dialog" aria-modal="true" aria-labelledby="dlg-t">
    <div class="hd"><img src="${iconOf(a)}" alt=""><div><h2 id="dlg-t">${esc(t.addT(nm))}</h2><p>${esc(a.tagline[resolved().lang])} · ${t.free}</p></div></div>
    <div class="bd">
      <div class="lbl">${t.willDownload}</div>
      <div class="pkg" role="table">
        <div class="row" role="row">${I("checkmark_circle_f")}<div>${esc(t.itself(nm))}<small>${t.itselfS}</small></div><span class="sz">${mb(a.size_mb)}</span></div>
        ${od.map((o) => `<div class="row" role="row">${I("cloud")}<div>${esc(o.what[resolved().lang])}<small>${t.laterSmall}</small></div><span class="sz muted">${mb(o.size_mb)} ${t.later}</span></div>`).join("")}
        <div class="row not" role="row">${I("dismiss")}<div>${t.nothingElse}<small>${t.nothingElseS}</small></div><span class="sz">${n(0)}</span></div>
        <div class="row total" role="row"><span></span><div>${t.nowTotal}</div><span class="sz">≈ ${mb(a.size_mb)}</span></div>
      </div>
      <div class="lbl">${t.from}</div>
      <div class="src" role="radiogroup" aria-label="${esc(t.from)}">
        ${store}
        <label class="dis"><span class="radio"></span><div>${t.direct}<small>${t.directS}</small></div><span class="tag">${t.later}</span></label>
      </div>
      <div class="note">${I("info")}<span>${esc(t.oneCopy(nm))}</span></div>
      <div class="note warnnote" id="noinstall" hidden>${I("warning_f")}<span>${t.protoNoInstall}</span></div>
    </div>
    <div class="ft"><button class="btn primary" data-fakeadd ${a.store ? "" : "disabled"}>${I("arrow_download")} ${esc(t.addVia(mb(a.size_mb)))}</button><button class="btn" data-dlgclose>${t.cancel}</button><span class="grow"></span><span class="faint">${t.removeAnytime}</span></div>
  </div></div>`;
}
function infoDialog(title, body, ic) {
  const t = T();
  return `<div class="smoke" data-smoke><div class="dlg small" role="dialog" aria-modal="true" aria-labelledby="dlg-t">
    <div class="hd"><span class="dlgic">${I(ic, 24)}</span><div><h2 id="dlg-t">${title}</h2></div></div>
    <div class="bd"><p style="margin:0;line-height:22px">${body}</p></div>
    <div class="ft"><span class="grow"></span><button class="btn primary" data-dlgclose>${t.ok}</button></div>
  </div></div>`;
}
function dialogHtml() {
  const t = T();
  if (!S.dlg) return "";
  if (S.dlg.kind === "add") return addDialog(app(S.dlg.app));
  if (S.dlg.kind === "ask") return infoDialog(t.askT, t.askP, "person_feedback");
  return infoDialog(t.privT, t.privP, "lock_closed");
}

// ---------- render ----------
let lastFocus = null;
function render() {
  if (!S.st) return;
  const views = { home: homeView, app: appView, updates: updatesView, settings: settingsView };
  const body = (views[S.view] ?? homeView)();
  $("#app").innerHTML = `${hubbar()}<div class="surface" id="surface">${body}</div>${dialogHtml()}`;
  syncModule();
  if (S.dlg) {
    const d = $(".dlg");
    (d.querySelector("[data-dlgclose]") ?? d).focus();
  } else if (lastFocus) {
    const el = document.querySelector(lastFocus);
    if (el) el.focus();
    lastFocus = null;
  }
}

// Keep the module's child WebView over #modhost, visible only on the app page with no dialog.
let ro = null;
function syncModule() {
  const host = $("#modhost");
  const visible = S.view === "app" && !S.dlg && !!host && S.mod[S.cur]?.status !== "unavailable";
  if (ro) { ro.disconnect(); ro = null; }
  if (host) {
    const send = () => {
      const r = host.getBoundingClientRect();
      ipc({ cmd: "module-bounds", x: r.left, y: r.top, w: r.width, h: r.height });
    };
    send();
    ro = new ResizeObserver(send);
    ro.observe(host);
  }
  ipc({ cmd: "module-visible", app: S.cur, visible });
}

function go(view) {
  S.view = view;
  S.dlg = null;
  render();
}
function openHosted(id) {
  if (!S.open.includes(id)) S.open.push(id);
  S.cur = id;
  S.view = "app";
  S.dlg = null;
  if (!S.mod[id]) S.mod[id] = { status: "loading" };
  render();
  ipc({ cmd: "open-module", app: id });
}
function closeTab(id) {
  S.open = S.open.filter((x) => x !== id);
  delete S.mod[id];
  ipc({ cmd: "close-module", app: id });
  if (S.cur === id) { S.cur = S.open.at(-1) ?? null; S.view = S.cur ? "app" : "home"; }
  render();
  if (S.cur) ipc({ cmd: "open-module", app: S.cur });
}

let toastT = 0;
function toast(msg, bad = false) {
  // On the app page the module's WebView covers the window below the bars, so the notice
  // goes into the module bar instead.
  if (S.view === "app") {
    S.note = { msg, bad };
    render();
    clearTimeout(toastT);
    toastT = setTimeout(() => { S.note = null; if (S.view === "app") render(); }, 4500);
    return;
  }
  let el = $("#toast");
  if (!el) { el = document.createElement("div"); el.id = "toast"; el.setAttribute("role", "status"); document.body.append(el); }
  el.className = `toast${bad ? " bad" : ""}`;
  el.textContent = msg;
  el.hidden = false;
  clearTimeout(toastT);
  toastT = setTimeout(() => { el.hidden = true; }, 3500);
}

// ---------- events ----------
document.addEventListener("click", (e) => {
  const el = e.target.closest("[data-go],[data-tab],[data-close],[data-dlg],[data-launch],[data-host],[data-add],[data-dlgclose],[data-smoke],[data-set],[data-hide],[data-follow],[data-rescan],[data-demo],[data-fakeadd]");
  if (!el) return;
  const d = el.dataset;
  if (d.smoke !== undefined && e.target !== el) return; // clicks inside the dialog
  e.preventDefault();
  if (d.close) { e.stopPropagation(); closeTab(d.close); return; }
  if (d.go) return go(d.go);
  if (d.tab) { S.cur = d.tab; S.view = "app"; render(); ipc({ cmd: "open-module", app: d.tab }); return; }
  if (d.dlg) { lastFocus = `[data-dlg="${d.dlg}"]`; S.dlg = { kind: d.dlg }; return render(); }
  if (d.launch) { ipc({ cmd: "launch", app: d.launch, install: Number(d.i) || 0 }); return; }
  if (d.host) return openHosted(d.host);
  if (d.add) { lastFocus = `[data-add="${d.add}"]`; S.dlg = { kind: "add", app: d.add }; return render(); }
  if (d.dlgclose !== undefined || d.smoke !== undefined) { S.dlg = null; return render(); }
  if (d.fakeadd !== undefined) { const w = $("#noinstall"); if (w) w.hidden = false; return; }
  if (d.set) { lastFocus = `[data-set="${d.set}"][data-v="${d.v}"]`; return setSetting(d.set, d.v); }
  if (d.hide) {
    const h = new Set(S.st.settings.hidden ?? []);
    h.has(d.hide) ? h.delete(d.hide) : h.add(d.hide);
    lastFocus = `[data-hide="${d.hide}"]`;
    return setSetting("hidden", [...h].sort());
  }
  if (d.follow !== undefined) { lastFocus = "[data-follow]"; return setSetting("follow", !S.st.settings.follow); }
  if (d.rescan !== undefined) { S.scanning = true; render(); ipc({ cmd: "rescan" }); return; }
  if (d.demo !== undefined) return runDemo();
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && S.dlg) { S.dlg = null; render(); return; }
  if (e.key === "Tab" && S.dlg) { // keep focus inside the dialog
    const f = [...document.querySelectorAll(".dlg button:not([disabled]), .dlg [href]")];
    if (!f.length) return;
    const first = f[0], last = f.at(-1);
    if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
  }
});

// The update demo animates the fixture row and sends update-progress to a hosted InstaWords.
function runDemo() {
  let p = 0;
  const step = () => {
    p = Math.min(1, p + 0.05);
    S.demo = p;
    ipc({ cmd: "update-demo", app: "instawords", state: p < 0.8 ? "downloading" : p < 1 ? "installing" : "done", ...(p < 1 ? { progress: p } : {}) });
    if (S.view === "updates") render();
    if (p < 1) setTimeout(step, 150); else setTimeout(() => { S.demo = null; }, 4000);
  };
  step();
}

// ---------- messages from FastKiwi.exe ----------
window.__hub = {
  recv(m) {
    const t = T();
    switch (m.type) {
      case "state":
        S.st = m;
        if (m.shot) { S.st.settings = { ...S.st.settings, theme: m.shot.theme, lang: m.shot.lang }; }
        S.scanning = m.detection === null;
        if (!S.scanning) S.scannedAt = new Date();
        applySettings();
        render();
        if (m.shot) startShot(m.shot);
        break;
      case "scanning": S.scanning = true; render(); break;
      case "detection":
        S.st.detection = m.detection;
        S.scanning = false;
        S.scannedAt = new Date();
        render();
        break;
      case "module":
        S.mod[m.app] = { ...(S.mod[m.app] ?? {}), status: m.status, source: m.source ?? S.mod[m.app]?.source, version: m.module_version };
        if (S.view === "app" && S.cur === m.app) render();
        break;
      case "launched": {
        const a = app(m.app); const nm = a?.name?.[resolved().lang] ?? m.app;
        if (m.ok) toast(t.launched(nm));
        else if (m.error !== "busy") toast(m.error === "not installed" ? t.noCopy : t.launchFail(nm), true);
        break;
      }
      case "blocked": toast(t.blocked(m.host), true); break;
      case "proto-error": toast(t.protoErr, true); break;
    }
  },
};

// ---------- dev screenshots (--dev-shot): go to a screen, wait until it is settled ----------
async function startShot(shot) {
  const settled = () => new Promise((r) => setTimeout(r, 300));
  const until = async (f, ms = 8000) => { const t0 = Date.now(); while (!f() && Date.now() - t0 < ms) await new Promise((r) => setTimeout(r, 100)); };
  await until(() => S.st.detection !== null);
  await document.fonts.ready;
  if (shot.screen.startsWith("app")) {
    const id = apps().find((a) => canHost(a.id))?.id;
    if (id) { openHosted(id); await until(() => ["ready", "no-proto", "unavailable", "error"].includes(S.mod[id]?.status), 10000); }
    const test = shot.screen.split("-")[1]; // app-away | app-bad | app-own: dev self-tests
    if (id && test) { ipc({ cmd: "dev-click", app: id, el: test }); await new Promise((r) => setTimeout(r, 1500)); }
  } else if (shot.screen === "launch") {
    const b = document.querySelector("[data-launch]:not([disabled])");
    if (b) { b.click(); await new Promise((r) => setTimeout(r, 1500)); }
  } else if (shot.screen === "add") {
    const a = apps().find((x) => !isInstalled(x.id) && x.id === "instawords") ?? apps().find((x) => !isInstalled(x.id));
    if (a) { S.dlg = { kind: "add", app: a.id }; render(); }
  } else if (shot.screen !== "home") {
    go(shot.screen);
  }
  await settled();
  ipc({ cmd: "shot-ready" });
}

ipc({ cmd: "hello" });
