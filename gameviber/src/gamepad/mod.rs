//! Gamepad normalization: key/axis codes (Linux's numbering, see `codes`) to the Xbox-layout names
//! exposed to modes (A, B, LB, DPAD_UP, LX, LT...), plus idle tracking and
//! detection of the panic, "mark this moment" and "capture the screen" combos.
//! The gamepads whose driver does not give the Xbox layout get it from a
//! mapping (`mapping`).

pub mod mapping;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicU8, Ordering};

/// Button and axis codes: Linux's input event codes (`input-event-codes.h`),
/// which every source translates its gamepad's events to.
pub mod codes {
    pub const KEY_RECORD: u16 = 167;
    /// The first joystick button: SDL numbers buttons from it (`mapping::Layout`).
    pub const BTN_JOYSTICK: u16 = 0x120;
    pub const BTN_SOUTH: u16 = 0x130;
    pub const BTN_EAST: u16 = 0x131;
    pub const BTN_NORTH: u16 = 0x133;
    pub const BTN_WEST: u16 = 0x134;
    pub const BTN_TL: u16 = 0x136;
    pub const BTN_TR: u16 = 0x137;
    pub const BTN_TL2: u16 = 0x138;
    pub const BTN_TR2: u16 = 0x139;
    pub const BTN_SELECT: u16 = 0x13a;
    pub const BTN_START: u16 = 0x13b;
    pub const BTN_MODE: u16 = 0x13c;
    pub const BTN_THUMBL: u16 = 0x13d;
    pub const BTN_THUMBR: u16 = 0x13e;
    pub const BTN_DPAD_UP: u16 = 0x220;
    pub const BTN_DPAD_DOWN: u16 = 0x221;
    pub const BTN_DPAD_LEFT: u16 = 0x222;
    pub const BTN_DPAD_RIGHT: u16 = 0x223;
    pub const BTN_GRIPL: u16 = 0x224;
    pub const BTN_GRIPR: u16 = 0x225;
    pub const BTN_GRIPL2: u16 = 0x226;
    pub const BTN_GRIPR2: u16 = 0x227;
    pub const BTN_TRIGGER_HAPPY1: u16 = 0x2c0;
    pub const BTN_TRIGGER_HAPPY2: u16 = 0x2c1;
    pub const BTN_TRIGGER_HAPPY3: u16 = 0x2c2;
    pub const BTN_TRIGGER_HAPPY4: u16 = 0x2c3;
    pub const BTN_TRIGGER_HAPPY5: u16 = 0x2c4;
    pub const BTN_TRIGGER_HAPPY6: u16 = 0x2c5;
    pub const BTN_TRIGGER_HAPPY7: u16 = 0x2c6;
    pub const BTN_TRIGGER_HAPPY8: u16 = 0x2c7;
    pub const ABS_X: u16 = 0x00;
    pub const ABS_Y: u16 = 0x01;
    pub const ABS_Z: u16 = 0x02;
    pub const ABS_RX: u16 = 0x03;
    pub const ABS_RY: u16 = 0x04;
    pub const ABS_RZ: u16 = 0x05;
    pub const ABS_GAS: u16 = 0x09;
    pub const ABS_BRAKE: u16 = 0x0a;
    pub const ABS_HAT0X: u16 = 0x10;
    pub const ABS_HAT0Y: u16 = 0x11;
    pub const ABS_HAT3Y: u16 = 0x17;
    pub const ABS_MAX: u16 = 0x3f;
}

use codes as c;

pub const BUTTONS: [&str; 22] = [
    "A", "B", "X", "Y", "LB", "RB", "BACK", "START", "GUIDE", "LS", "RS", "DPAD_UP", "DPAD_DOWN",
    "DPAD_LEFT", "DPAD_RIGHT", "LT", "RT", "P1", "P2", "P3", "P4", "SHARE",
];
/// Buttons games do not use: the back paddles and the share button (on the
/// gamepads whose driver reports them). A combo may be one of them alone, and
/// the proxy does not pass on those a combo uses.
pub const EXTRA_BUTTONS: [&str; 5] = ["P1", "P2", "P3", "P4", "SHARE"];

/// The extra buttons the combos use (a bit per `EXTRA_BUTTONS` entry), which
/// the proxy keeps from the game.
static RESERVED: AtomicU8 = AtomicU8::new(0);

/// The combos (button sets) GameViber listens to now.
pub fn reserve_extras<'a>(combos: impl IntoIterator<Item = &'a BTreeSet<&'static str>>) {
    let mut bits = 0;
    for combo in combos {
        for (i, extra) in EXTRA_BUTTONS.iter().enumerate() {
            if combo.contains(extra) {
                bits |= 1 << i;
            }
        }
    }
    RESERVED.store(bits, Ordering::Relaxed);
}

/// A key code is an extra button a combo uses: not for the game.
pub fn reserved(code: u16) -> bool {
    let bits = RESERVED.load(Ordering::Relaxed);
    button_name(code).and_then(|name| EXTRA_BUTTONS.iter().position(|e| *e == name)).is_some_and(|i| bits & (1 << i) != 0)
}
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
/// Default combo capturing the game's image into the active mode's captures.
pub const DEFAULT_CAPTURE_COMBO: [&str; 2] = ["BACK", "LS"];
/// A combo needs at least this many buttons, so that no single press triggers it.
pub const PANIC_COMBO_MIN: usize = 2;

/// Known button names of a combo, or None if it has fewer than
/// `PANIC_COMBO_MIN` distinct buttons (one is enough when it is an extra button).
pub fn parse_combo(names: &[String]) -> Option<BTreeSet<&'static str>> {
    let combo: BTreeSet<&'static str> =
        names.iter().filter_map(|n| BUTTONS.iter().find(|b| **b == n.as_str()).copied()).collect();
    let enough = combo.len() >= PANIC_COMBO_MIN || (combo.len() == 1 && combo.iter().all(|b| EXTRA_BUTTONS.contains(b)));
    (enough && combo.len() == names.len()).then_some(combo)
}

/// "BACK + START"
pub fn combo_text(names: &[String]) -> String {
    names.join(" + ")
}

fn button_name(code: u16) -> Option<&'static str> {
    Some(match code {
        c::BTN_SOUTH => "A",
        c::BTN_EAST => "B",
        // xpad reports the Xbox X/Y buttons as BTN_X/BTN_Y (aliases of NORTH/WEST).
        c::BTN_NORTH => "X",
        c::BTN_WEST => "Y",
        c::BTN_TL => "LB",
        c::BTN_TR => "RB",
        c::BTN_SELECT => "BACK",
        c::BTN_START => "START",
        c::BTN_MODE => "GUIDE",
        c::BTN_THUMBL => "LS",
        c::BTN_THUMBR => "RS",
        c::BTN_DPAD_UP | c::BTN_TRIGGER_HAPPY3 => "DPAD_UP",
        c::BTN_DPAD_DOWN | c::BTN_TRIGGER_HAPPY4 => "DPAD_DOWN",
        c::BTN_DPAD_LEFT | c::BTN_TRIGGER_HAPPY1 => "DPAD_LEFT",
        c::BTN_DPAD_RIGHT | c::BTN_TRIGGER_HAPPY2 => "DPAD_RIGHT",
        c::BTN_TL2 => "LT",
        c::BTN_TR2 => "RT",
        // Back paddles: xpad (Elite Series 2) and newer kernels' BTN_GRIPL, GRIPR, GRIPL2, GRIPR2.
        c::BTN_TRIGGER_HAPPY5 | c::BTN_GRIPL => "P1",
        c::BTN_TRIGGER_HAPPY6 | c::BTN_GRIPR => "P2",
        c::BTN_TRIGGER_HAPPY7 | c::BTN_GRIPL2 => "P3",
        c::BTN_TRIGGER_HAPPY8 | c::BTN_GRIPR2 => "P4",
        // The share button of Xbox Series and recent gamepads.
        c::KEY_RECORD => "SHARE",
        _ => return None,
    })
}

/// The key code a source reports `name` with (the first of `button_name`'s);
/// None for the triggers, which are axes.
pub fn button_code(name: &str) -> Option<u16> {
    Some(match name {
        "A" => c::BTN_SOUTH,
        "B" => c::BTN_EAST,
        "X" => c::BTN_NORTH,
        "Y" => c::BTN_WEST,
        "LB" => c::BTN_TL,
        "RB" => c::BTN_TR,
        "BACK" => c::BTN_SELECT,
        "START" => c::BTN_START,
        "GUIDE" => c::BTN_MODE,
        "LS" => c::BTN_THUMBL,
        "RS" => c::BTN_THUMBR,
        "DPAD_UP" => c::BTN_DPAD_UP,
        "DPAD_DOWN" => c::BTN_DPAD_DOWN,
        "DPAD_LEFT" => c::BTN_DPAD_LEFT,
        "DPAD_RIGHT" => c::BTN_DPAD_RIGHT,
        "P1" => c::BTN_TRIGGER_HAPPY5,
        "P2" => c::BTN_TRIGGER_HAPPY6,
        "P3" => c::BTN_TRIGGER_HAPPY7,
        "P4" => c::BTN_TRIGGER_HAPPY8,
        "SHARE" => c::KEY_RECORD,
        _ => return None,
    })
}

/// Axis value range of one device, as its driver reports it.
#[derive(Debug, Clone, Default)]
pub struct AxisRanges(HashMap<u16, (i32, i32)>);

impl AxisRanges {
    /// (code, (minimum, maximum)) of each axis.
    pub fn new(ranges: impl IntoIterator<Item = (u16, (i32, i32))>) -> Self {
        Self(ranges.into_iter().collect())
    }

    /// Sticks and hats to -1..1, triggers to 0..1.
    pub fn normalize(&self, code: u16, value: i32) -> f64 {
        let Some(unit) = self.unit(code, value) else { return 0.0 };
        match code {
            c::ABS_Z | c::ABS_RZ | c::ABS_GAS | c::ABS_BRAKE => unit.clamp(0.0, 1.0),
            _ => (unit * 2.0 - 1.0).clamp(-1.0, 1.0),
        }
    }

    /// Any axis to -1..1 from one end to the other, whatever it is (a gamepad
    /// still to be mapped).
    pub fn full(&self, code: u16, value: i32) -> f64 {
        self.unit(code, value).map_or(0.0, |unit| (unit * 2.0 - 1.0).clamp(-1.0, 1.0))
    }

    /// 0..1 across the range; None for an empty range.
    fn unit(&self, code: u16, value: i32) -> Option<f64> {
        let (min, max) = self.0.get(&code).copied().unwrap_or((-32768, 32767));
        (max > min).then(|| (value - min) as f64 / (max - min) as f64)
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
        let name = match code {
            c::ABS_X => "LX",
            c::ABS_Y => "LY",
            c::ABS_RX => "RX",
            c::ABS_RY => "RY",
            c::ABS_Z => "LT",
            c::ABS_RZ => "RT",
            c::ABS_HAT0X => {
                hat(self, "DPAD_LEFT", "DPAD_RIGHT");
                return events;
            }
            c::ABS_HAT0Y => {
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
        assert_eq!(pad.key(c::BTN_SOUTH, true, 1.0), Some(ButtonEvent { name: "A", pressed: true }));
        assert_eq!(pad.key(c::BTN_SOUTH, true, 1.1), None);
        assert!(pad.held().contains("A"));
        assert_eq!(pad.key(c::BTN_SOUTH, false, 1.2), Some(ButtonEvent { name: "A", pressed: false }));
        assert_eq!(pad.input_idle(3.2), 2.0);
    }

    #[test]
    fn hat_axis_becomes_dpad_buttons() {
        let mut pad = PadState::default();
        let ev = pad.axis(c::ABS_HAT0X, -1.0, 0.0);
        assert_eq!(ev, vec![ButtonEvent { name: "DPAD_LEFT", pressed: true }]);
        let ev = pad.axis(c::ABS_HAT0X, 0.0, 0.1);
        assert_eq!(ev, vec![ButtonEvent { name: "DPAD_LEFT", pressed: false }]);
    }

    #[test]
    fn trigger_is_axis_and_button() {
        let mut pad = PadState::default();
        let ev = pad.axis(c::ABS_RZ, 0.8, 0.0);
        assert_eq!(pad.axes()["RT"], 0.8);
        assert_eq!(ev, vec![ButtonEvent { name: "RT", pressed: true }]);
    }

    #[test]
    fn stick_inside_deadzone_is_not_input() {
        let mut pad = PadState::default();
        pad.axis(c::ABS_X, 0.05, 5.0);
        assert_eq!(pad.input_idle(5.0), 5.0);
        pad.axis(c::ABS_X, 0.5, 5.0);
        assert_eq!(pad.input_idle(5.0), 0.0);
    }

    #[test]
    fn normalize_uses_device_ranges() {
        let ranges = AxisRanges::new([(c::ABS_Z, (0, 255)), (c::ABS_X, (-32768, 32767))]);
        assert_eq!(ranges.normalize(c::ABS_Z, 255), 1.0);
        assert!(ranges.normalize(c::ABS_X, 0).abs() < 0.001);
        assert_eq!(ranges.normalize(c::ABS_X, -32768), -1.0);
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
        pad.axis(c::ABS_X, 0.8, 0.0);
        let released = pad.release_all();
        assert_eq!(released, vec![ButtonEvent { name: "A", pressed: false }]);
        assert!(pad.held().is_empty());
        assert_eq!(pad.axes()["LX"], 0.0);
    }

    #[test]
    fn a_back_paddle_alone_is_a_combo_kept_from_the_game() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(parse_combo(&names(&["P1"])).is_some());
        assert!(parse_combo(&names(&["A"])).is_none());
        let mut pad = PadState::default();
        assert_eq!(pad.key(c::BTN_TRIGGER_HAPPY5, true, 0.0).map(|e| e.name), Some("P1"));
        assert_eq!(pad.key(c::BTN_GRIPR2, true, 0.0).map(|e| e.name), Some("P4"));
        reserve_extras([&parse_combo(&names(&["P1"])).unwrap(), &parse_combo(&names(&["BACK", "START"])).unwrap()]);
        assert!(reserved(c::BTN_TRIGGER_HAPPY5) && reserved(c::BTN_GRIPL));
        assert!(!reserved(c::BTN_TRIGGER_HAPPY6) && !reserved(c::BTN_SELECT));
        reserve_extras([]);
    }
}
