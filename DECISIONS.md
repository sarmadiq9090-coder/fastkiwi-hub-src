# FastKiwi Hub prototype (Phase C): decisions

Each decision below would normally go to the owner. Per the owner's standing instruction («اي قرار لي اسئل المجلس ونفذ») it was taken after a short internal council review with the seats of HUB-DESIGN.md (architect, Windows security and Store, release/QA and platform risk, realist, UX), then carried out. Nobody else was asked.

Design source: `C:\BahthTest\kiwi-hub-design\HUB-DESIGN.md` (2026-11-14) and its mockups.

## D-H01 Start Phase C now (2026-11-14), not after 2026-11-28

- **Context.** HUB-DESIGN §8 put Phase C "after the D-073 freeze (≥ 2026-11-28), only when P0–P3 are clear". On 2026-11-14 the owner asked directly to start the Hub app now, while the other agents run.
- **Council.** 5 of 5 to go ahead. The freeze exists to protect InstaConvert's three writer lanes. This work:
  - lives only in `E:\fast kiwi pro neq\kiwi-hub\` (a new local folder, local `git init`);
  - uses its own `CARGO_TARGET_DIR` (`kiwi-hub\target`);
  - reads no InstaConvert code and touches no `C:\BahthTest\ic-*` worktree, no `kiwi-scan`, no other repo (other repos were only read, to find detection keys and the InstaWords web build);
  - makes no GitHub write of any kind (no push, no new repo: the council still forbids a new repo, and the manager session is the only GitHub writer).
  So it takes nothing from those lanes. The realist seat noted that the prototype must stay "never on a train" (§8), which it does.
- **Decision.** Start now. Phase D, E and F gates are unchanged.

## D-H02 Fixture catalog format

- `catalog/apps.json` is the site's `apps.json` schema 1 (same `id`, `status`, `name`, `tagline`, `store`, `winget`, `icon`) plus a draft `hub` block per app: `kind` (S/P/L), `size_mb`, `on_demand`, `needs`, `data_dir`, `module`, `detect` (probes).
- `serial` and `expires` are parsed (TUF-style fields from §4) but not enforced: the fixture is compiled into the exe and never fetched. Signature, expiry and rollback checks belong to Phase E (§7 "Update metadata").
- Parsing is strict. A bad id, a probe path outside `%LOCALAPPDATA%`/`%APPDATA%`/`%USERPROFILE%`/`%PROGRAMFILES%`/`%PROGRAMFILES(X86)%`, `..`, a non-`.exe` target, a malformed PFN, an unknown probe kind or an unknown `hub` field makes the whole catalog invalid. The security seat asked for this so a damaged catalog can never make the Hub read or launch something unexpected.

## D-H03 InstaConvert hidden (D-069)

- Council verdict 4: the Hub hides every app whose status is not `released`. InstaConvert is `coming-soon`, so it is not in the shipped view.
- It appears only with `--dev` (the dev fixture flag), marked «غير منشور». `--catalog` and `--dev-module-dir` also turn dev mode on.

## D-H04 Detection probes (read only)

Found from the apps' own code (read only) and checked on this PC on 2026-11-14:

| App | Probes, in order |
|---|---|
| InstaFind | Store `FastKiwi.InstaFind_sm1xjmj6v76yc!InstaFind`; winget `Sarmad.InstaFind` (`Links\bahth.exe`, `Packages\Sarmad.InstaFind_*\bahth.exe`); `%LOCALAPPDATA%\Programs\InstaFind\bahth.exe`; the old `%LOCALAPPDATA%\Programs\Bahth\bahth.exe` (instafind `relocate.rs`); Scoop `%USERPROFILE%\scoop\apps\instafind\current\bahth.exe`; the `Run` value `BahthSari`; the hint `HKCU\Software\BahthSari` |
| InstaWords | Store `FastKiwi.InstaWords_sm1xjmj6v76yc!App`; Inno key `{8F3B6C2A-5D4E-4A7B-9C1F-2E6D8A0B4C71}_is1`; winget `Sarmad.InstaWords`; Scoop |
| Remember Me | Store `FastKiwi.RememberMeReminders_sm1xjmj6v76yc!RememberMe`; NSIS key `a92ab8ea-9a18-5b57-8667-052d208c8425` |
| Lemon | winget `Sarmad.Lemon`; `%LOCALAPPDATA%\Programs\LocalAiChat\local-ai-chat.exe`; `%LOCALAPPDATA%\Programs\Lemon\local-ai-chat.exe` |
| InstaConvert (dev only) | `%LOCALAPPDATA%\Programs\InstaConvert\instaconvert.exe` (placeholder: InstaConvert has no installer yet) |

Notes:
- INVENTORY.md names Remember Me's NSIS key as `com.waqt.reminder`. That is the electron-builder **appId**; the real uninstall key is the GUID electron-builder derives from it (`a92ab8ea-…`, seen in `HKCU\…\Uninstall` here). Its install folder is in `HKCU\Software\a92ab8ea-…\InstallLocation`; the Hub also falls back to the uninstaller's folder.
- Lemon's `Programs\LocalAiChat` folder was seen on this PC but is not written down in Lemon's repo. Kept as a path hint; to confirm in Phase D.
- `HKCU\Software\BahthSari` alone is a trace, not an install: it shows «استُعمل على هذا الجهاز من قبل» on the Add card, never «مثبّت».
- Both copies are shown when both exist (§4). The same exe found by several probes (winget link, package folder, `Run` value) is listed once.
- Registry keys are opened with `KEY_READ` only; Store packages are only queried. `tests/guards.rs` fails if any registry or package write API appears in the sources.

## D-H05 Process apps launch standalone until Phase D

InstaFind and Lemon are kind P. Hosting them needs `--hub-host` (Phase D), so their cards say «يفتح هنا لاحقًا، في المرحلة D» and «افتح» launches them in their own window. Remember Me is kind L and always launches. Launch is ShellExecute and forget (REVIEW-V2 §12.2): no handle kept, no Job object, and only a path or AUMID that detection found (the UI sends an app id and an index, never a path).

## D-H06 Where a hosted module comes from

Order: (1) the installed copy's `web\index.html` (Store or Inno install); (2) `--dev-module-dir <folder>` (dev only); (3) the stub module compiled into the exe (dev only).
- The shipped (non-dev) build never hosts a module without an installed copy, because "the Hub ships no app" (§1).
- No InstaWords package ships a `web\` folder yet, and InstaWords is not installed on this PC. The real local web build (`E:\fast kiwi pro neq\InstaWords\_site`, read in place, not copied) was hosted with `--dev-module-dir` and works; it does not speak the protocol yet, so the module bar says so. The stub module speaks the protocol and shows the shared settings arriving.

## D-H07 Message transport

wry's IPC only accepts string web messages, so both directions carry **JSON text** (`postMessage(JSON.stringify(m))` / `PostWebMessageAsString`). Validation is strict and identical both ways (`src/protocol.rs`); the contract is `docs/MODULE-CONTRACT.md`. A module that has not sent `ready` 2.5 s after load is shown as «لا يتحدث fastkiwi-module@1 بعد».

## D-H08 "No network" is enforced, not only promised

A navigation allowlist does not stop `fetch()`. Every response from the Hub's protocol handlers therefore carries a CSP with `connect-src 'self'` (and no frames, workers or objects). The real InstaWords web build asks the GitHub API for its latest release; that request is blocked, and the page falls back to its built-in release data, as it does offline. The Hub itself has no network code (`tests/guards.rs`).

## D-H09 WebView2 hardening

For the Hub and every module: host objects off, status bar off, new windows refused, downloads refused, all permission requests refused, dev tools only with `--dev`. The Hub page also has zoom, browser accelerator keys and (outside dev) the context menu turned off. Each module has its own user-data folder (`%LOCALAPPDATA%\FastKiwi\Hub\WebView2\module-<id>`).

## D-H10 Updates and Add are UI only

- **Updates** shows the mockup's four states from `catalog/updates.fixture.json`, with a visible note that it is demo data. «تحقق الآن» is disabled. «اعرض تقدّم التحديث» animates the fixture row and sends `update-progress` to a hosted InstaWords module, to exercise that message.
- **Add** builds the dialog from the catalog (size now, on-demand parts, Store first, direct download greyed «لاحقًا» per council verdict 5). Lemon shows «ليس في المتجر بعد» (verdict 2: after G3). The primary button only shows «هذا النموذج التجريبي لا يثبّت شيئًا».
- «أزل» and the update-check switch are present but disabled (no uninstall, no network in Phase C).

## D-H11 Screenshots from the running app

`--dev-shot <png> --screen <home|app|add|settings|updates> --theme <…> --lang <…>` drives the real UI to a screen and captures the window with `PrintWindow(PW_RENDERFULLCONTENT)`.
- The first attempt used an always-on-top window. The person at the PC was typing and closed those windows, so it was changed (UX and realist seats): the capture window now opens **off screen, never focused**, with its own WebView2 profile (`WebView2-shot`) and Chromium's occlusion throttling off, so nobody is interrupted.
- Shot runs never save settings.

## D-H12 Shared settings storage

The Hub's own settings live in `%LOCALAPPDATA%\FastKiwi\Hub\settings.json`. «اجعل برامجي تتبع هذه الإعدادات» is stored, but until spike S3 it only affects hosted modules (verdict 6). App folders are never touched.

## D-H13 Numbers, versions and bidi

- Sizes, counts and times follow the digits setting (CLDR `arab`/`latn`; «حسب اللغة»: `ar` → `arab`, `en` → `latn`).
- Versions are identifiers (SemVer) and are always shown in Latin digits, isolated LTR.
- Strings that would end in a Latin letter followed by a bracket inside RTL text («(المرحلة D)») rendered badly (UAX #9 bracket pairs); they were reworded («، في المرحلة D»).

## D-H14 Name, icon and binary

Window title «FastKiwi», product FastKiwi Hub (verdict 1), icon `brand/icons/fastkiwi/fastkiwi.ico` (brand v2 kiwi tile, copied from the site repo). The release exe is built to `dist\FastKiwi.exe` and is **not** committed (`.gitignore`). Version 0.1.0 (SemVer).
