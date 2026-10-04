//! "ebpf" source: passive observation, games keep seeing the real gamepad.
//!
//! - Effect uploads / erasures (EVIOCSFF / EVIOCRMFF ioctls) are captured by
//!   the eBPF probe, loaded in-process when running as root, through the
//!   privileged helper otherwise. The ioctl's fd, resolved through
//!   `/proc/<pid>/fd/<fd>`, tells which gamepad is targeted.
//! - Play/stop/gain (EV_FF writes) are echoed by the kernel to every evdev
//!   reader: each gamepad is read without grabbing it, which also yields
//!   buttons and axes. No privilege needed for this part.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use aya::maps::{MapData, RingBuf};
use aya::programs::TracePoint;
use aya::{include_bytes_aligned, Ebpf};
use evdev::Device;
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;

use super::{axis_ranges, list_ff_devices, translate_input};
use crate::platform::linux::helper::client::{Helper, Phase};
use crate::platform::linux::helper::{read_record, Request, WireProbe};
use crate::platform::linux::is_root;
use crate::source::{ActiveSource, EventSender, SourceEvent, SourceHealth, SourceKind};

const RESCAN_INTERVAL: Duration = Duration::from_secs(2);

/// Loads and attaches the probe (root only). Returns the program handle, which
/// must stay alive, and its event ring buffer.
pub fn load_probe() -> anyhow::Result<(Ebpf, RingBuf<MapData>)> {
    anyhow::ensure!(is_root(), "loading the eBPF probe needs root");
    let bytes = include_bytes_aligned!(concat!(env!("OUT_DIR"), "/gameviber-ebpf"));
    anyhow::ensure!(!bytes.is_empty(), "binary built without the eBPF probe (SKIP_EBPF_BUILD)");
    let mut bpf = Ebpf::load(bytes).context("cannot load the eBPF probe")?;
    for (name, tracepoint) in [("ioctl_enter", "sys_enter_ioctl"), ("ioctl_exit", "sys_exit_ioctl")] {
        let program: &mut TracePoint = bpf.program_mut(name).context(name)?.try_into()?;
        program.load()?;
        program.attach("syscalls", tracepoint)?;
    }
    let ring = RingBuf::try_from(bpf.take_map("RECORDS").context("RECORDS map")?)?;
    log::info!("eBPF probe attached (EVIOCSFF / EVIOCRMFF)");
    Ok((bpf, ring))
}

enum Probe {
    /// Keeps the in-process probe attached.
    Local { _bpf: Ebpf },
    Helper(Arc<Helper>),
}

pub struct EbpfSource {
    probe: Probe,
    stop: Arc<AtomicBool>,
    /// Watched device path -> gamepad name.
    watched: Arc<Mutex<HashMap<String, String>>>,
}

impl EbpfSource {
    /// Must be called from within a tokio runtime.
    pub fn start(tx: EventSender, helper: &Arc<Helper>) -> anyhow::Result<Self> {
        let (probe_tx, probe_rx) = mpsc::unbounded_channel::<WireProbe>();
        tokio::spawn(forward_probe_events(probe_rx, tx.clone()));
        let probe = if is_root() {
            let (bpf, ring) = load_probe()?;
            tokio::spawn(read_local_ring(AsyncFd::new(ring)?, probe_tx));
            Probe::Local { _bpf: bpf }
        } else {
            helper.set_probe_sink(Some(probe_tx));
            helper.request(Request::StartEbpf);
            Probe::Helper(helper.clone())
        };
        let stop = Arc::new(AtomicBool::new(false));
        let watched = Arc::new(Mutex::new(HashMap::new()));
        spawn_device_watcher(tx, watched.clone(), stop.clone());
        Ok(Self { probe, stop, watched })
    }
}

impl ActiveSource for EbpfSource {
    fn status(&self) -> String {
        let watching = || {
            let mut devices: Vec<_> = self.watched.lock().unwrap().keys().cloned().collect();
            devices.sort();
            format!("ebpf: watching {}", if devices.is_empty() { "no device".into() } else { devices.join(", ") })
        };
        match &self.probe {
            Probe::Local { .. } => watching(),
            Probe::Helper(helper) => {
                let state = helper.state();
                match state.phase {
                    Phase::NotStarted | Phase::Authorizing => "ebpf: waiting for authorization...".into(),
                    Phase::Failed(e) => format!("ebpf unavailable: {e}"),
                    Phase::Ready if state.ebpf => watching(),
                    Phase::Ready => match state.last_error {
                        Some(e) => format!("ebpf unavailable: {e}"),
                        None => "ebpf: loading the probe...".into(),
                    },
                }
            }
        }
    }

    /// Names of the watched gamepads.
    fn gamepads(&self) -> Vec<String> {
        let mut names: Vec<_> = self.watched.lock().unwrap().values().cloned().collect();
        names.sort();
        names
    }

    fn health(&self) -> SourceHealth {
        let Probe::Helper(helper) = &self.probe else { return SourceHealth::Working };
        let state = helper.state();
        match state.phase {
            Phase::NotStarted | Phase::Authorizing => SourceHealth::Waiting("waiting for your password".into()),
            Phase::Failed(e) => SourceHealth::Failed(e),
            Phase::Ready if state.ebpf => SourceHealth::Working,
            Phase::Ready => match state.last_error {
                Some(e) => SourceHealth::Failed(e),
                None => SourceHealth::Waiting("loading the probe".into()),
            },
        }
    }

    fn shutdown(self: Box<Self>) {
        self.stop.store(true, Ordering::Relaxed);
        if let Probe::Helper(helper) = &self.probe {
            helper.set_probe_sink(None);
            helper.request(Request::StopEbpf);
        }
    }
}

async fn read_local_ring(mut ring: AsyncFd<RingBuf<MapData>>, probe_tx: mpsc::UnboundedSender<WireProbe>) {
    loop {
        let mut guard = match ring.readable_mut().await {
            Ok(g) => g,
            Err(e) => {
                log::error!("eBPF ring buffer unusable: {e}");
                return;
            }
        };
        while let Some(item) = guard.get_inner_mut().next() {
            let Some(record) = read_record(&item) else { continue };
            if probe_tx.send(WireProbe::from(&record)).is_err() {
                return;
            }
        }
        guard.clear_ready();
    }
}

/// Probe events -> source events, attributed to the gamepad the ioctl targeted.
async fn forward_probe_events(mut probe_rx: mpsc::UnboundedReceiver<WireProbe>, tx: EventSender) {
    while let Some(probe) = probe_rx.recv().await {
        let Some(device) = resolve_fd(probe.pid, probe.fd) else {
            log::debug!("effect {} of pid {}: fd {} not resolved, ignored", probe.effect_id, probe.pid, probe.fd);
            continue;
        };
        let kind = match probe.effect() {
            Some(effect) => SourceKind::Upload { id: probe.effect_id, effect },
            None if probe.erased => SourceKind::Erase { id: probe.effect_id },
            None => continue,
        };
        if tx.send(SourceEvent { device, kind }).is_err() {
            return;
        }
    }
}

/// `/proc/<pid>/fd/<fd>` -> `/dev/input/eventN`. Works unprivileged for games
/// running as the same user.
fn resolve_fd(tgid: u32, fd: i32) -> Option<String> {
    let target = std::fs::read_link(format!("/proc/{tgid}/fd/{fd}")).ok()?;
    let target = target.to_string_lossy().into_owned();
    target.starts_with("/dev/input/event").then_some(target)
}

/// Opens a passive reader on every FF device, including those plugged after
/// startup (gamepads, Steam Input's virtual gamepad...).
fn spawn_device_watcher(tx: EventSender, watched: Arc<Mutex<HashMap<String, String>>>, stop: Arc<AtomicBool>) {
    let _ = std::thread::Builder::new().name("ff-watcher".into()).spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            for (path, dev) in list_ff_devices() {
                let name = dev.name().unwrap_or("?").to_owned();
                if watched.lock().unwrap().insert(path.clone(), name.clone()).is_some() {
                    continue;
                }
                log::info!("watching {path} ({name})");
                let (tx, watched, stop) = (tx.clone(), watched.clone(), stop.clone());
                let _ = std::thread::Builder::new().name(format!("read {path}")).spawn(move || {
                    read_device(&path, dev, &tx, &stop);
                    log::info!("{path} is no longer watched");
                    watched.lock().unwrap().remove(&path);
                    let _ = tx.send(SourceEvent { device: path, kind: SourceKind::Removed });
                });
            }
            std::thread::sleep(RESCAN_INTERVAL);
        }
    });
}

/// Blocking reader; after `stop` it exits on the next event of the device.
fn read_device(path: &str, mut dev: Device, tx: &EventSender, stop: &AtomicBool) {
    let ranges = axis_ranges(&dev);
    loop {
        let events = match dev.fetch_events() {
            Ok(it) => it.collect::<Vec<_>>(),
            Err(e) => {
                log::debug!("reading {path}: {e}");
                return;
            }
        };
        if stop.load(Ordering::Relaxed) {
            return;
        }
        for ev in events {
            if let Some(kind) = translate_input(ev, &ranges) {
                if tx.send(SourceEvent { device: path.to_owned(), kind }).is_err() {
                    return;
                }
            }
        }
    }
}
