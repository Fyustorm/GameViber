//! Linux: XDG directories (of the sudo caller when run through sudo), the
//! privileged helper started through pkexec, and the device hider it uses,
//! the desktop's portals and its file dialogs.

mod files;
pub mod helper;
pub mod hider;
pub mod portal;

pub use files::{open_file, open_files, save_file};

use std::ffi::CStr;
use std::path::{Path, PathBuf};

use tokio::signal::unix::{signal, Signal, SignalKind};

/// Home of the user who started us: the sudo caller rather than root.
pub fn user_home() -> PathBuf {
    if let Ok(user) = std::env::var("SUDO_USER") {
        if let Ok(name) = std::ffi::CString::new(user) {
            // SAFETY: getpwnam returns a pointer to static storage or null.
            let pw = unsafe { libc::getpwnam(name.as_ptr()) };
            if !pw.is_null() {
                let dir = unsafe { CStr::from_ptr((*pw).pw_dir) };
                return PathBuf::from(dir.to_string_lossy().into_owned());
            }
        }
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// `~/.config/gameviber`.
pub fn config_dir() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if std::env::var_os("SUDO_USER").is_none() => PathBuf::from(dir).join("gameviber"),
        _ => user_home().join(".config").join("gameviber"),
    }
}

/// `$XDG_DATA_HOME`, `~/.local/share` by default.
pub fn data_home() -> PathBuf {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(dir) if std::env::var_os("SUDO_USER").is_none() => PathBuf::from(dir),
        _ => user_home().join(".local").join("share"),
    }
}

/// `~/.local/share/gameviber`.
pub fn data_dir() -> PathBuf {
    data_home().join("gameviber")
}

/// `$XDG_RUNTIME_DIR`, or a directory of our own in `/tmp` without it.
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("/tmp/gameviber-{}", uid())))
}

/// The Steam app id of a running process, as Steam sets it in the game's
/// environment (`SteamAppId`; Proton games too). None for a game Steam did not
/// start, or a process of another user.
pub fn steam_app_id(pid: u32) -> Option<u32> {
    let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    app_id_in(&environ)
}

/// `SteamAppId` in a process environment (`NAME=value` entries ending in NUL); 0 is no game.
fn app_id_in(environ: &[u8]) -> Option<u32> {
    environ
        .split(|b| *b == 0)
        .find_map(|entry| entry.strip_prefix(b"SteamAppId="))
        .and_then(|value| std::str::from_utf8(value).ok()?.parse().ok())
        .filter(|id| *id != 0)
}

pub fn uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

pub fn is_root() -> bool {
    // SAFETY: geteuid cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// Gives files created as root back to the sudo caller.
pub fn chown_to_caller(path: &Path) {
    let (Ok(uid), Ok(gid)) = (std::env::var("SUDO_UID"), std::env::var("SUDO_GID")) else { return };
    let (Ok(uid), Ok(gid)) = (uid.parse(), gid.parse()) else { return };
    let _ = std::os::unix::fs::chown(path, Some(uid), Some(gid));
}

/// "2026-10-03 21:14:05", local time.
pub fn local_time() -> String {
    // SAFETY: localtime_r only writes the tm struct it is given.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return format!("{now}");
        }
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

/// SIGTERM and SIGINT, which stop the engine cleanly.
pub struct StopSignals {
    term: Signal,
    int: Signal,
}

impl StopSignals {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self { term: signal(SignalKind::terminate())?, int: signal(SignalKind::interrupt())? })
    }

    pub async fn recv(&mut self) {
        tokio::select! {
            _ = self.term.recv() => {}
            _ = self.int.recv() => {}
        }
    }
}

/// `gameviber helper`: runs the privileged helper instead of the app.
pub fn privileged_subcommand() -> Option<anyhow::Result<()>> {
    (std::env::args().nth(1).as_deref() == Some(helper::SUBCOMMAND)).then(|| {
        crate::logging::init(false);
        helper::server::run()
    })
}

/// A GUI window is coming: the password dialog waits for it (see `helper::dialog`).
pub fn window_expected() {
    helper::dialog::expect_window();
}

pub fn window_created(cc: &eframe::CreationContext) {
    helper::dialog::set_window(cc);
}

/// Called every frame.
pub fn window_focused(focused: bool) {
    helper::dialog::set_focused(focused);
}

/// Must be called before the window goes away.
pub fn window_closing() {
    helper::dialog::forget_window();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_app_id_is_read_from_the_environment() {
        assert_eq!(app_id_in(b"HOME=/home/a\0SteamAppId=2679460\0SteamGameId=2679460\0"), Some(2679460));
        assert_eq!(app_id_in(b"SteamAppId=0\0"), None, "not a Steam game");
        assert_eq!(app_id_in(b"HOME=/home/a\0"), None);
    }
}
