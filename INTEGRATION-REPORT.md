# FastKiwi Hub: first end-to-end integration, 2026-11-14

Hub 0.2.0. The run started at commit `134cdfa`; this report's commit adds the D-H28 fix. Three apps were run on their `claude/hub-mode` branches, and every window was captured off screen with `PrintWindow`.

## Result

| Contract item | InstaWords (S) `13e740d` | Lemon (P) `981999a` | InstaFind (P) `555b2f7` |
|---|---|---|---|
| `check-app` (M1, H1–H14, U1, E1, E2, C1) | **PASS**, conforms; H* skipped (kind S) | **PASS**, 19 of 19 | **PASS**, 19 of 19 |
| `web\` package: pinned `embed.js`, meta tags (kind S, checked by hand) | **PASS** | n/a | n/a |
| Handshake inside the Hub | n/a (the Hub serves `web\`) | **PASS** (≈140–270 ms) | **PASS** (≈160–300 ms) |
| `ready` → settings sent | **PASS** | **PASS** | **PASS**, after the D-H28 Hub fix |
| Theme followed (light, dark, midnight, kiwi) | **PASS**, all four exact | **PASS**, tone only: midnight→dark, kiwi→light (allowed, §6) | **FAIL**: midnight and kiwi show its own `auto`; light was `auto` in one run of two (F1) |
| Language and direction followed (ar ↔ en) | **PASS** (F6 minor) | **PASS** | **PASS**, after the D-H28 Hub fix (the page reloads in the language) |
| Digits followed (arab ↔ latn) | **PASS** | **PASS** (F4 minor) | **PARTIAL** (F2) |
| Brand bar hidden (`data-fk-brand`) | **PASS** (2 of 2) | **PASS** (1 of 1) | n/a: the page has no brand bar |
| «افتح في نافذته» from the page (`open-own-window`) | **PASS** (request reached the Hub; launch recorded, not executed) | **PASS** (same) | **PASS** (same) |
| Updates screen, `--hub-update --check-only` | **PASS** (`none`) | **PASS** (`none`) | **PASS** (`none`) |
| Nothing left running after the Hub closed | **PASS** | **PASS** | **PASS** |

**Overall: InstaWords and Lemon pass. InstaFind passes the CLI contract, but not the page's theme following (F1) or all of its digits (F2).** Each run also found a Hub bug, fixed here (below).

## Hub bug found and fixed (D-H28)

- **The cause.** wry defines a frozen, non-configurable `window.ipc` in every WebView that has an IPC handler, and the Hub's module WebViews had one. InstaFind's hub page defines its own `window.ipc`, to send its requests to its own server. Inside the Hub that definition threw.
- **What it broke.**
  - InstaFind's own app messages went to the Hub as web messages. The Hub dropped them and showed «proto is not fastkiwi-module@1».
  - The language reload in the same script never ran, so the page stayed Arabic in English mode.
- **Fix.** Module WebViews now take `WebMessageReceived` from WebView2 directly. The page gets no global except the `__FASTKIWI_HUB__` marker.
  - `docs/MODULE-CONTRACT.md` §2 now says so.
  - A guard test keeps wry's IPC handler on the Hub's own page only.
  - All three apps were run again after the fix. These results and screenshots are from after the fix.
- **For the apps:** no change needed. The contract never promised `window.ipc`.

## Findings for the app sessions

| # | App | Severity | What | Contract | Evidence |
|---|---|---|---|---|---|
| F1 | InstaFind | **Medium** | The page does not keep the Hub's theme. With midnight or kiwi, `<html data-theme>` ends up `auto` and the page uses its own palette (light on this PC). With light, one run of two also ended `auto`. Its own settings appear to set the theme again after `embed.js` applied the Hub's, so the result depends on timing. InstaFind already has midnight and kiwi palettes (its themes list), so mapping all four should be direct. | MODULE-CONTRACT §4 `theme`; HUB-APP-CONTRACT §6 | `docs/integration/2026-11-14/it-instafind-app-instafind-{midnight-ar,kiwi-en,light-ar}.json`, `shots/it-instafind-app-instafind-midnight-ar-1366x768.png` |
| F2 | InstaFind | Low | With `arab` digits, some numbers the page shows stay Latin: «عدد الملفات: 832,285» in the status bar, and the day in «30 أكتوبر» in the recent files list. | MODULE-CONTRACT §4 `digits` | `shots/it-instafind-app-instafind-midnight-ar-1366x768.png` |
| F3 | InstaFind | Low–medium | A fresh build has no `fastkiwi-hub.json` next to `bahth.exe`. The commit writes it only "next to a self-updating copy at its first tick", plus in the MSIX. A portable copy started from the Hub before it has ever run alone therefore opens in its own window. For this run, the file was copied from `packaging/msix/`. Suggest shipping it in the portable zip and in the build output, as Lemon's `build.rs` and InstaWords' csproj do. | HUB-APP-CONTRACT §3 | — |
| F4 | Lemon | Low | With `arab` digits, the sidebar heading «آخر 30 يومًا» keeps Latin digits. | MODULE-CONTRACT §4 `digits` | `shots/it-lemon-app-lemon-light-ar-1366x768.png` |
| F5 | Lemon | Info | Midnight shows as dark and kiwi as light: Lemon has only two palettes. This is allowed (§6, "map them"); listed so the owner knows. | HUB-APP-CONTRACT §6 | `it-lemon-app-lemon-{midnight-ar,kiwi-en}.json` |
| F6 | InstaWords | Low | In English, the skip link «تخطَّ إلى المحتوى» and the page `<title>` stay Arabic. | MODULE-CONTRACT §4 `lang` | `it-instawords-app-instawords-dark-en.json` (title), screenshot |

None of these blocks the CLI contract. F1 is the only one that is visible at once.

## How it was run

**Worktrees**
- Created with `git worktree add --detach C:\BahthTest\hub-it\<app> <commit>`. The app repos' own checkouts and branches were not touched.
- InstaWords borrowed the main checkout's `node_modules` through a junction, read only. The worktrees were removed afterwards.

**Builds**, each with its own output folder:
- **Lemon:** `cargo build --release`. Its `build.rs` put `fastkiwi-hub.json` next to the exe.
- **InstaFind:** `cargo build --release`, with the capability file copied from `packaging/msix/` (F3).
- **InstaWords:** `npm run build:web`, then `dotnet build -c Release` of `InstaWords.App`. The output had `web\` and `fastkiwi-hub.json`.

**Harness:** `check-app <exe>` (kiwi-hub `tools/conformance`, unchanged since step 1). Outputs are in `docs/integration/2026-11-14/check-*.txt`.

**In the Hub:**

```
FastKiwi.exe --dev-host <id>=<exe> --dev-report <json> --dev-shot <png> \
  --screen app:<id>|updates-check --theme <t> --lang <l>
```

- The window was off screen and never focused.
- The report holds the handshake, `ready`, a probe of the hosted page (theme, tone, lang, dir, digits, brand hidden, marker), then the page's own `open-own-window` through its own `embed.js`, and the update results.
- **«افتح في نافذته» was recorded, not executed**, during these runs, so no app window opened on the PC. The request reaching the Hub and the right exe being chosen were checked. The window itself was not opened: the app's own launch is unchanged from Phase C.

**Screenshots** (`shots/it-*-1366x768.png`, 19 files):
- each app in light/Arabic and dark/English;
- InstaFind and Lemon also in midnight/Arabic and kiwi/English, and InstaWords in midnight and kiwi;
- the Updates screen per app, light and dark.

**Results.** The raw reports are in `docs/integration/2026-11-14/it-*.json`, with the page-text sample removed.

## Not covered

- A full `--hub-update` (download and install). Only `--check-only` was run, as asked. All three apps are at their latest release, so `none` was the expected answer.
- Store copies (spikes S1 and S2), and real windows opening.
- InstaFind's search inside the Hub: disabled in hub mode by its session while a native crash is being fixed.
- `--export-data` from the Hub UI, which has no «أزل» flow yet. It was checked by the harness only (E1 and E2).

## Data note

- The harness's E1 copied each app's own data folder on this PC into a temporary folder (4, 11 and 0 files) and deleted it afterwards. The original data was only read.
- The Lemon screenshots show three existing chat titles on this PC, which look like test prompts. The InstaFind screenshots show one recent file, `E:\BahthTest\ocr\sign.png`, and the index size. They were kept, since the shots are local only.
