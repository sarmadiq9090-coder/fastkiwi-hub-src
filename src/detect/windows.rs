//! The real [`SystemView`]: this PC's registry, files and Store packages, read only.
//!
//! - Registry keys are opened with `KEY_READ` (plus a WOW64 view flag) and nothing else; the Hub
//!   has no code that creates, sets or deletes a key or value (a test greps the sources for it).
//! - Store packages come from `PackageManager.FindPackagesForUser("", pfn)` (the current user,
//!   no admin rights) and are only read.

use super::{FileFacts, Hive, PackageFacts, RegValue, SystemView};
use std::collections::BTreeMap;
use std::path::Path;
use windows::Management::Deployment::PackageManager;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    REG_DWORD, REG_EXPAND_SZ, REG_SZ, RegCloseKey, RegEnumValueW, RegOpenKeyExW,
};
use windows::core::{HSTRING, PWSTR, w};

/// Stop counting a folder's size after this many entries (keeps a scan fast).
const DIR_SIZE_LIMIT: usize = 20_000;

#[derive(Debug, Default)]
pub struct WindowsView;

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the handle came from RegOpenKeyExW and is closed once.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn open_read(hive: Hive, sub: &str) -> Option<Key> {
    let (root, view) = match hive {
        Hive::Hkcu => (HKEY_CURRENT_USER, KEY_WOW64_64KEY),
        Hive::Hklm => (HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY),
        Hive::Hklm32 => (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY),
    };
    let mut h = HKEY::default();
    // SAFETY: valid root handle, NUL-terminated sub key, out pointer to a local.
    let r = unsafe { RegOpenKeyExW(root, &HSTRING::from(sub), Some(0), KEY_READ | view, &mut h) };
    (r == ERROR_SUCCESS).then_some(Key(h))
}

fn read_values(key: &Key) -> BTreeMap<String, RegValue> {
    let mut out = BTreeMap::new();
    let mut name = vec![0u16; 16_384];
    let mut data = vec![0u8; 64 * 1024];
    for index in 0..1024u32 {
        let mut name_len = name.len() as u32;
        let mut data_len = data.len() as u32;
        let mut ty = 0u32;
        // SAFETY: buffers and their lengths are valid for the call.
        let r = unsafe {
            RegEnumValueW(
                key.0,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut ty),
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
        if r != ERROR_SUCCESS {
            // ERROR_NO_MORE_ITEMS ends the list; ERROR_MORE_DATA (a huge value) is skipped.
            if r.0 == 234 {
                continue;
            }
            break;
        }
        let n = String::from_utf16_lossy(&name[..name_len as usize]);
        let bytes = &data[..data_len as usize];
        let value = if ty == REG_SZ.0 || ty == REG_EXPAND_SZ.0 {
            let wide: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .take_while(|&c| c != 0)
                .collect();
            RegValue::Str(String::from_utf16_lossy(&wide))
        } else if ty == REG_DWORD.0 && bytes.len() == 4 {
            RegValue::Dword(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        } else {
            continue;
        };
        out.insert(n, value);
    }
    out
}

/// The `a.b.c` (or `a.b.c.d` when d ≠ 0) file version of an exe, if it has a version resource.
pub fn file_version(path: &str) -> Option<String> {
    let p = HSTRING::from(path);
    // SAFETY: plain Win32 calls on a local buffer of the size the system asked for.
    unsafe {
        let size = GetFileVersionInfoSizeW(&p, None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        GetFileVersionInfoW(&p, None, size, buf.as_mut_ptr().cast()).ok()?;
        let mut info: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(buf.as_ptr().cast(), w!("\\"), &mut info, &mut len).as_bool()
            || info.is_null()
            || (len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
        {
            return None;
        }
        let fi = &*(info as *const VS_FIXEDFILEINFO);
        if fi.dwSignature != 0xFEEF_04BD {
            return None;
        }
        let (a, b, c, d) = (
            fi.dwFileVersionMS >> 16,
            fi.dwFileVersionMS & 0xFFFF,
            fi.dwFileVersionLS >> 16,
            fi.dwFileVersionLS & 0xFFFF,
        );
        Some(if d == 0 {
            format!("{a}.{b}.{c}")
        } else {
            format!("{a}.{b}.{c}.{d}")
        })
    }
}

fn dir_size_capped(dir: &Path, budget: &mut usize) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0;
    for e in rd.flatten() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            total += dir_size_capped(&e.path(), budget);
        } else if let Ok(m) = e.metadata() {
            total += m.len();
        }
    }
    total
}

impl SystemView for WindowsView {
    fn env(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }

    fn read_text(&self, path: &str) -> Option<String> {
        let m = std::fs::metadata(path).ok()?;
        if !m.is_file() || m.len() > 4096 {
            return None;
        }
        std::fs::read_to_string(path).ok()
    }

    fn file(&self, path: &str) -> Option<FileFacts> {
        let m = std::fs::metadata(path).ok()?;
        m.is_file().then(|| FileFacts {
            size: m.len(),
            version: file_version(path),
        })
    }

    fn list_dir(&self, path: &str) -> Vec<String> {
        std::fs::read_dir(path)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn read_link(&self, path: &str) -> Option<String> {
        let t = std::fs::read_link(path).ok()?;
        let t = if t.is_relative() {
            Path::new(path).parent()?.join(t)
        } else {
            t
        };
        let s = t.to_string_lossy().into_owned();
        Some(s.strip_prefix("\\\\?\\").map(str::to_string).unwrap_or(s))
    }

    fn reg_values(&self, hive: Hive, key: &str) -> Option<BTreeMap<String, RegValue>> {
        open_read(hive, key).map(|k| read_values(&k))
    }

    fn store_package(&self, pfn: &str) -> Option<PackageFacts> {
        let pm = PackageManager::new().ok()?;
        let found = pm
            .FindPackagesByUserSecurityIdPackageFamilyName(&HSTRING::new(), &HSTRING::from(pfn))
            .ok()?;
        let pkg = found.into_iter().next()?;
        let v = pkg.Id().ok()?.Version().ok()?;
        let location = pkg.InstalledPath().ok()?.to_string();
        Some(PackageFacts {
            version: format!("{}.{}.{}.{}", v.Major, v.Minor, v.Build, v.Revision),
            location,
        })
    }

    fn dir_size(&self, path: &str) -> Option<u64> {
        let mut budget = DIR_SIZE_LIMIT;
        let s = dir_size_capped(Path::new(path), &mut budget);
        (s > 0).then_some(s)
    }
}
