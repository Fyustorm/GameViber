//! Event sources: they observe what the game sends to the gamepad and turn
//! it into `SourceEvent`s, identical whatever the interception method and the
//! OS. The methods themselves are OS backends (`linux`: proxy and eBPF), started
//! through `Sources`.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub use linux::Sources;
#[cfg(not(target_os = "linux"))]
mod unsupported;
#[cfg(not(target_os = "linux"))]
pub use unsupported::Sources;

use std::path::PathBuf;

use tokio::sync::mpsc::UnboundedSender;

use crate::rumble::Effect;

#[derive(Debug, Clone)]
pub enum SourceKind {
    /// Effect uploaded (new or updated) by the game.
    Upload { id: i16, effect: Effect },
    Erase { id: i16 },
    /// Play (count > 0) or stop (count = 0), like Linux's EV_FF.
    Play { id: i16, count: i32 },
    Gain(u16),
    /// Key pressed / released (`gamepad::codes` KEY_* / BTN_* code).
    Button { code: u16, pressed: bool },
    /// Axis (`gamepad::codes` ABS_* code), normalized with the device's range.
    Axis { code: u16, value: f64 },
    /// The gamepad was unplugged: its effects are gone.
    Removed,
}

#[derive(Debug, Clone)]
pub struct SourceEvent {
    /// Gamepad the event belongs to (its device path on Linux: /dev/input/eventN).
    pub device: String,
    pub kind: SourceKind,
}

pub type EventSender = UnboundedSender<SourceEvent>;

/// Whether a source is capturing, in terms the GUI can show to players.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum SourceHealth {
    /// No source selected (simulator only).
    #[default]
    Off,
    /// Starting up, e.g. waiting for the user's password.
    Waiting(String),
    Working,
    Failed(String),
}

/// How a source is started.
#[derive(Debug, Clone, Default)]
pub struct SourceOptions {
    /// The gamepad to use (else: found automatically).
    pub device: Option<PathBuf>,
    /// Proxy: the rumble also goes to the real gamepad.
    pub passthrough: bool,
    /// Proxy: the real gamepad is hidden from games.
    pub hide: bool,
}

/// A running source.
pub trait ActiveSource {
    /// One line for the logs and the GUI.
    fn status(&self) -> String;
    fn health(&self) -> SourceHealth;
    /// Names of the gamepads it sees (none once it failed).
    fn gamepads(&self) -> Vec<String>;
    fn shutdown(self: Box<Self>);
}
