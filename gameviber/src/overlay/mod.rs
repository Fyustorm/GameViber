//! In-game overlay, GameViber side: the overlays of running games (protocol
//! in `gameviber_common::overlay`), the copies of the game's image they share,
//! and the installation of the overlay. How games reach GameViber and how the
//! overlay gets into them are OS backends (`linux`: a Unix socket and a Vulkan
//! layer / `LD_PRELOAD` library; `windows`: no overlay, GameViber captures the
//! game's window itself and shows it here like a game's overlay).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{install, install_state, launcher_path, packaged, uninstall, update_installed, NO_IMAGE_HINT, WINDOW_CAPTURE};
#[cfg(target_os = "linux")]
use linux::{FrameMemory, Socket};
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use self::windows::{install, install_state, launcher_path, packaged, uninstall, update_installed, NO_IMAGE_HINT, WINDOW_CAPTURE};
#[cfg(target_os = "windows")]
use self::windows::{FrameMemory, Socket};
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub use unsupported::{install, install_state, launcher_path, packaged, uninstall, update_installed, NO_IMAGE_HINT, WINDOW_CAPTURE};
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
use unsupported::{FrameMemory, Socket};

use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

use gameviber_common::overlay::frames::{self, Frame};
use gameviber_common::overlay::{self, Hello, OverlayState};

/// While another process holds the socket, binding it is tried again this often.
const BIND_RETRY: Duration = Duration::from_secs(5);
/// A game sending no frame for this long gives its place to another.
const WATCH_STALE: Duration = Duration::from_secs(1);

/// Overlays of running games, by their socket name.
pub struct Server {
    socket: Option<Socket>,
    /// Last failed attempt to bind the socket.
    bind_failed: Option<Instant>,
    clients: HashMap<Vec<u8>, Client>,
    buf: Vec<u8>,
    /// The game whose image is read.
    watched: Option<Vec<u8>>,
}

struct Client {
    hello: Hello,
    seen: Instant,
    frames: Option<FrameMemory>,
    /// Count of the last frame read, and when a new one last came.
    last_frame: Option<(u32, Instant)>,
}

impl Server {
    pub fn new() -> Self {
        let mut server =
            Self { socket: None, bind_failed: None, clients: HashMap::new(), buf: vec![0; overlay::MAX_DATAGRAM], watched: None };
        server.bind();
        server
    }

    /// Binds the socket games talk to. Another process holding it (another
    /// GameViber) is reported once, then tried again quietly.
    fn bind(&mut self) {
        match Socket::bind() {
            Ok(s) => {
                if self.bind_failed.is_some() {
                    log::info!("in-game overlay available again");
                }
                self.socket = Some(s);
                self.bind_failed = None;
            }
            Err(e) => {
                if self.bind_failed.is_none() {
                    let hint = if e.kind() == io::ErrorKind::AddrInUse { " (is GameViber already running?)" } else { "" };
                    log::warn!("in-game overlay unavailable{hint}, retrying: {e}");
                }
                self.bind_failed = Some(Instant::now());
            }
        }
    }

    /// The socket is held by another process: games show its state, not ours.
    pub fn unavailable(&self) -> bool {
        self.socket.is_none()
    }

    /// Reads the overlays' hellos and sends them `state`.
    pub fn update(&mut self, state: &OverlayState) {
        if self.bind_failed.is_some_and(|at| at.elapsed() >= BIND_RETRY) {
            self.bind();
        }
        let Some(socket) = &self.socket else { return };
        let now = Instant::now();
        while let Some((name, n, handle)) = socket.receive(&mut self.buf) {
            let Ok(hello) = serde_json::from_slice::<Hello>(&self.buf[..n]) else { continue };
            let client = self.clients.entry(name).or_insert_with(|| {
                log::info!("in-game overlay connected: {} ({})", hello.exe, hello.api);
                Client { hello: hello.clone(), seen: now, frames: None, last_frame: None }
            });
            client.hello = hello;
            client.seen = now;
            // Every hello carries the memory again: map it the first time only.
            if let (Some(handle), None, true) = (handle, &client.frames, client.hello.frames) {
                client.frames = FrameMemory::map(handle);
                if client.frames.is_none() {
                    log::warn!("{}: its image memory is not usable", client.hello.exe);
                }
            }
        }
        let Ok(data) = serde_json::to_vec(state) else { return };
        self.clients.retain(|name, client| {
            let alive = now.duration_since(client.seen).as_secs_f64() < overlay::CLIENT_TIMEOUT_SECS
                && socket.send_to(name, &data).is_ok();
            if !alive {
                log::info!("in-game overlay disconnected: {}", client.hello.exe);
            }
            alive
        });
    }

    /// Games currently showing the overlay.
    pub fn clients(&self) -> Vec<Hello> {
        let mut clients: Vec<_> = self.clients.values().map(|c| c.hello.clone()).collect();
        clients.sort_by(|a, b| a.exe.cmp(&b.exe));
        clients
    }

    /// The newest copy of a game's image, when one came since the last call.
    /// With several games, the one already watched keeps the place while it
    /// sends frames.
    pub fn frame(&mut self) -> Option<(Hello, Frame)> {
        let now = Instant::now();
        let mut new = Vec::new();
        for (name, client) in &mut self.clients {
            let Some(memory) = &client.frames else { continue };
            // SAFETY: the mapping is `frames::SIZE` bytes, checked when mapped.
            if let Some(frame) = unsafe { frames::read(memory.base(), client.last_frame.map(|(c, _)| c)) } {
                client.last_frame = Some((frame.count, now));
                new.push((name.clone(), client.hello.clone(), frame));
            }
        }
        let watched_alive = self.watched.as_ref().and_then(|w| self.clients.get(w)).and_then(|c| c.last_frame).is_some_and(|(_, t)| now.duration_since(t) < WATCH_STALE);
        let pick = new.iter().position(|(name, _, _)| Some(name) == self.watched.as_ref()).or((!watched_alive && !new.is_empty()).then_some(0))?;
        let (name, hello, frame) = new.swap_remove(pick);
        self.watched = Some(name);
        Some((hello, frame))
    }
}

/// Games are 64-bit or 32-bit processes (old games through Proton): each
/// needs a layer library of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    X86,
}

impl Arch {
    pub const ALL: [Arch; 2] = [Arch::X86_64, Arch::X86];

    pub fn label(self) -> &'static str {
        match self {
            Arch::X86_64 => "64-bit",
            Arch::X86 => "32-bit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstallState {
    /// This GameViber has no layer for the architecture.
    NotBuilt,
    #[default]
    NotInstalled,
    /// Installed, but different from the one shipped with this GameViber.
    Outdated,
    Installed,
}
