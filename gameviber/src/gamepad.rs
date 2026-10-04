//! Gamepad normalization: evdev key/axis codes to the Xbox-layout names
//! exposed to modes (A, B, LB, DPAD_UP, LX, LT...), plus idle tracking and
//! detection of the panic, "mark this moment" and "capture the screen" combos.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use evdev::{AbsoluteAxisCode as Abs, KeyCode as Key};

pub const BUTTONS: [&str; 17] = [
    "A", "B", "X", "Y", "LB", "RB", "BACK", "START", "GUIDE", "LS", "RS", "DPAD_UP", "DPAD_DOWN",
    "DPAD_LEFT", "DPAD_RIGHT", "LT", "RT",
];
pub const AXES: [&str; 6] = ["LX", "LY", "RX", "RY", "LT", "RT"];

/// Normalized sticks below this magnitude count as centered.
pub const DEADZONE: f64 = 0.1;
/// Analog triggers also generate button events around this value.
const TRIGGER_PRESS: f64 = 0.5;
/// The panic combo held this long triggers the panic stop.
pub const PANIC_HOLD_SECS: f64 = 0.5;
/// Default panic combo.
pub const DEFAULT_PANIC_COMBO: [&str; 2] = ["BACK", "START"];
/// The mark combo held this long marks the moment (once per hold).
pub const MARK_HOLD_SECS: f64 = 0.3;
/// Default combo marking a moment that felt wrong.
pub const DEFAULT_MARK_COMBO: [&str; 2] = ["BACK", "RS"];
/// Default combo capturing the game's image into its profile.
pub const DEFAULT_CAPTURE_COMBO: [&str; 2] = ["BACK", "LS"];
/// A combo needs at least this many buttons, so that no single press triggers it.
pub const PANIC_COMBO_MIN: usize = 2;

/// Known button names of a combo, or None if it has fewer than
/// `PANIC_COMBO_MIN` distinct buttons.
pub fn parse_combo(names: &[String]) -> Option<BTreeSet<&'static str>> {
    let combo: BTreeSet<&'static str> =
        names.iter().filter_map(|n| BUTTONS.iter().find(|b| **b == n.as_str()).copied()).collect();
    (combo.len() >= PANIC_COMBO_MIN && combo.len() == names.len()).then_some(combo)
}

/// "BACK + START"
pub fn combo_text(names: &[String]) -> String {
    names.join(" + ")
}

fn button_name(code: u16) -> Option<&'static str> {
    Some(match Key(code) {
        Key::BTN_SOUTH => "A",
        Key::BTN_EAST => "B",
        // xpad reports the Xbox X/Y buttons as BTN_X/BTN_Y (aliases of NORTH/WEST).
        Key::BTN_NORTH => "X",
        Key::BTN_WEST => "Y",
        Key::BTN_TL => "LB",
        Key::BTN_TR => "RB",
        Key::BTN_SELECT => "BACK",
        Key::BTN_START => "START",
        Key::BTN_MODE => "GUIDE",
        Key::BTN_THUMBL => "LS",
        Key::BTN_THUMBR => "RS",
        Key::BTN_DPAD_UP | Key::BTN_TRIGGER_HAPPY3 => "DPAD_UP",
        Key::BTN_DPAD_DOWN | Key::BTN_TRIGGER_HAPPY4 => "DPAD_DOWN",
        Key::BTN_DPAD_LEFT | Key::BTN_TRIGGER_HAPPY1 => "DPAD_LEFT",
        Key::BTN_DPAD_RIGHT | Key::BTN_TRIGGER_HAPPY2 => "DPAD_RIGHT",
        Key::BTN_TL2 => "LT",
        Key::BTN_TR2 => "RT",
        _ => return None,
    })
}

/// Axis value range of one device, read from its absinfo.
#[derive(Debug, Clone, Default)]
pub struct AxisRanges(HashMap<u16, (i32, i32)>);

impl AxisRanges {
    pub fn from_device(dev: &evdev::Device) -> Self {
        let ranges = dev
            .get_absinfo()
            .map(|it| it.map(|(code, info)| (code.0, (info.minimum(), info.maximum()))).collect())
            .unwrap_or_default();
        Self(ranges)
    }

    /// Sticks and hats to -1..1, triggers to 0..1.
    pub fn normalize(&self, code: u16, value: i32) -> f64 {
        let (min, max) = self.0.get(&code).copied().unwrap_or((-32768, 32767));
        if max <= min {
            return 0.0;
        }
        let unit = (value - min) as f64 / (max - min) as f64;
        match Abs(code) {
            Abs::ABS_Z | Abs::ABS_RZ | Abs::ABS_GAS | Abs::ABS_BRAKE => unit.clamp(0.0, 1.0),
            _ => (unit * 2.0 - 1.0).clamp(-1.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ButtonEvent {
    pub name: &'static str,
    pub pressed: bool,
}

/// Current state of the (merged) gamepad as seen by modes.
#[derive(Debug)]
pub struct PadState {
    held: BTreeSet<&'static str>,
    axes: BTreeMap<&'static str, f64>,
    last_input: f64,
    panic_combo: BTreeSet<&'static str>,
    panic_since: Option<f64>,
    mark_combo: BTreeSet<&'static str>,
    /// Since when the mark combo is held, and whether this hold already marked.
    mark_since: Option<(f64, bool)>,
    capture_combo: BTreeSet<&'static str>,
    capture_since: Option<(f64, bool)>,
}

impl Default for PadState {
    fn default() -> Self {
        Self {
            held: BTreeSet::new(),
            axes: BTreeMap::new(),
            last_input: 0.0,
            panic_combo: DEFAULT_PANIC_COMBO.into_iter().collect(),
            panic_since: None,
            mark_combo: DEFAULT_MARK_COMBO.into_iter().collect(),
            mark_since: None,
            capture_combo: DEFAULT_CAPTURE_COMBO.into_iter().collect(),
            capture_since: None,
        }
    }
}

impl PadState {
    /// Buttons to hold together to mark a moment (see `parse_combo`).
    pub fn set_mark_combo(&mut self, combo: BTreeSet<&'static str>) {
        self.mark_combo = combo;
        self.mark_since = None;
    }

    /// Buttons to hold together to capture the game's image (see `parse_combo`).
    pub fn set_capture_combo(&mut self, combo: BTreeSet<&'static str>) {
        self.capture_combo = combo;
        self.capture_since = None;
    }

    /// Buttons to hold together for the panic stop (see `parse_combo`).
    pub fn set_panic_combo(&mut self, combo: BTreeSet<&'static str>) {
        self.panic_combo = combo;
        self.panic_since = None;
    }

    /// The gamepad is gone: releases every held button (returning the events)
    /// and centers the axes. This is not player input: idle time keeps counting.
    pub fn release_all(&mut self) -> Vec<ButtonEvent> {
        self.axes.values_mut().for_each(|v| *v = 0.0);
        self.panic_since = None;
        self.mark_since = None;
        self.capture_since = None;
        std::mem::take(&mut self.held).into_iter().map(|name| ButtonEvent { name, pressed: false }).collect()
    }

    pub fn held(&self) -> &BTreeSet<&'static str> {
        &self.held
    }

    pub fn axes(&self) -> &BTreeMap<&'static str, f64> {
        &self.axes
    }

    /// Seconds since the last player input at `time`.
    pub fn input_idle(&self, time: f64) -> f64 {
        (time - self.last_input).max(0.0)
    }

    pub fn key(&mut self, code: u16, pressed: bool, time: f64) -> Option<ButtonEvent> {
        self.button(button_name(code)?, pressed, time)
    }

    /// Named button (from a key code or the simulator). Returns an event on state change.
    pub fn button(&mut self, name: &'static str, pressed: bool, time: f64) -> Option<ButtonEvent> {
        let changed = if pressed { self.held.insert(name) } else { self.held.remove(name) };
        if !changed {
            return None;
        }
        self.last_input = time;
        self.update_panic(time);
        Some(ButtonEvent { name, pressed })
    }

    /// Normalized axis value. Hats and analog triggers also produce button events.
    pub fn axis(&mut self, code: u16, value: f64, time: f64) -> Vec<ButtonEvent> {
        let mut events = Vec::new();
        let mut hat = |this: &mut Self, neg: &'static str, pos: &'static str| {
            events.extend(this.button(neg, value < -0.5, time));
            events.extend(this.button(pos, value > 0.5, time));
        };
        let name = match Abs(code) {
            Abs::ABS_X => "LX",
            Abs::ABS_Y => "LY",
            Abs::ABS_RX => "RX",
            Abs::ABS_RY => "RY",
            Abs::ABS_Z => "LT",
            Abs::ABS_RZ => "RT",
            Abs::ABS_HAT0X => {
                hat(self, "DPAD_LEFT", "DPAD_RIGHT");
                return events;
            }
            Abs::ABS_HAT0Y => {
                hat(self, "DPAD_UP", "DPAD_DOWN");
                return events;
            }
            _ => return events,
        };
        self.axes.insert(name, value);
        if value.abs() > DEADZONE {
            self.last_input = time;
        }
        if name == "LT" || name == "RT" {
            events.extend(self.button(name, value > TRIGGER_PRESS, time));
        }
        events
    }

    /// Named axis already normalized (replayed session): no button events, those
    /// were recorded on their own.
    pub fn set_axis(&mut self, name: &'static str, value: f64, time: f64) {
        self.axes.insert(name, value);
        if value.abs() > DEADZONE {
            self.last_input = time;
        }
    }

    fn update_panic(&mut self, time: f64) {
        let held = |combo: &BTreeSet<&str>, since: Option<(f64, bool)>| match (combo.is_subset(&self.held), since) {
            (true, None) => Some((time, false)),
            (true, since) => since,
            (false, _) => None,
        };
        self.mark_since = held(&self.mark_combo, self.mark_since);
        self.capture_since = held(&self.capture_combo, self.capture_since);
        let combo = self.panic_combo.is_subset(&self.held);
        self.panic_since = match (combo, self.panic_since) {
            (true, None) => Some(time),
            (true, since) => since,
            (false, _) => None,
        };
    }

    /// True once per hold of the mark combo, after `MARK_HOLD_SECS`.
    pub fn take_mark(&mut self, time: f64) -> bool {
        once_held(&mut self.mark_since, time)
    }

    /// True once per hold of the capture combo, after `MARK_HOLD_SECS`.
    pub fn take_capture(&mut self, time: f64) -> bool {
        once_held(&mut self.capture_since, time)
    }

    /// True once the panic combo has been held for `PANIC_HOLD_SECS`.
    pub fn panic_combo(&self, time: f64) -> bool {
        self.panic_since.is_some_and(|since| time - since >= PANIC_HOLD_SECS)
    }
}

fn once_held(since: &mut Option<(f64, bool)>, time: f64) -> bool {
    match since {
        Some((since, fired)) if !*fired && time - *since >= MARK_HOLD_SECS => {
            *fired = true;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_press_and_release_emit_named_events_once() {
        let mut pad = PadState::default();
        assert_eq!(pad.key(Key::BTN_SOUTH.0, true, 1.0), Some(ButtonEvent { name: "A", pressed: true }));
        assert_eq!(pad.key(Key::BTN_SOUTH.0, true, 1.1), None);
        assert!(pad.held().contains("A"));
        assert_eq!(pad.key(Key::BTN_SOUTH.0, false, 1.2), Some(ButtonEvent { name: "A", pressed: false }));
        assert_eq!(pad.input_idle(3.2), 2.0);
    }

    #[test]
    fn hat_axis_becomes_dpad_buttons() {
        let mut pad = PadState::default();
        let ev = pad.axis(Abs::ABS_HAT0X.0, -1.0, 0.0);
        assert_eq!(ev, vec![ButtonEvent { name: "DPAD_LEFT", pressed: true }]);
        let ev = pad.axis(Abs::ABS_HAT0X.0, 0.0, 0.1);
        assert_eq!(ev, vec![ButtonEvent { name: "DPAD_LEFT", pressed: false }]);
    }

    #[test]
    fn trigger_is_axis_and_button() {
        let mut pad = PadState::default();
        let ev = pad.axis(Abs::ABS_RZ.0, 0.8, 0.0);
        assert_eq!(pad.axes()["RT"], 0.8);
        assert_eq!(ev, vec![ButtonEvent { name: "RT", pressed: true }]);
    }

    #[test]
    fn stick_inside_deadzone_is_not_input() {
        let mut pad = PadState::default();
        pad.axis(Abs::ABS_X.0, 0.05, 5.0);
        assert_eq!(pad.input_idle(5.0), 5.0);
        pad.axis(Abs::ABS_X.0, 0.5, 5.0);
        assert_eq!(pad.input_idle(5.0), 0.0);
    }

    #[test]
    fn normalize_uses_device_ranges() {
        let ranges = AxisRanges([(Abs::ABS_Z.0, (0, 255)), (Abs::ABS_X.0, (-32768, 32767))].into_iter().collect());
        assert_eq!(ranges.normalize(Abs::ABS_Z.0, 255), 1.0);
        assert!(ranges.normalize(Abs::ABS_X.0, 0).abs() < 0.001);
        assert_eq!(ranges.normalize(Abs::ABS_X.0, -32768), -1.0);
    }

    #[test]
    fn panic_combo_needs_hold_time() {
        let mut pad = PadState::default();
        pad.button("BACK", true, 0.0);
        pad.button("START", true, 0.1);
        assert!(!pad.panic_combo(0.5));
        assert!(pad.panic_combo(0.6));
        pad.button("START", false, 0.7);
        assert!(!pad.panic_combo(1.0));
    }

    #[test]
    fn panic_combo_is_configurable() {
        let mut pad = PadState::default();
        pad.set_panic_combo(parse_combo(&["LB".into(), "RB".into(), "Y".into()]).unwrap());
        pad.button("BACK", true, 0.0);
        pad.button("START", true, 0.0);
        assert!(!pad.panic_combo(1.0));
        pad.button("LB", true, 1.0);
        pad.button("RB", true, 1.0);
        pad.button("Y", true, 1.0);
        assert!(pad.panic_combo(1.5));
    }

    #[test]
    fn mark_combo_fires_once_per_hold() {
        let mut pad = PadState::default();
        pad.button("BACK", true, 0.0);
        pad.button("RS", true, 0.1);
        assert!(!pad.take_mark(0.3));
        assert!(pad.take_mark(0.4));
        assert!(!pad.take_mark(1.0), "still the same hold");
        pad.button("RS", false, 1.1);
        pad.button("RS", true, 1.2);
        assert!(pad.take_mark(1.5), "new hold");
        assert!(!pad.panic_combo(2.0));
    }

    #[test]
    fn capture_combo_fires_once_per_hold_apart_from_the_mark() {
        let mut pad = PadState::default();
        pad.button("BACK", true, 0.0);
        pad.button("LS", true, 0.1);
        assert!(!pad.take_capture(0.3));
        assert!(pad.take_capture(0.4));
        assert!(!pad.take_capture(1.0), "still the same hold");
        assert!(!pad.take_mark(1.0), "not the mark combo");
        pad.set_capture_combo(["LB", "RB"].into_iter().collect());
        pad.button("LB", true, 2.0);
        pad.button("RB", true, 2.0);
        assert!(pad.take_capture(2.5), "the new combo");
    }

    #[test]
    fn panic_combo_needs_two_known_buttons() {
        assert!(parse_combo(&["A".into()]).is_none());
        assert!(parse_combo(&["A".into(), "A".into()]).is_none());
        assert!(parse_combo(&["A".into(), "FOO".into()]).is_none());
        assert!(parse_combo(&["A".into(), "B".into()]).is_some());
    }

    #[test]
    fn release_all_emits_releases_and_centers_axes() {
        let mut pad = PadState::default();
        pad.button("A", true, 0.0);
        pad.axis(Abs::ABS_X.0, 0.8, 0.0);
        let released = pad.release_all();
        assert_eq!(released, vec![ButtonEvent { name: "A", pressed: false }]);
        assert!(pad.held().is_empty());
        assert_eq!(pad.axes()["LX"], 0.0);
    }
}
