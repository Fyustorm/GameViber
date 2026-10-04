//! eBPF probe: reports the force-feedback effects any process uploads to an
//! evdev device (EVIOCSFF) or erases (EVIOCRMFF), which games do not show to
//! other readers of the device. Playing and stopping them needs no probe: the
//! kernel sends those to every reader of the device.
//!
//! On the way in, an ioctl's command, file descriptor and argument are noted;
//! on the way out, if it succeeded, the effect is read from the caller's
//! memory (the kernel has written its id back by then) and recorded.
#![no_std]
#![no_main]

use aya_ebpf::helpers::{bpf_get_current_pid_tgid, bpf_probe_read_user_buf};
use aya_ebpf::macros::{map, tracepoint};
use aya_ebpf::maps::{LruHashMap, RingBuf};
use aya_ebpf::programs::TracePointContext;
use gameviber_common::{ProbeRecord, EVIOCRMFF, EVIOCSFF, FF_EFFECT_SIZE};

/// The kernel only lets programs under a GPL-compatible license read a
/// process's memory (`bpf_probe_read_user`).
#[no_mangle]
#[link_section = "license"]
pub static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";

// Field offsets in the syscalls:sys_enter_ioctl and sys_exit_ioctl records
// (/sys/kernel/tracing/events/syscalls/sys_*_ioctl/format).
const ENTER_FD: usize = 16;
const ENTER_CMD: usize = 24;
const ENTER_ARG: usize = 32;
const EXIT_RET: usize = 16;

/// An ioctl of interest, between its entry and its exit.
#[derive(Clone, Copy)]
#[repr(C)]
struct Pending {
    cmd: u32,
    fd: i32,
    arg: u64,
}

/// Calls in progress, by thread. LRU: a thread killed during the call never exits it.
#[map]
static PENDING: LruHashMap<u64, Pending> = LruHashMap::with_max_entries(1024, 0);

/// Records for GameViber.
#[map]
static RECORDS: RingBuf = RingBuf::with_byte_size(256 * 1024, 0);

#[tracepoint]
pub fn ioctl_enter(ctx: TracePointContext) -> i32 {
    let _ = note(&ctx);
    0
}

#[tracepoint]
pub fn ioctl_exit(ctx: TracePointContext) -> i32 {
    let _ = record(&ctx);
    0
}

fn note(ctx: &TracePointContext) -> Result<(), i64> {
    // SAFETY: offsets of the tracepoint's own fields.
    let cmd = unsafe { ctx.read_at::<u64>(ENTER_CMD)? } as u32;
    if cmd != EVIOCSFF && cmd != EVIOCRMFF {
        return Ok(());
    }
    let pending = Pending {
        cmd,
        fd: unsafe { ctx.read_at::<u64>(ENTER_FD)? } as i32,
        arg: unsafe { ctx.read_at::<u64>(ENTER_ARG)? },
    };
    PENDING.insert(&bpf_get_current_pid_tgid(), &pending, 0)?;
    Ok(())
}

fn record(ctx: &TracePointContext) -> Result<(), i64> {
    let thread = bpf_get_current_pid_tgid();
    // SAFETY: the map's values are only written by `note`.
    let Some(pending) = (unsafe { PENDING.get(&thread) }).copied() else { return Ok(()) };
    PENDING.remove(&thread)?;
    let ret = unsafe { ctx.read_at::<i64>(EXIT_RET)? };
    if ret != 0 {
        return Ok(());
    }
    let mut effect = [0u8; FF_EFFECT_SIZE];
    if pending.cmd == EVIOCSFF {
        // SAFETY: a user pointer, read through the helper that checks it.
        unsafe { bpf_probe_read_user_buf(pending.arg as *const u8, &mut effect)? };
    } else {
        let id = (pending.arg as i16).to_ne_bytes();
        effect[2] = id[0];
        effect[3] = id[1];
    }
    if let Some(mut slot) = RECORDS.reserve::<ProbeRecord>(0) {
        slot.write(ProbeRecord { pid: (thread >> 32) as u32, fd: pending.fd, cmd: pending.cmd, effect });
        slot.submit(0);
    }
    Ok(())
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
