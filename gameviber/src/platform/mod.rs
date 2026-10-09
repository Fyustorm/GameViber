//! What differs between operating systems, for the code that does not care
//! which one it runs on: where files go, the local time, the signals that stop
//! the engine, the GUI window system dialogs open over, file dialogs, and the
//! privileged helper's entry point.
//!
//! OS-specific code lives only in modules named after the OS (`linux.rs` or
//! `linux/`, `windows.rs` or `windows/`, declared with `#[cfg(target_os = ...)]`):
//! here, and in the parts with a backend per OS (`source`, `audio::capture`,
//! `overlay`, `shortcuts`, `external`, `links`, `update`). Other systems get each
//! part's `unsupported` module, so that everything else builds and runs there
//! without these features. See AGENTS.md, Platforms.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub use linux::{
    chown_to_caller, config_dir, data_dir, keep_private, local_time, open_file, open_files, privileged_subcommand, save_file, steam_app_id, window_closing,
    window_created, window_expected, window_focused, StopSignals,
};

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "windows")]
pub use self::windows::{
    chown_to_caller, config_dir, data_dir, keep_private, local_time, open_file, open_files, privileged_subcommand, save_file, steam_app_id, window_closing,
    window_created, window_expected, window_focused, StopSignals,
};

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub use unsupported::{
    chown_to_caller, config_dir, data_dir, keep_private, local_time, open_file, open_files, privileged_subcommand, save_file, steam_app_id, window_closing,
    window_created, window_expected, window_focused, StopSignals,
};
