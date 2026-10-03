//! Connection to GameViber: hello datagrams out, overlay states in. Never blocks.

use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use gameviber_common::overlay::{self, Hello, OverlayState};

/// Sockets made by this process, to give each a distinct name.
static SOCKETS: AtomicU32 = AtomicU32::new(0);

pub struct Client {
    socket: Option<UnixDatagram>,
    hello: Vec<u8>,
    last_hello: Option<Instant>,
    state: Option<(OverlayState, Instant)>,
    buf: Vec<u8>,
}

impl Client {
    pub fn new(api: &str) -> Self {
        let hello = Hello {
            version: overlay::PROTOCOL_VERSION,
            pid: std::process::id(),
            exe: exe_name(),
            api: api.to_owned(),
        };
        Self {
            socket: None,
            hello: serde_json::to_vec(&hello).unwrap_or_default(),
            last_hello: None,
            state: None,
            buf: vec![0; overlay::MAX_DATAGRAM],
        }
    }

    /// The latest state, or `None` when GameViber is silent.
    pub fn poll(&mut self) -> Option<&OverlayState> {
        let now = Instant::now();
        if self.last_hello.is_none_or(|t| now.duration_since(t).as_secs_f64() >= overlay::HELLO_SECS) {
            self.last_hello = Some(now);
            self.send_hello();
        }
        if let Some(socket) = &self.socket {
            while let Ok(n) = socket.recv(&mut self.buf) {
                if let Ok(state) = serde_json::from_slice::<OverlayState>(&self.buf[..n]) {
                    self.state = Some((state, now));
                }
            }
        }
        if self.state.as_ref().is_some_and(|(_, t)| now.duration_since(*t).as_secs_f64() > overlay::STATE_TIMEOUT_SECS) {
            self.state = None;
        }
        self.state.as_ref().map(|(s, _)| s)
    }

    fn send_hello(&mut self) {
        if self.socket.is_none() {
            self.socket = connect();
        }
        let sent = self.socket.as_ref().is_some_and(|s| s.send(&self.hello).is_ok());
        if !sent {
            // GameViber is not running (or restarted): reconnect on the next hello.
            self.socket = None;
        }
    }
}

fn connect() -> Option<UnixDatagram> {
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    // A name of our own, so GameViber can answer.
    let n = SOCKETS.fetch_add(1, Ordering::Relaxed);
    let name = format!("{}-{}-{n}", overlay::server_name(uid), std::process::id());
    let socket = UnixDatagram::bind_addr(&SocketAddr::from_abstract_name(name.as_bytes()).ok()?).ok()?;
    let server = SocketAddr::from_abstract_name(overlay::server_name(uid).as_bytes()).ok()?;
    socket.connect_addr(&server).ok()?;
    socket.set_nonblocking(true).ok()?;
    Some(socket)
}

fn exe_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}
