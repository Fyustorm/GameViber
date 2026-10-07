//! Gamepad mappings: which of a gamepad's buttons, axes and hats is which Xbox
//! button or axis, for the gamepads whose driver does not say (DInput mode,
//! generic HID). In SDL's format, so that a line of SDL_GameControllerDB (whose
//! Linux mappings are embedded, `gamepads/`), or one another program built on
//! SDL wrote, works here too; the player's own lines, made on the Gamepad page,
//! go in `gamecontrollerdb.txt` of the config directory.
//!
//! A gamepad's elements are numbered like SDL numbers them on Linux (`Layout`),
//! from the key and axis codes it reports (`codes`, which every source
//! translates to).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::Context;

use super::{codes as c, AXES};

/// SDL_GameControllerDB's Linux mappings.
const BUNDLED: &str = include_str!("../../gamepads/gamecontrollerdb.txt");
/// The player's mappings, in the config directory.
const USER_FILE: &str = "gamecontrollerdb.txt";
/// Written at the end of the lines GameViber saves.
const PLATFORM: &str = "Linux";

/// SDL's names of the buttons and axes, with ours (`BUTTONS`, `AXES`).
const TARGETS: [(&str, &str, bool); 26] = [
    ("a", "A", false),
    ("b", "B", false),
    ("x", "X", false),
    ("y", "Y", false),
    ("back", "BACK", false),
    ("guide", "GUIDE", false),
    ("start", "START", false),
    ("leftstick", "LS", false),
    ("rightstick", "RS", false),
    ("leftshoulder", "LB", false),
    ("rightshoulder", "RB", false),
    ("dpup", "DPAD_UP", false),
    ("dpdown", "DPAD_DOWN", false),
    ("dpleft", "DPAD_LEFT", false),
    ("dpright", "DPAD_RIGHT", false),
    ("misc1", "SHARE", false),
    ("paddle1", "P1", false),
    ("paddle2", "P2", false),
    ("paddle3", "P3", false),
    ("paddle4", "P4", false),
    ("leftx", "LX", true),
    ("lefty", "LY", true),
    ("rightx", "RX", true),
    ("righty", "RY", true),
    ("lefttrigger", "LT", true),
    ("righttrigger", "RT", true),
];

/// One side of an axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Half {
    Plus,
    Minus,
}

impl Half {
    fn sign(self) -> &'static str {
        match self {
            Half::Plus => "+",
            Half::Minus => "-",
        }
    }
}

/// A button, axis or hat of the gamepad, by SDL's numbering (`Layout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element {
    Button(u8),
    /// The whole axis (inverted or not), or one side of it.
    Axis { index: u8, half: Option<Half>, invert: bool },
    /// One direction of a hat: 1 up, 2 right, 4 down, 8 left.
    Hat { index: u8, mask: u8 },
}

impl Element {
    fn parse(text: &str) -> Option<Self> {
        let (half, rest) = match text.as_bytes().first()? {
            b'+' => (Some(Half::Plus), &text[1..]),
            b'-' => (Some(Half::Minus), &text[1..]),
            _ => (None, text),
        };
        let (rest, invert) = match rest.strip_suffix('~') {
            Some(rest) => (rest, true),
            None => (rest, false),
        };
        if let Some(index) = rest.strip_prefix('a') {
            return Some(Element::Axis { index: index.parse().ok()?, half, invert });
        }
        if half.is_some() || invert {
            return None;
        }
        if let Some(index) = rest.strip_prefix('b') {
            return Some(Element::Button(index.parse().ok()?));
        }
        let (index, mask) = rest.strip_prefix('h')?.split_once('.')?;
        Some(Element::Hat { index: index.parse().ok()?, mask: mask.parse().ok()? })
    }

    fn text(&self) -> String {
        match *self {
            Element::Button(i) => format!("b{i}"),
            Element::Axis { index, half, invert } => {
                format!("{}a{index}{}", half.map_or("", Half::sign), if invert { "~" } else { "" })
            }
            Element::Hat { index, mask } => format!("h{index}.{mask}"),
        }
    }

    /// Its value now: -1..1 for a whole axis, 0..1 for the rest.
    fn value(&self, raw: &RawState) -> f64 {
        match *self {
            Element::Button(i) => f64::from(u8::from(raw.buttons.get(i as usize).copied().unwrap_or(false))),
            Element::Hat { index, mask } => {
                f64::from(u8::from(raw.hats.get(index as usize).is_some_and(|hat| hat & mask != 0)))
            }
            Element::Axis { index, half, invert } => {
                let v = raw.axes.get(index as usize).copied().unwrap_or(0.0);
                let v = if invert { -v } else { v };
                match half {
                    None => v,
                    Some(Half::Plus) => v.max(0.0),
                    Some(Half::Minus) => (-v).max(0.0),
                }
            }
        }
    }

    /// Its value as 0..1 (a whole axis from one end to the other).
    fn unit(&self, raw: &RawState) -> f64 {
        match self {
            Element::Axis { half: None, .. } => (self.value(raw) + 1.0) / 2.0,
            _ => self.value(raw),
        }
    }
}

/// What an element is mapped to: one of our buttons (`BUTTONS`) or axes
/// (`AXES`), or one side of a stick's axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Button(&'static str),
    Axis { name: &'static str, half: Option<Half> },
}

impl Target {
    fn parse(text: &str) -> Option<Self> {
        let (half, name) = match text.as_bytes().first()? {
            b'+' => (Some(Half::Plus), &text[1..]),
            b'-' => (Some(Half::Minus), &text[1..]),
            _ => (None, text),
        };
        let &(_, ours, axis) = TARGETS.iter().find(|(sdl, ..)| *sdl == name)?;
        match axis {
            true => Some(Target::Axis { name: ours, half }),
            false if half.is_none() => Some(Target::Button(ours)),
            false => None,
        }
    }

    fn text(&self) -> String {
        let sdl = |ours: &str| TARGETS.iter().find(|(_, o, _)| *o == ours).map_or("", |(sdl, ..)| *sdl);
        match *self {
            Target::Button(name) => sdl(name).to_owned(),
            Target::Axis { name, half } => format!("{}{}", half.map_or("", Half::sign), sdl(name)),
        }
    }

    pub fn name(&self) -> &'static str {
        match *self {
            Target::Button(name) | Target::Axis { name, .. } => name,
        }
    }
}

fn is_trigger(name: &str) -> bool {
    name == "LT" || name == "RT"
}

/// A gamepad's mapping: its SDL GUID and name, and what each element is.
#[derive(Debug, Clone, PartialEq)]
pub struct Mapping {
    pub guid: String,
    pub name: String,
    pub binds: Vec<(Target, Element)>,
}

impl Mapping {
    /// A line of SDL's format: `guid,name,a:b0,leftx:a0,...`. Elements SDL
    /// knows and GameViber does not (touchpad, misc2...) are left out.
    pub fn parse(line: &str) -> Option<Self> {
        let mut fields = line.trim().split(',');
        let guid = fields.next()?.trim().to_ascii_lowercase();
        if guid.len() != 32 || !guid.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let name = fields.next()?.trim().to_owned();
        let binds = fields
            .filter_map(|field| field.split_once(':'))
            .filter_map(|(target, element)| Some((Target::parse(target.trim())?, Element::parse(element.trim())?)))
            .collect();
        Some(Self { guid, name, binds })
    }

    pub fn line(&self) -> String {
        let mut line = format!("{},{},", self.guid, self.name.replace(',', " "));
        for (target, element) in &self.binds {
            line.push_str(&format!("{}:{},", target.text(), element.text()));
        }
        line.push_str(&format!("platform:{PLATFORM},"));
        line
    }

    /// The Xbox buttons held and axes (sticks -1..1, triggers 0..1) the gamepad's state makes.
    pub fn apply(&self, raw: &RawState) -> PadOutput {
        let mut out = PadOutput::default();
        for (target, element) in &self.binds {
            match *target {
                Target::Button(name) => {
                    if element.unit(raw) > 0.5 {
                        out.held.insert(name);
                    }
                }
                Target::Axis { name, .. } if is_trigger(name) => {
                    let axis = out.axes.get_mut(name).expect("every axis");
                    *axis = axis.max(element.unit(raw));
                }
                Target::Axis { name, half } => {
                    let v = match half {
                        None => element.value(raw),
                        Some(Half::Plus) => element.unit(raw),
                        Some(Half::Minus) => -element.unit(raw),
                    };
                    *out.axes.get_mut(name).expect("every axis") += v;
                }
            }
        }
        for v in out.axes.values_mut() {
            *v = v.clamp(-1.0, 1.0);
        }
        out
    }
}

/// What a mapped gamepad makes: the Xbox buttons held (triggers left out, see
/// `axes`) and every axis (sticks -1..1, triggers 0..1).
#[derive(Debug, Clone, PartialEq)]
pub struct PadOutput {
    pub held: BTreeSet<&'static str>,
    pub axes: BTreeMap<&'static str, f64>,
}

impl Default for PadOutput {
    fn default() -> Self {
        Self { held: BTreeSet::new(), axes: AXES.iter().map(|a| (*a, 0.0)).collect() }
    }
}

/// The state of a gamepad's elements, numbered like SDL (`Layout`): axes -1..1
/// from one end to the other, hats as directions (`Element::Hat`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawState {
    pub buttons: Vec<bool>,
    pub axes: Vec<f64>,
    pub hats: Vec<u8>,
}

/// How SDL numbers a gamepad's elements on Linux, from the key and axis codes
/// it reports: buttons from `BTN_JOYSTICK` up, then those below; axes in code
/// order, hats left out; hats by pair of axes (`ABS_HAT0X`, `ABS_HAT0Y`...).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    keys: Vec<u16>,
    axes: Vec<u16>,
    /// Their X axis code.
    hats: Vec<u16>,
}

impl Layout {
    pub fn new(keys: impl IntoIterator<Item = u16>, axes: impl IntoIterator<Item = u16>) -> Self {
        let mut keys: Vec<u16> = keys.into_iter().collect();
        keys.sort_unstable_by_key(|&k| (k < c::BTN_JOYSTICK, k));
        keys.dedup();
        let axes: BTreeSet<u16> = axes.into_iter().filter(|&a| a <= c::ABS_MAX).collect();
        let hats = (c::ABS_HAT0X..=c::ABS_HAT3Y)
            .step_by(2)
            .filter(|&x| axes.contains(&x) || axes.contains(&(x + 1)))
            .collect();
        let axes = axes.into_iter().filter(|a| !(c::ABS_HAT0X..=c::ABS_HAT3Y).contains(a)).collect();
        Self { keys, axes, hats }
    }

    /// Every element at rest.
    pub fn rest(&self) -> RawState {
        RawState { buttons: vec![false; self.keys.len()], axes: vec![0.0; self.axes.len()], hats: vec![0; self.hats.len()] }
    }

    pub fn key(&self, raw: &mut RawState, code: u16, pressed: bool) {
        if let Some(i) = self.keys.iter().position(|&k| k == code) {
            raw.buttons[i] = pressed;
        }
    }

    /// An axis moved: `value` from -1 to 1 (a hat's: its sign).
    pub fn axis(&self, raw: &mut RawState, code: u16, value: f64) {
        if let Some(i) = self.axes.iter().position(|&a| a == code) {
            raw.axes[i] = value;
            return;
        }
        let Some(i) = self.hats.iter().position(|&x| x == code || x + 1 == code) else { return };
        let (minus, plus) = if code == self.hats[i] { (8, 2) } else { (1, 4) };
        let hat = &mut raw.hats[i];
        *hat &= !(minus | plus);
        if value < -0.5 {
            *hat |= minus;
        } else if value > 0.5 {
            *hat |= plus;
        }
    }
}

/// SDL's GUID of a gamepad on Linux: its bus, the CRC of its name, vendor,
/// product and version.
pub fn sdl_guid(bus: u16, vendor: u16, product: u16, version: u16, name: &str) -> String {
    let words = [bus, crc16(name.as_bytes()), vendor, 0, product, 0, version, 0];
    words.iter().flat_map(|w| w.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
}

/// CRC-16/ARC, as SDL_crc16 computes it.
fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc ^= u16::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xa001 } else { crc >> 1 };
        }
    }
    crc
}

/// Where a gamepad's mapping comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Set up by the player.
    User,
    /// SDL_GameControllerDB.
    Community,
}

fn guid_bytes(guid: &str) -> Option<[u8; 16]> {
    let mut bytes = [0; 16];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(guid.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(bytes)
}

/// How well a mapping's GUID fits a gamepad's: SDL's rules (same bus, vendor
/// and product; the name's CRC and the version the same or left at 0, as in
/// most lines of its database; no driver of SDL's own). None: not this gamepad.
fn fit(entry: &str, pad: &[u8; 16]) -> Option<u8> {
    let entry = guid_bytes(entry)?;
    let word = |g: &[u8; 16], i: usize| u16::from_le_bytes([g[i * 2], g[i * 2 + 1]]);
    if [0, 2, 4].iter().any(|&i| word(&entry, i) != word(pad, i)) || word(pad, 2) == 0 || entry[14] != 0 || entry[15] != 0 {
        return None;
    }
    let mut score = 0;
    for i in [1, 6] {
        match word(&entry, i) {
            w if w == word(pad, i) => score += 2,
            0 => score += 1,
            _ => return None,
        }
    }
    Some(score)
}

/// The best mapping of `text` for the gamepad `guid`.
fn best(text: &str, guid: &str) -> Option<Mapping> {
    let pad = guid_bytes(guid)?;
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| {
            let guid = line.trim().get(..32)?;
            Some((fit(guid, &pad)?, line))
        })
        .max_by_key(|(score, _)| *score)
        .and_then(|(_, line)| Mapping::parse(line))
}

fn user_file() -> PathBuf {
    crate::platform::config_dir().join(USER_FILE)
}

fn user_text() -> String {
    std::fs::read_to_string(user_file()).unwrap_or_default()
}

/// The mapping of the gamepad `guid`: the player's, else SDL_GameControllerDB's.
pub fn find(guid: &str) -> Option<(Mapping, Origin)> {
    best(&user_text(), guid)
        .map(|m| (m, Origin::User))
        .or_else(|| best(BUNDLED, guid).map(|m| (m, Origin::Community)))
}

/// Saves the player's mapping of a gamepad, in place of the one they had.
pub fn save(mapping: &Mapping) -> anyhow::Result<()> {
    let text = without(&user_text(), &mapping.guid);
    write_user(&format!("{text}{}\n", mapping.line()))
}

/// Forgets the player's mapping of the gamepad `guid`.
pub fn forget(guid: &str) -> anyhow::Result<()> {
    write_user(&without(&user_text(), guid))
}

/// `text` without the lines of `guid`.
fn without(text: &str, guid: &str) -> String {
    text.lines()
        .filter(|line| !line.trim().to_ascii_lowercase().starts_with(guid))
        .map(|line| format!("{line}\n"))
        .collect()
}

fn write_user(text: &str) -> anyhow::Result<()> {
    let path = user_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))
}

/// One thing the player is asked to press or move while setting up a gamepad.
pub struct Step {
    pub target: Target,
    /// Some gamepads have none.
    pub optional: bool,
}

const fn button(name: &'static str, optional: bool) -> Step {
    Step { target: Target::Button(name), optional }
}

/// A stick's axis is asked for its right or down end; a trigger, pulled.
const fn axis(name: &'static str) -> Step {
    Step { target: Target::Axis { name, half: None }, optional: false }
}

/// The steps of the setup, in order.
pub const STEPS: [Step; 21] = [
    button("A", false),
    button("B", false),
    button("X", false),
    button("Y", false),
    button("LB", false),
    button("RB", false),
    axis("LT"),
    axis("RT"),
    button("BACK", false),
    button("START", false),
    button("GUIDE", true),
    button("LS", true),
    button("RS", true),
    button("DPAD_UP", false),
    button("DPAD_DOWN", false),
    button("DPAD_LEFT", false),
    button("DPAD_RIGHT", false),
    axis("LX"),
    axis("LY"),
    axis("RX"),
    axis("RY"),
];

/// The step of the button or axis `name`.
pub fn step_of(name: &str) -> Option<usize> {
    STEPS.iter().position(|s| s.target.name() == name)
}

/// Moved this far from rest, an axis counts as pushed.
const PUSHED: f64 = 0.5;
/// Back this share of the way from its farthest to its rest, a pushed axis
/// counts as let go (a stick that drifts, a trigger that never quite returns).
const LET_GO: f64 = 0.5;

/// Watches the gamepad for one press (or push) and its release, during a step
/// of the setup.
#[derive(Debug, Clone)]
pub struct Learner {
    /// Every element at rest, taken when the setup started.
    rest: RawState,
    /// What moved during this press, in the order it moved; for axes, the
    /// value farthest from rest.
    moved: Vec<(Element, f64)>,
}

impl Learner {
    pub fn new(rest: RawState) -> Self {
        Self { rest, moved: Vec::new() }
    }

    /// Forgets the press under way (the player moved on to another step).
    pub fn reset(&mut self) {
        self.moved.clear();
    }

    /// Feeds the gamepad's state; once the press is let go, the element it
    /// gives `target`, if one fits (a stick's axis needs an axis).
    pub fn update(&mut self, now: &RawState, target: &Target) -> Option<Element> {
        let mut calm = true;
        for (i, (&pressed, &rest)) in now.buttons.iter().zip(&self.rest.buttons).enumerate() {
            if pressed && !rest {
                calm = false;
                note(&mut self.moved, Element::Button(i as u8), 1.0);
            }
        }
        for (i, (&hat, &rest)) in now.hats.iter().zip(&self.rest.hats).enumerate() {
            for mask in [1, 2, 4, 8] {
                if hat & mask != 0 && rest & mask == 0 {
                    calm = false;
                    note(&mut self.moved, Element::Hat { index: i as u8, mask }, 1.0);
                }
            }
        }
        for (i, (&v, &rest)) in now.axes.iter().zip(&self.rest.axes).enumerate() {
            let element = Element::Axis { index: i as u8, half: None, invert: false };
            if (v - rest).abs() > PUSHED && v.abs() > PUSHED {
                note(&mut self.moved, element, v);
            }
            // Only the axes pushed during this press: others may drift.
            if let Some((_, far)) = self.moved.iter().find(|(e, _)| *e == element) {
                if (v - rest).abs() > (far - rest).abs() * LET_GO {
                    calm = false;
                }
            }
        }
        if !calm || self.moved.is_empty() {
            return None;
        }
        let moved = std::mem::take(&mut self.moved);
        self.pick(&moved, target)
    }

    fn pick(&self, moved: &[(Element, f64)], target: &Target) -> Option<Element> {
        let axis = moved.iter().find_map(|&(e, far)| match e {
            Element::Axis { index, .. } => Some((index, far, self.rest.axes[index as usize])),
            _ => None,
        });
        let other = moved.iter().map(|(e, _)| *e).find(|e| matches!(e, Element::Button(_)))
            .or_else(|| moved.iter().map(|(e, _)| *e).find(|e| matches!(e, Element::Hat { .. })));
        let half = |far: f64| if far > 0.0 { Half::Plus } else { Half::Minus };
        match *target {
            // Asked for its right or down end: an axis going the other way is inverted.
            Target::Axis { name, .. } if !is_trigger(name) => {
                axis.map(|(index, far, rest)| Element::Axis { index, half: None, invert: far < rest })
            }
            // A trigger resting at one end is the whole axis; at the center, one side of it.
            Target::Axis { .. } => match axis {
                Some((index, _, rest)) if rest <= -PUSHED => Some(Element::Axis { index, half: None, invert: false }),
                Some((index, _, rest)) if rest >= PUSHED => Some(Element::Axis { index, half: None, invert: true }),
                Some((index, far, _)) => Some(Element::Axis { index, half: Some(half(far)), invert: false }),
                None => other,
            },
            Target::Button(_) => other.or(axis.map(|(index, far, _)| Element::Axis { index, half: Some(half(far)), invert: false })),
        }
    }
}

/// Notes what moved; an axis keeps its farthest value.
fn note(moved: &mut Vec<(Element, f64)>, element: Element, value: f64) {
    match moved.iter_mut().find(|(e, _)| *e == element) {
        Some((_, far)) if value.abs() > far.abs() => *far = value,
        Some(_) => {}
        None => moved.push((element, value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An Xbox 360 pad as xpad reports it: SDL_GameControllerDB's line for it.
    const XPAD: &str = "030000005e0400008e02000014010000,Xbox 360 Controller,a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b8,leftshoulder:b4,leftstick:b9,lefttrigger:a2,leftx:a0,lefty:a1,rightshoulder:b5,rightstick:b10,righttrigger:a5,rightx:a3,righty:a4,start:b7,x:b2,y:b3,platform:Linux,";

    fn xpad_layout() -> Layout {
        let keys = [c::BTN_SOUTH, c::BTN_EAST, c::BTN_NORTH, c::BTN_WEST, c::BTN_TL, c::BTN_TR, c::BTN_SELECT, c::BTN_START, c::BTN_MODE, c::BTN_THUMBL, c::BTN_THUMBR];
        let axes = [c::ABS_X, c::ABS_Y, c::ABS_Z, c::ABS_RX, c::ABS_RY, c::ABS_RZ, c::ABS_HAT0X, c::ABS_HAT0Y];
        Layout::new(keys, axes)
    }

    #[test]
    fn crc_and_guid_are_sdl_s() {
        assert_eq!(crc16(b"123456789"), 0xbb3d);
        assert_eq!(sdl_guid(3, 0x045e, 0x028e, 0x0114, ""), "030000005e0400008e02000014010000");
        let d10 = sdl_guid(3, 0x2345, 0xe06a, 0x0110, "Easy SMX D10");
        assert_eq!((&d10[..4], &d10[8..]), ("0300", "452300006ae0000010010000"));
        assert_ne!(&d10[4..8], "0000", "the name's CRC");
    }

    #[test]
    fn lines_are_read_and_written_back() {
        let mapping = Mapping::parse(XPAD).unwrap();
        assert_eq!(mapping.name, "Xbox 360 Controller");
        assert!(mapping.binds.contains(&(Target::Button("A"), Element::Button(0))));
        assert!(mapping.binds.contains(&(Target::Button("DPAD_LEFT"), Element::Hat { index: 0, mask: 8 })));
        assert!(mapping.binds.contains(&(Target::Axis { name: "LT", half: None }, Element::Axis { index: 2, half: None, invert: false })));
        assert_eq!(Mapping::parse(&mapping.line()).unwrap(), mapping);
        let odd = Mapping::parse("030000005e0400008e02000014010000,Odd,+leftx:h0.2,-leftx:h0.8,lefttrigger:-a5~,dpup:+a1,touchpad:b9,").unwrap();
        assert_eq!(odd.binds.len(), 4, "touchpad left out");
        assert_eq!(Mapping::parse(&odd.line()).unwrap(), odd);
        assert!(Mapping::parse("not a guid,Name,a:b0").is_none());
    }

    #[test]
    fn elements_are_numbered_like_sdl() {
        // Joystick buttons (from BTN_JOYSTICK) come before the keys below them.
        let layout = Layout::new([c::BTN_SOUTH, 0x120, 0x110], [c::ABS_X, c::ABS_HAT0X, c::ABS_HAT0Y, c::ABS_Y, c::ABS_GAS]);
        let mut raw = layout.rest();
        assert_eq!((raw.buttons.len(), raw.axes.len(), raw.hats.len()), (3, 3, 1));
        layout.key(&mut raw, 0x110, true);
        assert_eq!(raw.buttons, [false, false, true]);
        layout.axis(&mut raw, c::ABS_GAS, 0.5);
        assert_eq!(raw.axes, [0.0, 0.0, 0.5]);
        layout.axis(&mut raw, c::ABS_HAT0X, -1.0);
        layout.axis(&mut raw, c::ABS_HAT0Y, 1.0);
        assert_eq!(raw.hats, [8 | 4]);
        layout.axis(&mut raw, c::ABS_HAT0X, 0.0);
        assert_eq!(raw.hats, [4]);
    }

    #[test]
    fn a_mapping_makes_xbox_buttons_and_axes() {
        let mapping = Mapping::parse(XPAD).unwrap();
        let layout = xpad_layout();
        let mut raw = layout.rest();
        layout.key(&mut raw, c::BTN_NORTH, true);
        layout.axis(&mut raw, c::ABS_HAT0Y, -1.0);
        layout.axis(&mut raw, c::ABS_Z, -1.0);
        layout.axis(&mut raw, c::ABS_RZ, 1.0);
        layout.axis(&mut raw, c::ABS_RX, -0.5);
        let out = mapping.apply(&raw);
        assert_eq!(out.held, ["X", "DPAD_UP"].into_iter().collect());
        assert_eq!((out.axes["LT"], out.axes["RT"], out.axes["RX"], out.axes["LX"]), (0.0, 1.0, -0.5, 0.0));
    }

    #[test]
    fn halves_and_inverted_axes_apply() {
        let mapping = Mapping::parse("030000005e0400008e02000014010000,Odd,+leftx:h0.2,-leftx:h0.8,lefttrigger:-a0,righttrigger:a1~,dpup:-a2,").unwrap();
        let raw = RawState { buttons: vec![], axes: vec![-0.6, 1.0, -0.9], hats: vec![8] };
        let out = mapping.apply(&raw);
        assert_eq!(out.axes["LX"], -1.0);
        assert!((out.axes["LT"] - 0.6).abs() < 1e-9);
        assert_eq!(out.axes["RT"], 0.0, "inverted: at rest");
        assert!(out.held.contains("DPAD_UP"));
    }

    #[test]
    fn the_closest_line_of_the_database_is_found() {
        let pad = sdl_guid(3, 0x045e, 0x028e, 0x0114, "Microsoft X-Box 360 pad");
        let db = format!(
            "# comment\n{}\n030000005e0400008e02000099990000,Other version,a:b1,\n050000005e0400008e02000014010000,Bluetooth,a:b2,\n",
            XPAD.replace("Xbox 360 Controller", "Exact version")
        );
        assert_eq!(best(&db, &pad).unwrap().name, "Exact version");
        let generic = "030000005e0400008e02000000000000,Any version,a:b0,";
        assert_eq!(best(generic, &pad).unwrap().name, "Any version");
        assert!(best(&db, &sdl_guid(3, 0x045e, 0x0b12, 0x0114, "")).is_none());
        // The bundled database has the 360 pad.
        assert!(best(BUNDLED, &pad).is_some());
    }

    #[test]
    fn saved_lines_replace_the_previous_one() {
        let text = format!("{XPAD}\n030000005e0400000b12000000000000,Series,a:b0,\n");
        let kept = without(&text, "030000005e0400008e02000014010000");
        assert_eq!(kept, "030000005e0400000b12000000000000,Series,a:b0,\n");
    }

    #[test]
    fn the_learner_takes_each_press_once_let_go() {
        // A DInput pad: triggers resting at one end, the right stick inverted.
        let layout = Layout::new([0x130, 0x131], [c::ABS_X, c::ABS_RZ, c::ABS_GAS, c::ABS_HAT0X, c::ABS_HAT0Y]);
        let mut rest = layout.rest();
        layout.axis(&mut rest, c::ABS_GAS, -1.0);
        let mut learner = Learner::new(rest.clone());
        let a = Target::Button("A");
        let mut now = rest.clone();
        now.buttons[1] = true;
        assert_eq!(learner.update(&now, &a), None, "still held");
        assert_eq!(learner.update(&rest, &a), Some(Element::Button(1)));
        assert_eq!(learner.update(&rest, &a), None, "nothing new");

        let lt = Target::Axis { name: "LT", half: None };
        let mut now = rest.clone();
        now.axes[2] = 0.2;
        now.buttons[0] = true; // a digital press along with the axis
        assert_eq!(learner.update(&now, &lt), None);
        now.axes[2] = 1.0;
        assert_eq!(learner.update(&now, &lt), None);
        assert_eq!(learner.update(&rest, &lt), Some(Element::Axis { index: 2, half: None, invert: false }));

        let rx = Target::Axis { name: "RX", half: None };
        let mut now = rest.clone();
        now.axes[1] = -0.9;
        learner.update(&now, &rx);
        now.axes[1] = -0.1;
        assert_eq!(learner.update(&now, &rx), Some(Element::Axis { index: 1, half: None, invert: true }));

        let up = Target::Button("DPAD_UP");
        let mut now = rest.clone();
        now.hats[0] = 1;
        learner.update(&now, &up);
        assert_eq!(learner.update(&rest, &up), Some(Element::Hat { index: 0, mask: 1 }));

        // A trigger that does not quite come back, a stick that drifts: let go all the same.
        let mut now = rest.clone();
        now.axes[2] = 0.9;
        now.axes[0] = 0.3;
        learner.update(&now, &lt);
        now.axes[2] = -0.6;
        assert_eq!(learner.update(&now, &lt), Some(Element::Axis { index: 2, half: None, invert: false }));

        // A stick needs an axis: a button press gives nothing.
        let mut now = rest.clone();
        now.buttons[0] = true;
        learner.update(&now, &rx);
        assert_eq!(learner.update(&rest, &rx), None);
    }
}
