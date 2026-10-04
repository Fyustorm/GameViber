//! "proxy" source: the real gamepad is grabbed and a virtual copy (uinput,
//! same name and VID/PID) is exposed to games. Inputs are forwarded to the
//! copy; the force feedback games upload to it is captured, then forwarded
//! to the real gamepad (passthrough).

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
    AbsInfo, AttributeSet, Device, EventSummary, EventType, FFEffect, FFEffectCode, InputEvent, SynchronizationCode,
    UInputCode, UinputAbsSetup,
};

use super::{axis_ranges, effect_from_evdev, find_gamepad, translate_input};
use crate::gamepad::AxisRanges;
use crate::platform::linux::helper::client::{Helper, Phase};
use crate::platform::linux::helper::Request;
use crate::platform::linux::hider::DeviceHider;
use crate::source::{ActiveSource, EventSender, SourceEvent, SourceHealth, SourceKind};

const FF_CODES: [FFEffectCode; 6] = [
    FFEffectCode::FF_RUMBLE,
    FFEffectCode::FF_PERIODIC,
    FFEffectCode::FF_SQUARE,
    FFEffectCode::FF_TRIANGLE,
    FFEffectCode::FF_SINE,
    FFEffectCode::FF_GAIN,
];

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
}

impl ProxySource {
    pub fn start(device: Option<&Path>, passthrough: bool, hide: Hide, tx: EventSender) -> anyhow::Result<Self> {
        let (real_path, mut real) = find_gamepad(device)?;
        let mut virt = build_virtual(&real).context("cannot create the virtual gamepad")?;
        let virt_path = virt
            .enumerate_dev_nodes_blocking()
            .context("cannot find the virtual gamepad node")?
            .filter_map(Result::ok)
            .find(|p| p.to_string_lossy().contains("event"))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "?".into());
        real.grab().context("cannot grab the real gamepad")?;
        let name = real.name().unwrap_or("?").to_owned();
        log::info!("virtual gamepad '{name}' created at {virt_path} (real one at {real_path} grabbed)");

        let hidden = match hide {
            Hide::No => Hidden::No,
            Hide::Local => Hidden::Local(DeviceHider::hide(&real_path)?),
            Hide::Helper(helper) => {
                helper.request(Request::Hide { device: real_path.clone() });
                Hidden::Helper(helper)
            }
        };
        let status = Arc::new(Mutex::new(format!("proxy: {name} ({real_path} → {virt_path})")));

        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let thread = {
            let (stop, status, error) = (stop.clone(), status.clone(), error.clone());
            let ranges = axis_ranges(&real);
            std::thread::Builder::new().name("proxy".into()).spawn(move || {
                let mut proxy =
                    Proxy { real, virt, ranges, real_effects: HashMap::new(), passthrough, tx, device: virt_path };
                if let Err(e) = proxy.run(&stop) {
                    log::error!("proxy stopped: {e:#}");
                    *status.lock().unwrap() = format!("proxy stopped: {e:#}");
                    *error.lock().unwrap() = Some(format!("{e:#}"));
                }
            })?
        };
        Ok(Self { stop, thread: Some(thread), hidden, status, name, error })
    }
}

impl ActiveSource for ProxySource {
    fn health(&self) -> SourceHealth {
        if let Some(e) = self.error.lock().unwrap().clone() {
            return SourceHealth::Failed(e);
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

fn build_virtual(real: &Device) -> anyhow::Result<VirtualDevice> {
    let name = real.name().unwrap_or("Gamepad").to_owned();
    let mut builder = VirtualDevice::builder()?
        .name(&name)
        .input_id(real.input_id())
        // No with_phys: evdev 0.13 encodes UI_SET_PHYS with a 1-byte size instead
        // of a pointer and the kernel rejects it. No game reads phys anyway.
        .with_ff(&FF_CODES.iter().copied().collect::<AttributeSet<_>>())
        .context("ff")?
        .with_ff_effects_max(real.max_ff_effects().max(1) as u32);
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

struct Proxy {
    real: Device,
    virt: VirtualDevice,
    ranges: AxisRanges,
    /// Effect id on the virtual gamepad -> effect uploaded to the real one.
    real_effects: HashMap<i16, FFEffect>,
    passthrough: bool,
    tx: EventSender,
    device: String,
}

impl Proxy {
    fn run(&mut self, stop: &AtomicBool) -> anyhow::Result<()> {
        self.real.set_nonblocking(true)?;
        set_nonblocking(self.virt.as_raw_fd())?;
        let mut pending: Vec<InputEvent> = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            let mut fds = [
                libc::pollfd { fd: self.real.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.virt.as_raw_fd(), events: libc::POLLIN, revents: 0 },
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
        for ev in events {
            match ev.event_type() {
                EventType::SYNCHRONIZATION if ev.code() == SynchronizationCode::SYN_REPORT.0 => {
                    self.virt.emit(pending)?; // appends the SYN_REPORT itself
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
        let events: Vec<InputEvent> = match self.virt.fetch_events() {
            Ok(it) => it.collect(),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for ev in events {
            match ev.destructure() {
                EventSummary::UInput(ev, UInputCode::UI_FF_UPLOAD, _) => {
                    let mut upload = self.virt.process_ff_upload(ev)?;
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
                    let erase = self.virt.process_ff_erase(ev)?;
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

fn set_nonblocking(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
