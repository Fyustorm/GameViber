//! Sonde eBPF : capture les uploads (EVIOCSFF) et effacements (EVIOCRMFF)
//! d'effets force-feedback faits par n'importe quel process.
//!
//! Dérivé de linux-game-haptics-router (Apache-2.0). Les play/stop ne passent
//! pas par ici : le noyau les renvoie à tous les lecteurs evdev, le démon les
//! lit directement sur les manettes.
#![no_std]
#![no_main]

use aya_ebpf::helpers::{bpf_get_current_pid_tgid, bpf_probe_read_user_buf};
use aya_ebpf::macros::{map, tracepoint};
use aya_ebpf::maps::{LruHashMap, RingBuf};
use aya_ebpf::programs::TracePointContext;
use gameviber_common::{
    EnterScratch, FfEffect, ProbeEvent, EVIOCRMFF_NR, EVIOCSFF_NR, FF_UNION_WORDS,
    PROBE_EVENT_KIND_ERASED, PROBE_EVENT_KIND_UPLOADED,
};

// Offsets des arguments dans le contexte du tracepoint syscalls:sys_enter_ioctl.
const ARG_FD: usize = 16;
const ARG_CMD: usize = 24;
const ARG_PTR: usize = 32;

/// tgid<<32|pid -> upload en cours. LRU : un thread tué n'atteint jamais sys_exit.
#[map]
static ENTER_SCRATCH: LruHashMap<u64, EnterScratch> = LruHashMap::with_max_entries(1024, 0);

#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(256 * 1024, 0);

#[tracepoint]
pub fn sys_enter_ioctl(ctx: TracePointContext) -> i32 {
    let _ = try_enter(&ctx);
    0
}

#[tracepoint]
pub fn sys_exit_ioctl(_ctx: TracePointContext) -> i32 {
    let _ = try_exit();
    0
}

fn try_enter(ctx: &TracePointContext) -> Result<(), i64> {
    let cmd = unsafe { ctx.read_at::<u64>(ARG_CMD)? } as u32;
    if cmd != EVIOCSFF_NR && cmd != EVIOCRMFF_NR {
        return Ok(());
    }
    let fd = unsafe { ctx.read_at::<u64>(ARG_FD)? } as i32;
    let arg = unsafe { ctx.read_at::<u64>(ARG_PTR)? };
    let tgid_pid = bpf_get_current_pid_tgid();

    if cmd == EVIOCRMFF_NR {
        // L'argument est directement l'id de l'effet.
        submit(PROBE_EVENT_KIND_ERASED, (tgid_pid >> 32) as u32, fd, arg as i32 as i16, FfEffect::default());
        return Ok(());
    }

    // struct ff_effect (LP64) : 0 type, 2 id, 4 direction, 6-8 trigger,
    // 10 replay.length, 12 replay.delay, 16 union u.
    let mut raw = [0u8; 16 + FF_UNION_WORDS * 2];
    unsafe { bpf_probe_read_user_buf(arg as *const u8, &mut raw)? };
    let u16_at = |off: usize| u16::from_ne_bytes([raw[off], raw[off + 1]]);

    let mut effect = FfEffect {
        kind: u16_at(0),
        id: 0, // attribué par le noyau, relu à la sortie du syscall
        direction: u16_at(4),
        replay_length: u16_at(10),
        replay_delay: u16_at(12),
        u: [0; FF_UNION_WORDS],
    };
    let mut i = 0;
    while i < FF_UNION_WORDS {
        effect.u[i] = u16_at(16 + i * 2);
        i += 1;
    }

    let scratch = EnterScratch { ff_effect_ptr: arg, fd, _pad: 0, effect };
    ENTER_SCRATCH.insert(&tgid_pid, &scratch, 0)?;
    Ok(())
}

fn try_exit() -> Result<(), i64> {
    let tgid_pid = bpf_get_current_pid_tgid();
    let scratch = match unsafe { ENTER_SCRATCH.get(&tgid_pid) } {
        Some(s) => *s,
        None => return Ok(()),
    };
    ENTER_SCRATCH.remove(&tgid_pid)?;

    let mut id_bytes = [0u8; 2];
    unsafe { bpf_probe_read_user_buf((scratch.ff_effect_ptr + 2) as *const u8, &mut id_bytes)? };
    let mut effect = scratch.effect;
    effect.id = i16::from_ne_bytes(id_bytes);
    submit(PROBE_EVENT_KIND_UPLOADED, (tgid_pid >> 32) as u32, scratch.fd, effect.id, effect);
    Ok(())
}

fn submit(kind: u8, tgid: u32, fd: i32, effect_id: i16, effect: FfEffect) {
    if let Some(mut entry) = EVENTS.reserve::<ProbeEvent>(0) {
        entry.write(ProbeEvent { kind, tgid, fd, effect_id, _pad: 0, effect });
        entry.submit(0);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
