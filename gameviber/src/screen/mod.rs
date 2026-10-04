//! The game's image: small copies of its frames made by the in-game overlay
//! (`gameviber_common::overlay::frames`), and what is measured on them:
//! brightness, motion and flashes here, embeddings for scenes (`clip`, in a
//! thread of its own) and the profile's zones (`zones`). The copies stay in
//! memory and are never saved.

pub mod clip;
pub mod zones;

use std::collections::VecDeque;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use crate::models::Embedding;

pub use gameviber_common::overlay::frames::Frame;

/// Measures are made on a grayscale grid this many cells wide.
const GRID_WIDTH: usize = 64;
/// Frame rate is measured over this many seconds.
const RATE_SECS: f64 = 2.0;
/// Mean change of a grid cell between two frames counted as full motion.
const FULL_MOTION: f32 = 0.25;
/// `action` follows the motion over about this many seconds.
const ACTION_SECS: f64 = 6.0;
/// The motion counted as full action (most of a game's motion is far below full motion).
const FULL_ACTION: f32 = 0.3;
/// A brightness rise this large from one copy to the next is a flash.
const FLASH_RISE: f32 = 0.12;
/// A rise this large is the strongest flash.
const FULL_FLASH: f32 = 0.4;

/// What is measured on one frame, each 0..1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScreenLevels {
    pub brightness: f32,
    /// How much the image changed since the previous copy.
    pub motion: f32,
    /// Motion over the last ~6 s: a slow "how busy is the screen".
    pub action: f32,
}

#[derive(Default)]
pub struct Analyzer {
    previous: Option<(usize, usize, Vec<f32>)>,
    times: VecDeque<f64>,
    brightness: Option<f32>,
    action: f32,
}

impl Analyzer {
    /// Measures a new copy; also returns the strength of a flash (0..1) when the image just lit up.
    pub fn push(&mut self, time: f64, frame: &Frame) -> (ScreenLevels, Option<f64>) {
        let dt = self.times.back().map_or(0.0, |t| (time - t).clamp(0.0, 1.0));
        self.times.push_back(time);
        while self.times.front().is_some_and(|t| time - t > RATE_SECS) {
            self.times.pop_front();
        }
        let (w, h, grid) = grid(frame);
        let brightness = grid.iter().sum::<f32>() / grid.len().max(1) as f32;
        let motion = match &self.previous {
            Some((pw, ph, previous)) if (*pw, *ph) == (w, h) => {
                let change = grid.iter().zip(previous).map(|(a, b)| (a - b).abs()).sum::<f32>() / grid.len().max(1) as f32;
                (change / FULL_MOTION).min(1.0)
            }
            _ => 0.0,
        };
        self.previous = Some((w, h, grid));
        self.action += ((motion / FULL_ACTION).min(1.0) - self.action) * (dt / ACTION_SECS).min(1.0) as f32;
        let rise = brightness - self.brightness.replace(brightness).unwrap_or(brightness);
        let flash = (rise >= FLASH_RISE).then(|| (rise / FULL_FLASH).min(1.0) as f64);
        (ScreenLevels { brightness, motion, action: self.action }, flash)
    }

    /// Copies received per second, lately.
    pub fn rate(&self) -> f64 {
        match (self.times.front(), self.times.back()) {
            (Some(first), Some(last)) if last > first => (self.times.len() - 1) as f64 / (last - first),
            _ => 0.0,
        }
    }

    /// When the last copy came.
    pub fn last(&self) -> Option<f64> {
        self.times.back().copied()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Luma (gamma encoded, 0..1) averaged over the cells of a grid `GRID_WIDTH` wide.
fn grid(frame: &Frame) -> (usize, usize, Vec<f32>) {
    let (fw, fh) = (frame.width as usize, frame.height as usize);
    if fw == 0 || fh == 0 || frame.pixels.len() < fw * fh * 4 {
        return (0, 0, Vec::new());
    }
    let w = GRID_WIDTH.min(fw);
    let h = (fh * w / fw).max(1);
    let mut sums = vec![0f32; w * h];
    let mut counts = vec![0u32; w * h];
    for y in 0..fh {
        let gy = y * h / fh;
        for x in 0..fw {
            let p = &frame.pixels[(y * fw + x) * 4..][..3];
            let luma = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
            let cell = gy * w + x * w / fw;
            sums[cell] += luma / 255.0;
            counts[cell] += 1;
        }
    }
    let grid = sums.iter().zip(&counts).map(|(s, &c)| s / c.max(1) as f32).collect();
    (w, h, grid)
}

/// The image model is unloaded after this long without an image to embed.
const MODEL_IDLE: Duration = Duration::from_secs(30);

/// Embeds copies of the game's image with the CLIP model, in a thread of its
/// own; the model is loaded on the first image.
pub struct ImageScenes {
    frames: mpsc::SyncSender<Arc<Frame>>,
    embeddings: mpsc::Receiver<Embedding>,
}

impl ImageScenes {
    pub fn start() -> Self {
        let (frames, frames_rx) = mpsc::sync_channel::<Arc<Frame>>(1);
        let (tx, embeddings) = mpsc::channel();
        std::thread::Builder::new()
            .name("screen-scenes".into())
            .spawn(move || embed_frames(frames_rx, tx))
            .expect("spawn the image scenes thread");
        Self { frames, embeddings }
    }

    /// Asks for the embedding of `frame`; skipped while the previous one is in progress.
    pub fn submit(&self, frame: Arc<Frame>) {
        let _ = self.frames.try_send(frame);
    }

    pub fn poll(&self) -> Vec<Embedding> {
        self.embeddings.try_iter().collect()
    }
}

fn embed_frames(frames: mpsc::Receiver<Arc<Frame>>, tx: mpsc::Sender<Embedding>) {
    let mut encoder: Option<clip::ImageEncoder> = None;
    let mut failed = false;
    loop {
        let frame = match frames.recv_timeout(MODEL_IDLE) {
            Ok(frame) => frame,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if encoder.take().is_some() {
                    log::debug!("image scene model unloaded");
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if encoder.is_none() && !failed {
            match clip::ImageEncoder::load() {
                Ok(e) => {
                    log::info!("image scene model loaded");
                    encoder = Some(e);
                }
                Err(e) => {
                    log::error!("cannot load the image scene model: {e:#}");
                    failed = true;
                }
            }
        }
        let Some(encoder) = encoder.as_mut() else { continue };
        match encoder.embed(&frame) {
            Ok(embedding) => {
                if tx.send(embedding).is_err() {
                    break;
                }
            }
            Err(e) => log::warn!("image embedding failed: {e:#}"),
        }
    }
}

/// What the GUI shows of the game's image.
#[derive(Debug, Clone, Default)]
pub struct ScreenView {
    /// The game the copies come from.
    pub game: Option<String>,
    pub frame: Option<Arc<Frame>>,
    pub levels: Option<ScreenLevels>,
    /// Copies per second.
    pub rate: f64,
    /// The image model is ready and scenes or examples use it.
    pub model: crate::models::ModelState,
    /// Raw measure of each zone of the game's profile (similarity or fill).
    pub zones: Vec<(String, f32, Option<crate::mode::ZoneValue>)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32, pixel: impl Fn(u32, u32) -> u8) -> Frame {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let v = pixel(x, y);
                pixels.extend([v, v, v, 255]);
            }
        }
        Frame { width, height, source_width: width * 4, source_height: height * 4, count: 1, pixels }
    }

    #[test]
    fn brightness_and_motion_follow_the_image() {
        let mut analyzer = Analyzer::default();
        let black = frame(128, 72, |_, _| 0);
        let (still, flash) = analyzer.push(0.0, &black);
        assert_eq!((still, flash), (ScreenLevels::default(), None));
        assert_eq!(analyzer.push(0.1, &black).0.motion, 0.0, "nothing moved");

        // Half the screen lights up: motion and a flash.
        let half = frame(128, 72, |x, _| if x < 64 { 255 } else { 0 });
        let (levels, flash) = analyzer.push(0.2, &half);
        assert!((levels.brightness - 0.5).abs() < 0.01, "{levels:?}");
        assert_eq!(levels.motion, 1.0, "a big change is full motion");
        assert!(levels.action > 0.0 && levels.action < 0.1, "action builds up slowly: {levels:?}");
        assert_eq!(flash, Some(1.0));
        assert!((analyzer.rate() - 10.0).abs() < 0.01);
        assert_eq!(analyzer.push(0.3, &black).1, None, "darkening is no flash");

        // A small object moving a little.
        let a = frame(128, 72, |x, y| if (10..20).contains(&x) && (10..20).contains(&y) { 255 } else { 0 });
        let b = frame(128, 72, |x, y| if (14..24).contains(&x) && (10..20).contains(&y) { 255 } else { 0 });
        analyzer.push(0.4, &a);
        let small = analyzer.push(0.5, &b).0.motion;
        assert!(small > 0.0 && small < 0.1, "{small}");
    }
}
