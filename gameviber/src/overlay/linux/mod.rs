//! Linux: games' overlays send their hellos to an abstract Unix datagram
//! socket named after the user, with their frame memory (a sealed memfd) as
//! a descriptor; the overlay is a Vulkan layer and an `LD_PRELOAD` library
//! (`install`).

mod install;

pub use install::{install, install_state, launcher_path, uninstall, update_installed};

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};

use gameviber_common::overlay::{self, frames};

/// The socket games' overlays talk to.
pub struct Socket(UnixDatagram);

impl Socket {
    pub fn bind() -> io::Result<Self> {
        let name = overlay::server_name(crate::platform::linux::uid());
        let socket = UnixDatagram::bind_addr(&SocketAddr::from_abstract_name(name.as_bytes())?)?;
        socket.set_nonblocking(true)?;
        Ok(Self(socket))
    }

    /// A datagram: the sender's name, the length, and its frame memory if it sent one.
    pub fn receive(&self, buf: &mut [u8]) -> Option<(Vec<u8>, usize, Option<OwnedFd>)> {
        receive(&self.0, buf)
    }

    pub fn send_to(&self, name: &[u8], data: &[u8]) -> io::Result<()> {
        self.0.send_to_addr(data, &SocketAddr::from_abstract_name(name)?).map(drop)
    }
}

/// A game's shared frame memory, mapped read-only.
pub struct FrameMemory {
    base: *const u8,
    _fd: OwnedFd,
}

// Read-only mapping, only touched by the engine thread.
unsafe impl Send for FrameMemory {}

impl FrameMemory {
    /// Maps `fd` once it is checked to be sealed memory of the expected size:
    /// a game shrinking it under us would crash GameViber.
    pub fn map(fd: OwnedFd) -> Option<Self> {
        // SAFETY: fcntl, fstat and mmap on a descriptor we own; the mapping lives as long as `self`.
        unsafe {
            let seals = libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS);
            if seals < 0 || seals & libc::F_SEAL_SHRINK == 0 {
                return None;
            }
            let mut stat: libc::stat = std::mem::zeroed();
            if libc::fstat(fd.as_raw_fd(), &mut stat) != 0 || (stat.st_size as usize) < frames::SIZE {
                return None;
            }
            let base = libc::mmap(std::ptr::null_mut(), frames::SIZE, libc::PROT_READ, libc::MAP_SHARED, fd.as_raw_fd(), 0);
            (base != libc::MAP_FAILED).then(|| Self { base: base as *const u8, _fd: fd })
        }
    }
}

impl FrameMemory {
    /// Start of the mapping, `frames::SIZE` bytes.
    pub fn base(&self) -> *const u8 {
        self.base
    }
}

impl Drop for FrameMemory {
    fn drop(&mut self) {
        // SAFETY: mapped in `map` with this size.
        unsafe { libc::munmap(self.base as *mut libc::c_void, frames::SIZE) };
    }
}

/// Receives a datagram and the descriptors passed with it: the sender's
/// abstract name, the length, and the first descriptor (others are closed).
fn receive(socket: &UnixDatagram, buf: &mut [u8]) -> Option<(Vec<u8>, usize, Option<OwnedFd>)> {
    // SAFETY: recvmsg into buffers we own; descriptors received are wrapped in OwnedFd at once.
    unsafe {
        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        let mut iov = libc::iovec { iov_base: buf.as_mut_ptr() as *mut libc::c_void, iov_len: buf.len() };
        let mut control = [0u64; 16];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_name = &mut addr as *mut _ as *mut libc::c_void;
        msg.msg_namelen = std::mem::size_of::<libc::sockaddr_un>() as u32;
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
        msg.msg_controllen = std::mem::size_of_val(&control) as _;
        let n = libc::recvmsg(socket.as_raw_fd(), &mut msg, libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC);
        if n < 0 {
            return None;
        }
        let mut fd = None;
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(cmsg) as *const i32;
                let count = ((*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / std::mem::size_of::<i32>();
                for i in 0..count {
                    let received = OwnedFd::from_raw_fd(std::ptr::read_unaligned(data.add(i)));
                    if fd.is_none() {
                        fd = Some(received);
                    }
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
        // Abstract names start with a 0 byte and are not 0-terminated.
        let path_offset = std::mem::size_of::<libc::sa_family_t>();
        let len = (msg.msg_namelen as usize).saturating_sub(path_offset);
        let path = std::slice::from_raw_parts(addr.sun_path.as_ptr() as *const u8, len.min(addr.sun_path.len()));
        let name = path.strip_prefix(&[0])?.to_vec();
        Some((name, n as usize, fd))
    }
}
