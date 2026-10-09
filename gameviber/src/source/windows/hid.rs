//! Gamepads XInput does not see (DualShock, Switch, generic ones), read as
//! HID game controllers: their buttons, axes and hat numbered like SDL numbers
//! them on Windows (buttons and axes in the order of their HID usages), so
//! that SDL_GameControllerDB's Windows mappings and the player's own apply.
//! XInput gamepads also show as HID devices (`IG_` in their path): they are
//! left to XInput.

use std::io;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_IDW, CM_Get_Parent, CM_Locate_DevNodeW, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, CM_LOCATE_DEVNODE_NORMAL, CR_SUCCESS, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid, HidD_GetPreparsedData, HidD_GetProductString, HidP_GetButtonCaps, HidP_GetCaps,
    HidP_GetUsageValue, HidP_GetUsages, HidP_GetValueCaps, HidP_Input, HIDD_ATTRIBUTES, HIDP_BUTTON_CAPS, HIDP_CAPS, HIDP_STATUS_SUCCESS,
    HIDP_VALUE_CAPS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{CloseHandle, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, ReadFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use crate::gamepad::mapping::RawState;

const GENERIC_DESKTOP: u16 = 0x01;
const BUTTON_PAGE: u16 = 0x09;
const JOYSTICK: u16 = 0x04;
const GAMEPAD: u16 = 0x05;
/// X, Y, Z, Rx, Ry, Rz, slider, dial, wheel.
const AXES: std::ops::RangeInclusive<u16> = 0x30..=0x38;
const HAT: u16 = 0x39;

/// A HID game controller found.
#[derive(Debug, Clone, PartialEq)]
pub struct HidInfo {
    pub path: String,
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
}

/// The game controllers plugged in, XInput ones left out.
pub fn list() -> Vec<HidInfo> {
    let mut pads = Vec::new();
    for path in interface_paths() {
        if path.to_ascii_uppercase().contains("IG_") {
            continue;
        }
        let Ok(handle) = open(&path, false) else { continue };
        // SAFETY: our handle, closed below; the structures are sized for the calls.
        unsafe {
            if let Some(caps) = caps(handle) {
                if caps.UsagePage == GENERIC_DESKTOP && (caps.Usage == JOYSTICK || caps.Usage == GAMEPAD) {
                    let mut attributes = HIDD_ATTRIBUTES { Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32, ..Default::default() };
                    if HidD_GetAttributes(handle, &mut attributes) {
                        let mut name = [0u16; 128];
                        let name = if HidD_GetProductString(handle, name.as_mut_ptr() as _, std::mem::size_of_val(&name) as u32) {
                            String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())]).trim().to_owned()
                        } else {
                            String::new()
                        };
                        let name = if name.is_empty() { format!("Gamepad {:04x}:{:04x}", attributes.VendorID, attributes.ProductID) } else { name };
                        pads.push(HidInfo {
                            path: path.clone(),
                            name,
                            vendor: attributes.VendorID,
                            product: attributes.ProductID,
                            version: attributes.VersionNumber,
                        });
                    }
                }
            }
            let _ = CloseHandle(handle);
        }
    }
    pads
}

/// Paths of every HID device interface present.
pub fn interface_paths() -> Vec<String> {
    let mut paths = Vec::new();
    // SAFETY: SetupAPI calls on a list destroyed below; the detail buffer is sized as asked.
    unsafe {
        let guid = HidD_GetHidGuid();
        let Ok(set) = SetupDiGetClassDevsW(Some(&guid), PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE) else { return paths };
        let mut index = 0;
        loop {
            let mut interface = SP_DEVICE_INTERFACE_DATA { cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInterfaces(set, None, &guid, index, &mut interface).is_err() {
                break;
            }
            index += 1;
            let mut size = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(set, &interface, None, 0, Some(&mut size), None);
            if size == 0 {
                continue;
            }
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            let mut info = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if SetupDiGetDeviceInterfaceDetailW(set, &interface, Some(detail), size, None, Some(&mut info)).is_ok() {
                let path = PCWSTR((*detail).DevicePath.as_ptr());
                paths.push(path.to_string().unwrap_or_default());
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    paths
}

fn open(path: &str, overlapped: bool) -> windows::core::Result<HANDLE> {
    let flags = if overlapped { FILE_FLAG_OVERLAPPED } else { Default::default() };
    // SAFETY: plain call; the caller closes the handle.
    unsafe {
        CreateFileW(&HSTRING::from(path), (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, flags, None)
            .or_else(|_| CreateFileW(&HSTRING::from(path), GENERIC_READ.0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, flags, None))
    }
}

unsafe fn caps(handle: HANDLE) -> Option<HIDP_CAPS> {
    let mut data = PHIDP_PREPARSED_DATA::default();
    if !HidD_GetPreparsedData(handle, &mut data) {
        return None;
    }
    let mut caps = HIDP_CAPS::default();
    let ok = HidP_GetCaps(data, &mut caps) == HIDP_STATUS_SUCCESS;
    let _ = HidD_FreePreparsedData(data);
    ok.then_some(caps)
}

/// The device instance ids HidHide hides a device by: the device's and its parent's.
pub fn instance_ids(path: &str) -> Vec<String> {
    // \\?\HID#VID_054C&PID_09CC&MI_03#7&1a2b&0&0000#{guid} -> HID\VID_054C&PID_09CC&MI_03\7&1A2B&0&0000
    let trimmed = path.trim_start_matches(r"\\?\");
    let mut parts: Vec<&str> = trimmed.split('#').collect();
    if parts.len() < 3 {
        return Vec::new();
    }
    parts.truncate(3);
    let id = parts.join("\\").to_ascii_uppercase();
    let mut ids = vec![id.clone()];
    if let Some(parent) = parent_id(&id) {
        ids.push(parent);
    }
    ids
}

fn parent_id(id: &str) -> Option<String> {
    // SAFETY: Configuration Manager calls with buffers sized for them.
    unsafe {
        let mut node = 0u32;
        if CM_Locate_DevNodeW(&mut node, &HSTRING::from(id), CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS {
            return None;
        }
        let mut parent = 0u32;
        if CM_Get_Parent(&mut parent, node, 0) != CR_SUCCESS {
            return None;
        }
        let mut buffer = [0u16; 512];
        if CM_Get_Device_IDW(parent, &mut buffer, 0) != CR_SUCCESS {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..buffer.iter().position(|c| *c == 0).unwrap_or(0)]))
    }
}

/// One of a pad's values: its usage and logical range.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Value {
    usage: u16,
    min: i32,
    max: i32,
    bits: u16,
}

/// A HID game controller being read.
pub struct HidPad {
    pub info: HidInfo,
    handle: HANDLE,
    data: PHIDP_PREPARSED_DATA,
    /// Button usages, in SDL's order.
    buttons: Vec<u16>,
    axes: Vec<Value>,
    hat: Option<Value>,
    report: Vec<u8>,
    /// The read in progress.
    overlapped: Box<OVERLAPPED>,
    pending: bool,
}

// The handles are only used by the proxy thread.
unsafe impl Send for HidPad {}

impl HidPad {
    pub fn open(info: HidInfo) -> anyhow::Result<Self> {
        let handle = open(&info.path, true)?;
        // SAFETY: our handle and preparsed data, freed on drop; the cap arrays are sized as HID says.
        unsafe {
            let mut data = PHIDP_PREPARSED_DATA::default();
            anyhow::ensure!(HidD_GetPreparsedData(handle, &mut data), "cannot read {}'s description", info.name);
            let mut caps = HIDP_CAPS::default();
            let _ = HidP_GetCaps(data, &mut caps);
            let mut count = caps.NumberInputButtonCaps;
            let mut button_caps = vec![HIDP_BUTTON_CAPS::default(); usize::from(count)];
            let _ = HidP_GetButtonCaps(HidP_Input, button_caps.as_mut_ptr(), &mut count, data);
            button_caps.truncate(usize::from(count));
            let mut buttons: Vec<u16> = Vec::new();
            for c in button_caps.iter().filter(|c| c.UsagePage == BUTTON_PAGE) {
                if c.IsRange {
                    buttons.extend(c.Anonymous.Range.UsageMin..=c.Anonymous.Range.UsageMax);
                } else {
                    buttons.push(c.Anonymous.NotRange.Usage);
                }
            }
            buttons.sort_unstable();
            buttons.dedup();
            let mut count = caps.NumberInputValueCaps;
            let mut value_caps = vec![HIDP_VALUE_CAPS::default(); usize::from(count)];
            let _ = HidP_GetValueCaps(HidP_Input, value_caps.as_mut_ptr(), &mut count, data);
            value_caps.truncate(usize::from(count));
            let mut axes = Vec::new();
            let mut hat = None;
            for c in value_caps.iter().filter(|c| c.UsagePage == GENERIC_DESKTOP) {
                let usages = if c.IsRange { c.Anonymous.Range.UsageMin..=c.Anonymous.Range.UsageMax } else { c.Anonymous.NotRange.Usage..=c.Anonymous.NotRange.Usage };
                for usage in usages {
                    let value = Value { usage, min: c.LogicalMin, max: c.LogicalMax, bits: c.BitSize };
                    match usage {
                        HAT => hat = hat.or(Some(value)),
                        u if AXES.contains(&u) => axes.push(value),
                        _ => {}
                    }
                }
            }
            axes.sort_by_key(|a| a.usage);
            axes.dedup_by_key(|a| a.usage);
            let event = CreateEventW(None, true, false, None)?;
            let overlapped = Box::new(OVERLAPPED { hEvent: event, ..Default::default() });
            Ok(Self { info, handle, data, buttons, axes, hat, report: vec![0; usize::from(caps.InputReportByteLength)], overlapped, pending: false })
        }
    }

    /// Every element at rest.
    pub fn rest(&self) -> RawState {
        RawState { buttons: vec![false; self.buttons.len()], axes: vec![0.0; self.axes.len()], hats: vec![0; usize::from(self.hat.is_some())] }
    }

    /// Reads the reports that came since the last call into `raw`; true when one did.
    pub fn poll(&mut self, raw: &mut RawState) -> io::Result<bool> {
        let mut changed = false;
        loop {
            // SAFETY: the report buffer and the OVERLAPPED stay where they are while a read is pending.
            unsafe {
                if !self.pending {
                    match ReadFile(self.handle, Some(&mut self.report), None, Some(&mut *self.overlapped)) {
                        Ok(()) => {}
                        Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {}
                        Err(e) => return Err(io::Error::from_raw_os_error(e.code().0 & 0xffff)),
                    }
                    self.pending = true;
                }
                let mut read = 0u32;
                match GetOverlappedResult(self.handle, &*self.overlapped, &mut read, false) {
                    Ok(()) => {
                        self.pending = false;
                        let report = self.report.clone();
                        self.parse(&report, raw);
                        changed = true;
                    }
                    Err(e) if e.code() == ERROR_IO_INCOMPLETE.to_hresult() => return Ok(changed),
                    Err(e) => return Err(io::Error::from_raw_os_error(e.code().0 & 0xffff)),
                }
            }
        }
    }

    fn parse(&self, report: &[u8], raw: &mut RawState) {
        let mut report = report.to_vec();
        // SAFETY: the preparsed data is this device's; the usage list is sized for every button.
        unsafe {
            let mut usages = vec![0u16; self.buttons.len().max(1)];
            let mut count = usages.len() as u32;
            if HidP_GetUsages(HidP_Input, BUTTON_PAGE, None, usages.as_mut_ptr(), &mut count, self.data, &mut report) == HIDP_STATUS_SUCCESS {
                let down = &usages[..count as usize];
                for (i, usage) in self.buttons.iter().enumerate() {
                    raw.buttons[i] = down.contains(usage);
                }
            }
            for (i, axis) in self.axes.iter().enumerate() {
                let mut value = 0u32;
                if HidP_GetUsageValue(HidP_Input, GENERIC_DESKTOP, None, axis.usage, &mut value, self.data, &report) == HIDP_STATUS_SUCCESS {
                    raw.axes[i] = axis.unit(value);
                }
            }
            if let Some(hat) = self.hat {
                let mut value = 0u32;
                if HidP_GetUsageValue(HidP_Input, GENERIC_DESKTOP, None, HAT, &mut value, self.data, &report) == HIDP_STATUS_SUCCESS {
                    raw.hats[0] = hat.direction(value);
                }
            }
        }
    }
}

impl Value {
    /// The value as HID gives it, signed when its range is.
    fn logical(&self, value: u32) -> i64 {
        let bits = u32::from(self.bits.clamp(1, 32));
        if self.min < 0 && bits < 32 && value & (1 << (bits - 1)) != 0 {
            i64::from(value) - (1i64 << bits)
        } else if self.min < 0 && bits == 32 {
            i64::from(value as i32)
        } else {
            i64::from(value)
        }
    }

    /// -1 to 1 from one end of the range to the other.
    fn unit(&self, value: u32) -> f64 {
        let (min, max) = if self.max > self.min { (i64::from(self.min), i64::from(self.max)) } else { (0, (1i64 << self.bits.clamp(1, 32)) - 1) };
        let v = self.logical(value).clamp(min, max);
        ((v - min) as f64 / (max - min) as f64 * 2.0 - 1.0).clamp(-1.0, 1.0)
    }

    /// A hat's directions as SDL's mask (1 up, 2 right, 4 down, 8 left); outside its range: centered.
    fn direction(&self, value: u32) -> u8 {
        const EIGHT: [u8; 8] = [1, 1 | 2, 2, 2 | 4, 4, 4 | 8, 8, 8 | 1];
        let v = self.logical(value);
        let (min, max) = (i64::from(self.min), i64::from(self.max));
        if v < min || v > max {
            return 0;
        }
        match max - min + 1 {
            4 => [1, 2, 4, 8][(v - min) as usize],
            8 => EIGHT[(v - min) as usize],
            _ => 0,
        }
    }
}

impl Drop for HidPad {
    fn drop(&mut self) {
        // SAFETY: our handles; the pending read is cancelled before the buffers go.
        unsafe {
            if self.pending {
                let _ = CancelIoEx(self.handle, Some(&*self.overlapped));
                let mut read = 0;
                let _ = GetOverlappedResult(self.handle, &*self.overlapped, &mut read, true);
            }
            let _ = HidD_FreePreparsedData(self.data);
            let _ = CloseHandle(self.overlapped.hEvent);
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_read_in_their_range() {
        let stick = Value { usage: 0x30, min: 0, max: 255, bits: 8 };
        assert_eq!((stick.unit(0), stick.unit(255)), (-1.0, 1.0));
        assert!(stick.unit(128).abs() < 0.01);
        let signed = Value { usage: 0x31, min: -127, max: 127, bits: 8 };
        assert_eq!(signed.unit(0x81), -1.0, "0x81 is -127");
        assert_eq!(signed.unit(127), 1.0);
        let hat = Value { usage: HAT, min: 0, max: 7, bits: 4 };
        assert_eq!((hat.direction(0), hat.direction(3), hat.direction(7), hat.direction(8)), (1, 2 | 4, 8 | 1, 0), "8: centered");
        let from_one = Value { usage: HAT, min: 1, max: 8, bits: 4 };
        assert_eq!((from_one.direction(1), from_one.direction(0)), (1, 0));
    }

    #[test]
    fn instance_ids_come_from_the_interface_path() {
        let path = r"\\?\hid#vid_054c&pid_09cc&mi_03#7&1a2b3c&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(instance_ids(path).first().map(String::as_str), Some(r"HID\VID_054C&PID_09CC&MI_03\7&1A2B3C&0&0000"));
        assert!(instance_ids("nonsense").is_empty());
    }
}
