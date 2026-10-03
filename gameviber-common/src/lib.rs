//! Types shared between the eBPF probe and the daemon.
//!
//! Derived from linux-game-haptics-router (Apache-2.0, see
//! LICENSE-APACHE-linux-game-haptics-router). Additions: the ioctl's fd is
//! captured (to know which gamepad is targeted), as well as the whole
//! `struct ff_effect` union (periodic effect envelopes included).
#![cfg_attr(not(feature = "user"), no_std)]
// `bpf_target_arch` is a cfg set by aya-build when compiling the eBPF program.
#![allow(unexpected_cfgs)]

#[cfg(feature = "user")]
pub mod overlay;

pub const FF_RUMBLE: u16 = 0x50;
pub const FF_PERIODIC: u16 = 0x51;
pub const FF_CONSTANT: u16 = 0x52;
pub const FF_RAMP: u16 = 0x57;

/// Number of u16 words captured from `struct ff_effect`'s union (offset 16).
/// 12 words = 24 bytes: enough for periodic (waveform..envelope), constant, ramp and rumble.
pub const FF_UNION_WORDS: usize = 12;

/// Compact copy of `struct ff_effect` as read by the probe.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct FfEffect {
    pub kind: u16,
    pub id: i16,
    pub direction: u16,
    pub replay_length: u16,
    pub replay_delay: u16,
    /// Raw union, as u16 words at the kernel offsets (16, 18, 20, ...).
    pub u: [u16; FF_UNION_WORDS],
}

/// Data kept between sys_enter and sys_exit of an EVIOCSFF.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct EnterScratch {
    pub ff_effect_ptr: u64,
    pub fd: i32,
    pub _pad: u32,
    pub effect: FfEffect,
}

pub const PROBE_EVENT_KIND_UPLOADED: u8 = 0;
pub const PROBE_EVENT_KIND_ERASED: u8 = 1;

/// Event pushed to the daemon through the ring buffer.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ProbeEvent {
    pub kind: u8,
    pub tgid: u32,
    /// fd the ioctl was issued on: `/proc/<tgid>/fd/<fd>` gives the gamepad.
    pub fd: i32,
    pub effect_id: i16,
    pub _pad: u16,
    pub effect: FfEffect,
}

#[cfg(all(feature = "user", not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
compile_error!("KERNEL_FF_EFFECT_SIZE=48 is only verified for x86_64/aarch64 (LP64)");
#[cfg(all(
    not(feature = "user"),
    not(any(bpf_target_arch = "x86_64", bpf_target_arch = "aarch64"))
))]
compile_error!("KERNEL_FF_EFFECT_SIZE=48 is only verified for x86_64/aarch64 (LP64)");

/// Real size of `struct ff_effect` on LP64: the union holds a pointer
/// (`custom_data`) that aligns it on 8 bytes. Encoded in the ioctl number.
pub const KERNEL_FF_EFFECT_SIZE: u32 = 48;

const IOC_WRITE: u32 = 1 << 30;
const IOC_TYPE_EVDEV: u32 = b'E' as u32;

/// `_IOW('E', 0x80, struct ff_effect)`
pub const EVIOCSFF_NR: u32 = IOC_WRITE | (KERNEL_FF_EFFECT_SIZE << 16) | (IOC_TYPE_EVDEV << 8) | 0x80;
/// `_IOW('E', 0x81, int)`
pub const EVIOCRMFF_NR: u32 = IOC_WRITE | (4 << 16) | (IOC_TYPE_EVDEV << 8) | 0x81;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_kernel() {
        // Value verified with strace in linux-game-haptics-router.
        assert_eq!(EVIOCSFF_NR, 0x4030_4580);
        assert_eq!(EVIOCRMFF_NR, 0x4004_4581);
    }

    #[test]
    fn captured_union_fits_in_kernel_struct() {
        assert!(16 + FF_UNION_WORDS * 2 <= KERNEL_FF_EFFECT_SIZE as usize);
    }
}
