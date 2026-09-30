//! Source « eBPF » : observation passive, le jeu voit la vraie manette.
//!
//! - Les uploads / effacements d'effets (ioctl EVIOCSFF / EVIOCRMFF) sont
//!   capturés par la sonde eBPF (root requis). Le fd de l'ioctl, résolu via
//!   `/proc/<pid>/fd/<fd>`, indique la manette visée.
//! - Les play/stop/gain (écritures EV_FF) sont renvoyés par le noyau à tous
//!   les lecteurs evdev : on lit chaque manette sans grab, ce qui donne aussi
//!   les boutons et axes.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use aya::maps::RingBuf;
use aya::programs::TracePoint;
use aya::{include_bytes_aligned, Ebpf};
use evdev::Device;
use gameviber_common::{ProbeEvent, PROBE_EVENT_KIND_ERASED};
use tokio::io::unix::AsyncFd;

use super::{list_ff_devices, translate_input, EventSender, SourceEvent, SourceKind};
use crate::rumble::Effect;

const RESCAN_INTERVAL: Duration = Duration::from_secs(2);

pub struct EbpfSource {
    _bpf: Ebpf,
}

impl EbpfSource {
    pub fn start(tx: EventSender) -> anyhow::Result<Self> {
        anyhow::ensure!(unsafe { libc::geteuid() } == 0, "la source ebpf nécessite root (sudo)");
        let bytes = include_bytes_aligned!(concat!(env!("OUT_DIR"), "/gameviber-ebpf"));
        anyhow::ensure!(!bytes.is_empty(), "binaire compilé sans la sonde eBPF (SKIP_EBPF_BUILD)");
        let mut bpf = Ebpf::load(bytes).context("chargement de la sonde eBPF")?;
        for name in ["sys_enter_ioctl", "sys_exit_ioctl"] {
            let program: &mut TracePoint = bpf.program_mut(name).context(name)?.try_into()?;
            program.load()?;
            program.attach("syscalls", name)?;
        }
        log::info!("Sonde eBPF attachée (EVIOCSFF / EVIOCRMFF)");

        let ring = RingBuf::try_from(bpf.take_map("EVENTS").context("map EVENTS")?)?;
        tokio::spawn(read_probe(AsyncFd::new(ring)?, tx.clone()));
        spawn_device_watcher(tx);
        Ok(Self { _bpf: bpf })
    }
}

async fn read_probe(mut ring: AsyncFd<RingBuf<aya::maps::MapData>>, tx: EventSender) {
    loop {
        let mut guard = match ring.readable_mut().await {
            Ok(g) => g,
            Err(e) => {
                log::error!("ring buffer eBPF inutilisable : {e}");
                return;
            }
        };
        while let Some(item) = guard.get_inner_mut().next() {
            if item.len() < std::mem::size_of::<ProbeEvent>() {
                continue;
            }
            let ev: ProbeEvent = unsafe { std::ptr::read_unaligned(item.as_ptr() as *const ProbeEvent) };
            let Some(device) = resolve_fd(ev.tgid, ev.fd) else {
                log::debug!("effet {} de pid {} : fd {} non résolu, ignoré", ev.effect_id, ev.tgid, ev.fd);
                continue;
            };
            let kind = if ev.kind == PROBE_EVENT_KIND_ERASED {
                SourceKind::Erase { id: ev.effect_id }
            } else {
                SourceKind::Upload { id: ev.effect_id, effect: Effect::from_probe(&ev.effect) }
            };
            let _ = tx.send(SourceEvent { device, kind });
        }
        guard.clear_ready();
    }
}

/// `/proc/<pid>/fd/<fd>` -> `/dev/input/eventN`.
fn resolve_fd(tgid: u32, fd: i32) -> Option<String> {
    let target = std::fs::read_link(format!("/proc/{tgid}/fd/{fd}")).ok()?;
    let target = target.to_string_lossy().into_owned();
    target.starts_with("/dev/input/event").then_some(target)
}

/// Ouvre un lecteur passif sur chaque device FF, y compris ceux branchés après
/// le démarrage (manettes, manette virtuelle de Steam Input...).
fn spawn_device_watcher(tx: EventSender) {
    let watched = Arc::new(Mutex::new(HashSet::<String>::new()));
    std::thread::Builder::new()
        .name("ff-watcher".into())
        .spawn(move || loop {
            for (path, dev) in list_ff_devices() {
                if !watched.lock().unwrap().insert(path.clone()) {
                    continue;
                }
                log::info!("Observation de {path} ({})", dev.name().unwrap_or("?"));
                let (tx, watched) = (tx.clone(), watched.clone());
                let _ = std::thread::Builder::new().name(format!("read {path}")).spawn(move || {
                    read_device(&path, dev, &tx);
                    log::info!("{path} n'est plus observé");
                    watched.lock().unwrap().remove(&path);
                });
            }
            std::thread::sleep(RESCAN_INTERVAL);
        })
        .expect("thread ff-watcher");
}

fn read_device(path: &str, mut dev: Device, tx: &EventSender) {
    loop {
        let events = match dev.fetch_events() {
            Ok(it) => it.collect::<Vec<_>>(),
            Err(e) => {
                log::debug!("lecture de {path} : {e}");
                return;
            }
        };
        for ev in events {
            if let Some(kind) = translate_input(ev) {
                if tx.send(SourceEvent { device: path.to_owned(), kind }).is_err() {
                    return;
                }
            }
        }
    }
}
