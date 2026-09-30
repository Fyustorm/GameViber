//! Types partagés entre la sonde eBPF et le démon.
//!
//! Dérivé de linux-game-haptics-router (Apache-2.0, voir
//! LICENSE-APACHE-linux-game-haptics-router) : on capture en plus le fd de
//! l'ioctl (pour savoir quelle manette est visée) et l'union complète de
//! `struct ff_effect` (enveloppes des effets périodiques comprises).
#![cfg_attr(not(feature = "user"), no_std)]
// `bpf_target_arch` est un cfg posé par aya-build lors de la compilation eBPF.
#![allow(unexpected_cfgs)]

pub const FF_RUMBLE: u16 = 0x50;
pub const FF_PERIODIC: u16 = 0x51;
pub const FF_CONSTANT: u16 = 0x52;
pub const FF_RAMP: u16 = 0x57;

/// Nombre de mots u16 capturés dans l'union de `struct ff_effect` (offset 16).
/// 12 mots = 24 octets, assez pour periodic (waveform..envelope), constant, ramp et rumble.
pub const FF_UNION_WORDS: usize = 12;

/// Copie compacte de `struct ff_effect` telle que lue par la sonde.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct FfEffect {
    pub kind: u16,
    pub id: i16,
    pub direction: u16,
    pub replay_length: u16,
    pub replay_delay: u16,
    /// Union brute, en mots u16 aux offsets noyau (16, 18, 20, ...).
    pub u: [u16; FF_UNION_WORDS],
}

/// Données conservées entre sys_enter et sys_exit d'un EVIOCSFF.
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

/// Événement poussé dans le ring buffer vers le démon.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ProbeEvent {
    pub kind: u8,
    pub tgid: u32,
    /// fd sur lequel l'ioctl a été fait : `/proc/<tgid>/fd/<fd>` donne la manette.
    pub fd: i32,
    pub effect_id: i16,
    pub _pad: u16,
    pub effect: FfEffect,
}

#[cfg(all(feature = "user", not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
compile_error!("KERNEL_FF_EFFECT_SIZE=48 n'est vérifié que pour x86_64/aarch64 (LP64)");
#[cfg(all(
    not(feature = "user"),
    not(any(bpf_target_arch = "x86_64", bpf_target_arch = "aarch64"))
))]
compile_error!("KERNEL_FF_EFFECT_SIZE=48 n'est vérifié que pour x86_64/aarch64 (LP64)");

/// Taille réelle de `struct ff_effect` en LP64 : l'union contient un pointeur
/// (`custom_data`) qui l'aligne sur 8 octets. Encodée dans le numéro d'ioctl.
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
        // Valeur vérifiée par strace dans linux-game-haptics-router.
        assert_eq!(EVIOCSFF_NR, 0x4030_4580);
        assert_eq!(EVIOCRMFF_NR, 0x4004_4581);
    }

    #[test]
    fn captured_union_fits_in_kernel_struct() {
        assert!(16 + FF_UNION_WORDS * 2 <= KERNEL_FF_EFFECT_SIZE as usize);
    }
}
