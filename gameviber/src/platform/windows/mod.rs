//! Windows: the user's application data directories, the local time, the
//! console's stop events, the system's file dialogs over GameViber's window,
//! the registry (Steam's running game, GameViber's links), HidHide.

mod files;
pub mod hidhide;
pub mod pipe;
pub mod registry;

pub use files::{open_file, open_files, save_file};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::OnceLock;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close, ctrl_shutdown, CtrlBreak, CtrlC, CtrlClose, CtrlShutdown};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HMODULE};
use windows::Win32::System::Diagnostics::Debug::{SetUnhandledExceptionFilter, EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER};
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, ProcessPowerThrottling, QueryFullProcessImageNameW, SetProcessInformation, PROCESS_NAME_WIN32,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_LIMITED_INFORMATION,
};

fn known_dir(var: &str, fallback: &[&str]) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        fallback.iter().fold(home, |path, part| path.join(part))
    })
}

/// `%APPDATA%\GameViber`: settings, modes and games, which roam with the user.
pub fn config_dir() -> PathBuf {
    known_dir("APPDATA", &["AppData", "Roaming"]).join("GameViber")
}

/// `%LOCALAPPDATA%\GameViber`: recordings, models, logs.
pub fn data_dir() -> PathBuf {
    known_dir("LOCALAPPDATA", &["AppData", "Local"]).join("GameViber")
}

/// Nothing to do: files in the user's profile are only theirs (inherited permissions).
pub fn keep_private(_path: &Path) {}

/// Nothing to do: GameViber never runs as another user.
pub fn chown_to_caller(_path: &Path) {}

/// The filter that was there before ours, called after it.
static PREVIOUS_FILTER: OnceLock<LPTOP_LEVEL_EXCEPTION_FILTER> = OnceLock::new();

/// Crashes (an access violation, a stack overflow) go to the log: their code
/// and the module and offset they happened at; Windows then handles them as before.
pub fn log_crashes() {
    // SAFETY: the filter only reads the record Windows hands it.
    let previous = unsafe { SetUnhandledExceptionFilter(Some(on_crash)) };
    let _ = PREVIOUS_FILTER.set(previous);
}

unsafe extern "system" fn on_crash(info: *const EXCEPTION_POINTERS) -> i32 {
    if let Some(record) = info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) {
        let address = record.ExceptionAddress as usize;
        let mut module = HMODULE::default();
        let mut name = [0u16; 260];
        let place = if GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, PCWSTR(address as *const u16), &mut module).is_ok() {
            let n = GetModuleFileNameW(Some(module), &mut name) as usize;
            let path = String::from_utf16_lossy(&name[..n]);
            format!("{}+{:#x}", path.rsplit('\\').next().unwrap_or(&path), address - module.0 as usize)
        } else {
            format!("{address:#x}")
        };
        let thread = std::thread::current();
        log::error!("crash in thread {}: exception {:#010x} at {place}", thread.name().unwrap_or("?"), record.ExceptionCode.0 as u32);
    }
    match PREVIOUS_FILTER.get().copied().flatten() {
        Some(previous) => previous(info),
        None => 0, // EXCEPTION_CONTINUE_SEARCH
    }
}

/// GameViber keeps its speed and its timers' precision while a game is in
/// front: Windows otherwise slows background processes down (power throttling).
pub fn full_speed() {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        StateMask: 0,
    };
    // SAFETY: the state and its size, for this process.
    let result = unsafe {
        SetProcessInformation(GetCurrentProcess(), ProcessPowerThrottling, &state as *const _ as _, std::mem::size_of_val(&state) as u32)
    };
    if let Err(e) = result {
        log::debug!("cannot turn power throttling off: {e}");
    }
}

/// "2026-10-03 21:14:05", local time.
pub fn local_time() -> String {
    // SAFETY: GetLocalTime cannot fail.
    let t = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

/// Ctrl+C, Ctrl+Break, the console closed, the session ending: the engine stops cleanly.
pub struct StopSignals {
    c: CtrlC,
    break_: CtrlBreak,
    close: CtrlClose,
    shutdown: CtrlShutdown,
}

impl StopSignals {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self { c: ctrl_c()?, break_: ctrl_break()?, close: ctrl_close()?, shutdown: ctrl_shutdown()? })
    }

    pub async fn recv(&mut self) {
        tokio::select! {
            _ = self.c.recv() => {}
            _ = self.break_.recv() => {}
            _ = self.close.recv() => {}
            _ = self.shutdown.recv() => {}
        }
    }
}

/// `gameviber hidhide ...`: changes HidHide's lists as administrator instead of running the app.
pub fn privileged_subcommand() -> Option<anyhow::Result<()>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    (args.first().map(String::as_str) == Some(hidhide::SUBCOMMAND)).then(|| {
        crate::logging::init(false);
        hidhide::subcommand(&args[1..])
    })
}

/// GameViber's window (an `HWND`), which dialogs open over; 0 without one.
static WINDOW: AtomicIsize = AtomicIsize::new(0);

pub(crate) fn window() -> Option<isize> {
    Some(WINDOW.load(Ordering::Relaxed)).filter(|w| *w != 0)
}

pub fn window_expected() {}

pub fn window_created(cc: &eframe::CreationContext) {
    if let Ok(RawWindowHandle::Win32(handle)) = cc.window_handle().map(|h| h.as_raw()) {
        WINDOW.store(handle.hwnd.get(), Ordering::Relaxed);
    }
}

pub fn window_focused(_focused: bool) {}

pub fn window_closing() {
    WINDOW.store(0, Ordering::Relaxed);
}

/// The Steam app id of the game Steam says it runs (`RunningAppID`, the
/// running game's environment being out of reach); None when it runs none.
pub fn steam_app_id(_pid: u32) -> Option<u32> {
    registry::read_u32(registry::Root::CurrentUser, r"Software\Valve\Steam", "RunningAppID").filter(|id| *id != 0)
}

/// The executable of a process (`C:\Games\Game.exe`); None for a process of another user.
pub fn process_path(pid: u32) -> Option<String> {
    // SAFETY: the handle is closed below; the buffer's size is passed.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size);
        let _ = CloseHandle(process);
        result.ok()?;
        Some(String::from_utf16_lossy(&buffer[..size as usize]))
    }
}
