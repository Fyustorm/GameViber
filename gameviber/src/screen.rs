//! The game's image: small copies of its frames made by the in-game overlay
//! (`gameviber_common::overlay::frames`), and what is measured on them. The
//! copies stay in memory and are never saved.

use std::collections::VecDeque;
use std::sync::Arc;

pub use gameviber_common::overlay::frames::Frame;

/// Measures are made on a grayscale grid this many cells wide.
const GRID_WIDTH: usize = 64;
/// Frame rate is measured over this many seconds.
const RATE_SECS: f64 = 2.0;
/// Mean change of a grid cell between two frames counted as full motion.
const FULL_MOTION: f32 = 0.25;

/// What is measured on one frame, each 0..1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScreenLevels {
    pub brightness: f32,
    /// How much the image changed since the previous copy.
    pub motion: f32,
}

#[derive(Default)]
pub struct Analyzer {
    previous: Option<(usize, usize, Vec<f32>)>,
    times: VecDeque<f64>,
}

impl Analyzer {
    pub fn push(&mut self, time: f64, frame: &Frame) -> ScreenLevels {
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
        ScreenLevels { brightness, motion }
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
        self.previous = None;
        self.times.clear();
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

/// What the GUI shows of the game's image.
#[derive(Debug, Clone, Default)]
pub struct ScreenView {
    /// The game the copies come from.
    pub game: Option<String>,
    pub frame: Option<Arc<Frame>>,
    pub levels: Option<ScreenLevels>,
    /// Copies per second.
    pub rate: f64,
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
        let still = analyzer.push(0.0, &black);
        assert_eq!(still, ScreenLevels { brightness: 0.0, motion: 0.0 });
        assert_eq!(analyzer.push(0.1, &black).motion, 0.0, "nothing moved");

        // Half the screen lights up.
        let half = frame(128, 72, |x, _| if x < 64 { 255 } else { 0 });
        let levels = analyzer.push(0.2, &half);
        assert!((levels.brightness - 0.5).abs() < 0.01, "{levels:?}");
        assert_eq!(levels.motion, 1.0, "a big change is full motion");
        assert!((analyzer.rate() - 10.0).abs() < 0.01);

        // A small object moving a little.
        let a = frame(128, 72, |x, y| if (10..20).contains(&x) && (10..20).contains(&y) { 255 } else { 0 });
        let b = frame(128, 72, |x, y| if (14..24).contains(&x) && (10..20).contains(&y) { 255 } else { 0 });
        analyzer.push(0.3, &a);
        let small = analyzer.push(0.4, &b).motion;
        assert!(small > 0.0 && small < 0.1, "{small}");
    }
}
