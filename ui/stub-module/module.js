// A stub web module that speaks fastkiwi-module@1 (docs/MODULE-CONTRACT.md). It stands in for
// the InstaWords web build until that build ships its embed adapter (Phase D). It is not
// InstaWords: it only shows that theme, language and digits reach a hosted module, and that
// open-own-window and update-progress work.
const PROTO = "fastkiwi-module@1";
const MODULE = "instawords";
const bridge = window.chrome?.webview;
const send = (m) => bridge?.postMessage(JSON.stringify({ proto: PROTO, ...m }));

const TXT = {
  ar: {
    k: "وحدة تجريبية تتحدث fastkiwi-module@1",
    h: "InstaWords داخل FastKiwi",
    lead: "هذه ليست InstaWords الحقيقية. نسخة الويب منه ستأتي من مجلد web في البرنامج المثبّت (المرحلة D). هنا نرى فقط أن إعدادات الهاب تصل إلى الوحدة.",
    amtL: "مبلغ للتجربة",
    own: "افتح InstaWords في نافذته",
    bad: "أرسل رسالة خاطئة (اختبار)",
    away: "حاول مغادرة الوحدة (اختبار)",
    gotH: "ما وصل من الهاب",
    tL: "المظهر", lL: "اللغة", dL: "الأرقام",
    note: "كل رسالة نص JSON واحد. لا كائنات مضيفة ولا اتصال بالإنترنت.",
    upd: { checking: "يتحقق من التحديث…", downloading: "يُنزَّل التحديث…", installing: "يُثبَّت التحديث…", done: "تم التحديث", "restart-needed": "أعد فتحه لإكمال التحديث", failed: "تعذّر التحديث" },
  },
  en: {
    k: "Stub module speaking fastkiwi-module@1",
    h: "InstaWords inside FastKiwi",
    lead: "This is not the real InstaWords. Its web version will come from the installed app's web folder (Phase D). This only shows that the Hub's settings reach a module.",
    amtL: "An amount to try",
    own: "Open InstaWords in its own window",
    bad: "Send a malformed message (test)",
    away: "Try to leave the module (test)",
    gotH: "Received from the Hub",
    tL: "Theme", lL: "Language", dL: "Digits",
    note: "Each message is one JSON text. No host objects, no internet.",
    upd: { checking: "Checking for an update…", downloading: "Downloading the update…", installing: "Installing the update…", done: "Updated", "restart-needed": "Reopen to finish the update", failed: "The update failed" },
  },
};

const S = { theme: null, lang: "ar", digits: "arab" };
const $ = (id) => document.getElementById(id);

// Same closed sets as the Hub validates (src/protocol.rs).
const FIELDS = { theme: ["theme", "tone"], lang: ["lang", "dir"], digits: ["digits"], "update-progress": ["state", "progress"] };
const VALUES = {
  theme: ["light", "dark", "midnight", "kiwi"], tone: ["light", "dark"], lang: ["ar", "en"], dir: ["rtl", "ltr"],
  digits: ["arab", "latn"], state: ["checking", "downloading", "installing", "done", "restart-needed", "failed"],
};
function valid(m) {
  if (!m || typeof m !== "object" || Array.isArray(m) || m.proto !== PROTO || !(m.type in FIELDS)) return false;
  const allowed = FIELDS[m.type];
  for (const k of Object.keys(m)) {
    if (k === "proto" || k === "type") continue;
    if (!allowed.includes(k)) return false;
    if (k === "progress") { if (typeof m[k] !== "number" || !(m[k] >= 0 && m[k] <= 1)) return false; }
    else if (!VALUES[k]?.includes(m[k])) return false;
  }
  return allowed.filter((k) => k !== "progress").every((k) => k in m);
}

function fmt(x) {
  const v = Number(String(x).replace(/[٬,\s]/g, "").replace("٫", "."));
  if (!Number.isFinite(v)) return "—";
  return new Intl.NumberFormat(`${S.lang}-u-nu-${S.digits}`, { minimumFractionDigits: 2, maximumFractionDigits: 2 }).format(v);
}

function paint() {
  const t = TXT[S.lang];
  for (const k of ["k", "h", "lead", "amtL", "own", "bad", "away", "gotH", "tL", "lL", "dL", "note"]) $(k).textContent = t[k];
  $("tV").textContent = S.theme ?? "—";
  $("lV").textContent = S.lang ? `${S.lang} (${S.lang === "ar" ? "rtl" : "ltr"})` : "—";
  $("dV").textContent = S.digits ? `${S.digits} · ${fmt(1234567.89)}` : "—";
  $("out").textContent = fmt($("amt").value);
}

bridge?.addEventListener("message", (e) => {
  let m;
  try { m = JSON.parse(e.data); } catch { return; }
  if (!valid(m)) return; // drop anything outside the contract
  const html = document.documentElement;
  if (m.type === "theme") { S.theme = m.theme; html.dataset.theme = m.theme; html.dataset.tone = m.tone; }
  if (m.type === "lang") { S.lang = m.lang; html.lang = m.lang; html.dir = m.dir; }
  if (m.type === "digits") { S.digits = m.digits; }
  if (m.type === "update-progress") {
    const b = $("upd");
    const p = typeof m.progress === "number" ? ` ${new Intl.NumberFormat(`${S.lang}-u-nu-${S.digits}`, { style: "percent" }).format(m.progress)}` : "";
    b.textContent = TXT[S.lang].upd[m.state] + p;
    b.hidden = false;
    if (m.state === "done") setTimeout(() => { b.hidden = true; }, 3000);
  }
  paint();
});

$("amt").addEventListener("input", paint);
$("own").addEventListener("click", () => send({ type: "open-own-window" }));
$("bad").addEventListener("click", () => send({ type: "theme", theme: "dark", tone: "dark" })); // wrong direction
paint();
send({ type: "ready", module: MODULE, version: "0.0.1-stub" });
