//! Linux sources: the "proxy" (uinput virtual gamepad) and "ebpf" (kernel
//! probe) methods, reading the gamepads through evdev. Root-only work goes
//! through the privileged helper, shared by both.

mod ebpf;
mod proxy;

pub use ebpf::load_probe;

use std::path::Path;
use std::sync::Arc;

use evdev::{Device, EventSummary, EventType, FFEffectCode};

use super::{ActiveSource, EventSender, SourceKind, SourceOptions};
use crate::config::SourceChoice;
use crate::gamepad::AxisRanges;
use crate::platform::linux::helper::client::Helper;
use crate::platform::linux::is_root;
use crate::rumble::{EffectKind, Envelope, Effect};
use ebpf::EbpfSource;
use proxy::{Hide, ProxySource};

/// Starts the sources; owns the privileged helper they share.
pub struct Sources {
    helper: Arc<Helper>,
}

impl Sources {
    pub fn new() -> Self {
        Self { helper: Helper::new() }
    }

    /// Must be called from within a tokio runtime. `SourceChoice::None` is not a source.
    pub fn start(&self, choice: SourceChoice, opts: &SourceOptions, tx: EventSender) -> anyhow::Result<Box<dyn ActiveSource>> {
        Ok(match choice {
            SourceChoice::Proxy => {
                let hide = match (opts.hide, is_root()) {
                    (false, _) => Hide::No,
                    (true, true) => Hide::Local,
                    (true, false) => Hide::Helper(self.helper.clone()),
                };
                Box::new(ProxySource::start(opts.device.as_deref(), opts.passthrough, hide, tx)?)
            }
            SourceChoice::Ebpf => Box::new(EbpfSource::start(tx, &self.helper)?),
            SourceChoice::None => anyhow::bail!("no source selected"),
        })
    }

    /// Stops the privileged helper, which restores what it changed.
    pub fn shutdown(&self) {
        self.helper.shutdown();
    }
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

/// Axis value ranges, from the device's absinfo.
pub fn axis_ranges(dev: &Device) -> AxisRanges {
    AxisRanges::new(dev.get_absinfo().map(|it| it.map(|(code, info)| (code.0, (info.minimum(), info.maximum()))).collect::<Vec<_>>()).unwrap_or_default())
}

/// Converts an effect received by the uinput virtual gamepad.
pub fn effect_from_evdev(data: &evdev::FFEffectData) -> Effect {
    use evdev::FFEffectKind as K;
    let env = |e: &evdev::FFEnvelope| Envelope {
        attack_length: e.attack_length,
        attack_level: e.attack_level,
        fade_length: e.fade_length,
        fade_level: e.fade_level,
    };
    let kind = match &data.kind {
        K::Rumble { strong_magnitude, weak_magnitude } => EffectKind::Rumble { strong: *strong_magnitude, weak: *weak_magnitude },
        K::Periodic { magnitude, envelope, .. } => EffectKind::Periodic { magnitude: *magnitude, envelope: env(envelope) },
        K::Constant { level, envelope } => EffectKind::Constant { level: *level, envelope: env(envelope) },
        K::Ramp { start_level, end_level, envelope } => {
            EffectKind::Ramp { start: *start_level, end: *end_level, envelope: env(envelope) }
        }
        _ => EffectKind::Unsupported,
    };
    Effect { kind, length_ms: data.replay.length, delay_ms: data.replay.delay }
}
