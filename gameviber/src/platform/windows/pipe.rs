//! Named pipes other processes write lines to (`\\.\pipe\gameviber-<user>-<name>`):
//! the single instance and the external inputs. Their default permissions let
//! only the user who made them, and administrators, write to them.

use std::fs::File;
use std::io::{self, Write};
use std::os::windows::io::{FromRawHandle, OwnedHandle};

use windows::core::HSTRING;
use windows::Win32::Foundation::{ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND};
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT};

/// Where the pipe `name` of this user is.
pub fn path(name: &str) -> String {
    let user: String = std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_alphanumeric()).collect();
    format!(r"\\.\pipe\gameviber-{user}-{name}")
}

/// A pipe waiting for writers.
pub struct Server {
    path: String,
    /// The instance the next writer connects to.
    next: OwnedHandle,
}

impl Server {
    /// The pipe `name`; `first`: fails if another process already has it.
    pub fn bind(name: &str, first: bool) -> io::Result<Self> {
        let path = path(name);
        let next = instance(&path, first)?;
        Ok(Self { path, next })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Waits for a writer: what it writes, until it closes its end.
    pub fn accept(&mut self) -> io::Result<File> {
        // SAFETY: the handle is a pipe instance of ours.
        match unsafe { ConnectNamedPipe(HANDLE(raw(&self.next)), None) } {
            Ok(()) => {}
            // Connected before we waited; or connected, wrote and left: what it wrote is still there.
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() || e.code() == ERROR_NO_DATA.to_hresult() => {}
            Err(e) => return Err(io::Error::from_raw_os_error(e.code().0 & 0xffff)),
        }
        let connected = std::mem::replace(&mut self.next, instance(&self.path, false)?);
        Ok(File::from(connected))
    }
}

fn raw(handle: &OwnedHandle) -> *mut core::ffi::c_void {
    use std::os::windows::io::AsRawHandle;
    handle.as_raw_handle()
}

fn instance(path: &str, first: bool) -> io::Result<OwnedHandle> {
    let mode = if first { PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE } else { PIPE_ACCESS_INBOUND };
    // SAFETY: plain call; the handle is owned below.
    let handle = unsafe {
        CreateNamedPipeW(&HSTRING::from(path), mode, PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, 0, 4096, 0, None)
    };
    if handle.is_invalid() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a new handle nobody else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.0) })
}

/// Writes `line` to the pipe `name`, if a process has it.
pub fn send(name: &str, line: &str) -> io::Result<()> {
    let started = std::time::Instant::now();
    let mut pipe = loop {
        match std::fs::OpenOptions::new().write(true).open(path(name)) {
            Ok(pipe) => break pipe,
            // Every instance is taken: one is free again in a moment.
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) && started.elapsed().as_secs() < 2 => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    };
    writeln!(pipe, "{line}")
}
