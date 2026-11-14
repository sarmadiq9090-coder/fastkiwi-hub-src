# `fastkiwi-module@1`: the contract between FastKiwi Hub and a hosted module

Status: **version 1, draft for Phase D**, written 2026-11-14 (HUB-DESIGN §4 and §8, Phase A's remaining half day).
Reference implementation: `src/protocol.rs` (validation), `src/main.rs` (host), `ui/stub-module/module.js` (a conforming module).
Conformance tests: `tests/protocol.rs`, `tests/allowlist.rs`.

## 1. Scope

A *module* is the web UI of one FastKiwi app shown inside the Hub window:

- **Kind S** (static web module, e.g. InstaWords web): the Hub serves the installed app's `web/` folder itself.
- **Kind P** (process module, Phase D): the app runs `<app>.exe --hub-host` and serves its own loopback page. The page-to-Hub rules below are the same; the process handshake (`{proto, port, key}` on stdout) is specified separately in Phase D.

The contract covers only the page-to-Hub channel. It gives a module **no** access to the Hub, to other modules, to the file system or to the network.

## 2. Isolation (what the Hub guarantees, and what a module must not rely on)

| Rule | How |
|---|---|
| Own browser profile | Each module has its own WebView2 user-data folder: `%LOCALAPPDATA%\FastKiwi\Hub\WebView2\module-<id>`. Storage, cookies and cache are not shared with the Hub or other modules. |
| Own origin | A kind S module is served at `https://fkmod.<id>/` by a custom protocol handler. No local HTTP server. |
| Navigation allowlist | The top-level page may only be on its own origin (and `about:blank`). Every other URL (other hosts, `http:`, ports, user info, `file:`, `data:`, `javascript:`, `blob:`, store links) is cancelled, and the Hub shows a notice. |
| No network | Every response carries a Content-Security-Policy with `default-src 'self'`, `connect-src 'self'`, `frame-src 'none'`, `worker-src 'none'`, `object-src 'none'`, `form-action 'none'`. Fetches to other origins fail; a module must work offline. |
| No host objects, no globals | `AreHostObjectsAllowed = false`. The only bridge is WebView2 web messages (`window.chrome.webview`). The Hub adds no global to the page except the marker `window.__FASTKIWI_HUB__` (HUB-APP-CONTRACT §4.6); in particular there is **no `window.ipc`**, so a module may define its own (D-H28). |
| No pop-ups, downloads or permissions | New windows, downloads and permission requests (camera, microphone, location, notifications, clipboard read, …) are refused. Clipboard **write** after a user gesture works. |
| Files | Only regular files inside the module's folder are served (no `..`, no links out of the folder). Unknown file types are served as `application/octet-stream`. |

Inline `<script>` and `eval` are blocked by the CSP. Inline `style` attributes are allowed.

## 3. Transport and envelope

- **Module → Hub:** `window.chrome.webview.postMessage(JSON.stringify(message))`. The message must be a **string** holding JSON text.
- **Hub → module:** the Hub calls `PostWebMessageAsString`; the module receives it with
  `window.chrome.webview.addEventListener("message", e => { const m = JSON.parse(e.data); … })`.
- Every message is **one JSON object** with:
  - `"proto": "fastkiwi-module@1"`, exactly;
  - `"type"`: one of the types in §4 for that direction;
  - exactly the fields that type defines (no extra fields).
- Maximum size: **16 KiB** of UTF-8 per message.
- A receiver **drops** any message that breaks a rule (bad JSON, not an object, wrong `proto`, unknown type, a type sent in the wrong direction, a missing or extra field, a value outside its set, too large). It never answers with an error message. The Hub shows a short notice in the module bar when it drops one.
- The Hub accepts messages only from the module's own origin.

## 4. Messages

### Hub → module

| `type` | Fields | Values | Meaning |
|---|---|---|---|
| `theme` | `theme`, `tone` | `theme`: `light` · `dark` · `midnight` · `kiwi`; `tone`: `light` (light, kiwi) or `dark` (dark, midnight), must match | The family theme in use. «حسب Windows» is resolved by the Hub; a module never receives `auto`. Apply it as `<html data-theme=… data-tone=…>` with the family tokens (`base.css`). |
| `lang` | `lang`, `dir` | `lang`: BCP 47 `ar` or `en`; `dir`: `rtl` for `ar`, `ltr` for `en`, must match | UI language. Set `<html lang dir>`; isolate filenames and numbers (UAX #9: `<bdi>`, `unicode-bidi: isolate`). |
| `digits` | `digits` | CLDR numbering system: `arab` (٠١٢٣٤٥٦٧٨٩) or `latn` (0123456789) | Digits for **display** (format with `Intl.NumberFormat(lang + "-u-nu-" + digits)`). Numbers inside the user's files and exports never change. |
| `update-progress` | `state`, `progress`? | `state`: `checking` · `downloading` · `installing` · `done` · `restart-needed` · `failed`; `progress`: number 0…1, **required** for `downloading` and `installing`, optional otherwise | The app this module belongs to is being updated (Store: 0–0.8 download, 0.8–1 install). Show a quiet banner and save the user's work; do not start new work during `installing`. |

### Module → Hub

| `type` | Fields | Values | Meaning |
|---|---|---|---|
| `ready` | `module`, `version`? | `module`: the module's id (must equal the hosted id, e.g. `instawords`); `version`: SemVer 2.0.0 | Sent once the page can receive messages. The Hub answers with `theme`, `lang` and `digits`. |
| `open-own-window` | — | — | Ask the Hub to open the installed app in its own window. The Hub launches the copy it detected (ShellExecute, fire and forget; no path comes from the module). Repeated requests within 2 s are ignored. |

## 5. Sequence

```
Hub                                   module page
 |-- loads https://fkmod.<id>/<entry> -->|
 |<------------- ready -------------------|   (as soon as the listener is attached)
 |-------------- theme ------------------>|
 |-------------- lang ------------------->|
 |-------------- digits ----------------->|
 |   … the user changes a shared setting …
 |-------------- theme/lang/digits ------>|   (the changed ones, or all three)
 |-------------- update-progress -------->|   (any time, zero or more)
 |<------------- open-own-window ---------|   (on a user action)
```

If `ready` does not arrive within **2.5 s** after the page loads, the Hub marks the module «لا يتحدث fastkiwi-module@1 بعد» and still shows it (without shared settings). This is how an app's existing web build is shown before it gets its Phase D adapter.

## 6. Examples

```json
{"proto":"fastkiwi-module@1","type":"ready","module":"instawords","version":"3.0.0"}
{"proto":"fastkiwi-module@1","type":"theme","theme":"midnight","tone":"dark"}
{"proto":"fastkiwi-module@1","type":"lang","lang":"ar","dir":"rtl"}
{"proto":"fastkiwi-module@1","type":"digits","digits":"arab"}
{"proto":"fastkiwi-module@1","type":"update-progress","state":"downloading","progress":0.45}
{"proto":"fastkiwi-module@1","type":"update-progress","state":"done"}
{"proto":"fastkiwi-module@1","type":"open-own-window"}
```

Refused (each is dropped): `{"type":"ready","module":"instawords"}` (no `proto`); `{"proto":"fastkiwi-module@1","type":"theme","theme":"dark","tone":"dark"}` sent **by a module** (wrong direction); `{"proto":"fastkiwi-module@1","type":"open-own-window","path":"C:\\x.exe"}` (extra field); `{"proto":"fastkiwi-module@1","type":"lang","lang":"ar","dir":"ltr"}` (dir does not match).

## 7. What a module must do (Phase D `embed.js` adapter)

1. Work **unchanged on its own**: when `window.chrome?.webview` is missing (a normal browser or the standalone app), use its own settings and never wait for the Hub.
2. Attach the `message` listener first, then send `ready`.
3. Validate every incoming message against §4 and ignore anything else.
4. Inside the Hub, hide its own brand bar (the Hub's module bar shows the name) and follow the three shared settings.
5. Never fetch from another origin, open pop-ups, or depend on downloads; the Hub refuses them.
6. Keep its own settings file: the Hub's values apply only while hosted (until spike S3, council verdict 6).

## 8. Versioning

- `proto` names the contract major version (`@1`). It follows SemVer 2.0.0 for this document: **adding** an optional field or a new message type that receivers may ignore is a minor change and keeps `@1`; **removing or changing** a field, a value set or a rule is a major change and becomes `fastkiwi-module@2`.
- Until a minor addition is listed here, the strict rules in §3 apply: a v1.0 receiver drops a message with a field it does not know. A sender therefore only sends what this document lists.
- A Hub may speak several majors; a module speaks exactly one.
