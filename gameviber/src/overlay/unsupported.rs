//! Systems without an overlay backend yet: no game can reach GameViber.

use std::convert::Infallible;
use std::io;
use std::path::PathBuf;

use super::{Arch, InstallState};

pub const WINDOW_CAPTURE: bool = false;
pub const NO_IMAGE_HINT: &str = "The game's image cannot be read on this system yet.";

pub struct Socket;

impl Socket {
    pub fn bind() -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "not available on this system yet"))
    }

    pub fn receive(&self, _buf: &mut [u8]) -> Option<(Vec<u8>, usize, Option<Infallible>)> {
        None
    }

    pub fn send_to(&self, _name: &[u8], _data: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

pub enum FrameMemory {}

impl FrameMemory {
    pub fn map(handle: Infallible) -> Option<Self> {
        match handle {}
    }

    pub fn base(&self) -> *const u8 {
        match *self {}
    }
}

pub fn install_state(_all_games: bool) -> Vec<(Arch, InstallState)> {
    Arch::ALL.into_iter().map(|arch| (arch, InstallState::NotBuilt)).collect()
}

pub fn install(_all_games: bool) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "the in-game overlay is not available on this system yet"))
}

pub fn packaged() -> bool {
    false
}

pub fn update_installed(_all_games: bool) {}

pub fn uninstall() -> io::Result<()> {
    Ok(())
}

pub fn launcher_path() -> PathBuf {
    PathBuf::new()
}
