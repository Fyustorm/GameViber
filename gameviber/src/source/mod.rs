//! Event sources: they observe what the game sends to the gamepad and turn
//! it into `SourceEvent`s, identical whatever the interception method.

pub mod ebpf;
pub mod proxy;

use std::path::Path;

use evdev::{Device, EventSummary, EventType, FFEffectCode};
use tokio::sync::mpsc::UnboundedSender;

use crate::gamepad::AxisRanges;
use crate::rumble::Effect;

#[derive(Debug, Clone)]
pub enum SourceKind {
    /// Effect uploaded (new or updated) by the game.
    Upload { id: i16, effect: Effect },
    Erase { id: i16 },
    /// EV_FF play (count > 0) or stop (count = 0).
    Play { id: i16, count: i32 },
    Gain(u16),
    /// Key pressed / released (evdev KEY_* / BTN_* code).
    Button { code: u16, pressed: bool },
    /// Axis (evdev ABS_* code), normalized with the device's range.
    Axis { code: u16, value: f64 },
}

#[derive(Debug, Clone)]
pub struct SourceEvent {
    /// Gamepad the event belongs to (/dev/input/eventN).
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

/// A gamepad: face buttons and rumble force feedback.
pub fn is_rumble_gamepad(dev: &Device) -> bool {
    let has_rumble = dev.supported_ff().is_some_and(|ff| ff.contains(FFEffectCode::FF_RUMBLE));
    let has_pad_buttons = dev.supported_keys().is_some_and(|k| k.contains(evdev::KeyCode::BTN_SOUTH));
    has_rumble && has_pad_buttons
}

/// Every evdev device with force feedback, sorted by path.
pub fn list_ff_devices() -> Vec<(String, Device)> {
    let mut found: Vec<_> = evdev::enumerate()
        .filter(|(_, dev)| dev.supported_events().contains(EventType::FORCEFEEDBACK))
        .map(|(path, dev)| (path.to_string_lossy().into_owned(), dev))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

pub fn find_gamepad(path: Option<&Path>) -> anyhow::Result<(String, Device)> {
    if let Some(path) = path {
        return Ok((path.to_string_lossy().into_owned(), Device::open(path)?));
    }
    list_ff_devices()
        .into_iter()
        .find(|(_, dev)| is_rumble_gamepad(dev))
        .ok_or_else(|| anyhow::anyhow!("no gamepad with rumble found"))
}

/// Translates an evdev event read from a gamepad (FF upload/erase excluded).
pub fn translate_input(event: evdev::InputEvent, ranges: &AxisRanges) -> Option<SourceKind> {
    match event.destructure() {
        EventSummary::ForceFeedback(_, FFEffectCode::FF_GAIN, value) => Some(SourceKind::Gain(value as u16)),
        // Below FF_GAIN (= FF_MAX_EFFECTS) the code is an effect id; above, a setting
        // (FF_GAIN, FF_AUTOCENTER).
        EventSummary::ForceFeedback(_, code, value) if code.0 < FFEffectCode::FF_GAIN.0 => {
            Some(SourceKind::Play { id: code.0 as i16, count: value })
        }
        // value 2 = autorepeat
        EventSummary::Key(_, code, value) if value != 2 => Some(SourceKind::Button { code: code.0, pressed: value == 1 }),
        EventSummary::AbsoluteAxis(_, code, value) => {
            Some(SourceKind::Axis { code: code.0, value: ranges.normalize(code.0, value) })
        }
        _ => None,
    }
}
