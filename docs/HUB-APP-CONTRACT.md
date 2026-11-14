# `fastkiwi-hub-cli@1`: what each FastKiwi app implements for FastKiwi Hub

Status: **version 1.0, Phase D**, written 2026-11-14. Design source: HUB-DESIGN §2, §4, §5 and §8. Decisions: `DECISIONS.md` D-H15 to D-H22.

This document covers the app's side: three command-line modes, a capability file and the shared web adapter. The page-to-Hub messages are a separate contract, `docs/MODULE-CONTRACT.md` (`fastkiwi-module@1`), which this one builds on.

| Part | Where |
|---|---|
| Schema and validators (Rust; parsing only, no I/O) | `crates/hubcli` (`fastkiwi-hubcli`) |
| Web adapter, pinned by SHA-256 | `sdk/embed.js`, `sdk/embed.js.sha256` |
| Conformance harness | `tools/conformance`: `check-app` |
| A conforming app to read and copy from | `tools/conformance/src/bin/stub_app.rs` (`stub-app`) |

## 1. Who implements what

| App | Kind | `--hub-host` | `--hub-update` | `--export-data` | `embed.js` | Phase D order |
|---|---|---|---|---|---|---|
| InstaWords | S (web) / L (WPF) | no: the Hub serves `web\` itself | yes | yes | yes, in `web\` | 1st |
| Lemon | P | yes | yes | yes | yes | 2nd |
| InstaFind | P (new UI) | yes, after new UI 1.0 | yes | yes | yes | 3rd |
| Remember Me | L | no | yes (small helper) | yes | no | any time |
| InstaConvert | P | **not in Phase D**: only after trial #3 and the T-gates (D-069, D-073) | | | | — |

An app is done when `check-app <its exe>` prints `RESULT conforms` (§9). A check for a mode the app does not declare is skipped.

## 2. Rules for every mode

1. **The capability file decides.** The Hub runs a hub command only if the app's `fastkiwi-hub.json` (§3) declares it. An older build has no such file, so the Hub opens it in its own window (ShellExecute) and never passes it a flag it does not know.
2. **Command line.** The Hub starts `<the detected exe> <flag> [arguments]`: the same exe the user starts, at the path or Store alias that detection found. Arguments are exactly as listed in this document. For anything else (an unknown flag, a missing or extra argument), the app exits with code **2**, prints nothing and changes nothing.
3. **Pipes.**
   - stdin is a pipe from the Hub.
   - stdout is a pipe to the Hub and carries only the lines this contract defines.
   - stderr goes to `NUL`. Keep your logs in your own log file.
4. **Lines.** Each stdout line is one JSON object in UTF-8, ending in `\n` (`\r\n` is accepted), at most **1024 bytes**. It holds `"proto": "fastkiwi-hub-cli@1"` and exactly the fields this document gives its kind. An optional field is either present with a value or absent, never `null`. Values come from closed sets: **no free text crosses to the Hub**, ever.
5. **Exit codes.**

   | Code | Meaning |
   |---|---|
   | `0` | Success |
   | `1` | Failure. The last line said why, with a `code`. |
   | `2` | The command line was not understood. Nothing was printed or changed. |

6. **No UI of the app's own.** No window, tray icon, global hotkey, toast, browser or autostart entry, and no single-instance lock that would stop the user's standalone copy. The only exception is Windows' own consent dialogs during `--hub-update` (Store consent, UAC).
7. **No elevation.** The app runs `asInvoker`, as the same user as the Hub.
8. **No network**, except the app's existing, consented update check inside `--hub-update` (§5.1). An app's core local function stays as it is when run standalone; for example, Lemon talks to the user's own Ollama on loopback. Hub mode adds no network use.
9. **No IPC between apps.** An app talks only to the Hub that started it, and only through these pipes and its own loopback page.

## 3. `fastkiwi-hub.json`

This file sits **next to the exe**, inside the package or install folder, and ships with every build that supports any hub command.

```json
{"proto":"fastkiwi-hub-cli@1","app":"lemon","commands":["hub-host","hub-update","export-data"]}
```

- `app`: the catalog id (`instawords`, `lemon`, `instafind`, `remember-me`).
- `commands`: a non-empty list, without repeats, drawn from `hub-host`, `hub-update` and `export-data`.
- No other fields. A UTF-8 byte-order mark is tolerated.
- If the file is invalid, or names another app, the Hub treats the app as not supporting hub mode at all.
- Packaging:
  - MSIX: put the file at the package root, next to the exe.
  - Inno, NSIS and portable copies: put it in the install folder.
  - Electron: use electron-builder `extraFiles`, so the file sits next to `Remember Me.exe`.

## 4. `--hub-host` (kind P)

### 4.1 Start and handshake

The Hub runs `<exe> --hub-host`. No other arguments are allowed.

The app then does the following, in order:
1. Starts its UI server on **`127.0.0.1` port 0**, which gives an ephemeral port. Do not bind `0.0.0.0`, `::`, `::1` or `localhost`.
2. Draws a **key**: 32 bytes from the OS CSPRNG, encoded as base64url without padding (43 characters, `[A-Za-z0-9_-]`). It draws a new key for each run, keeps it only in memory, and never logs it, writes it to disk or puts it on a command line.
3. Prints **exactly one line** to stdout, within **10 s** of starting. After that line it prints nothing more.

```json
{"proto":"fastkiwi-hub-cli@1","port":49731,"key":"<43 chars>","name":"lemon","version":"2.3.0"}
```

| Field | Rule |
|---|---|
| `port` | 1024–65535, the port actually listening |
| `key` | as above |
| `name` | the catalog id, equal to `app` in `fastkiwi-hub.json` |
| `version` | the app's SemVer 2.0.0 version |

### 4.2 Lifetime

- The app runs until **stdin reaches end-of-file**: the Hub closed it, or the Hub died. It must then exit with code 0 within **2 s**, even if requests are still open.
- Bytes on stdin are reserved and ignored.
- The Hub closes stdin when the user closes the app's tab or the Hub window. If the app is still running 5 s later, the Hub terminates that process (only the one it started).
- Several `--hub-host` runs and the user's standalone copy can run at the same time. Each run has its own port and key, and none forwards to another instance.
- Data the standalone copy may also have open must stay consistent (for example SQLite in WAL mode). This is the app's own concurrency rule, not new to the Hub.

### 4.3 Session: key, then cookie

The Hub's WebView first opens:

```
http://127.0.0.1:<port>/__fastkiwi/enter?key=<key>
```

The app checks the key with a **constant-time** comparison.
- **Right key:** it answers `303 See Other`, `Location: /`, with these headers:
  ```
  Set-Cookie: fkhub=<session>; Path=/; HttpOnly; SameSite=Strict
  Cache-Control: no-store
  Referrer-Policy: no-referrer
  ```
- **Wrong, empty or missing key:** `403`, and no cookie is set.

About the cookie:
- `<session>` is opaque: at least 22 characters of `[A-Za-z0-9_-]` (128 bits or more). It may be the key itself, but a separate random value is better.
- It is a host-only session cookie: no `Domain`, `Expires` or `Max-Age`.
- The WebView has its own profile, so no other page ever holds the cookie.

### 4.4 Guards, on every request and before any routing

Answer `403` with an empty body when a guard fails. A missing `Host` may also get `400`.

| Guard | Rule |
|---|---|
| Host | `Host` is exactly `127.0.0.1:<port>`. `localhost:<port>` may also be accepted. Everything else is refused, including another port, no port, `[::1]` and any other name (DNS-rebinding defence). |
| Origin | If `Origin` is present, it is exactly `http://127.0.0.1:<port>` (or `http://localhost:<port>` if that Host is accepted). `null` is refused. |
| Unsafe methods | Every method other than `GET` and `HEAD` must carry that `Origin`. |
| Sec-Fetch-Site | If present, it is `same-origin` or `none`. `same-site` (another port on 127.0.0.1) and `cross-site` are refused. |
| Session | Every request except `enter` carries the cookie `fkhub=<session>`. That includes HTML, scripts, styles, images, API calls, SSE and WebSocket upgrades. |
| CORS | **None.** Never send `Access-Control-Allow-Origin` (no `*`, no echo). A preflight from a foreign origin is refused by the Origin guard. |

### 4.5 Responses

- Every response carries `X-Content-Type-Options: nosniff`.
- HTML responses carry a `Content-Security-Policy` whose `connect-src` (or, without it, `default-src`) allows only `'self'`. The page then cannot reach anything but its own server. For example:
  `default-src 'self'; connect-src 'self'; img-src 'self' data:; frame-src 'none'; worker-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'none'`
- `Cache-Control: no-store` is recommended everywhere; it is required on `enter`.
- `GET /` (after the redirect) is the app's page, with `200 text/html`. It must contain:
  ```html
  <meta name="fastkiwi-module" content="lemon">
  <meta name="fastkiwi-module-version" content="2.3.0">
  <script type="module" src="/__fastkiwi/embed.js"></script>
  ```
- `GET /__fastkiwi/embed.js` serves the pinned adapter (§6) byte for byte, as `text/javascript`.
- The app's own brand bar is marked `data-fk-brand`.
- The `/__fastkiwi/` prefix is reserved. Serve nothing else under it.

### 4.6 What the Hub does on its side (for information)

- It hosts the page in a WebView2 with its own user-data folder (`%LOCALAPPDATA%\FastKiwi\Hub\WebView2\module-<id>`).
- Navigation is allowed only to `http://127.0.0.1:<port>/…`. New windows, downloads, permission requests and host objects are refused, as in `MODULE-CONTRACT.md` §2.
- Before any page script runs, it adds an initialization script that sets `window.__FASTKIWI_HUB__ = {proto: "fastkiwi-module@1", module: "<id>"}`. `embed.js` uses this to tell the Hub's WebView from the app's own WebView2 window.
- The `fastkiwi-module@1` messages then flow as in `MODULE-CONTRACT.md` §5. On `ready`, the Hub sends `theme`, `lang` and `digits`.
- If the handshake is missing, late or invalid, or the process exits early, the Hub shows «يفتح في نافذته» and offers to open the app standalone.
- **Store copies (Hub 0.2.0).** The Hub drives only copies it can start by path: Inno, NSIS, winget and portable. A Store copy needs its app execution alias, so that it runs with package identity and `StoreContext` works. Until spike S1 proves that path, a Store copy opens in its own window and updates in the Store (D-H24). Nothing changes on the app's side: ship `fastkiwi-hub.json` in the package anyway.

## 5. `--hub-update`

```
<exe> --hub-update [--check-only] [--hub-owner <hwnd>]
```

The two options may come in either order.
- `--check-only`: only asks whether a newer version exists. It never downloads or installs. The conformance harness uses this mode.
- `--hub-owner <hwnd>`: the Hub window's handle in decimal; `0` means none. Use it as the owner of a Windows consent dialog (for the Store, `IInitializeWithWindow`; spike S2). Ignore it if unused, but always accept it: the Hub always passes it.

### 5.1 Consent and network

- The Hub runs `--hub-update` only when the user asks («تحقق الآن», «حدّث») or after the user turned on the Hub's update check, which is off by default. That is the user's consent for this one check, so the app does not also need its own automatic-check setting turned on. It never shows a prompt of its own.
- The network it uses is the app's existing update source only:
  - **Store copy:** `StoreContext.GetAppAndOptionalStorePackageUpdatesAsync`, then `RequestDownloadAndInstallStorePackageUpdatesAsync`, or `TrySilentDownloadAndInstall…` when allowed.
  - **Direct copy** (GitHub or fastkiwi.app): the app's own `latest.json` and minisign updater. It checks against the app's pinned public key, checks expiry and a non-decreasing serial, downloads, verifies, swaps and rolls back on failure. This is the same code as its in-app updater.
  - **winget, Scoop or a developer build:** do not update; return `error` `unsupported`.
- It **never opens a browser**, never shows its own window, and never restarts the app itself. The Hub offers «أعد فتحه».

### 5.2 Progress lines

| `state` | Fields | Final | Meaning |
|---|---|---|---|
| `checking` | none | | Always the first line, within **5 s** |
| `available` | `version` (required) | yes | `--check-only` only: a newer `version` exists |
| `downloading` | `progress` (required), `version` (optional) | | `progress` is the overall fraction, 0…1 |
| `installing` | `progress` (required), `version` (optional) | | |
| `done` | `version` (optional) | yes | Installed |
| `none` | none | yes | Already up to date |
| `error` | `code` (required) | yes | Failed |

Progress is **overall** and **never goes back**. The Store maps download to 0–0.8 and install to 0.8–1; direct updaters should do the same.

**Order:**
- `checking` comes once, first.
- `available` and `none` come right after `checking`.
- `downloading` never follows `installing`.
- The final line is the last line. It is followed by exit `0` (`available`, `done`, `none`) or `1` (`error`).
- `--check-only` never prints `downloading`, `installing` or `done`.
- Lines arrive at least every **120 s**. Send at most about 10 per second.

**`code` values (fixed set):**

| `code` | When |
|---|---|
| `offline` | No network, or the source cannot be reached |
| `server` | Unusable answer (HTTP error, bad file) |
| `metadata` | Update metadata invalid, expired or older than what is installed (rollback) |
| `signature` | Signature or hash check failed |
| `disk-full` | |
| `in-use` | Files locked, cannot replace |
| `declined` | The user declined a Windows dialog (Store consent, UAC) |
| `policy` | Updates blocked by policy or a metered network |
| `unsupported` | This copy cannot update itself (winget, Scoop, developer build) |
| `busy` | Another update of this app is running |
| `cancelled` | stdin closed before `installing` |
| `internal` | Anything else |

**Cancel:** closing stdin before `installing` means cancel. The app stops, cleans up and prints `error` `cancelled`. Once `installing` has started, the app finishes the install or rolls back safely, never half-way.

**What the Hub shows.** It also forwards the progress to the hosted module as `update-progress` (`MODULE-CONTRACT.md` §4).

| Hub-cli line | Module message | Updates screen |
|---|---|---|
| `checking` | `checking` | — |
| `downloading` | `downloading` with `progress` | «يُحدَّث… ٤٥٪» |
| `installing` | `installing` with `progress` | «يُحدَّث… ٤٥٪» |
| `done` | `done`, then `restart-needed` if the app is open | «تم التحديث», then «أعد فتحه» |
| `error` | `failed` | the message for its `code` |
| `available` | — | «تحديث متاح» |
| `none` | — | «محدَّث» |

A line that breaks the contract counts as `error` `internal`.

## 6. `embed.js`, the web adapter

`sdk/embed.js` is **fastkiwi-embed 1.0.0**, SHA-256 `655ee287d4b152c08bc443281553232edf17765b0dc055187acdbdc81b05e28b`. It is also in `sdk/embed.js.sha256` and in `hubcli::EMBED_RELEASES`.

**Shipping it**
- Copy the file **byte for byte**: LF line endings, not re-formatted, not minified, not bundled. Keep it out of any build step that rewrites files.
- In the app repo, add a `.gitattributes` line `**/embed.js -text`, and a test that checks the file's SHA-256 against the pinned value.
- Serve it at `/__fastkiwi/embed.js`:
  - kind P: from the app's own server;
  - kind S: as `web/__fastkiwi/embed.js`.
- Load it with `<script type="module" src="/__fastkiwi/embed.js">`, plus the two `<meta>` tags from §4.5. Kind S pages use the same tags.

**Outside the Hub it does nothing.** That covers a normal browser and the app's own WebView2 window, which has `chrome.webview` but not the Hub's marker. The page keeps its own settings and never waits for the Hub.

**Inside the Hub it:**
1. sets `<html data-fk-hosted>` and hides every `[data-fk-brand]` element, including ones added later;
2. attaches its `message` listener, then sends `ready` with the meta `module` and `version`;
3. validates every incoming message exactly as in `MODULE-CONTRACT.md` §4 and drops the rest silently;
4. applies the settings:
   - `theme` → `<html data-theme data-tone>`;
   - `lang` → `<html lang dir>`;
   - `digits` → `<html data-digits>`.

**API, as module exports:**

| Export | Use |
|---|---|
| `isHosted()` | `true` only in the Hub's WebView for this module |
| `settings()` | `{theme, tone, lang, dir, digits}`, or `null` |
| `on("settings", fn)` / `on("update", fn)` | Subscribe; returns an unsubscribe function. The same data also arrives as `fastkiwi:settings` and `fastkiwi:update` events on `window`. |
| `openOwnWindow()` | Sends `open-own-window` |
| `formatNumber(n, opts)` | `Intl.NumberFormat` with the Hub's digits (CLDR `arab` or `latn`) |

Apps whose themes are not the family tokens map them in their `on("settings")` handler. The Hub's values apply only while hosted. **Do not save them** into the app's own settings file (council verdict 6, until spike S3).

**Changes:** a new adapter release gets a new version, SHA-256 and `EMBED_RELEASES` entry. Apps update by copying the new file. The harness accepts any released hash and says when a newer one exists.

## 7. `--export-data <dir>`

```
<exe> --export-data <dir>
```

This keeps the user's data on uninstall (council verdict 3: «احتفظ ببياناتي» is the default). For MSIX apps, Windows deletes package data on uninstall, so the export must come first.

- `<dir>` is an absolute path. It must not exist yet, or must be empty: the app creates it. If it exists and is not empty, the app prints `error` `not-empty`, exits 1 and **touches nothing**.
- The Hub uses `Documents\FastKiwi\Backups\<app>-<YYYYMMDD-HHMMSS>\`.

**Layout:**

```
<dir>\
  fastkiwi-export.json
  data\…            the app's own files, in the app's own layout
```

```json
{"proto":"fastkiwi-hub-cli@1","app":"lemon","version":"2.3.0","created":"2026-11-14T08:30:00Z",
 "files":[{"path":"data/chats.db","size":81920,"sha256":"<64 lowercase hex>"}]}
```

**`fastkiwi-export.json`:**
- `created` is UTC, `YYYY-MM-DDTHH:MM:SSZ`.
- Every `path` is relative and `/`-separated, under `data/`, with:
  - no empty, `.` or `..` parts;
  - no `\ : * ? " < > |` or control characters;
  - no part ending in `.` or a space;
  - no reserved device names (`CON`, `NUL`, `COM1`, …);
  - at most 240 characters;
  - no two paths that differ only in case.
- **Every file under `data\` is listed, and nothing else is in `<dir>`.**

**What goes in, and what stays out:**
- **The app's own data only:** user content and the app's settings.
- Leave out caches, logs, downloaded models (they can be fetched again), WebView2 profiles, update staging, other apps' data and secrets that belong to the machine.
- Make a **consistent snapshot**, even while the app runs elsewhere (for SQLite, use the backup API).
- **Never change or delete the original data.** No network, no UI.

**Result line** (one line, printed last):
- success: `{"proto":"fastkiwi-hub-cli@1","state":"done","files":3,"bytes":82011}`, then exit 0. An app with no data yet exports an empty `files` list.
- failure: `{"proto":"fastkiwi-hub-cli@1","state":"error","code":"not-empty"}`, then exit 1.

`code` values: `not-empty`, `disk-full`, `access-denied`, `in-use`, `internal`. After a failure, the Hub never deletes anything.

Restoring an export is the app's own feature (a later import), outside this contract.

## 8. Security summary

| Threat | Defence |
|---|---|
| A web page in the user's browser calls the app's port | Session cookie (`SameSite=Strict`, `HttpOnly`), Origin and Sec-Fetch-Site guards, no CORS |
| DNS rebinding | Strict `Host` guard |
| Another local process reads the key | The key is sent only over the Hub's stdout pipe: never on a command line, never on disk, a new one each run. The cookie value can be separate from the key. |
| A local process guesses or reuses a session | 128 bits or more, constant-time compare, a new key each run, the process ends with its Hub |
| The hosted page reaches the network or another app | CSP `connect-src 'self'`, navigation allowlist, own WebView2 profile, no host objects, no IPC between apps |
| A malicious or broken app line drives the Hub | Closed schemas with no free text; the Hub validates every line (`crates/hubcli`) and treats a bad one as `error` `internal` |
| An update installs something unsigned | Store signing, or the app's pinned minisign key with expiry and serial (TUF-style, HUB-DESIGN §7). The Hub holds no keys and never writes into an app's folder. |
| Export writes outside its folder | Safe relative paths under `data/`; the harness checks the folder against the manifest |

## 9. Conformance

Build the harness once:

```
cargo build --release -p fastkiwi-hub-conformance
```

Then run it against an app build, with `fastkiwi-hub.json` next to the exe:

```
target\release\check-app.exe "<path to app.exe>" [--app <id>] [--update-silence <s>] [--no-window-check]
```

It exits 0 when the app conforms. `--hub-update --check-only` may ask the app's update source once. The harness never downloads or installs.

| ID | Check |
|---|---|
| M1 | `fastkiwi-hub.json` is valid and names the app |
| H1 | One valid handshake line within 10 s: fields, key format, `name`, SemVer |
| H2 | Not reachable on `[::1]` or on the machine's LAN address |
| H3 | Without the cookie: 403 on `/`, `embed.js` and other paths |
| H4 | Wrong, empty, missing or truncated key: 403 and no cookie |
| H5 | Right key: 303 to `/`, cookie attributes, `no-store`, `no-referrer` |
| H6 | `/` is HTML with nosniff, a CSP allowing only `'self'`, both meta tags and the embed script |
| H7 | `embed.js` is a pinned release |
| H8 | Host guard: other names, other port, no port, `[::1]`, missing Host |
| H9 | Origin and Sec-Fetch-Site guard, including POST without Origin; its own Origin is allowed |
| H10 | No `Access-Control-Allow-Origin` anywhere; a foreign preflight gets 403 |
| H11 | A second run gets its own port and key; the first key does not open it |
| H12 | No visible top-level window (Windows) |
| H13 | Nothing on stdout after the handshake |
| H14 | Exits with code 0 within 2 s of stdin closing |
| U1 | `--hub-update --check-only --hub-owner 0`: every line valid, order, final state, exit code. `error offline` still conforms. |
| E1 | Export into a new folder: result line, manifest, files match exactly (sizes and SHA-256) |
| E2 | Export into a non-empty folder: `not-empty`, exit 1, folder unchanged |
| C1 | Each declared command with a bad command line: exit 2, no output |

**Not covered by the harness** (each app's own tests, or a device run):
- a real Store update (`StoreContext`, consent dialog);
- a real direct update with signature and rollback;
- data consistency while the standalone copy runs;
- that the page actually follows the settings;
- that `--hub-host` starts no tray icon or hotkey;
- child processes' windows (only the main process is checked).

## 10. Versioning

`proto` is `fastkiwi-hub-cli@<major>`, and this document follows SemVer 2.0.0:
- **Minor change, keeps `@1`:** adding an optional field, a state, a code or a command that receivers may ignore. It is listed here before anyone sends it. Until then, receivers drop what they do not know, as the strict rules in §2 say.
- **Major change, `@2`:** removing or changing a field, a value set or a rule.

A Hub may support several majors; an app speaks one.
