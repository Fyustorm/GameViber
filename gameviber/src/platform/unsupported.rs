//! Systems without a backend yet: plain directories, UTC time, Ctrl+C, and
//! no privileged helper.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("GameViber")).unwrap_or_else(|| home().join(".config").join("gameviber"))
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("GameViber"))
        .unwrap_or_else(|| home().join(".local").join("share").join("gameviber"))
}

pub fn chown_to_caller(_path: &Path) {}

/// "2026-10-03 21:14:05", in UTC: the local time zone needs the OS.
pub fn local_time() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or_default();
    // Days to a civil date (Howard Hinnant's `civil_from_days`).
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// Ctrl+C stops the engine cleanly.
pub struct StopSignals;

impl StopSignals {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self)
    }

    pub async fn recv(&mut self) {
        let _ = tokio::signal::ctrl_c().await;
    }
}

pub fn privileged_subcommand() -> Option<anyhow::Result<()>> {
    None
}

pub fn window_expected() {}

pub fn window_created(_cc: &eframe::CreationContext) {}

pub fn window_focused(_focused: bool) {}

pub fn window_closing() {}

