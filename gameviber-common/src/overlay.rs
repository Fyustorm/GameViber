//! Protocol between GameViber and the in-game overlay (gameviber-overlay).
//!
//! GameViber binds a datagram socket in the abstract namespace, which games
//! reach even from the Steam Runtime or Flatpak containers since they share
//! the network namespace. Each overlay instance (one per game process) sends
//! a JSON [`Hello`] every second; GameViber answers with the JSON
//! [`OverlayState`] about 25 times per second and forgets overlays it has
//! not heard from for a few seconds.
//!
//! When GameViber asks for the game's image ([`OverlayState::capture`]), the
//! overlay copies small, downscaled frames into shared memory (see
//! [`frames`]) and passes its file descriptor along with its hellos.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
/// Overlays send a hello this often.
pub const HELLO_SECS: f64 = 1.0;
/// GameViber forgets an overlay after this long without a hello.
pub const CLIENT_TIMEOUT_SECS: f64 = 3.0;
/// The overlay hides itself after this long without a state.
pub const STATE_TIMEOUT_SECS: f64 = 2.0;
/// Datagrams never exceed this size.
pub const MAX_DATAGRAM: usize = 16 * 1024;

/// Abstract socket name GameViber listens on, for the user `uid`.
pub fn server_name(uid: u32) -> String {
    format!("gameviber-overlay-{uid}")
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hello {
    pub version: u32,
    pub pid: u32,
    /// Executable name of the game process.
    pub exe: String,
    /// Graphics API the overlay draws with ("vulkan", "opengl").
    pub api: String,
    /// The datagram carries the frame memory's file descriptor (`SCM_RIGHTS`).
    pub frames: bool,
}

/// GameViber wants copies of the game's frames.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureRequest {
    /// Width of the copies in pixels; the height keeps the game's aspect ratio.
    pub width: u32,
    /// Copies per second.
    pub fps: f32,
}

impl Default for CaptureRequest {
    fn default() -> Self {
        Self { width: 480, fps: 10.0 }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Corner {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub const ALL: [Corner; 4] = [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight];

    pub fn label(self) -> &'static str {
        match self {
            Corner::TopLeft => "Top left",
            Corner::TopRight => "Top right",
            Corner::BottomLeft => "Bottom left",
            Corner::BottomRight => "Bottom right",
        }
    }
}

/// A gauge a mode shows with `hud(label, value, max)`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gauge {
    pub label: String,
    pub value: f32,
    pub max: f32,
}

/// A short message a mode shows with `hud_event(text)`, fading out with age.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Event {
    pub text: String,
    /// Seconds since it was raised.
    pub age: f32,
}

/// Everything the overlay draws. GameViber computes it; the overlay only lays it out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayState {
    pub version: u32,
    pub visible: bool,
    pub corner: Corner,
    /// Size multiplier (1 = default size at 1080p).
    pub scale: f32,
    /// Background opacity, 0..1.
    pub opacity: f32,
    /// Strongest toy output after the safety layer, 0..1.
    pub output: f32,
    /// Global intensity cap, 0..1.
    pub cap: f32,
    /// The panic stop is engaged.
    pub panic: bool,
    pub mode: String,
    pub preset: Option<String>,
    /// Seconds since the mode or preset changed (the overlay shows them larger for a while).
    pub mode_age: f32,
    /// Scene the mode recognizes in the game's sound ("battle", "calm"...).
    pub scene: Option<String>,
    pub gauges: Vec<Gauge>,
    pub events: Vec<Event>,
    /// Problems the player should know about (toy lost, mode error...).
    pub alerts: Vec<String>,
    /// Copy the game's frames to GameViber, whether the panel is visible or not.
    pub capture: Option<CaptureRequest>,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            version: PROTOCOL_VERSION,
            visible: true,
            corner: Corner::default(),
            scale: 1.0,
            opacity: 0.75,
            output: 0.0,
            cap: 1.0,
            panic: false,
            mode: String::new(),
            preset: None,
            // JSON has no infinity.
            mode_age: 1e6,
            scene: None,
            gauges: Vec::new(),
            events: Vec::new(),
            alerts: Vec::new(),
            capture: None,
        }
    }
}

/// Shared memory holding the latest copy of the game's image: a
/// [`frames::Header`] followed by RGBA pixels (8 bits per channel, gamma
/// encoded, rows top to bottom, no padding). The overlay creates it with
/// `memfd_create` and seals its size; one overlay writes, GameViber reads,
/// and a sequence number tells a torn read apart (a seqlock).
pub mod frames {
    use std::sync::atomic::{fence, AtomicU32, Ordering};

    pub const MAGIC: u32 = u32::from_le_bytes(*b"GVfr");
    /// The copies are never larger than this.
    pub const MAX_WIDTH: u32 = 960;
    pub const MAX_HEIGHT: u32 = 960;
    pub const HEADER_SIZE: usize = 64;
    pub const SIZE: usize = HEADER_SIZE + (MAX_WIDTH * MAX_HEIGHT * 4) as usize;

    #[repr(C)]
    pub struct Header {
        pub magic: u32,
        /// Odd while a frame is being written.
        pub seq: AtomicU32,
        pub width: AtomicU32,
        pub height: AtomicU32,
        /// Size of the game's image the copy was made from.
        pub source_width: AtomicU32,
        pub source_height: AtomicU32,
        /// Frames written so far.
        pub count: AtomicU32,
    }

    /// A frame read from the shared memory.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct Frame {
        pub width: u32,
        pub height: u32,
        pub source_width: u32,
        pub source_height: u32,
        pub count: u32,
        pub pixels: Vec<u8>,
    }

    /// Height of a copy `width` pixels wide of a `source` image, even and within bounds.
    pub fn copy_size(width: u32, source: (u32, u32)) -> (u32, u32) {
        let width = width.clamp(16, MAX_WIDTH).min(source.0.max(16));
        let height = (width as u64 * source.1 as u64 / source.0.max(1) as u64) as u32;
        (width & !1, (height.clamp(16, MAX_HEIGHT)) & !1)
    }

    /// Writes a frame: `fill` gets the pixel bytes of `width` x `height`.
    ///
    /// # Safety
    /// `base` points to [`SIZE`] writable bytes, written by this process only.
    pub unsafe fn write(base: *mut u8, width: u32, height: u32, source: (u32, u32), fill: impl FnOnce(&mut [u8])) {
        if width > MAX_WIDTH || height > MAX_HEIGHT {
            return;
        }
        std::ptr::write_volatile(base as *mut u32, MAGIC);
        let header = &*(base as *const Header);
        let seq = header.seq.load(Ordering::Relaxed);
        header.seq.store(seq | 1, Ordering::Relaxed);
        fence(Ordering::Release);
        header.width.store(width, Ordering::Relaxed);
        header.height.store(height, Ordering::Relaxed);
        header.source_width.store(source.0, Ordering::Relaxed);
        header.source_height.store(source.1, Ordering::Relaxed);
        fill(std::slice::from_raw_parts_mut(base.add(HEADER_SIZE), (width * height * 4) as usize));
        header.count.fetch_add(1, Ordering::Relaxed);
        header.seq.store((seq | 1).wrapping_add(1), Ordering::Release);
    }

    /// The latest frame when it is newer than `after` (a frame count); `None`
    /// when nothing new was written or a write was in progress.
    ///
    /// # Safety
    /// `base` points to [`SIZE`] readable bytes.
    pub unsafe fn read(base: *const u8, after: Option<u32>) -> Option<Frame> {
        let header = &*(base as *const Header);
        if std::ptr::read_volatile(&header.magic) != MAGIC {
            return None;
        }
        let seq = header.seq.load(Ordering::Acquire);
        if seq & 1 == 1 {
            return None;
        }
        let count = header.count.load(Ordering::Relaxed);
        if after == Some(count) || count == 0 {
            return None;
        }
        let width = header.width.load(Ordering::Relaxed).min(MAX_WIDTH);
        let height = header.height.load(Ordering::Relaxed).min(MAX_HEIGHT);
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        std::ptr::copy_nonoverlapping(base.add(HEADER_SIZE), pixels.as_mut_ptr(), pixels.len());
        let frame = Frame {
            width,
            height,
            source_width: header.source_width.load(Ordering::Relaxed),
            source_height: header.source_height.load(Ordering::Relaxed),
            count,
            pixels,
        };
        fence(Ordering::Acquire);
        (header.seq.load(Ordering::Relaxed) == seq).then_some(frame)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn frames_written_are_read_once() {
            let mut memory = vec![0u64; SIZE / 8];
            let base = memory.as_mut_ptr() as *mut u8;
            unsafe {
                assert_eq!(read(base, None), None, "nothing written yet");
                write(base, 2, 2, (1920, 1080), |p| p.copy_from_slice(&[7; 16]));
                let frame = read(base, None).unwrap();
                assert_eq!((frame.width, frame.height, frame.source_width, frame.count), (2, 2, 1920, 1));
                assert_eq!(frame.pixels, vec![7; 16]);
                assert_eq!(read(base, Some(1)), None, "already read");
                write(base, 2, 2, (1920, 1080), |p| p.fill(9));
                assert_eq!(read(base, Some(1)).unwrap().pixels, vec![9; 16]);
            }
        }

        #[test]
        fn copies_keep_the_aspect_ratio() {
            assert_eq!(copy_size(480, (1920, 1080)), (480, 270 & !1));
            assert_eq!(copy_size(480, (2560, 1080)), (480, 202));
            assert_eq!(copy_size(4000, (800, 600)), (800, 600));
        }
    }
}
