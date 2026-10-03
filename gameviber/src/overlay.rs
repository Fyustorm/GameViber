//! In-game overlay, GameViber side: the socket games' overlay layers talk to
//! (protocol in `gameviber_common::overlay`), and installation of the Vulkan
//! layer for the current user.

use std::collections::HashMap;
use std::io;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::path::{Path, PathBuf};
use std::time::Instant;

use gameviber_common::overlay::{self, Hello, OverlayState};

use crate::config;

pub const LAYER_NAME: &str = "VK_LAYER_GAMEVIBER_overlay";
const LIBRARY: &str = "libgameviber_overlay.so";
const MANIFEST: &str = "gameviber_overlay.x86_64.json";
/// Environment variable that turns the layer on for one game.
pub const ENABLE_ENV: &str = "GAMEVIBER_OVERLAY";
pub const DISABLE_ENV: &str = "DISABLE_GAMEVIBER_OVERLAY";

/// Overlays of running games, by their socket name.
pub struct Server {
    socket: Option<UnixDatagram>,
    clients: HashMap<Vec<u8>, (Hello, Instant)>,
    buf: Vec<u8>,
}

impl Server {
    pub fn new() -> Self {
        // SAFETY: getuid cannot fail.
        let name = overlay::server_name(unsafe { libc::getuid() });
        let socket = SocketAddr::from_abstract_name(name.as_bytes())
            .and_then(|addr| UnixDatagram::bind_addr(&addr))
            .and_then(|s| s.set_nonblocking(true).map(|()| s));
        let socket = match socket {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("in-game overlay unavailable (is GameViber already running?): {e}");
                None
            }
        };
        Self { socket, clients: HashMap::new(), buf: vec![0; overlay::MAX_DATAGRAM] }
    }

    /// Reads the overlays' hellos and sends them `state`.
    pub fn update(&mut self, state: &OverlayState) {
        let Some(socket) = &self.socket else { return };
        let now = Instant::now();
        while let Ok((n, addr)) = socket.recv_from(&mut self.buf) {
            let (Some(name), Ok(hello)) = (addr.as_abstract_name(), serde_json::from_slice::<Hello>(&self.buf[..n])) else {
                continue;
            };
            if !self.clients.contains_key(name) {
                log::info!("in-game overlay connected: {} ({})", hello.exe, hello.api);
            }
            self.clients.insert(name.to_vec(), (hello, now));
        }
        let Ok(data) = serde_json::to_vec(state) else { return };
        self.clients.retain(|name, (hello, seen)| {
            let alive = now.duration_since(*seen).as_secs_f64() < overlay::CLIENT_TIMEOUT_SECS
                && SocketAddr::from_abstract_name(name).and_then(|addr| socket.send_to_addr(&data, &addr)).is_ok();
            if !alive {
                log::info!("in-game overlay disconnected: {}", hello.exe);
            }
            alive
        });
    }

    /// Games currently showing the overlay.
    pub fn clients(&self) -> Vec<Hello> {
        let mut clients: Vec<_> = self.clients.values().map(|(h, _)| h.clone()).collect();
        clients.sort_by(|a, b| a.exe.cmp(&b.exe));
        clients
    }
}

/// Where the Vulkan loader looks for the user's implicit layers.
fn manifest_path() -> PathBuf {
    data_home().join("vulkan/implicit_layer.d").join(MANIFEST)
}

/// Installed copy of the layer library.
fn library_path() -> PathBuf {
    data_home().join("gameviber").join(LIBRARY)
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| config::user_home().join(".local/share"))
}

/// The layer library built next to the GameViber executable.
fn bundled_library() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.parent()?.join(LIBRARY);
    path.exists().then_some(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstallState {
    #[default]
    NotInstalled,
    /// Installed, but the library differs from the one shipped with this GameViber.
    Outdated,
    Installed,
}

pub fn install_state() -> InstallState {
    let (Ok(installed), Ok(_)) = (std::fs::read(library_path()), std::fs::metadata(manifest_path())) else {
        return InstallState::NotInstalled;
    };
    match bundled_library().and_then(|p| std::fs::read(p).ok()) {
        Some(bundled) if bundled != installed => InstallState::Outdated,
        _ => InstallState::Installed,
    }
}

/// Copies the layer library and writes its manifest. With `all_games`, the
/// layer is on in every Vulkan program unless `DISABLE_GAMEVIBER_OVERLAY=1`;
/// otherwise only where `GAMEVIBER_OVERLAY=1` is set.
pub fn install(all_games: bool) -> io::Result<()> {
    let source = bundled_library()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{LIBRARY} not found next to the GameViber executable")))?;
    let library = library_path();
    let bytes = std::fs::read(&source)?;
    // Replace through a rename: running games keep the old file mapped.
    let tmp = library.with_extension("so.tmp");
    if let Some(dir) = library.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &library)?;
    config::write_file(&manifest_path(), &manifest(&library, all_games))?;
    log::info!("in-game overlay installed ({})", if all_games { "every Vulkan game" } else { "games with GAMEVIBER_OVERLAY=1" });
    Ok(())
}

pub fn uninstall() -> io::Result<()> {
    for path in [manifest_path(), library_path()] {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    log::info!("in-game overlay removed");
    Ok(())
}

fn manifest(library: &Path, all_games: bool) -> String {
    let enable = if all_games { String::new() } else { format!("\n    \"enable_environment\": {{ \"{ENABLE_ENV}\": \"1\" }},") };
    format!(
        r#"{{
  "file_format_version": "1.0.0",
  "layer": {{
    "name": "{LAYER_NAME}",
    "type": "GLOBAL",
    "api_version": "1.3.0",
    "library_path": "{}",
    "implementation_version": "1",
    "description": "GameViber in-game overlay",
    "functions": {{
      "vkGetInstanceProcAddr": "gameviber_GetInstanceProcAddr",
      "vkGetDeviceProcAddr": "gameviber_GetDeviceProcAddr"
    }},{enable}
    "disable_environment": {{ "{DISABLE_ENV}": "1" }}
  }}
}}
"#,
        library.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_valid_json_and_scoped() {
        let path = Path::new("/home/me/.local/share/gameviber/libgameviber_overlay.so");
        let per_game: serde_json::Value = serde_json::from_str(&manifest(path, false)).unwrap();
        assert_eq!(per_game["layer"]["enable_environment"][ENABLE_ENV], "1");
        assert_eq!(per_game["layer"]["library_path"], path.to_str().unwrap());
        let everywhere: serde_json::Value = serde_json::from_str(&manifest(path, true)).unwrap();
        assert!(everywhere["layer"].get("enable_environment").is_none());
        assert_eq!(everywhere["layer"]["disable_environment"][DISABLE_ENV], "1");
    }
}
