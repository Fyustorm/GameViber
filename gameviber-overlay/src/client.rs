//! Connection to GameViber: hello datagrams out, overlay states in. Never blocks.
//! While GameViber wants the game's image, the hellos also carry the frame
//! memory's file descriptor.

use std::os::fd::AsRawFd;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use gameviber_common::overlay::{self, CaptureRequest, Hello, OverlayState};

use crate::capture::{Pace, SharedFrames};

/// Sockets made by this process, to give each a distinct name.
static SOCKETS: AtomicU32 = AtomicU32::new(0);

pub struct Client {
    socket: Option<UnixDatagram>,
    hello: Hello,
    last_hello: Option<Instant>,
    state: Option<(OverlayState, Instant)>,
    buf: Vec<u8>,
    frames: Option<SharedFrames>,
    pace: Pace,
}

impl Client {
    pub fn new(api: &str) -> Self {
        let hello = Hello {
            version: overlay::PROTOCOL_VERSION,
            pid: std::process::id(),
            exe: exe_name(),
            api: api.to_owned(),
            frames: false,
        };
        Self {
            socket: None,
            hello,
            last_hello: None,
            state: None,
            buf: vec![0; overlay::MAX_DATAGRAM],
            frames: None,
            pace: Pace::default(),
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
        let wants_frames = self.state.as_ref().is_some_and(|(s, _)| s.capture.is_some());
        if wants_frames && self.frames.is_none() {
            self.frames = SharedFrames::new();
            if self.frames.is_none() {
                crate::log("cannot share the game's image: memfd unavailable");
            }
            // Hand the memory over right away.
            self.send_hello();
        }
        self.state.as_ref().map(|(s, _)| s)
    }

    /// What GameViber asks for, when it wants the game's image.
    pub fn capture_request(&self) -> Option<CaptureRequest> {
        self.state.as_ref().and_then(|(s, _)| s.capture)
    }

    /// Whether a copy of the game's image should be made for this frame.
    pub fn capture_due(&mut self) -> bool {
        let request = self.state.as_ref().and_then(|(s, _)| s.capture.as_ref());
        self.frames.is_some() && self.pace.due(request)
    }

    /// Where copies of the game's image go.
    pub fn frames(&self) -> Option<&SharedFrames> {
        self.frames.as_ref()
    }

    fn send_hello(&mut self) {
        if self.socket.is_none() {
            self.socket = connect();
        }
        self.hello.frames = self.frames.is_some();
        let hello = serde_json::to_vec(&self.hello).unwrap_or_default();
        let fd = self.frames.as_ref().map(SharedFrames::fd);
        let sent = self.socket.as_ref().is_some_and(|s| send(s, &hello, fd));
        if !sent {
            // GameViber is not running (or restarted): reconnect on the next hello.
            self.socket = None;
        }
    }
}

/// Sends `data`, with the descriptor `fd` attached when given.
fn send(socket: &UnixDatagram, data: &[u8], fd: Option<i32>) -> bool {
    let Some(fd) = fd else { return socket.send(data).is_ok() };
    // SAFETY: sendmsg with a control buffer sized by CMSG_SPACE for one descriptor.
    unsafe {
        let mut iov = libc::iovec { iov_base: data.as_ptr() as *mut libc::c_void, iov_len: data.len() };
        let space = libc::CMSG_SPACE(std::mem::size_of::<i32>() as u32) as usize;
        let mut control = vec![0u8; space];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
        msg.msg_controllen = space as _;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<i32>() as u32) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg) as *mut i32, fd);
        libc::sendmsg(socket.as_raw_fd(), &msg, libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL) >= 0
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
