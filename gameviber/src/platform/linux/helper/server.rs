//! Root side of the helper (`gameviber helper`, started through pkexec).

use std::io::{BufRead, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::Context;
use aya::maps::{MapData, RingBuf};
use aya::Ebpf;
use gameviber_common::ProbeEvent;

use super::{Reply, Request, WireProbe};
use crate::platform::linux::hider::DeviceHider;
use crate::source::linux::load_probe;

type Output = Arc<Mutex<std::io::Stdout>>;

fn send(out: &Output, reply: &Reply) {
    let mut out = out.lock().unwrap();
    // A closed stdout means the engine is gone: stdin EOF will end the loop.
    if let Ok(line) = serde_json::to_string(reply) {
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

struct RunningProbe {
    _bpf: Ebpf,
    stop: Arc<AtomicBool>,
    reader: JoinHandle<()>,
}

impl RunningProbe {
    fn start(out: Output) -> anyhow::Result<Self> {
        let (bpf, ring) = load_probe()?;
        let stop = Arc::new(AtomicBool::new(false));
        let reader = {
            let stop = stop.clone();
            std::thread::Builder::new().name("probe".into()).spawn(move || read_ring(ring, &stop, &out))?
        };
        Ok(Self { _bpf: bpf, stop, reader })
    }

    fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.reader.join();
    }
}

fn read_ring(mut ring: RingBuf<MapData>, stop: &AtomicBool, out: &Output) {
    while !stop.load(Ordering::Relaxed) {
        let mut fds = [libc::pollfd { fd: ring.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
        if unsafe { libc::poll(fds.as_mut_ptr(), 1, 200) } <= 0 {
            continue;
        }
        while let Some(item) = ring.next() {
            if item.len() < std::mem::size_of::<ProbeEvent>() {
                continue;
            }
            let ev: ProbeEvent = unsafe { std::ptr::read_unaligned(item.as_ptr() as *const ProbeEvent) };
            send(out, &Reply::Probe(WireProbe::from(&ev)));
        }
    }
}

/// Entry point of `gameviber helper`. Returns when stdin closes.
pub fn run() -> anyhow::Result<()> {
    anyhow::ensure!(unsafe { libc::geteuid() } == 0, "the helper must run as root (through pkexec)");
    // Only stdin EOF ends the helper, so it always gets to restore the gamepad:
    // a Ctrl+C in the engine's terminal or a closed session must not kill it.
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    let out: Output = Arc::new(Mutex::new(std::io::stdout()));
    let mut probe: Option<RunningProbe> = None;
    let mut hider: Option<DeviceHider> = None;
    send(&out, &Reply::Ready);

    for line in std::io::stdin().lock().lines() {
        let line = line.context("reading requests")?;
        let request = match serde_json::from_str::<Request>(&line) {
            Ok(r) => r,
            Err(e) => {
                send(&out, &Reply::Error { message: format!("invalid request: {e}") });
                continue;
            }
        };
        let reply = match request {
            Request::StartEbpf if probe.is_some() => Reply::EbpfStarted,
            Request::StartEbpf => match RunningProbe::start(out.clone()) {
                Ok(p) => {
                    probe = Some(p);
                    Reply::EbpfStarted
                }
                Err(e) => Reply::Error { message: format!("{e:#}") },
            },
            Request::StopEbpf => {
                if let Some(p) = probe.take() {
                    p.stop();
                }
                Reply::EbpfStopped
            }
            Request::Hide { device } => {
                if let Some(h) = hider.take() {
                    h.restore();
                }
                match DeviceHider::hide(&device) {
                    Ok(h) => {
                        let device = h.device().to_owned();
                        hider = Some(h);
                        Reply::Hidden { device }
                    }
                    Err(e) => Reply::Error { message: format!("{e:#}") },
                }
            }
            Request::Unhide => {
                if let Some(h) = hider.take() {
                    h.restore();
                }
                Reply::Unhidden
            }
        };
        send(&out, &reply);
    }

    // Engine gone: undo everything.
    if let Some(h) = hider.take() {
        h.restore();
    }
    if let Some(p) = probe.take() {
        p.stop();
    }
    Ok(())
}
