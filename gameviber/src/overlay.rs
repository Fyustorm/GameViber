//! In-game overlay, GameViber side: the socket games' overlay layers talk to
//! (protocol in `gameviber_common::overlay`), and installation of the Vulkan
//! layer for the current user.

use std::collections::HashMap;
use std::io;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gameviber_common::overlay::{self, Hello, OverlayState};

use crate::config;

const LAYER_NAME: &str = "VK_LAYER_GAMEVIBER_overlay";
const LIBRARY: &str = "libgameviber_overlay.so";
const MANIFEST_STEM: &str = "gameviber_overlay";
/// Environment variable that turns the layer on for one game.
pub const ENABLE_ENV: &str = "GAMEVIBER_OVERLAY";
pub const DISABLE_ENV: &str = "DISABLE_GAMEVIBER_OVERLAY";
/// While another process holds the socket, binding it is tried again this often.
const BIND_RETRY: Duration = Duration::from_secs(5);

/// Overlays of running games, by their socket name.
pub struct Server {
    socket: Option<UnixDatagram>,
    /// Last failed attempt to bind the socket.
    bind_failed: Option<Instant>,
    clients: HashMap<Vec<u8>, (Hello, Instant)>,
    buf: Vec<u8>,
}

impl Server {
    pub fn new() -> Self {
        let mut server = Self { socket: None, bind_failed: None, clients: HashMap::new(), buf: vec![0; overlay::MAX_DATAGRAM] };
        server.bind();
        server
    }

    /// Binds the socket games talk to. Another process holding it (another
    /// GameViber) is reported once, then tried again quietly.
    fn bind(&mut self) {
        // SAFETY: getuid cannot fail.
        let name = overlay::server_name(unsafe { libc::getuid() });
        let socket = SocketAddr::from_abstract_name(name.as_bytes())
            .and_then(|addr| UnixDatagram::bind_addr(&addr))
            .and_then(|s| s.set_nonblocking(true).map(|()| s));
        match socket {
            Ok(s) => {
                if self.bind_failed.is_some() {
                    log::info!("in-game overlay available again");
                }
                self.socket = Some(s);
                self.bind_failed = None;
            }
            Err(e) => {
                if self.bind_failed.is_none() {
                    log::warn!("in-game overlay unavailable (is GameViber already running?), retrying: {e}");
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

/// Games are 64-bit or 32-bit processes (old games through Proton): each
/// needs a layer library of its own, declared in a manifest of its own.
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

    fn suffix(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::X86 => "x86",
        }
    }

    /// What the dynamic linker expands `$PLATFORM` to in processes of this
    /// architecture (see also [`PLATFORM_ALIASES`]).
    fn platform(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::X86 => "i686",
        }
    }

    /// Distinct names, so the loader never mixes the two libraries up.
    fn layer_name(self) -> String {
        format!("{LAYER_NAME}_{}", self.suffix())
    }

    /// Where the Vulkan loader looks for the user's implicit layers.
    fn manifest_path(self) -> PathBuf {
        data_home().join("vulkan/implicit_layer.d").join(format!("{MANIFEST_STEM}.{}.json", self.suffix()))
    }

    /// Installed copy of the library, in a directory named like `$PLATFORM`
    /// so a single `LD_PRELOAD` entry suits both architectures.
    fn library_path(self) -> PathBuf {
        install_dir().join(self.platform()).join(LIBRARY)
    }

    /// The library shipped with this GameViber: next to the executable (32-bit in
    /// `lib32/`), or where cargo builds it (`target/i686-unknown-linux-gnu/<profile>/`).
    fn bundled_library(self) -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?;
        let candidates = match self {
            Arch::X86_64 => vec![dir.join(LIBRARY)],
            Arch::X86 => {
                let mut c = vec![dir.join("lib32").join(LIBRARY)];
                if let (Some(target), Some(profile)) = (dir.parent(), dir.file_name()) {
                    c.push(target.join("i686-unknown-linux-gnu").join(profile).join(LIBRARY));
                }
                c
            }
        };
        candidates.into_iter().find(|p| p.exists())
    }

    fn manifest(self, all_games: bool) -> String {
        manifest(&self.layer_name(), &self.library_path(), all_games)
    }
}

/// glibc replaces `x86_64` with these in `$PLATFORM` on CPUs that have the
/// matching features: they are links to the `x86_64` directory.
const PLATFORM_ALIASES: [&str; 2] = ["haswell", "xeon_phi"];

fn install_dir() -> PathBuf {
    data_home().join("gameviber")
}

/// Launcher for OpenGL games (and any game): `<launcher> %command%`. It turns
/// the Vulkan layer on and preloads the library for the OpenGL hooks.
pub fn launcher_path() -> PathBuf {
    install_dir().join("gameviber-overlay")
}

fn launcher() -> String {
    format!(
        r#"#!/bin/sh
# Starts a game with the GameViber in-game overlay (written by GameViber).
# Steam launch options: {} %command%
export {ENABLE_ENV}=1
# $PLATFORM is expanded by the dynamic linker: x86_64 or i686.
export LD_PRELOAD="${{LD_PRELOAD:+$LD_PRELOAD:}}{}/\$PLATFORM/{LIBRARY}"
exec "$@"
"#,
        launcher_path().display(),
        install_dir().display()
    )
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| config::user_home().join(".local/share"))
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

/// Installation state of each architecture, for the given scope.
pub fn install_state(all_games: bool) -> Vec<(Arch, InstallState)> {
    Arch::ALL.into_iter().map(|arch| (arch, arch_state(arch, all_games))).collect()
}

fn arch_state(arch: Arch, all_games: bool) -> InstallState {
    let bundled = arch.bundled_library().and_then(|p| std::fs::read(p).ok());
    let installed = std::fs::read(arch.library_path()).ok();
    let manifest = std::fs::read_to_string(arch.manifest_path()).ok();
    let launcher_ok = arch != Arch::X86_64
        || (std::fs::read_to_string(launcher_path()).is_ok_and(|l| l == launcher())
            && PLATFORM_ALIASES.iter().all(|a| install_dir().join(a).join(LIBRARY).exists()));
    match (installed, manifest) {
        (Some(library), Some(manifest)) => {
            if bundled.is_some_and(|b| b != library) || manifest != arch.manifest(all_games) || !launcher_ok {
                InstallState::Outdated
            } else {
                InstallState::Installed
            }
        }
        _ if bundled.is_none() => InstallState::NotBuilt,
        _ => InstallState::NotInstalled,
    }
}

/// Copies the layer libraries this GameViber ships and writes their
/// manifests. With `all_games`, the layer is on in every Vulkan program
/// unless `DISABLE_GAMEVIBER_OVERLAY=1`; otherwise only where
/// `GAMEVIBER_OVERLAY=1` is set.
pub fn install(all_games: bool) -> io::Result<()> {
    remove_legacy_files();
    if Arch::X86_64.bundled_library().is_none() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("{LIBRARY} not found next to the GameViber executable")));
    }
    for arch in Arch::ALL {
        let Some(source) = arch.bundled_library() else { continue };
        let library = arch.library_path();
        if let Some(dir) = library.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Replace through a rename: running games keep the old file mapped.
        let tmp = library.with_extension("so.tmp");
        std::fs::write(&tmp, std::fs::read(&source)?)?;
        std::fs::rename(&tmp, &library)?;
        config::write_file(&arch.manifest_path(), &arch.manifest(all_games))?;
        if arch == Arch::X86_64 {
            let path = launcher_path();
            config::write_file(&path, &launcher())?;
            std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
            for alias in PLATFORM_ALIASES {
                let link = install_dir().join(alias);
                let _ = std::fs::remove_file(&link);
                std::os::unix::fs::symlink(Arch::X86_64.platform(), &link)?;
            }
        }
        log::info!(
            "in-game overlay installed for {} games ({})",
            arch.label(),
            if all_games { "every Vulkan game" } else { "games with GAMEVIBER_OVERLAY=1" }
        );
    }
    Ok(())
}

/// Brings an installed overlay up to date with the one this GameViber ships,
/// keeping its scope, so that games get the features of this version. Does
/// nothing when the overlay is not installed.
pub fn update_installed(all_games: bool) {
    let states = install_state(all_games);
    let installed = states.iter().any(|(arch, s)| *arch == Arch::X86_64 && *s != InstallState::NotInstalled && *s != InstallState::NotBuilt);
    if !installed || !states.iter().any(|(_, s)| *s == InstallState::Outdated) {
        return;
    }
    match install(all_games) {
        Ok(()) => log::info!("in-game overlay updated to this version of GameViber: restart running games to get it"),
        Err(e) => log::warn!("cannot update the in-game overlay: {e}"),
    }
}

/// Library locations of earlier versions.
fn remove_legacy_files() {
    let dir = install_dir();
    let _ = std::fs::remove_file(dir.join(LIBRARY));
    let _ = std::fs::remove_file(dir.join("lib32").join(LIBRARY));
    let _ = std::fs::remove_dir(dir.join("lib32"));
}

pub fn uninstall() -> io::Result<()> {
    remove_legacy_files();
    for arch in Arch::ALL {
        let mut paths = vec![arch.manifest_path(), arch.library_path()];
        if arch == Arch::X86_64 {
            paths.push(launcher_path());
            paths.extend(PLATFORM_ALIASES.iter().map(|a| install_dir().join(a)));
        }
        for path in paths {
            match std::fs::remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        let _ = std::fs::remove_dir(install_dir().join(arch.platform()));
    }
    log::info!("in-game overlay removed");
    Ok(())
}

fn manifest(name: &str, library: &Path, all_games: bool) -> String {
    let enable = if all_games { String::new() } else { format!("\n    \"enable_environment\": {{ \"{ENABLE_ENV}\": \"1\" }},") };
    format!(
        r#"{{
  "file_format_version": "1.0.0",
  "layer": {{
    "name": "{name}",
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
        let per_game: serde_json::Value = serde_json::from_str(&manifest("VK_LAYER_X", path, false)).unwrap();
        assert_eq!(per_game["layer"]["name"], "VK_LAYER_X");
        assert_eq!(per_game["layer"]["enable_environment"][ENABLE_ENV], "1");
        assert_eq!(per_game["layer"]["library_path"], path.to_str().unwrap());
        let everywhere: serde_json::Value = serde_json::from_str(&manifest("VK_LAYER_X", path, true)).unwrap();
        assert!(everywhere["layer"].get("enable_environment").is_none());
        assert_eq!(everywhere["layer"]["disable_environment"][DISABLE_ENV], "1");
    }

    #[test]
    fn architectures_have_distinct_layers() {
        assert_ne!(Arch::X86_64.layer_name(), Arch::X86.layer_name());
        assert_ne!(Arch::X86_64.manifest_path(), Arch::X86.manifest_path());
        assert_ne!(Arch::X86_64.library_path(), Arch::X86.library_path());
    }

    #[test]
    fn launcher_preloads_by_platform() {
        let script = launcher();
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains("export GAMEVIBER_OVERLAY=1\n"));
        let preload = format!("{}/\\$PLATFORM/{LIBRARY}", install_dir().display());
        assert!(script.contains(&preload), "{script}");
        for arch in Arch::ALL {
            let expanded = preload.replace("\\$PLATFORM", arch.platform());
            assert_eq!(Path::new(&expanded), arch.library_path());
        }
    }
}
