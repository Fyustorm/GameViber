//! Types shared between the eBPF probe (`gameviber-ebpf`), GameViber and the
//! in-game overlay (`overlay`).
#![cfg_attr(not(any(feature = "user", feature = "overlay")), no_std)]
// `bpf_target_arch` is a cfg set by aya-build when compiling the eBPF program.
#![allow(unexpected_cfgs)]

#[cfg(feature = "overlay")]
pub mod overlay;

// Force-feedback effect types (linux/input.h).
pub const FF_RUMBLE: u16 = 0x50;
pub const FF_PERIODIC: u16 = 0x51;
pub const FF_CONSTANT: u16 = 0x52;
pub const FF_RAMP: u16 = 0x57;

/// Size of `struct ff_effect` on 64-bit Linux: its union holds a pointer
/// (`custom_data`), which aligns it on 8 bytes. It is part of the ioctl number.
pub const FF_EFFECT_SIZE: usize = 48;

/// `_IOW('E', nr, size)`: an evdev ioctl writing `size` bytes to the kernel.
const fn evdev_iow(nr: u32, size: u32) -> u32 {
    const WRITE: u32 = 1;
    (WRITE << 30) | (size << 16) | ((b'E' as u32) << 8) | nr
}

/// Uploads a force-feedback effect (`struct ff_effect *`); the kernel writes
/// the effect's id back into it.
pub const EVIOCSFF: u32 = evdev_iow(0x80, FF_EFFECT_SIZE as u32);
/// Erases an effect (the argument is its id).
pub const EVIOCRMFF: u32 = evdev_iow(0x81, 4);

/// What the probe records of a successful EVIOCSFF or EVIOCRMFF.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ProbeRecord {
    /// Process (thread group) that made the call.
    pub pid: u32,
    /// File descriptor the call was made on: `/proc/<pid>/fd/<fd>` is the gamepad.
    pub fd: i32,
    /// `EVIOCSFF` or `EVIOCRMFF`.
    pub cmd: u32,
    /// EVIOCSFF: the `struct ff_effect`, as the game's memory holds it after
    /// the call (with the id the kernel gave it). EVIOCRMFF: zeros but for the
    /// id, at its place in the struct.
    pub effect: [u8; FF_EFFECT_SIZE],
}

impl ProbeRecord {
    /// The effect's id (`struct ff_effect`, offset 2).
    pub fn effect_id(&self) -> i16 {
        i16::from_ne_bytes([self.effect[2], self.effect[3]])
    }
}

#[cfg(all(feature = "user", not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
compile_error!("FF_EFFECT_SIZE is only right for 64-bit Linux (x86_64, aarch64)");
// The overlay alone does not use the probe types.
#[cfg(all(
    not(feature = "user"),
    not(feature = "overlay"),
    not(any(bpf_target_arch = "x86_64", bpf_target_arch = "aarch64"))
))]
compile_error!("FF_EFFECT_SIZE is only right for 64-bit Linux (x86_64, aarch64)");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_are_the_kernels() {
        // As strace shows them on x86_64.
        assert_eq!(EVIOCSFF, 0x4030_4580);
        assert_eq!(EVIOCRMFF, 0x4004_4581);
    }

    #[test]
    fn the_effect_id_is_read_at_its_place() {
        let mut record = ProbeRecord { pid: 1, fd: 3, cmd: EVIOCSFF, effect: [0; FF_EFFECT_SIZE] };
        record.effect[2..4].copy_from_slice(&7i16.to_ne_bytes());
        assert_eq!(record.effect_id(), 7);
    }
}
