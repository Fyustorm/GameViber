//! GameViber in-game overlay: an implicit Vulkan layer, like MangoHud, that
//! draws the active mode, the toy output, mode gauges and events over the
//! game. It gets everything to show from GameViber over a local socket
//! (see `gameviber_common::overlay`) and shows nothing when GameViber is
//! not running.
//!
//! The layer is enabled per game with `GAMEVIBER_OVERLAY=1`, or for every
//! game if the player chose so in GameViber; `DISABLE_GAMEVIBER_OVERLAY=1`
//! turns it off.

mod client;
mod hud;
mod layer;
mod render;

/// Writes to stderr when `GAMEVIBER_OVERLAY_DEBUG` is set: games own stdout/stderr.
pub(crate) fn log(message: &str) {
    if std::env::var_os("GAMEVIBER_OVERLAY_DEBUG").is_some() {
        eprintln!("[gameviber-overlay] {message}");
    }
}
