//! Windows: no overlay gets into games yet (no in-game panel). GameViber reads
//! the image of the game's window itself (`capture`), and that game shows up
//! here as a game's overlay would: a client sending hellos with its frame
//! memory, and receiving the state, whose capture request starts and stops
//! the window's capture. `overlay::Server` and the engine read it the same way.
//!
//! The game is the window in front when it covers its screen (fullscreen or
//! borderless) or belongs to a game library (Steam, Epic, GOG, Xbox...); it
//! stays the game while it exists, even behind GameViber's window.

mod capture;

use std::cell::{RefCell, UnsafeCell};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gameviber_common::overlay::{self, frames, CaptureRequest, Hello, OverlayState};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible};

use super::{Arch, InstallState};
use capture::WindowCapture;

/// The games' image comes from their window: no overlay to install.
pub const WINDOW_CAPTURE: bool = true;
/// What to do when no image comes.
pub const NO_IMAGE_HINT: &str = "Bring the game to the front, fullscreen or in a borderless window: GameViber reads its window.";
/// The window in front is looked at this often.
const LOOK_EVERY: Duration = Duration::from_millis(500);
/// A capture that stopped by itself is tried again after this long.
const RETRY: Duration = Duration::from_secs(5);

/// Programs whose windows are never games, even fullscreen.
const NOT_GAMES: [&str; 18] = [
    "explorer.exe",
    "gameviber.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
    "textinputhost.exe",
    "lockapp.exe",
    "taskmgr.exe",
    "applicationframehost.exe",
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "opera.exe",
    "brave.exe",
    "vlc.exe",
    "mpc-hc64.exe",
    "obs64.exe",
    "discord.exe",
];
/// Where game libraries install games.
const GAME_LIBRARIES: [&str; 9] = [
    r"\steamapps\common\",
    r"\epic games\",
    r"\gog galaxy\games\",
    r"\gog games\",
    r"\xboxgames\",
    r"\ubisoft game launcher\games\",
    r"\ea games\",
    r"\riot games\",
    r"\battle.net\",
];

/// Whether a window is a game's, from its executable and whether it covers its screen.
fn looks_like_game(exe: &str, covers_screen: bool) -> bool {
    let exe = exe.to_lowercase();
    let name = exe.rsplit('\\').next().unwrap_or(&exe);
    if NOT_GAMES.contains(&name) {
        return false;
    }
    covers_screen || GAME_LIBRARIES.iter().any(|library| exe.contains(library))
}

/// Frame memory in this process: the capture thread writes, the engine reads (a seqlock, `frames`).
pub struct Frames(Box<[UnsafeCell<u64>]>);

// Written by one thread through `frames::write`, read through `frames::read`.
unsafe impl Send for Frames {}
unsafe impl Sync for Frames {}

impl Frames {
    fn new() -> Self {
        Self((0..frames::SIZE.div_ceil(8)).map(|_| UnsafeCell::new(0)).collect())
    }

    fn base(&self) -> *mut u8 {
        self.0.as_ptr() as *mut u8
    }
}

/// The game being watched.
struct Game {
    window: isize,
    pid: u32,
    exe: String,
    frames: Arc<Frames>,
    capture: Option<(WindowCapture, CaptureRequest)>,
    /// When its capture stopped by itself.
    failed: Option<Instant>,
}

#[derive(Default)]
struct State {
    game: Option<Game>,
    looked: Option<Instant>,
    hello: Option<Instant>,
}

/// Stands for the socket games' overlays talk to.
pub struct Socket(RefCell<State>);

impl Socket {
    pub fn bind() -> io::Result<Self> {
        Ok(Self(RefCell::new(State::default())))
    }

    /// The game's hello once a second, with its frame memory while it is captured.
    pub fn receive(&self, buf: &mut [u8]) -> Option<(Vec<u8>, usize, Option<Arc<Frames>>)> {
        let mut state = self.0.borrow_mut();
        let now = Instant::now();
        if state.looked.is_none_or(|t| now - t >= LOOK_EVERY) {
            state.looked = Some(now);
            let current = state.game.as_ref().map(|g| g.window);
            match find_game(current) {
                Some((window, _, _)) if Some(window) == current => {}
                Some((window, pid, exe)) => {
                    log::info!("the game's window: {exe} (pid {pid})");
                    state.game = Some(Game { window, pid, exe, frames: Arc::new(Frames::new()), capture: None, failed: None });
                    state.hello = None;
                }
                None if current.is_some() => state.game = None,
                None => {}
            }
        }
        if state.hello.is_some_and(|t| now - t < Duration::from_secs_f64(overlay::HELLO_SECS)) {
            return None;
        }
        let game = state.game.as_ref()?;
        let capturing = game.capture.as_ref().is_some_and(|(c, _)| !c.ended());
        let hello = Hello { version: overlay::PROTOCOL_VERSION, pid: game.pid, exe: game.exe.clone(), api: "window".into(), frames: capturing };
        let (name, frames) = (name(game.window), capturing.then(|| game.frames.clone()));
        let data = serde_json::to_vec(&hello).ok()?;
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        state.hello = Some(now);
        Some((name, n, frames))
    }

    /// The state for the game: its capture request starts, changes or stops the window's capture.
    pub fn send_to(&self, name: &[u8], data: &[u8]) -> io::Result<()> {
        let mut state = self.0.borrow_mut();
        let Some(game) = state.game.as_mut().filter(|g| self::name(g.window) == name) else {
            return Err(io::ErrorKind::NotFound.into());
        };
        let wanted = serde_json::from_slice::<OverlayState>(data).map_err(io::Error::other)?.capture;
        if game.capture.as_ref().is_some_and(|(c, _)| c.ended()) {
            game.capture = None;
            game.failed = Some(Instant::now());
        }
        let running = game.capture.as_ref().map(|(_, r)| *r);
        let waiting = game.failed.is_some_and(|t| t.elapsed() < RETRY);
        if wanted != running && !(wanted.is_some() && waiting) {
            game.capture = wanted.map(|r| (WindowCapture::start(game.window, game.frames.clone(), r.width, r.fps), r));
        }
        Ok(())
    }
}

fn name(window: isize) -> Vec<u8> {
    format!("window-{window}").into_bytes()
}

/// The game's window: the one in front if it is a game's, else the current one while it exists.
fn find_game(current: Option<isize>) -> Option<(isize, u32, String)> {
    // SAFETY: plain calls on window handles, which may be stale (they then fail).
    unsafe {
        let front = GetForegroundWindow();
        if let Some(found) = game_window(front) {
            return Some(found);
        }
        let current = current?;
        IsWindow(Some(HWND(current as _))).as_bool().then(|| game_window_info(HWND(current as _))).flatten()
    }
}

unsafe fn game_window(window: HWND) -> Option<(isize, u32, String)> {
    if window.is_invalid() || !IsWindowVisible(window).as_bool() || IsIconic(window).as_bool() {
        return None;
    }
    let mut class = [0u16; 64];
    let n = GetClassNameW(window, &mut class) as usize;
    let class = String::from_utf16_lossy(&class[..n]);
    if ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"].contains(&class.as_str()) {
        return None;
    }
    let (_, _, path) = window_process(window)?;
    looks_like_game(&path, covers_screen(window)).then(|| game_window_info(window)).flatten()
}

unsafe fn game_window_info(window: HWND) -> Option<(isize, u32, String)> {
    let (_, pid, path) = window_process(window)?;
    let exe = path.rsplit('\\').next().unwrap_or(&path).to_owned();
    Some((window.0 as isize, pid, exe))
}

unsafe fn window_process(window: HWND) -> Option<(HWND, u32, String)> {
    let mut pid = 0u32;
    GetWindowThreadProcessId(window, Some(&mut pid));
    if pid == 0 || pid == std::process::id() {
        return None;
    }
    Some((window, pid, crate::platform::windows::process_path(pid)?))
}

/// The window covers its whole screen (fullscreen or borderless).
unsafe fn covers_screen(window: HWND) -> bool {
    let mut rect = Default::default();
    if GetWindowRect(window, &mut rect).is_err() {
        return false;
    }
    let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if !GetMonitorInfoW(monitor, &mut info).as_bool() {
        return false;
    }
    let screen = info.rcMonitor;
    rect.left <= screen.left && rect.top <= screen.top && rect.right >= screen.right && rect.bottom >= screen.bottom
}

/// The window capture's frame memory, as the server maps it.
pub struct FrameMemory(Arc<Frames>);

impl FrameMemory {
    pub fn map(frames: Arc<Frames>) -> Option<Self> {
        Some(Self(frames))
    }

    pub fn base(&self) -> *const u8 {
        self.0.base()
    }
}

pub fn install_state(_all_games: bool) -> Vec<(Arch, InstallState)> {
    Arch::ALL.into_iter().map(|arch| (arch, InstallState::NotBuilt)).collect()
}

pub fn install(_all_games: bool) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "there is no in-game panel on Windows yet"))
}

pub fn packaged() -> bool {
    false
}

pub fn update_installed(_all_games: bool) {}

pub fn uninstall() -> io::Result<()> {
    Ok(())
}

pub fn launcher_path() -> PathBuf {
    PathBuf::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_are_told_apart_from_other_windows() {
        assert!(looks_like_game(r"C:\Games\Hades II\Hades2.exe", true), "fullscreen");
        assert!(looks_like_game(r"D:\SteamLibrary\steamapps\common\Hades II\Ship\Hades2.exe", false), "a library's game, in a window");
        assert!(!looks_like_game(r"C:\Tools\editor.exe", false));
        assert!(!looks_like_game(r"C:\Program Files\Google\Chrome\Application\chrome.exe", true), "a video in fullscreen");
        assert!(!looks_like_game(r"C:\Windows\explorer.exe", true));
    }

    #[test]
    fn the_capture_shows_up_like_an_overlay() {
        let frames = Arc::new(Frames::new());
        // SAFETY: the memory is frames::SIZE bytes.
        unsafe {
            frames::write(frames.base(), 2, 2, (1920, 1080), |p| p.fill(7));
            let memory = FrameMemory::map(frames.clone()).unwrap();
            assert_eq!(frames::read(memory.base(), None).map(|f| (f.width, f.source_width, f.pixels)), Some((2, 1920, vec![7; 16])));
        }
    }
}
