//! Windows-only helpers: launching an installed app (ShellExecute, fire and forget), opening
//! an allowed link in the default browser, the "Add a program" file picker and its icon, the
//! WebView2-missing message, and the dev-only window capture used for screenshots.

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, ClientToScreen,
    CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
    GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, SW_SHOWNORMAL};
use windows::core::{HSTRING, w};

/// What to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A desktop exe at an absolute path found by detection.
    Exe(String),
    /// A Store app's AUMID (`<pfn>!<app id>`), started through `shell:AppsFolder`.
    Aumid(String),
    /// A program the person added: the exact `.exe` or `.lnk` they picked.
    Picked(String),
}

/// True for `Name_publisherid!AppId` made only of safe characters.
pub fn valid_aumid(a: &str) -> bool {
    a.split_once('!').is_some_and(|(pfn, app)| {
        crate::catalog::valid_pfn(pfn)
            && !app.is_empty()
            && app.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
    })
}

/// Start the app and forget it: no handle kept, no Job object, no wait (REVIEW-V2 §12.2).
pub fn launch(t: &Target) -> Result<(), String> {
    let (file, dir) = match t {
        Target::Exe(p) => {
            let path = std::path::Path::new(p);
            if !path.is_absolute() || !p.to_ascii_lowercase().ends_with(".exe") || !path.is_file() {
                return Err("not an exe on this PC".into());
            }
            // The real path must be the one detection found: no junction or symbolic link in
            // between that leads somewhere else.
            let real = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
            let real = real.to_string_lossy();
            let real = real.strip_prefix(r"\\?\").unwrap_or(&real);
            if !real.eq_ignore_ascii_case(p) {
                return Err("the path leads somewhere else (a link)".into());
            }
            let dir = path.parent().map(|d| d.to_string_lossy().into_owned());
            (p.clone(), dir)
        }
        Target::Aumid(a) => {
            if !valid_aumid(a) {
                return Err("not a valid AUMID".into());
            }
            (format!("shell:AppsFolder\\{a}"), None)
        }
        Target::Picked(p) => {
            let path = std::path::Path::new(p);
            if !crate::mine::valid_path(p) || !path.is_file() {
                return Err("not on this PC any more".into());
            }
            let dir = path.parent().map(|d| d.to_string_lossy().into_owned());
            (p.clone(), dir)
        }
    };
    let file = HSTRING::from(file);
    let dir = dir.map(HSTRING::from);
    // SAFETY: plain ShellExecuteW with NUL-terminated strings that outlive the call.
    let r = unsafe {
        match &dir {
            Some(d) => ShellExecuteW(None, w!("open"), &file, None, d, SW_SHOWNORMAL),
            None => ShellExecuteW(None, w!("open"), &file, None, None, SW_SHOWNORMAL),
        }
    };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecute failed ({})", r.0 as isize))
    }
}

/// Open a URL in the default browser. Only `https:` URLs the caller has already checked
/// against the catalog rules (`links`).
pub fn open_url(url: &str) -> Result<(), String> {
    if crate::links::plain_https(url).is_none() {
        return Err("not an https URL".into());
    }
    let u = HSTRING::from(url);
    // SAFETY: plain ShellExecuteW with NUL-terminated strings that outlive the call.
    let r = unsafe { ShellExecuteW(None, w!("open"), &u, None, None, SW_SHOWNORMAL) };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecute failed ({})", r.0 as isize))
    }
}

/// File types the bar opens with their default program on Enter. Anything else (programs,
/// scripts, shortcuts, unknown types) is only shown in its folder.
pub const OPEN_TYPES: [&str; 22] = [
    "pdf", "doc", "docx", "xls", "xlsx", "csv", "ppt", "pptx", "txt", "md", "rtf", "odt", "ods",
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "mp3", "mp4", "zip",
];

/// Open a file found by InstaFind with its default program, or show it in its folder when
/// it is not a document type (see [`OPEN_TYPES`]).
pub fn open_found(path: &str, open: bool) -> Result<(), String> {
    use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};
    let p = std::path::Path::new(path);
    if !p.is_absolute() || !p.exists() {
        return Err("not on this PC any more".into());
    }
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let h = HSTRING::from(path);
    // SAFETY: shell calls with a NUL-terminated path; the PIDL is freed here.
    unsafe {
        if open && p.is_file() && OPEN_TYPES.contains(&ext.as_str()) {
            let r = ShellExecuteW(None, w!("open"), &h, None, None, SW_SHOWNORMAL);
            return if r.0 as isize > 32 {
                Ok(())
            } else {
                Err("ShellExecute failed".into())
            };
        }
        let pidl = ILCreateFromPathW(&h);
        if pidl.is_null() {
            return Err("no such item".into());
        }
        let r = SHOpenFolderAndSelectItems(pidl, None, 0);
        ILFree(Some(pidl));
        r.map_err(|e| e.to_string())
    }
}

/// The standard Open dialog for "Add a program": one `.exe` or shortcut (`.lnk`, not
/// followed). `None` when cancelled.
pub fn pick_program(owner: isize, title: &str, filter_label: &str) -> Option<String> {
    use windows::Win32::UI::Controls::Dialogs::*;
    use windows::core::{PCWSTR, PWSTR};
    let mut file = vec![0u16; 1024];
    let filter: Vec<u16> = format!("{filter_label}\0*.exe;*.lnk\0\0")
        .encode_utf16()
        .collect();
    let title = HSTRING::from(title);
    let start = std::env::var("APPDATA")
        .map(|a| format!("{a}\\Microsoft\\Windows\\Start Menu\\Programs"))
        .ok()
        .filter(|d| std::path::Path::new(d).is_dir())
        .map(HSTRING::from);
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: HWND(owner as *mut core::ffi::c_void),
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file.as_mut_ptr()),
        nMaxFile: file.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        lpstrInitialDir: start
            .as_ref()
            .map_or(PCWSTR::null(), |s| PCWSTR(s.as_ptr())),
        Flags: OFN_EXPLORER
            | OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NODEREFERENCELINKS
            | OFN_NOCHANGEDIR
            | OFN_DONTADDTORECENT,
        ..Default::default()
    };
    // SAFETY: the struct and its buffers live through the modal call.
    let ok = unsafe { GetOpenFileNameW(&mut ofn) }.as_bool();
    if !ok {
        return None;
    }
    let end = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    Some(String::from_utf16_lossy(&file[..end]))
}

/// The `FileDescription` in an exe's version resource, if any.
pub fn file_description(path: &str) -> Option<String> {
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    let p = HSTRING::from(path);
    // SAFETY: plain Win32 calls on a local buffer of the size the system asked for.
    unsafe {
        let size = GetFileVersionInfoSizeW(&p, None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        GetFileVersionInfoW(&p, None, size, buf.as_mut_ptr().cast()).ok()?;
        let mut tr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(
            buf.as_ptr().cast(),
            w!("\\VarFileInfo\\Translation"),
            &mut tr,
            &mut len,
        )
        .as_bool()
            || tr.is_null()
            || len < 4
        {
            return None;
        }
        let pair = std::slice::from_raw_parts(tr as *const u16, 2);
        let q = HSTRING::from(format!(
            "\\StringFileInfo\\{:04x}{:04x}\\FileDescription",
            pair[0], pair[1]
        ));
        let mut v: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut vlen = 0u32;
        if !VerQueryValueW(buf.as_ptr().cast(), &q, &mut v, &mut vlen).as_bool()
            || v.is_null()
            || vlen == 0
        {
            return None;
        }
        let s = std::slice::from_raw_parts(v as *const u16, vlen as usize);
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        let d = String::from_utf16_lossy(&s[..end]).trim().to_string();
        (!d.is_empty()).then_some(d)
    }
}

/// Save the shell's large icon of a file (an exe or a shortcut) as a 32×32 PNG.
pub fn save_file_icon(path: &str, out: &std::path::Path) -> Result<(), String> {
    use windows::Win32::Graphics::Gdi::{CreateDIBSection, GdiFlush};
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW};
    use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx};
    const S: i32 = 32;
    // SAFETY: shell and GDI calls on handles created and released here.
    unsafe {
        let mut fi = SHFILEINFOW::default();
        let r = SHGetFileInfoW(
            &HSTRING::from(path),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut fi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if r == 0 || fi.hIcon.is_invalid() {
            return Err("no icon".into());
        }
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: S,
                biHeight: -S,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let result = match CreateDIBSection(Some(mem), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bmp) if !bits.is_null() => {
                let old = SelectObject(mem, bmp.into());
                let drawn = DrawIconEx(mem, 0, 0, fi.hIcon, S, S, 0, None, DI_NORMAL);
                let _ = GdiFlush();
                let mut px =
                    std::slice::from_raw_parts(bits as *const u8, (S * S * 4) as usize).to_vec();
                SelectObject(mem, old);
                let _ = DeleteObject(bmp.into());
                drawn.map_err(|e| e.to_string()).and_then(|_| {
                    // Old icons without alpha come out fully transparent: make them opaque.
                    let no_alpha = px.as_chunks::<4>().0.iter().all(|p| p[3] == 0);
                    for p in px.as_chunks_mut::<4>().0 {
                        p.swap(0, 2);
                        if no_alpha && (p[0] | p[1] | p[2]) != 0 {
                            p[3] = 255;
                        }
                    }
                    write_png(out, S as u32, S as u32, &px)
                })
            }
            _ => Err("CreateDIBSection failed".into()),
        };
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        let _ = DestroyIcon(fi.hIcon);
        result
    }
}

fn write_png(out: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(out).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgba).map_err(|e| e.to_string())
}

/// WebView2 Runtime missing: say so in both languages and offer Microsoft's official link.
pub fn webview2_missing() {
    use windows::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONWARNING, MB_YESNO, MessageBoxW};
    let text = HSTRING::from(
        "يحتاج FastKiwi Hub إلى Microsoft Edge WebView2 Runtime، وهو غير موجود على هذا الجهاز.\n\
         هل تفتح رابط تنزيله الرسمي من Microsoft؟\n\n\
         FastKiwi Hub needs the Microsoft Edge WebView2 Runtime, which is not on this PC.\n\
         Open Microsoft's official download link?",
    );
    // SAFETY: a modal message box with NUL-terminated strings.
    let r = unsafe { MessageBoxW(None, &text, w!("FastKiwi Hub"), MB_YESNO | MB_ICONWARNING) };
    if r == IDYES {
        let _ = open_url(crate::links::WEBVIEW2_URL);
    }
}

/// Capture the window's client area as PNG (dev `--dev-shot` only). Uses PrintWindow with
/// PW_RENDERFULLCONTENT, so the window does not have to be on top or focused (the person using
/// the PC is not disturbed); falls back to copying the screen when that gives nothing.
pub fn capture_client_png(hwnd: isize, out: &std::path::Path) -> Result<(u32, u32), String> {
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    // SAFETY: GDI calls on handles created and released here.
    unsafe {
        let mut rc = RECT::default();
        GetClientRect(hwnd, &mut rc).map_err(|e| e.to_string())?;
        let mut origin = POINT { x: 0, y: 0 };
        let _ = ClientToScreen(hwnd, &mut origin);
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        if w <= 0 || h <= 0 {
            return Err("empty window".into());
        }
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        // PW_CLIENTONLY (1) | PW_RENDERFULLCONTENT (2): includes the WebView2 composition.
        let blt = if PrintWindow(hwnd, mem, PRINT_WINDOW_FLAGS(3)).as_bool() {
            Ok(())
        } else {
            BitBlt(
                mem,
                0,
                0,
                w,
                h,
                Some(screen),
                origin.x,
                origin.y,
                SRCCOPY | CAPTUREBLT,
            )
        };
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(
            mem,
            bmp,
            0,
            h as u32,
            Some(px.as_mut_ptr().cast()),
            &mut bi,
            DIB_RGB_COLORS,
        );
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        blt.map_err(|e| e.to_string())?;
        if lines != h {
            return Err("GetDIBits failed".into());
        }
        for p in px.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
            p[3] = 255;
        }
        write_png(out, w as u32, h as u32, &px)?;
        Ok((w as u32, h as u32))
    }
}
