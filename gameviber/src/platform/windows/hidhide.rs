//! HidHide (https://github.com/nefarius/HidHide): hides devices from every
//! program but those on its list. GameViber puts the real gamepad on its
//! list of hidden devices, and itself on the list of programs still seeing
//! them, then takes the gamepad off when it stops. HidHide's control device
//! only lets administrators change its lists: when GameViber may not, it
//! starts itself as administrator for that (`gameviber hidhide hide|unhide
//! <device>...`), which Windows asks the user to allow.
//!
//! The devices GameViber hid are noted in its data directory, so that a
//! GameViber that crashed gives them back the next time.

use std::path::PathBuf;

use anyhow::Context;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, QueryDosDeviceW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

/// `gameviber hidhide ...`: changes HidHide's lists, as administrator.
pub const SUBCOMMAND: &str = "hidhide";
const DEVICE: &str = r"\\.\HidHide";
/// Noted in the data directory: the devices hidden, one per line.
const NOTE: &str = "hidhide-hidden.txt";

/// HidHide's requests: CTL_CODE(32769, function, METHOD_BUFFERED, FILE_READ_DATA).
const fn ioctl(function: u32) -> u32 {
    (32769 << 16) | (1 << 14) | (function << 2)
}
const GET_WHITELIST: u32 = ioctl(2048);
const SET_WHITELIST: u32 = ioctl(2049);
const GET_BLACKLIST: u32 = ioctl(2050);
const SET_BLACKLIST: u32 = ioctl(2051);
const GET_ACTIVE: u32 = ioctl(2052);
const SET_ACTIVE: u32 = ioctl(2053);

/// HidHide is installed (its control device exists).
pub fn installed() -> bool {
    match Control::open() {
        Ok(_) => true,
        Err(e) => e.downcast_ref::<windows::core::Error>().is_some_and(|e| e.code() == ERROR_ACCESS_DENIED.to_hresult()),
    }
}

/// Hides `devices` (device instance ids) from every program but GameViber.
pub fn hide(devices: &[String]) -> anyhow::Result<()> {
    note(devices);
    run_or_elevate("hide", devices)
}

/// Gives back `devices`, and those a GameViber that crashed left hidden.
pub fn unhide(devices: &[String]) -> anyhow::Result<()> {
    let mut all: Vec<String> = noted();
    all.extend(devices.iter().cloned());
    all.sort();
    all.dedup();
    if all.is_empty() {
        return Ok(());
    }
    let result = run_or_elevate("unhide", &all);
    if result.is_ok() {
        let _ = std::fs::remove_file(note_path());
    }
    result
}

/// Devices left hidden by a GameViber that did not stop cleanly.
pub fn left_hidden() -> Vec<String> {
    noted()
}

fn note_path() -> PathBuf {
    super::data_dir().join(NOTE)
}

fn noted() -> Vec<String> {
    std::fs::read_to_string(note_path()).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

fn note(devices: &[String]) {
    let mut all = noted();
    all.extend(devices.iter().cloned());
    all.sort();
    all.dedup();
    let _ = std::fs::create_dir_all(super::data_dir());
    let _ = std::fs::write(note_path(), all.join("\n") + "\n");
}

/// Changes the lists here, or through an elevated GameViber when HidHide refuses.
fn run_or_elevate(action: &str, devices: &[String]) -> anyhow::Result<()> {
    match apply(action, devices) {
        Ok(()) => Ok(()),
        Err(e) if e.downcast_ref::<windows::core::Error>().is_some_and(|e| e.code() == ERROR_ACCESS_DENIED.to_hresult()) => {
            elevated(action, devices)
        }
        Err(e) => Err(e),
    }
}

/// Starts this executable as administrator for `action`, and waits for it.
fn elevated(action: &str, devices: &[String]) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let arguments = std::iter::once(SUBCOMMAND.to_owned())
        .chain(std::iter::once(action.to_owned()))
        .chain(devices.iter().map(|d| format!("\"{d}\"")))
        .collect::<Vec<_>>()
        .join(" ");
    log::info!("hiding the gamepad: asking Windows for administrator rights");
    let (verb, file, params) = (HSTRING::from("runas"), HSTRING::from(exe.as_os_str()), HSTRING::from(arguments));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: the strings outlive the call; the process handle is closed below.
    unsafe {
        ShellExecuteExW(&mut info).context("administrator rights refused")?;
        let process = info.hProcess;
        WaitForSingleObject(process, INFINITE);
        let mut code = 1;
        let _ = GetExitCodeProcess(process, &mut code);
        let _ = CloseHandle(process);
        anyhow::ensure!(code == 0, "HidHide refused the change");
    }
    Ok(())
}

/// `gameviber hidhide hide|unhide <device>...`, run as administrator.
pub fn subcommand(args: &[String]) -> anyhow::Result<()> {
    let (action, devices) = args.split_first().context("usage: gameviber hidhide hide|unhide <device>...")?;
    apply(action, devices)
}

fn apply(action: &str, devices: &[String]) -> anyhow::Result<()> {
    let control = Control::open()?;
    let mut hidden = control.list(GET_BLACKLIST)?;
    match action {
        "hide" => {
            let me = nt_path(&std::env::current_exe()?.display().to_string());
            let mut allowed = control.list(GET_WHITELIST)?;
            if !allowed.iter().any(|a| a.eq_ignore_ascii_case(&me)) {
                allowed.push(me);
                control.set_list(SET_WHITELIST, &allowed)?;
            }
            for device in devices {
                if !hidden.iter().any(|h| h.eq_ignore_ascii_case(device)) {
                    hidden.push(device.clone());
                }
            }
            control.set_list(SET_BLACKLIST, &hidden)?;
            if !control.active()? {
                control.set_active(true)?;
            }
        }
        "unhide" => {
            hidden.retain(|h| !devices.iter().any(|d| d.eq_ignore_ascii_case(h)));
            control.set_list(SET_BLACKLIST, &hidden)?;
        }
        other => anyhow::bail!("unknown HidHide action {other}"),
    }
    Ok(())
}

/// HidHide's control device.
struct Control(HANDLE);

impl Control {
    fn open() -> anyhow::Result<Self> {
        // SAFETY: plain call; the handle is closed on drop.
        let handle = unsafe {
            CreateFileW(
                &HSTRING::from(DEVICE),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }?;
        Ok(Self(handle))
    }

    fn request(&self, code: u32, input: &[u8], output: Option<&mut Vec<u8>>) -> windows::core::Result<u32> {
        let mut returned = 0u32;
        let (out_ptr, out_len) = match output {
            Some(buffer) => (Some(buffer.as_mut_ptr() as *mut core::ffi::c_void), buffer.len() as u32),
            None => (None, 0),
        };
        // SAFETY: the buffers and their sizes are ours.
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                (!input.is_empty()).then_some(input.as_ptr() as *const core::ffi::c_void),
                input.len() as u32,
                out_ptr,
                out_len,
                Some(&mut returned),
                None,
            )?;
        }
        Ok(returned)
    }

    fn list(&self, code: u32) -> anyhow::Result<Vec<String>> {
        // Asked once for its size (in bytes), then filled.
        let mut size = vec![0u8; 0];
        let needed = self.request(code, &[], Some(&mut size)).unwrap_or(0);
        let mut buffer = vec![0u8; (needed as usize).max(64 * 1024)];
        let got = self.request(code, &[], Some(&mut buffer))? as usize;
        Ok(from_multi(&buffer[..got.min(buffer.len())]))
    }

    fn set_list(&self, code: u32, entries: &[String]) -> anyhow::Result<()> {
        self.request(code, &to_multi(entries), None)?;
        Ok(())
    }

    fn active(&self) -> anyhow::Result<bool> {
        let mut out = vec![0u8; 1];
        self.request(GET_ACTIVE, &[], Some(&mut out))?;
        Ok(out[0] != 0)
    }

    fn set_active(&self, on: bool) -> anyhow::Result<()> {
        self.request(SET_ACTIVE, &[u8::from(on)], None)?;
        Ok(())
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        // SAFETY: our handle.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Strings as HidHide takes them: UTF-16, each ending with a NUL, then one more.
fn to_multi(entries: &[String]) -> Vec<u8> {
    let mut words: Vec<u16> = Vec::new();
    for entry in entries {
        words.extend(entry.encode_utf16());
        words.push(0);
    }
    words.push(0);
    words.into_iter().flat_map(u16::to_le_bytes).collect()
}

fn from_multi(bytes: &[u8]) -> Vec<String> {
    let words: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    words.split(|w| *w == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

/// `C:\Program Files\x.exe` as HidHide names programs: `\Device\HarddiskVolume3\Program Files\x.exe`.
fn nt_path(path: &str) -> String {
    let Some((drive, rest)) = path.split_once(':') else { return path.to_owned() };
    let mut target = [0u16; 512];
    // SAFETY: the buffer's size is passed.
    let n = unsafe { QueryDosDeviceW(&HSTRING::from(format!("{drive}:")), Some(&mut target)) } as usize;
    match target[..n].split(|w| *w == 0).next() {
        Some(device) if !device.is_empty() => format!("{}{rest}", String::from_utf16_lossy(device)),
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_are_written_as_hidhide_reads_them() {
        let entries = vec![r"HID\VID_054C&PID_09CC&MI_03\7&1A2B&0&0000".to_owned(), "é".to_owned()];
        let bytes = to_multi(&entries);
        assert_eq!(&bytes[bytes.len() - 4..], &[0, 0, 0, 0], "the last entry's NUL, then the list's");
        assert_eq!(from_multi(&bytes), entries);
        assert_eq!(to_multi(&[]), [0, 0]);
        // CTL_CODE(32769, 2048, METHOD_BUFFERED, FILE_READ_DATA), as HidHide's headers compute it.
        assert_eq!(GET_WHITELIST, 0x8001_6000);
        assert_eq!(SET_ACTIVE, 0x8001_6014);
    }
}
