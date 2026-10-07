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

use crate::gamepad::mapping::{Mapping, Origin, RawState};
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

/// Where the Xbox layout of the gamepad a source reads comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadLayout {
    /// Its driver's: games get a copy of it.
    Driver,
    /// A mapping: games get an Xbox 360 controller.
    Mapped(Origin),
    /// None: its buttons are to be set up before games get it.
    Missing,
}

/// The gamepad a source reads, to set up its buttons.
#[derive(Debug, Clone, PartialEq)]
pub struct PadInfo {
    pub name: String,
    /// SDL's GUID, what its mapping is saved under (`gamepad::mapping`).
    pub guid: String,
    pub layout: PadLayout,
    /// Its mapping, when it has one.
    pub mapping: Option<Mapping>,
    /// Its buttons, axes and hats now, by SDL's numbering.
    pub raw: RawState,
    /// It can vibrate itself.
    pub rumble: bool,
}

/// A running source.
pub trait ActiveSource {
    /// One line for the logs and the GUI.
    fn status(&self) -> String;
    fn health(&self) -> SourceHealth;
    /// Names of the gamepads it sees (none once it failed).
    fn gamepads(&self) -> Vec<String>;
    /// The gamepad whose buttons can be set up (proxy).
    fn pad(&self) -> Option<PadInfo> {
        None
    }
    /// What players should know about their gamepads, in plain words.
    fn hint(&self) -> Option<String> {
        None
    }
    fn shutdown(self: Box<Self>);
}
