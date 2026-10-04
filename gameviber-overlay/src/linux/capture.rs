//! Copies of the game's image for GameViber: the shared memory they go to
//! (`gameviber_common::overlay::frames`) and their pace. The renderers make
//! the copies on the GPU and hand the pixels over here a few frames later.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Instant;

use gameviber_common::overlay::{frames, CaptureRequest};

pub struct SharedFrames {
    fd: OwnedFd,
    base: *mut u8,
}

// The mapping is only written under the owner's mutex.
unsafe impl Send for SharedFrames {}

impl SharedFrames {
    pub fn new() -> Option<Self> {
        // SAFETY: plain syscalls on a descriptor we own; the mapping lives as long as `self`.
        unsafe {
            let fd = libc::memfd_create(c"gameviber-frames".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING);
            if fd < 0 {
                return None;
            }
            let fd = OwnedFd::from_raw_fd(fd);
            if libc::ftruncate(fd.as_raw_fd(), frames::SIZE as libc::off_t) != 0 {
                return None;
            }
            // GameViber maps it too: its size must never change under it.
            libc::fcntl(fd.as_raw_fd(), libc::F_ADD_SEALS, libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL);
            let base = libc::mmap(
                std::ptr::null_mut(),
                frames::SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            );
            if base == libc::MAP_FAILED {
                return None;
            }
            Some(Self { fd, base: base as *mut u8 })
        }
    }

    pub fn fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    /// Publishes a frame of `width` x `height`, whose rows are `stride` bytes apart in `pixels`.
    pub fn publish(&self, width: u32, height: u32, source: (u32, u32), pixels: &[u8], stride: usize, bgra: bool) {
        let row = width as usize * 4;
        if stride < row || pixels.len() < stride * (height as usize - 1) + row {
            return;
        }
        // SAFETY: the mapping is `frames::SIZE` bytes and only this process writes it.
        unsafe {
            frames::write(self.base, width, height, source, |out| {
                for (y, dst) in out.chunks_exact_mut(row).enumerate() {
                    dst.copy_from_slice(&pixels[y * stride..y * stride + row]);
                    if bgra {
                        dst.chunks_exact_mut(4).for_each(|p| p.swap(0, 2));
                    }
                }
            });
        }
    }
}

impl Drop for SharedFrames {
    fn drop(&mut self) {
        // SAFETY: mapped in `new` with this size.
        unsafe { libc::munmap(self.base as *mut libc::c_void, frames::SIZE) };
    }
}

/// When the next copy is due.
#[derive(Default)]
pub struct Pace {
    last: Option<Instant>,
}

impl Pace {
    /// Whether a copy should be made now, for GameViber's `request`.
    pub fn due(&mut self, request: Option<&CaptureRequest>) -> bool {
        let Some(request) = request else { return false };
        let now = Instant::now();
        let period = 1.0 / request.fps.clamp(0.5, 30.0) as f64;
        if self.last.is_some_and(|t| now.duration_since(t).as_secs_f64() < period) {
            return false;
        }
        self.last = Some(now);
        true
    }
}
