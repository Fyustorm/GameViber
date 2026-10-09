//! "proxy" source: games get a virtual Xbox 360 controller (ViGEmBus) and
//! GameViber hears the rumble they send it. The real gamepad is read and
//! passed on: an Xbox controller through XInput as it is (its rumble passed
//! back to it too), any other through HID and its mapping (`gamepad::mapping`,
//! SDL_GameControllerDB's Windows lines or the player's). Without a mapping,
//! the real gamepad is only read, for the player to set up its buttons.
//!
//! The real gamepad can be hidden from games with HidHide, before the virtual
//! one comes: games then only see the virtual controller.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Context;
use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::UI::Input::XboxController::{XInputGetState, XInputSetState, XINPUT_GAMEPAD, XINPUT_STATE, XINPUT_VIBRATION};

use super::hid::{self, HidInfo, HidPad};
use crate::gamepad::mapping::{self, Mapping, Origin, PadOutput, RawState};
use crate::platform::windows::hidhide;
use crate::rumble::{Effect, EffectKind};
use crate::source::{changes, ActiveSource, EventSender, PadInfo, PadLayout, SourceEvent, SourceHealth, SourceKind, SourceOptions};

/// The real gamepad is read this often.
const POLL: Duration = Duration::from_millis(4);
/// How long the virtual controller may take to show in XInput.
const PLUG_WAIT: Duration = Duration::from_secs(2);
const XINPUT_SLOTS: u32 = 4;
/// XInput's buttons, by our names.
const XBOX_BUTTONS: [(&str, u16); 15] = [
    ("DPAD_UP", XButtons::UP),
    ("DPAD_DOWN", XButtons::DOWN),
    ("DPAD_LEFT", XButtons::LEFT),
    ("DPAD_RIGHT", XButtons::RIGHT),
    ("START", XButtons::START),
    ("BACK", XButtons::BACK),
    ("LS", XButtons::LTHUMB),
    ("RS", XButtons::RTHUMB),
    ("LB", XButtons::LB),
    ("RB", XButtons::RB),
    ("GUIDE", XButtons::GUIDE),
    ("A", XButtons::A),
    ("B", XButtons::B),
    ("X", XButtons::X),
    ("Y", XButtons::Y),
];

/// The real gamepad.
enum Real {
    /// An Xbox controller, in this XInput slot.
    XInput(u32),
    Hid(HidPad),
}

/// What games get.
enum Shape {
    /// The Xbox controller's state as it is.
    Copy,
    /// An Xbox 360 controller made from the gamepad's mapping.
    Mapped(Mapping),
    /// Nothing: the gamepad's buttons are to be set up.
    Unmapped,
}

pub struct ProxySource {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    status: Arc<Mutex<String>>,
    name: String,
    /// Why the proxy stopped.
    error: Arc<Mutex<Option<String>>>,
    /// Hiding the real gamepad waits for the user to allow it.
    hiding: Arc<AtomicBool>,
    /// What was hidden, given back when the source stops.
    hidden: Arc<Mutex<Vec<String>>>,
    hint: Option<String>,
    pad: PadInfo,
    raw: Arc<Mutex<RawState>>,
}

fn connected(slot: u32) -> bool {
    let mut state = XINPUT_STATE::default();
    // SAFETY: plain call with a state to fill.
    unsafe { XInputGetState(slot, &mut state) == ERROR_SUCCESS.0 }
}

fn connected_slots() -> Vec<u32> {
    (0..XINPUT_SLOTS).filter(|&slot| connected(slot)).collect()
}

/// SDL's GUID of an XInput controller ("xinput" and its subtype, a gamepad).
fn xinput_guid() -> String {
    let mut bytes = [0u8; 16];
    bytes[..6].copy_from_slice(b"xinput");
    bytes[14] = 1;
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The gamepad `device` names ("xinput:<slot>", a HID path or a name), else the first one.
fn find_gamepad(device: Option<&str>) -> anyhow::Result<(Real, String, String)> {
    let slots = connected_slots();
    let hids = hid::list();
    if let Some(wanted) = device {
        if let Some(slot) = wanted.strip_prefix("xinput:").and_then(|s| s.parse::<u32>().ok()) {
            anyhow::ensure!(slots.contains(&slot), "no Xbox controller in XInput slot {slot}");
            return Ok((Real::XInput(slot), format!("Xbox controller (XInput {})", slot + 1), wanted.to_owned()));
        }
        let info = hids.into_iter().find(|h| h.path.eq_ignore_ascii_case(wanted) || h.name == wanted).with_context(|| format!("no gamepad {wanted}"))?;
        return open_hid(info);
    }
    if let Some(&slot) = slots.first() {
        return Ok((Real::XInput(slot), format!("Xbox controller (XInput {})", slot + 1), format!("xinput:{slot}")));
    }
    let info = hids.into_iter().next().context("no gamepad found: plug one in")?;
    open_hid(info)
}

fn open_hid(info: HidInfo) -> anyhow::Result<(Real, String, String)> {
    let (name, path) = (info.name.clone(), info.path.clone());
    Ok((Real::Hid(HidPad::open(info)?), name, path))
}

impl ProxySource {
    pub fn start(opts: &SourceOptions, tx: EventSender) -> anyhow::Result<Self> {
        let wanted = opts.device.as_ref().map(|d| d.display().to_string());
        let (real, name, real_path) = find_gamepad(wanted.as_deref())?;
        let (guid, rumble, raw) = match &real {
            Real::XInput(_) => (xinput_guid(), true, xbox_raw(&XINPUT_GAMEPAD::default())),
            Real::Hid(pad) => (mapping::sdl_guid(0x03, pad.info.vendor, pad.info.product, pad.info.version, &pad.info.name), false, pad.rest()),
        };
        let (shape, layout) = match (&real, mapping::find(&guid)) {
            (Real::XInput(_), Some((m, Origin::User))) => (Shape::Mapped(m), PadLayout::Mapped(Origin::User)),
            (Real::XInput(_), _) => (Shape::Copy, PadLayout::Driver),
            (Real::Hid(_), Some((m, origin))) => (Shape::Mapped(m), PadLayout::Mapped(origin)),
            (Real::Hid(_), None) => (Shape::Unmapped, PadLayout::Missing),
        };
        let mapping = match &shape {
            Shape::Mapped(m) => Some(m.clone()),
            Shape::Copy | Shape::Unmapped => None,
        };
        let pad = PadInfo { name: name.clone(), guid: guid.clone(), layout, mapping, raw: raw.clone(), rumble };

        let client = match shape {
            Shape::Unmapped => None,
            _ => Some(Client::connect().map_err(|e| match e {
                vigem_client::Error::BusNotFound => anyhow::anyhow!(
                    "ViGEmBus is not installed: GameViber needs it for the virtual controller (its installer offers it, \
                     or https://github.com/nefarius/ViGEmBus/releases)"
                ),
                e => anyhow::anyhow!("cannot reach ViGEmBus: {e}"),
            })?),
        };
        let hide = opts.hide && client.is_some();
        let hint = (hide && !hidhide::installed()).then(|| "Install HidHide to hide the real gamepad from games (GameViber's installer offers it).".to_owned());
        let hidden_ids = match (&real, hide && hint.is_none()) {
            (_, false) => Vec::new(),
            (Real::Hid(pad), true) => hid::instance_ids(&pad.info.path),
            // The XInput gamepads' HID side, before the virtual one is plugged in.
            (Real::XInput(_), true) => {
                hid::interface_paths().iter().filter(|p| p.to_ascii_uppercase().contains("IG_")).flat_map(|p| hid::instance_ids(p)).collect()
            }
        };
        log::info!("{name} ({real_path}, {guid}): {}", match shape {
            Shape::Copy => "passed on as it is",
            Shape::Mapped(_) => "passed on as an Xbox 360 controller",
            Shape::Unmapped => "no mapping: read until its buttons are set up",
        });

        let status = Arc::new(Mutex::new(format!("proxy: {name} ({real_path})")));
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let hiding = Arc::new(AtomicBool::new(!hidden_ids.is_empty()));
        let hidden = Arc::new(Mutex::new(Vec::new()));
        let shared_raw = Arc::new(Mutex::new(raw.clone()));
        let thread = {
            let (stop, status, error, hiding, hidden, shared_raw) =
                (stop.clone(), status.clone(), error.clone(), hiding.clone(), hidden.clone(), shared_raw.clone());
            let name = name.clone();
            std::thread::Builder::new().name("proxy".into()).spawn(move || {
                if !hidden_ids.is_empty() {
                    match hidhide::hide(&hidden_ids) {
                        Ok(()) => {
                            log::info!("{name} hidden from games");
                            *hidden.lock().unwrap() = hidden_ids;
                        }
                        Err(e) => log::warn!("cannot hide {name}: {e:#}"),
                    }
                    hiding.store(false, Ordering::Relaxed);
                }
                let mut proxy = Proxy { real, shape, raw, shared_raw, out: PadOutput::default(), tx, device: real_path, rumble_thread: None };
                if let Err(e) = proxy.run(client, &stop, &status, &name) {
                    log::error!("proxy stopped: {e:#}");
                    *status.lock().unwrap() = format!("proxy stopped: {e:#}");
                    *error.lock().unwrap() = Some(format!("{e:#}"));
                }
            })?
        };
        Ok(Self { stop, thread: Some(thread), status, name, error, hiding, hidden, hint, pad, raw: shared_raw })
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
        if self.hiding.load(Ordering::Relaxed) {
            return SourceHealth::Waiting("waiting for you to allow hiding the real gamepad".into());
        }
        SourceHealth::Working
    }

    fn status(&self) -> String {
        let hidden = if self.hidden.lock().unwrap().is_empty() { "" } else { ", real gamepad hidden" };
        format!("{}{hidden}", self.status.lock().unwrap())
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

    fn hint(&self) -> Option<String> {
        self.hint.clone()
    }

    fn shutdown(mut self: Box<Self>) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let hidden = std::mem::take(&mut *self.hidden.lock().unwrap());
        if !hidden.is_empty() {
            if let Err(e) = hidhide::unhide(&hidden) {
                log::warn!("cannot give {} back to games: {e:#}", self.name);
            }
        }
    }
}

struct Proxy {
    real: Real,
    shape: Shape,
    /// The real gamepad's elements (HID), and the copy the GUI reads.
    raw: RawState,
    shared_raw: Arc<Mutex<RawState>>,
    /// What the gamepad made last, in the Xbox layout.
    out: PadOutput,
    tx: EventSender,
    device: String,
    rumble_thread: Option<JoinHandle<()>>,
}

impl Proxy {
    fn run(&mut self, client: Option<Client>, stop: &AtomicBool, status: &Mutex<String>, name: &str) -> anyhow::Result<()> {
        let mut virt = match client {
            Some(client) => Some(self.plug(client, status, name)?),
            None => None,
        };
        let mut last_packet = None;
        while !stop.load(Ordering::Relaxed) {
            let started = Instant::now();
            match &mut self.real {
                Real::XInput(slot) => {
                    let mut state = XINPUT_STATE::default();
                    // SAFETY: plain call with a state to fill.
                    if unsafe { XInputGetState(*slot, &mut state) } != ERROR_SUCCESS.0 {
                        anyhow::bail!("real gamepad disconnected");
                    }
                    if last_packet != Some(state.dwPacketNumber) {
                        last_packet = Some(state.dwPacketNumber);
                        self.raw = xbox_raw(&state.Gamepad);
                        self.shared_raw.lock().unwrap().clone_from(&self.raw);
                        let out = match &self.shape {
                            Shape::Mapped(mapping) => mapping.apply(&self.raw),
                            _ => xbox_output(&state.Gamepad),
                        };
                        if let Some(virt) = virt.as_mut() {
                            let report = match &self.shape {
                                Shape::Copy => XGamepad::from_xinput(&state.Gamepad),
                                _ => report(&out),
                            };
                            virt.update(&report).map_err(|e| anyhow::anyhow!("the virtual controller stopped: {e}"))?;
                        }
                        self.heard(out);
                    }
                }
                Real::Hid(pad) => {
                    if pad.poll(&mut self.raw).context("real gamepad disconnected")? {
                        self.shared_raw.lock().unwrap().clone_from(&self.raw);
                        if let Shape::Mapped(mapping) = &self.shape {
                            let out = mapping.apply(&self.raw);
                            if let Some(virt) = virt.as_mut() {
                                virt.update(&report(&out)).map_err(|e| anyhow::anyhow!("the virtual controller stopped: {e}"))?;
                            }
                            self.heard(out);
                        }
                    }
                }
            }
            std::thread::sleep(POLL.saturating_sub(started.elapsed()));
        }
        // Unplugs it, which ends the rumble thread.
        drop(virt);
        if let Some(t) = self.rumble_thread.take() {
            let _ = t.join();
        }
        Ok(())
    }

    /// Plugs the virtual controller in, and listens to the rumble games send it.
    fn plug(&mut self, client: Client, status: &Mutex<String>, name: &str) -> anyhow::Result<Xbox360Wired<Client>> {
        let before = connected_slots();
        let mut virt = Xbox360Wired::new(client, TargetId::XBOX360_WIRED);
        virt.plugin().map_err(|e| anyhow::anyhow!("cannot create the virtual controller: {e}"))?;
        virt.wait_ready().map_err(|e| anyhow::anyhow!("the virtual controller did not start: {e}"))?;
        let started = Instant::now();
        let slot = loop {
            if let Some(slot) = connected_slots().into_iter().find(|s| !before.contains(s)) {
                break Some(slot);
            }
            if started.elapsed() > PLUG_WAIT {
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let virtual_name = match slot {
            Some(slot) => format!("virtual Xbox 360 controller, XInput {}", slot + 1),
            None => "virtual Xbox 360 controller".to_owned(),
        };
        self.device = virtual_name.clone();
        log::info!("{virtual_name} created for {name}");
        *status.lock().unwrap() = match self.shape {
            Shape::Copy => format!("proxy: {name} → {virtual_name}"),
            _ => format!("proxy: {name} → {virtual_name}, with its mapping"),
        };
        let passthrough = match self.real {
            Real::XInput(slot) => Some(slot),
            Real::Hid(_) => None,
        };
        let (tx, device) = (self.tx.clone(), self.device.clone());
        let notifications = virt.request_notification().map_err(|e| anyhow::anyhow!("cannot hear the virtual controller's rumble: {e}"))?;
        let mut uploaded = false;
        self.rumble_thread = Some(notifications.spawn_thread(move |_, n| {
            let send = |kind| {
                let _ = tx.send(SourceEvent { device: device.clone(), kind });
            };
            let (strong, weak) = (u16::from(n.large_motor) * 257, u16::from(n.small_motor) * 257);
            if strong > 0 || weak > 0 || uploaded {
                send(SourceKind::Upload { id: 0, effect: Effect { kind: EffectKind::Rumble { strong, weak }, length_ms: 0, delay_ms: 0 } });
                uploaded = true;
            }
            send(SourceKind::Play { id: 0, count: i32::from(strong > 0 || weak > 0) });
            if let Some(slot) = passthrough {
                let vibration = XINPUT_VIBRATION { wLeftMotorSpeed: strong, wRightMotorSpeed: weak };
                // SAFETY: plain call.
                unsafe { XInputSetState(slot, &vibration) };
            }
        }));
        Ok(virt)
    }

    /// What GameViber hears of the gamepad now.
    fn heard(&mut self, out: PadOutput) {
        for kind in changes(&self.out, &out) {
            let _ = self.tx.send(SourceEvent { device: self.device.clone(), kind });
        }
        self.out = out;
    }
}

trait FromXInput {
    fn from_xinput(gamepad: &XINPUT_GAMEPAD) -> Self;
}

impl FromXInput for XGamepad {
    fn from_xinput(g: &XINPUT_GAMEPAD) -> Self {
        XGamepad {
            buttons: XButtons(g.wButtons.0),
            left_trigger: g.bLeftTrigger,
            right_trigger: g.bRightTrigger,
            thumb_lx: g.sThumbLX,
            thumb_ly: g.sThumbLY,
            thumb_rx: g.sThumbRX,
            thumb_ry: g.sThumbRY,
        }
    }
}

/// An Xbox controller in our layout: sticks -1..1 with down positive (like Linux), triggers 0..1.
fn xbox_output(g: &XINPUT_GAMEPAD) -> PadOutput {
    let mut out = PadOutput::default();
    for (name, bit) in XBOX_BUTTONS {
        if g.wButtons.0 & bit != 0 {
            out.held.insert(name);
        }
    }
    let stick = |v: i16| (f64::from(v) / 32767.0).clamp(-1.0, 1.0);
    out.axes.insert("LX", stick(g.sThumbLX));
    out.axes.insert("LY", -stick(g.sThumbLY));
    out.axes.insert("RX", stick(g.sThumbRX));
    out.axes.insert("RY", -stick(g.sThumbRY));
    out.axes.insert("LT", f64::from(g.bLeftTrigger) / 255.0);
    out.axes.insert("RT", f64::from(g.bRightTrigger) / 255.0);
    out
}

/// An Xbox controller's elements as SDL numbers them on Windows (XInput): buttons
/// A B X Y LB RB BACK START LS RS GUIDE, axes LX LY RX RY LT RT, the d-pad as hat 0.
fn xbox_raw(g: &XINPUT_GAMEPAD) -> RawState {
    let pressed = |bit: u16| g.wButtons.0 & bit != 0;
    let buttons = [XButtons::A, XButtons::B, XButtons::X, XButtons::Y, XButtons::LB, XButtons::RB, XButtons::BACK, XButtons::START, XButtons::LTHUMB, XButtons::RTHUMB, XButtons::GUIDE]
        .map(pressed)
        .to_vec();
    let out = xbox_output(g);
    let axes = ["LX", "LY", "RX", "RY"].iter().map(|a| out.axes[a]).chain(["LT", "RT"].iter().map(|t| out.axes[t] * 2.0 - 1.0)).collect();
    let hat = [(XButtons::UP, 1), (XButtons::RIGHT, 2), (XButtons::DOWN, 4), (XButtons::LEFT, 8)].iter().filter(|(bit, _)| pressed(*bit)).map(|(_, m)| m).sum();
    RawState { buttons, axes, hats: vec![hat] }
}

/// The virtual controller's report for `out`.
fn report(out: &PadOutput) -> XGamepad {
    let mut buttons = 0;
    for (name, bit) in XBOX_BUTTONS {
        if out.held.contains(name) {
            buttons |= bit;
        }
    }
    let stick = |v: f64| (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
    let trigger = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    XGamepad {
        buttons: XButtons(buttons),
        left_trigger: trigger(out.axes["LT"]),
        right_trigger: trigger(out.axes["RT"]),
        thumb_lx: stick(out.axes["LX"]),
        thumb_ly: stick(-out.axes["LY"]),
        thumb_rx: stick(out.axes["RX"]),
        thumb_ry: stick(-out.axes["RY"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_BUTTON_FLAGS;

    #[test]
    fn an_xbox_controller_reads_and_writes_in_our_layout() {
        let g = XINPUT_GAMEPAD {
            wButtons: XINPUT_GAMEPAD_BUTTON_FLAGS(XButtons::A | XButtons::LEFT),
            bLeftTrigger: 255,
            sThumbLY: 32767,
            ..Default::default()
        };
        let out = xbox_output(&g);
        assert!(out.held.contains("A") && out.held.contains("DPAD_LEFT"));
        assert_eq!((out.axes["LY"], out.axes["LT"]), (-1.0, 1.0), "up is negative, like Linux");
        let back = report(&out);
        assert_eq!((back.buttons.raw, back.thumb_ly, back.left_trigger), (XButtons::A | XButtons::LEFT, 32767, 255));
        let raw = xbox_raw(&g);
        assert_eq!((raw.buttons[0], raw.hats[0], raw.axes[4]), (true, 8, 1.0));
        assert!(changes(&PadOutput::default(), &out).iter().any(|k| matches!(k, SourceKind::Button { pressed: true, .. })));
    }

    #[test]
    fn the_xinput_guid_is_sdls() {
        assert_eq!(xinput_guid(), "78696e70757400000000000000000100");
    }
}
