//! Captures the game's sound: what to listen to (`choose`), and the capture
//! itself, an OS backend (`linux`: PipeWire) providing `Graph` (the playing
//! applications' streams) and `Capture` (mono samples at `SAMPLE_RATE`).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{Capture, Graph};
#[cfg(not(target_os = "linux"))]
mod unsupported;
#[cfg(not(target_os = "linux"))]
pub use unsupported::{Capture, Graph};

use crate::config::AudioSource;

/// A playback stream of an application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    /// Id of the stream in the sound server (PipeWire: its node).
    pub node: u32,
    /// Application name, as shown in the GUI.
    pub app: String,
    pub binary: String,
    pub pid: Option<u32>,
}

/// What the capture listens to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Every stream of one application.
    App { name: String, streams: Vec<Stream> },
    /// Everything the default output plays.
    Everything,
}

impl Target {
    pub fn describe(&self) -> String {
        match self {
            Target::App { name, streams } if streams.len() > 1 => format!("{name} ({} streams)", streams.len()),
            Target::App { name, .. } => name.clone(),
            Target::Everything => "everything the computer plays".to_owned(),
        }
    }

    /// Same capture: only the streams of an application may come and go.
    fn same_capture(&self, other: &Target) -> bool {
        match (self, other) {
            (Target::App { name: a, .. }, Target::App { name: b, .. }) => a == b,
            (a, b) => a == b,
        }
    }
}

/// Picks what to listen to. `games` are the processes showing the overlay
/// (pid, executable name).
pub fn choose(source: &AudioSource, streams: &[Stream], games: &[(u32, String)]) -> Option<Target> {
    let app = |matches: &dyn Fn(&Stream) -> bool| {
        let streams: Vec<Stream> = streams.iter().filter(|s| matches(s)).cloned().collect();
        let name = streams.first()?.app.clone();
        Some(Target::App { name, streams })
    };
    match source {
        AudioSource::Off => None,
        AudioSource::Everything => Some(Target::Everything),
        AudioSource::App(name) => app(&|s| s.app == *name || s.binary == *name),
        AudioSource::Auto => {
            let stem = |exe: &str| exe.rsplit(['/', '\\']).next().unwrap_or(exe).trim_end_matches(".exe").to_lowercase();
            let game = |s: &Stream| {
                games.iter().any(|(pid, exe)| {
                    let exe = stem(exe);
                    s.pid == Some(*pid) || (!exe.is_empty() && (stem(&s.app) == exe || stem(&s.binary) == exe))
                })
            };
            Some(app(&game).unwrap_or(Target::Everything))
        }
    }
}
