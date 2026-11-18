//! The bar's global shortcut. Registered with `RegisterHotKey` only: Windows tells the Hub
//! when that one key combination is pressed, and nothing else. There is no keyboard hook, so
//! the Hub never sees any other key you type (it is not a keylogger).

/// The choices offered in Settings, as stored (`ctrl+alt+space`), plus `off`.
pub const CHOICES: [&str; 5] = [
    "ctrl+alt+space",
    "alt+shift+space",
    "ctrl+shift+space",
    "ctrl+alt+k",
    "off",
];
pub const DEFAULT: &str = "ctrl+alt+space";

pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;
pub const MOD_NOREPEAT: u32 = 0x4000;

/// `ctrl+alt+space` → (modifiers, virtual key). At least one of Ctrl/Alt is required, so a
/// plain key is never taken from other programs.
pub fn parse(s: &str) -> Option<(u32, u32)> {
    let mut mods = 0;
    let mut key = None;
    for part in s.to_ascii_lowercase().split('+') {
        match part.trim() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" => mods |= MOD_WIN,
            "space" => key = Some(0x20),
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => {
                key = Some(k.as_bytes()[0].to_ascii_uppercase() as u32)
            }
            k if k.len() >= 2 && k.starts_with('f') => {
                let n: u32 = k[1..].parse().ok()?;
                (1..=12).contains(&n).then_some(())?;
                key = Some(0x70 + n - 1);
            }
            _ => return None,
        }
    }
    (mods & (MOD_CONTROL | MOD_ALT) != 0).then_some(())?;
    key.map(|k| (mods, k))
}

/// What happened when the shortcut was (re)registered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum Status {
    Ok { keys: String },
    Off,
    /// Another program already uses this combination.
    Taken { keys: String },
    Failed { keys: String },
}

#[cfg(windows)]
pub mod windows {
    use super::Status;
    use std::sync::mpsc;
    use windows::Win32::Foundation::{GetLastError, LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY,
    };

    const ID: i32 = 1;
    const WM_REBIND: u32 = WM_APP + 1;

    /// A thread that owns the registration: `rebind` changes it, `on_press` runs on each press.
    pub struct Hotkey {
        thread: u32,
        tx: mpsc::Sender<String>,
    }

    impl Hotkey {
        pub fn start(
            keys: &str,
            on_press: impl Fn() + Send + 'static,
            on_status: impl Fn(Status) + Send + 'static,
        ) -> Option<Hotkey> {
            let (tx, rx) = mpsc::channel::<String>();
            let (tid_tx, tid_rx) = mpsc::channel::<u32>();
            let first = keys.to_string();
            std::thread::spawn(move || {
                // SAFETY: Win32 calls on this thread's own message queue.
                unsafe {
                    let mut msg = MSG::default();
                    // Make sure the queue exists before anyone posts to it.
                    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                    let _ = tid_tx.send(GetCurrentThreadId());
                    let mut registered = false;
                    let bind = |keys: &str, registered: &mut bool| {
                        if *registered {
                            let _ = UnregisterHotKey(None, ID);
                            *registered = false;
                        }
                        let status = match super::parse(keys) {
                            None => Status::Off,
                            Some((mods, vk)) => {
                                match RegisterHotKey(
                                    None,
                                    ID,
                                    HOT_KEY_MODIFIERS(mods | super::MOD_NOREPEAT),
                                    vk,
                                ) {
                                    Ok(()) => {
                                        *registered = true;
                                        Status::Ok { keys: keys.into() }
                                    }
                                    // ERROR_HOTKEY_ALREADY_REGISTERED
                                    Err(_) if GetLastError().0 == 1409 => {
                                        Status::Taken { keys: keys.into() }
                                    }
                                    Err(_) => Status::Failed { keys: keys.into() },
                                }
                            }
                        };
                        on_status(status);
                    };
                    bind(&first, &mut registered);
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == ID {
                            on_press();
                        } else if msg.message == WM_REBIND {
                            while let Ok(k) = rx.try_recv() {
                                bind(&k, &mut registered);
                            }
                        }
                    }
                    if registered {
                        let _ = UnregisterHotKey(None, ID);
                    }
                }
            });
            let thread = tid_rx.recv().ok()?;
            Some(Hotkey { thread, tx })
        }

        pub fn rebind(&self, keys: &str) {
            let _ = self.tx.send(keys.to_string());
            // SAFETY: posting to the hotkey thread's own queue.
            unsafe {
                let _ = PostThreadMessageW(self.thread, WM_REBIND, WPARAM(0), LPARAM(0));
            }
        }
    }
}
