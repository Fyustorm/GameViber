//! The few registry values GameViber reads and writes.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_WRITE,
    REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

#[derive(Debug, Clone, Copy)]
pub enum Root {
    CurrentUser,
    LocalMachine,
}

impl Root {
    fn key(self) -> HKEY {
        match self {
            Root::CurrentUser => HKEY_CURRENT_USER,
            Root::LocalMachine => HKEY_LOCAL_MACHINE,
        }
    }
}

pub fn read_u32(root: Root, key: &str, name: &str) -> Option<u32> {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: the buffer is a u32 and its size says so.
    let status = unsafe {
        RegGetValueW(root.key(), &HSTRING::from(key), &HSTRING::from(name), RRF_RT_REG_DWORD, None, Some(&mut value as *mut u32 as _), Some(&mut size))
    };
    (status == ERROR_SUCCESS).then_some(value)
}

/// A string value; `name` "" is the key's default value.
pub fn read_string(root: Root, key: &str, name: &str) -> Option<String> {
    let (key, name) = (HSTRING::from(key), HSTRING::from(name));
    let value_name = if name.is_empty() { PCWSTR::null() } else { PCWSTR(name.as_ptr()) };
    let mut size = 0u32;
    // SAFETY: the first call only asks for the size, the second fills a buffer that size.
    unsafe {
        if RegGetValueW(root.key(), &key, value_name, RRF_RT_REG_SZ, None, None, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
        if RegGetValueW(root.key(), &key, value_name, RRF_RT_REG_SZ, None, Some(buffer.as_mut_ptr() as _), Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        buffer.truncate(size as usize / 2);
        Some(String::from_utf16_lossy(&buffer).trim_end_matches('\0').to_owned())
    }
}

/// Writes string values in `key` of the current user, created if needed; a name "" is the default value.
pub fn write_strings(key: &str, values: &[(&str, &str)]) -> windows::core::Result<()> {
    let mut handle = HKEY::default();
    // SAFETY: the key is closed below; values are NUL-terminated UTF-16.
    unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, &HSTRING::from(key), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut handle, None)
            .ok()?;
        let result = values.iter().try_for_each(|(name, value)| {
            let data: Vec<u8> = value.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
            let name = HSTRING::from(*name);
            let name = if name.is_empty() { PCWSTR::null() } else { PCWSTR(name.as_ptr()) };
            RegSetValueExW(handle, name, None, REG_SZ, Some(&data)).ok()
        });
        let _ = RegCloseKey(handle);
        result
    }
}

/// Removes `key` of the current user and everything under it.
pub fn delete_tree(key: &str) {
    // SAFETY: plain call; a missing key is fine.
    let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key)) };
}
