//! Windows-only helpers: launching an installed app (ShellExecute, fire and forget) and the
//! dev-only window capture used to take the prototype's screenshots.

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, ClientToScreen,
    CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
    GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
};
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
            if !path.is_absolute()
                || !p.to_ascii_lowercase().ends_with(".exe")
                || !path.is_file()
            {
                return Err("not an exe on this PC".into());
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

/// Capture the window's client area from the screen as PNG (dev `--dev-shot` only). The
/// window must be visible and on top; the caller makes it so.
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
        let blt = BitBlt(mem, 0, 0, w, h, Some(screen), origin.x, origin.y, SRCCOPY | CAPTUREBLT);
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
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2);
            p[3] = 255;
        }
        let file = std::fs::File::create(out).map_err(|e| e.to_string())?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(&px).map_err(|e| e.to_string())?;
        Ok((w as u32, h as u32))
    }
}
