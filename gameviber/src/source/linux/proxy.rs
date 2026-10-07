//! "proxy" source: the real gamepad is grabbed and a virtual one (uinput) is
//! exposed to games. Inputs are forwarded to it; the force feedback games
//! upload to it is captured, then forwarded to the real gamepad when that one
//! can vibrate (passthrough).
//!
//! The virtual gamepad is a copy of the real one (same name and VID/PID) when
//! its driver gives the Xbox layout; otherwise (DInput mode, generic HID) it is
//! an Xbox 360 controller, made from the real one's mapping
//! (`gamepad::mapping`). Without a mapping, the real gamepad is only read, for
//! the player to set up its buttons.

use std::collections::HashMap;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::Context;
use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, Device, EventSummary, EventType, FFEffect, FFEffectCode, InputEvent,
    InputId, KeyCode, SynchronizationCode, UInputCode, UinputAbsSetup,
};

use super::{axis_ranges, driver, effect_from_evdev, find_gamepad, has_rumble, translate_input};
use crate::gamepad::mapping::{self, Layout, Mapping, Origin, PadOutput, RawState};
use crate::gamepad::{button_code, codes as c, AxisRanges, BUTTONS};
use crate::platform::linux::helper::client::{Helper, Phase};
use crate::platform::linux::helper::Request;
use crate::platform::linux::hider::DeviceHider;
use crate::source::{ActiveSource, EventSender, PadInfo, PadLayout, SourceEvent, SourceHealth, SourceKind};

const FF_CODES: [FFEffectCode; 6] = [
    FFEffectCode::FF_RUMBLE,
    FFEffectCode::FF_PERIODIC,
    FFEffectCode::FF_SQUARE,
    FFEffectCode::FF_TRIANGLE,
    FFEffectCode::FF_SINE,
    FFEffectCode::FF_GAIN,
];

/// Effects the virtual gamepad takes when the real one has none.
const FF_EFFECTS: u32 = 16;

/// The Xbox 360 controller games get for a mapped gamepad: xpad's.
const XBOX_360: (&str, u16, u16, u16) = ("Microsoft X-Box 360 pad", 0x045e, 0x028e, 0x0114);
const XBOX_360_BUTTONS: [(&str, KeyCode); 11] = [
    ("A", KeyCode::BTN_SOUTH),
    ("B", KeyCode::BTN_EAST),
    ("X", KeyCode::BTN_NORTH),
    ("Y", KeyCode::BTN_WEST),
    ("LB", KeyCode::BTN_TL),
    ("RB", KeyCode::BTN_TR),
    ("BACK", KeyCode::BTN_SELECT),
    ("START", KeyCode::BTN_START),
    ("GUIDE", KeyCode::BTN_MODE),
    ("LS", KeyCode::BTN_THUMBL),
    ("RS", KeyCode::BTN_THUMBR),
];
/// Its axes: code, our name (None: a hat, from the d-pad), range.
const XBOX_360_AXES: [(AbsoluteAxisCode, Option<&str>, i32, i32); 8] = [
    (AbsoluteAxisCode::ABS_X, Some("LX"), -32768, 32767),
    (AbsoluteAxisCode::ABS_Y, Some("LY"), -32768, 32767),
    (AbsoluteAxisCode::ABS_RX, Some("RX"), -32768, 32767),
    (AbsoluteAxisCode::ABS_RY, Some("RY"), -32768, 32767),
    (AbsoluteAxisCode::ABS_Z, Some("LT"), 0, 255),
    (AbsoluteAxisCode::ABS_RZ, Some("RT"), 0, 255),
    (AbsoluteAxisCode::ABS_HAT0X, None, -1, 1),
    (AbsoluteAxisCode::ABS_HAT0Y, None, -1, 1),
];

/// What games get.
enum Shape {
    /// A copy of the real gamepad: its driver gives the Xbox layout.
    Copy,
    /// An Xbox 360 controller, from the real one's mapping.
    Mapped(Mapping),
    /// Nothing: the real gamepad's buttons are to be set up.
    Unmapped,
}

/// How the real gamepad is hidden from games.
pub enum Hide {
    No,
    /// We already run as root.
    Local,
    Helper(Arc<Helper>),
}

enum Hidden {
    No,
    Local(DeviceHider),
    Helper(Arc<Helper>),
}

pub struct ProxySource {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    hidden: Hidden,
    status: Arc<Mutex<String>>,
    /// Name of the real gamepad.
    name: String,
    /// Why the proxy thread stopped.
    error: Arc<Mutex<Option<String>>>,
    /// The real gamepad, for setting up its buttons (`raw` is updated as it is read).
    pad: PadInfo,
    raw: Arc<Mutex<RawState>>,
}

impl ProxySource {
    pub fn start(device: Option<&Path>, passthrough: bool, hide: Hide, tx: EventSender) -> anyhow::Result<Self> {
        let (real_path, mut real) = find_gamepad(device)?;
        let name = real.name().unwrap_or("?").to_owned();
        let id = real.input_id();
        let guid = mapping::sdl_guid(id.bus_type().0, id.vendor(), id.product(), id.version(), &name);
        let layout = Layout::new(
            real.supported_keys().map(|k| k.iter().map(|k| k.0).collect::<Vec<_>>()).unwrap_or_default(),
            real.supported_absolute_axes().map(|a| a.iter().map(|a| a.0).collect::<Vec<_>>()).unwrap_or_default(),
        );
        let rumble = has_rumble(&real);
        // The player's mapping first; then the driver's layout, unless the
        // generic HID driver gave the buttons in the gamepad's own order.
        let (shape, pad_layout) = match mapping::find(&guid) {
            Some((m, Origin::User)) => (Shape::Mapped(m), PadLayout::Mapped(Origin::User)),
            _ if rumble || driver(&real_path).as_deref() != Some("hid-generic") => (Shape::Copy, PadLayout::Driver),
            Some((m, origin)) => (Shape::Mapped(m), PadLayout::Mapped(origin)),
            None => (Shape::Unmapped, PadLayout::Missing),
        };
        let ranges = axis_ranges(&real);
        let initial = current_state(&real, &layout, &ranges);
        let raw = Arc::new(Mutex::new(initial.clone()));
        let mapping = match &shape {
            Shape::Mapped(m) => Some(m.clone()),
            Shape::Copy | Shape::Unmapped => None,
        };
        let pad = PadInfo { name: name.clone(), guid: guid.clone(), layout: pad_layout, mapping, raw: layout.rest(), rumble };

        let mut virt = match shape {
            Shape::Copy => Some(build_copy(&real).context("cannot create the virtual gamepad")?),
            Shape::Mapped(_) => Some(build_xbox_360().context("cannot create the virtual gamepad")?),
            Shape::Unmapped => None,
        };
        let virt_path = match virt.as_mut() {
            Some(virt) => virt
                .enumerate_dev_nodes_blocking()
                .context("cannot find the virtual gamepad node")?
                .filter_map(Result::ok)
                .find(|p| p.to_string_lossy().contains("event"))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "?".into()),
            None => real_path.clone(),
        };
        let hidden = match (&virt, hide) {
            (None, _) => {
                log::info!("{name} ({real_path}, {guid}) has no mapping: read until its buttons are set up");
                Hidden::No
            }
            (Some(_), hide) => {
                real.grab().context("cannot grab the real gamepad")?;
                log::info!("virtual gamepad for '{name}' created at {virt_path} (real one at {real_path} grabbed)");
                match hide {
                    Hide::No => Hidden::No,
                    Hide::Local => Hidden::Local(DeviceHider::hide(&real_path)?),
                    Hide::Helper(helper) => {
                        helper.request(Request::Hide { device: real_path.clone() });
                        Hidden::Helper(helper)
                    }
                }
            }
        };
        let status = Arc::new(Mutex::new(match shape {
            Shape::Copy => format!("proxy: {name} ({real_path} → {virt_path})"),
            Shape::Mapped(_) => format!("proxy: {name} ({real_path} → {virt_path}, as an Xbox 360 controller)"),
            Shape::Unmapped => format!("proxy: {name} ({real_path}): its buttons are to be set up"),
        }));

        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let thread = {
            let (stop, status, error, shared_raw) = (stop.clone(), status.clone(), error.clone(), raw.clone());
            std::thread::Builder::new().name("proxy".into()).spawn(move || {
                let mut proxy = Proxy {
                    real,
                    virt,
                    ranges,
                    raw: initial,
                    layout,
                    shared_raw,
                    shape,
                    out: PadOutput::default(),
                    real_effects: HashMap::new(),
                    passthrough: passthrough && rumble,
                    tx,
                    device: virt_path,
                };
                if let Err(e) = proxy.run(&stop) {
                    log::error!("proxy stopped: {e:#}");
                    *status.lock().unwrap() = format!("proxy stopped: {e:#}");
                    *error.lock().unwrap() = Some(format!("{e:#}"));
                }
            })?
        };
        Ok(Self { stop, thread: Some(thread), hidden, status, name, error, pad, raw })
    }
}

impl ActiveSource for ProxySource {
    fn health(&self) -> SourceHealth {
        if let Some(e) = self.error.lock().unwrap().clone() {
            return SourceHealth::Failed(e);
        }
        if self.pad.layout == PadLayout::Missing {
            return SourceHealth::Waiting(format!("set up the buttons of {} first", self.name));
        }
        match &self.hidden {
            Hidden::Helper(helper) if matches!(helper.state().phase, Phase::NotStarted | Phase::Authorizing) => {
                SourceHealth::Waiting("waiting for your password to hide the real gamepad".into())
            }
            _ => SourceHealth::Working,
        }
    }

    fn status(&self) -> String {
        let base = self.status.lock().unwrap().clone();
        let hidden = match &self.hidden {
            Hidden::No => String::new(),
            Hidden::Local(_) => ", real gamepad hidden".into(),
            Hidden::Helper(helper) => {
                let state = helper.state();
                match (&state.phase, &state.hidden, &state.last_error) {
                    (_, Some(_), _) => ", real gamepad hidden".into(),
                    (Phase::NotStarted | Phase::Authorizing, ..) => ", hiding: waiting for authorization...".into(),
                    (Phase::Failed(e), ..) => format!(", not hidden: {e}"),
                    (Phase::Ready, None, Some(e)) => format!(", not hidden: {e}"),
                    (Phase::Ready, None, None) => ", hiding...".into(),
                }
            }
        };
        format!("{base}{hidden}")
    }

    fn gamepads(&self) -> Vec<String> {
        match self.health() {
            SourceHealth::Failed(_) => Vec::new(),
            _ => vec![self.name.clone()],
        }
    }

    fn pad(&self) -> Option<PadInfo> {
        match self.health() {
            SourceHealth::Failed(_) => None,
            _ => Some(PadInfo { raw: self.raw.lock().unwrap().clone(), ..self.pad.clone() }),
        }
    }

    fn shutdown(mut self: Box<Self>) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        match std::mem::replace(&mut self.hidden, Hidden::No) {
            Hidden::No => {}
            Hidden::Local(h) => h.restore(),
            Hidden::Helper(helper) => helper.request(Request::Unhide),
        }
    }
}

fn build_copy(real: &Device) -> anyhow::Result<VirtualDevice> {
    let name = real.name().unwrap_or("Gamepad").to_owned();
    let mut builder = VirtualDevice::builder()?
        .name(&name)
        .input_id(real.input_id())
        // No with_phys: evdev 0.13 encodes UI_SET_PHYS with a 1-byte size instead
        // of a pointer and the kernel rejects it. No game reads phys anyway.
        .with_ff(&FF_CODES.iter().copied().collect::<AttributeSet<_>>())
        .context("ff")?
        .with_ff_effects_max(match real.max_ff_effects() {
            0 => FF_EFFECTS,
            n => n as u32,
        });
    if let Some(keys) = real.supported_keys() {
        builder = builder.with_keys(keys).context("keys")?;
    }
    if let Some(axes) = real.supported_absolute_axes() {
        let state = real.get_abs_state()?;
        for axis in axes.iter() {
            let a = state[axis.0 as usize];
            let info = AbsInfo::new(a.value, a.minimum, a.maximum, a.fuzz, a.flat, a.resolution);
            builder = builder
                .with_absolute_axis(&UinputAbsSetup::new(axis, info))
                .with_context(|| format!("axis {axis:?}"))?;
        }
    }
    builder.build().context("UI_DEV_CREATE")
}

fn build_xbox_360() -> anyhow::Result<VirtualDevice> {
    let (name, vendor, product, version) = XBOX_360;
    let keys: AttributeSet<KeyCode> = XBOX_360_BUTTONS.iter().map(|(_, code)| *code).collect();
    let mut builder = VirtualDevice::builder()?
        .name(name)
        .input_id(InputId::new(BusType::BUS_USB, vendor, product, version))
        .with_ff(&FF_CODES.iter().copied().collect::<AttributeSet<_>>())
        .context("ff")?
        .with_ff_effects_max(FF_EFFECTS)
        .with_keys(&keys)
        .context("keys")?;
    for (code, _, min, max) in XBOX_360_AXES {
        // xpad's noise filter and dead zone on the sticks.
        let (fuzz, flat) = if max > 255 { (16, 128) } else { (0, 0) };
        builder = builder
            .with_absolute_axis(&UinputAbsSetup::new(code, AbsInfo::new(0, min, max, fuzz, flat, 0)))
            .with_context(|| format!("axis {code:?}"))?;
    }
    builder.build().context("UI_DEV_CREATE")
}

/// The real gamepad's elements as the kernel last saw them: a trigger nobody
/// touched yet rests at its end, not at the center.
fn current_state(real: &Device, layout: &Layout, ranges: &AxisRanges) -> RawState {
    let mut raw = layout.rest();
    if let (Some(axes), Ok(state)) = (real.supported_absolute_axes(), real.get_abs_state()) {
        for axis in axes.iter() {
            layout.axis(&mut raw, axis.0, ranges.full(axis.0, state[axis.0 as usize].value));
        }
    }
    if let Ok(keys) = real.get_key_state() {
        for key in keys.iter() {
            layout.key(&mut raw, key.0, true);
        }
    }
    raw
}

/// A mapped axis's value on the virtual gamepad.
fn scaled(value: f64, min: i32, max: i32) -> i32 {
    if min < 0 {
        (value * f64::from(max)).round() as i32
    } else {
        (value * f64::from(max)).round().clamp(0.0, f64::from(max)) as i32
    }
}

struct Proxy {
    real: Device,
    /// None until the real gamepad is mapped (`Shape::Unmapped`).
    virt: Option<VirtualDevice>,
    ranges: AxisRanges,
    layout: Layout,
    /// The real gamepad's elements, and the copy the GUI reads.
    raw: RawState,
    shared_raw: Arc<Mutex<RawState>>,
    shape: Shape,
    /// What the mapping made last (`Shape::Mapped`).
    out: PadOutput,
    /// Effect id on the virtual gamepad -> effect uploaded to the real one.
    real_effects: HashMap<i16, FFEffect>,
    passthrough: bool,
    tx: EventSender,
    device: String,
}

impl Proxy {
    fn run(&mut self, stop: &AtomicBool) -> anyhow::Result<()> {
        self.real.set_nonblocking(true)?;
        if let Some(virt) = &self.virt {
            set_nonblocking(virt.as_raw_fd())?;
        }
        let mut pending: Vec<InputEvent> = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            let mut fds = [
                libc::pollfd { fd: self.real.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.virt.as_ref().map_or(-1, |v| v.as_raw_fd()), events: libc::POLLIN, revents: 0 },
            ];
            if unsafe { libc::poll(fds.as_mut_ptr(), 2, 200) } < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err.into());
            }
            if fds[0].revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                anyhow::bail!("real gamepad disconnected");
            }
            if fds[0].revents & libc::POLLIN != 0 {
                self.forward_inputs(&mut pending)?;
            }
            if fds[1].revents & libc::POLLIN != 0 {
                self.handle_game_ff()?;
            }
        }
        Ok(())
    }

    /// Real gamepad -> virtual gamepad, one frame (SYN_REPORT) at a time.
    fn forward_inputs(&mut self, pending: &mut Vec<InputEvent>) -> anyhow::Result<()> {
        let events: Vec<InputEvent> = match self.real.fetch_events() {
            Ok(it) => it.collect(),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for &ev in &events {
            match ev.destructure() {
                EventSummary::Key(_, code, value) if value != 2 => self.layout.key(&mut self.raw, code.0, value == 1),
                EventSummary::AbsoluteAxis(_, code, value) => {
                    self.layout.axis(&mut self.raw, code.0, self.ranges.full(code.0, value))
                }
                _ => {}
            }
        }
        self.shared_raw.lock().unwrap().clone_from(&self.raw);
        let Some(virt) = self.virt.as_mut() else { return Ok(()) };
        if let Shape::Mapped(mapping) = &self.shape {
            if events.iter().any(|ev| ev.event_type() == EventType::SYNCHRONIZATION && ev.code() == SynchronizationCode::SYN_REPORT.0) {
                let out = mapping.apply(&self.raw);
                let changed = xbox_360_events(&self.out, &out);
                if !changed.is_empty() {
                    virt.emit(&changed)?;
                }
                for kind in changes(&self.out, &out) {
                    self.send(kind);
                }
                self.out = out;
            }
            return Ok(());
        }
        for ev in events {
            match ev.event_type() {
                EventType::SYNCHRONIZATION if ev.code() == SynchronizationCode::SYN_REPORT.0 => {
                    self.virt.as_mut().expect("virtual gamepad").emit(pending)?; // appends the SYN_REPORT itself
                    pending.clear();
                }
                EventType::SYNCHRONIZATION | EventType::FORCEFEEDBACK => {}
                _ => {
                    // Extra buttons a combo uses stay out of the game.
                    if !(ev.event_type() == EventType::KEY && crate::gamepad::reserved(ev.code())) {
                        pending.push(ev);
                    }
                    if let Some(kind @ (SourceKind::Button { .. } | SourceKind::Axis { .. })) =
                        translate_input(ev, &self.ranges)
                    {
                        self.send(kind);
                    }
                }
            }
        }
        Ok(())
    }

    /// Game -> virtual gamepad: uploads, erasures, play/stop, gain.
    fn handle_game_ff(&mut self) -> anyhow::Result<()> {
        let Some(virt) = self.virt.as_mut() else { return Ok(()) };
        let events: Vec<InputEvent> = match virt.fetch_events() {
            Ok(it) => it.collect(),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for ev in events {
            match ev.destructure() {
                EventSummary::UInput(ev, UInputCode::UI_FF_UPLOAD, _) => {
                    let mut upload = self.virt.as_mut().expect("virtual gamepad").process_ff_upload(ev)?;
                    let id = upload.effect_id();
                    let data = upload.effect();
                    if self.passthrough {
                        let result = match self.real_effects.get_mut(&id) {
                            Some(effect) => effect.update(data),
                            None => self.real.upload_ff_effect(data).map(|e| {
                                self.real_effects.insert(id, e);
                            }),
                        };
                        if let Err(e) = result {
                            log::warn!("upload to the real gamepad failed: {e}");
                        }
                    }
                    upload.set_retval(0);
                    drop(upload); // UI_END_FF_UPLOAD: unblocks the game
                    self.send(SourceKind::Upload { id, effect: effect_from_evdev(&data) });
                }
                EventSummary::UInput(ev, UInputCode::UI_FF_ERASE, _) => {
                    let erase = self.virt.as_mut().expect("virtual gamepad").process_ff_erase(ev)?;
                    let id = erase.effect_id() as i16;
                    drop(erase);
                    self.real_effects.remove(&id); // dropping an FFEffect erases it
                    self.send(SourceKind::Erase { id });
                }
                _ => {
                    let Some(kind) = translate_input(ev, &self.ranges) else { continue };
                    if self.passthrough {
                        self.passthrough_ff(&kind);
                    }
                    self.send(kind);
                }
            }
        }
        Ok(())
    }

    fn passthrough_ff(&mut self, kind: &SourceKind) {
        let result = match *kind {
            SourceKind::Play { id, count } => match self.real_effects.get_mut(&id) {
                Some(effect) if count > 0 => effect.play(count),
                Some(effect) => effect.stop(),
                None => Ok(()),
            },
            SourceKind::Gain(gain) => self.real.send_events(&[InputEvent::new(
                EventType::FORCEFEEDBACK.0,
                FFEffectCode::FF_GAIN.0,
                gain as i32,
            )]),
            _ => Ok(()),
        };
        if let Err(e) = result {
            log::warn!("FF passthrough failed: {e}");
        }
    }

    fn send(&self, kind: SourceKind) {
        let _ = self.tx.send(SourceEvent { device: self.device.clone(), kind });
    }
}

/// The events that take the virtual Xbox 360 controller from `was` to `now`
/// (the extra buttons, which it does not have, left out).
fn xbox_360_events(was: &PadOutput, now: &PadOutput) -> Vec<InputEvent> {
    let mut events = Vec::new();
    for (name, code) in XBOX_360_BUTTONS {
        let pressed = now.held.contains(name);
        if was.held.contains(name) != pressed {
            events.push(InputEvent::new(EventType::KEY.0, code.0, i32::from(pressed)));
        }
    }
    let hat = |out: &PadOutput, minus: &str, plus: &str| i32::from(out.held.contains(plus)) - i32::from(out.held.contains(minus));
    for (code, name, min, max) in XBOX_360_AXES {
        let value = |out: &PadOutput| match (name, code) {
            (Some(name), _) => scaled(out.axes[name], min, max),
            (None, AbsoluteAxisCode::ABS_HAT0X) => hat(out, "DPAD_LEFT", "DPAD_RIGHT"),
            (None, _) => hat(out, "DPAD_UP", "DPAD_DOWN"),
        };
        if value(was) != value(now) {
            events.push(InputEvent::new(EventType::ABSOLUTE.0, code.0, value(now)));
        }
    }
    events
}

/// What GameViber hears of a mapped gamepad going from `was` to `now`.
fn changes(was: &PadOutput, now: &PadOutput) -> Vec<SourceKind> {
    let mut kinds = Vec::new();
    for name in BUTTONS {
        let Some(code) = button_code(name) else { continue };
        let pressed = now.held.contains(name);
        if was.held.contains(name) != pressed {
            kinds.push(SourceKind::Button { code, pressed });
        }
    }
    for (name, code) in [("LX", c::ABS_X), ("LY", c::ABS_Y), ("RX", c::ABS_RX), ("RY", c::ABS_RY), ("LT", c::ABS_Z), ("RT", c::ABS_RZ)] {
        if was.axes[name] != now.axes[name] {
            kinds.push(SourceKind::Axis { code, value: now.axes[name] });
        }
    }
    kinds
}

fn set_nonblocking(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mapped_gamepad_moves_the_xbox_360_controller() {
        let was = PadOutput::default();
        let mut now = PadOutput::default();
        now.held.extend(["A", "DPAD_LEFT", "P1"]);
        now.axes.insert("LX", -1.0);
        now.axes.insert("RT", 0.5);
        let events: Vec<_> = xbox_360_events(&was, &now).iter().map(|e| (e.event_type(), e.code(), e.value())).collect();
        assert_eq!(
            events,
            [
                (EventType::KEY, KeyCode::BTN_SOUTH.0, 1),
                (EventType::ABSOLUTE, AbsoluteAxisCode::ABS_X.0, -32767),
                (EventType::ABSOLUTE, AbsoluteAxisCode::ABS_RZ.0, 128),
                (EventType::ABSOLUTE, AbsoluteAxisCode::ABS_HAT0X.0, -1),
            ],
            "the back paddle stays out"
        );
        assert!(xbox_360_events(&now, &now).is_empty());
        let heard = changes(&was, &now);
        assert!(heard.iter().any(|k| matches!(k, SourceKind::Button { code: c::BTN_TRIGGER_HAPPY5, pressed: true })));
        assert!(heard.iter().any(|k| matches!(k, SourceKind::Button { code: c::BTN_DPAD_LEFT, pressed: true })));
        assert!(heard.iter().any(|k| matches!(k, SourceKind::Axis { code: c::ABS_RZ, value } if *value == 0.5)));
    }
}
