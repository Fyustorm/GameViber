//! Sources d'événements : elles observent ce que le jeu envoie à la manette
//! et le traduisent en `SourceEvent`, identiques quelle que soit la source.

pub mod ebpf;
pub mod proxy;

use std::path::Path;

use evdev::{Device, EventSummary, EventType, FFEffectCode};
use tokio::sync::mpsc::UnboundedSender;

use crate::rumble::Effect;

#[derive(Debug, Clone)]
pub enum SourceKind {
    /// Effet téléversé (nouveau ou mis à jour) par le jeu.
    Upload { id: i16, effect: Effect },
    Erase { id: i16 },
    /// EV_FF play (count > 0) ou stop (count = 0).
    Play { id: i16, count: i32 },
    Gain(u16),
    /// Bouton pressé / relâché (code evdev KEY_* / BTN_*).
    Button { code: u16, pressed: bool },
    /// Axe (code evdev ABS_*), valeur brute.
    Axis { code: u16, value: i32 },
}

#[derive(Debug, Clone)]
pub struct SourceEvent {
    /// Manette concernée (chemin /dev/input/eventN).
    pub device: String,
    pub kind: SourceKind,
}

pub type EventSender = UnboundedSender<SourceEvent>;

/// Une manette : touches de jeu + retour de force de type rumble.
pub fn is_rumble_gamepad(dev: &Device) -> bool {
    let has_rumble = dev.supported_ff().is_some_and(|ff| ff.contains(FFEffectCode::FF_RUMBLE));
    let has_pad_buttons = dev.supported_keys().is_some_and(|k| k.contains(evdev::KeyCode::BTN_SOUTH));
    has_rumble && has_pad_buttons
}

/// Tous les devices evdev avec retour de force.
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
        .ok_or_else(|| anyhow::anyhow!("aucune manette avec rumble trouvée"))
}

/// Traduit un événement evdev lu sur une manette (hors FF upload/erase).
pub fn translate_input(event: evdev::InputEvent) -> Option<SourceKind> {
    match event.destructure() {
        EventSummary::ForceFeedback(_, FFEffectCode::FF_GAIN, value) => Some(SourceKind::Gain(value as u16)),
        EventSummary::ForceFeedback(_, code, value) if code.0 < FFEffectCode::FF_GAIN.0 => {
            // Sous FF_GAIN (= FF_MAX_EFFECTS) le code est un id d'effet ; au-delà, un réglage
            // (FF_GAIN, FF_AUTOCENTER).
            Some(SourceKind::Play { id: code.0 as i16, count: value })
        }
        EventSummary::Key(_, code, value) if value != 2 => {
            Some(SourceKind::Button { code: code.0, pressed: value == 1 })
        }
        EventSummary::AbsoluteAxis(_, code, value) => Some(SourceKind::Axis { code: code.0, value }),
        _ => None,
    }
}
