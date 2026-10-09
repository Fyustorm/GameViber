//! Windows: system-wide hot keys (`RegisterHotKey`), held back from the game,
//! on a thread of their own that receives them. Their keys are the preferred
//! ones; a key another program already holds is left without one.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_HOTKEY, WM_QUIT, WM_USER};

use super::{Action, Status};

/// The keys can only be changed here, not in the system's settings.
pub const CONFIGURABLE: bool = false;

pub struct Shortcuts {
    actions: Mutex<Receiver<Action>>,
    status: Arc<Mutex<Status>>,
    /// The thread receiving the keys, to stop it.
    thread: Option<u32>,
}

impl Shortcuts {
    pub fn start() -> Self {
        let status = Arc::new(Mutex::new(Status::Connecting));
        let (tx, actions) = mpsc::channel();
        let (thread_tx, thread_rx) = mpsc::channel();
        let st = status.clone();
        let spawned = std::thread::Builder::new().name("shortcuts".into()).spawn(move || {
            // SAFETY: the message loop of this thread only; its keys are unregistered before it ends.
            unsafe {
                let mut msg = MSG::default();
                // Makes the thread's message queue, so that the quit message waits in it.
                let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
                let _ = thread_tx.send(GetCurrentThreadId());
                let mut bound = Vec::new();
                for (i, action) in Action::ALL.into_iter().enumerate() {
                    let keys = match parse(action.preferred()) {
                        Some((modifiers, key)) if RegisterHotKey(None, i as i32 + 1, modifiers | MOD_NOREPEAT, key).is_ok() => {
                            text(action.preferred())
                        }
                        _ => {
                            log::warn!("shortcuts: {} is taken by another program", text(action.preferred()));
                            String::new()
                        }
                    };
                    bound.push((action, keys));
                }
                log::info!("keyboard shortcuts: {bound:?}");
                *st.lock().unwrap() = Status::Ready(bound);
                // 0: WM_QUIT; -1: an error.
                while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                    if msg.message == WM_HOTKEY {
                        if let Some(action) = Action::ALL.get((msg.wParam.0 as usize).wrapping_sub(1)) {
                            let _ = tx.send(*action);
                        }
                    }
                }
                for i in 0..Action::ALL.len() {
                    let _ = UnregisterHotKey(None, i as i32 + 1);
                }
            }
        });
        let thread = match spawned {
            Ok(_) => thread_rx.recv().ok(),
            Err(e) => {
                *status.lock().unwrap() = Status::Failed(e.to_string());
                None
            }
        };
        Self { actions: Mutex::new(actions), status, thread }
    }

    /// Actions whose keys were pressed since the last call.
    pub fn poll(&self) -> Vec<Action> {
        self.actions.lock().unwrap().try_iter().collect()
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// Nothing to open: see `CONFIGURABLE`.
    pub fn configure(&self) {}
}

impl Drop for Shortcuts {
    fn drop(&mut self) {
        if let Some(thread) = self.thread {
            // SAFETY: plain call; the thread ends its loop on WM_QUIT.
            let _ = unsafe { PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
        }
    }
}

/// "CTRL+ALT+X": the modifiers and the key's virtual-key code (a letter or a digit).
fn parse(keys: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    let mut key = None;
    for part in keys.split('+') {
        match part.trim().to_ascii_uppercase().as_str() {
            "CTRL" => modifiers |= MOD_CONTROL,
            "ALT" => modifiers |= MOD_ALT,
            "SHIFT" => modifiers |= MOD_SHIFT,
            "LOGO" | "WIN" => modifiers |= MOD_WIN,
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => key = Some(u32::from(k.as_bytes()[0])),
            _ => return None,
        }
    }
    key.map(|key| (modifiers, key))
}

/// "CTRL+ALT+X" as Windows writes keys: "Ctrl+Alt+X".
fn text(keys: &str) -> String {
    keys.split('+')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map(|first| first.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_read_and_written_like_windows() {
        assert_eq!(parse("CTRL+ALT+X"), Some((MOD_CONTROL | MOD_ALT, u32::from(b'X'))));
        assert_eq!(parse("CTRL+ALT+F1"), None, "letters and digits only");
        assert_eq!(text("CTRL+ALT+M"), "Ctrl+Alt+M");
        for action in Action::ALL {
            assert!(parse(action.preferred()).is_some(), "{action:?}");
        }
    }
}
