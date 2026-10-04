//! Zones of the game's screen declared in its profile (`profile::Zone`):
//! whether an element is shown (its look compared with a reference taken when
//! the zone was drawn) and how full a bar is (the share of it in the bar's
//! color).

use std::collections::BTreeMap;

use super::Frame;
use crate::mode::ZoneValue;
use crate::profile::{Direction, Zone, ZoneKind};

/// References are compared on a grayscale grid of this size.
pub const REF_WIDTH: usize = 32;
pub const REF_HEIGHT: usize = 24;
/// A shown element is hidden once its similarity drops this far below the threshold.
const HYSTERESIS: f32 = 0.05;
/// Bar values are reported when they move this much.
const BAR_STEP: f64 = 0.02;

/// Pixel bounds of a zone in `frame`, at least 2 x 2.
fn bounds(frame: &Frame, rect: [f32; 4]) -> (usize, usize, usize, usize) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let x0 = ((rect[0].clamp(0.0, 1.0) * w as f32) as usize).min(w.saturating_sub(2));
    let y0 = ((rect[1].clamp(0.0, 1.0) * h as f32) as usize).min(h.saturating_sub(2));
    let x1 = (((rect[0] + rect[2]).clamp(0.0, 1.0) * w as f32).ceil() as usize).clamp(x0 + 2, w);
    let y1 = (((rect[1] + rect[3]).clamp(0.0, 1.0) * h as f32).ceil() as usize).clamp(y0 + 2, h);
    (x0, y0, x1, y1)
}

fn pixel(frame: &Frame, x: usize, y: usize) -> [u8; 3] {
    let i = (y * frame.width as usize + x) * 4;
    [frame.pixels[i], frame.pixels[i + 1], frame.pixels[i + 2]]
}

/// The zone's look: luma averaged on a `REF_WIDTH` x `REF_HEIGHT` grid.
pub fn reference(frame: &Frame, rect: [f32; 4]) -> Vec<u8> {
    gray(frame, rect).iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect()
}

fn gray(frame: &Frame, rect: [f32; 4]) -> Vec<f32> {
    let (x0, y0, x1, y1) = bounds(frame, rect);
    let mut sums = vec![0f32; REF_WIDTH * REF_HEIGHT];
    let mut counts = vec![0u32; REF_WIDTH * REF_HEIGHT];
    for y in y0..y1 {
        let gy = (y - y0) * REF_HEIGHT / (y1 - y0);
        for x in x0..x1 {
            let gx = (x - x0) * REF_WIDTH / (x1 - x0);
            let [r, g, b] = pixel(frame, x, y);
            sums[gy * REF_WIDTH + gx] += 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
            counts[gy * REF_WIDTH + gx] += 1;
        }
    }
    // Cells with no pixel (zones smaller than the grid) take their left neighbour.
    for i in 0..sums.len() {
        if counts[i] == 0 && i > 0 {
            sums[i] = sums[i - 1];
            counts[i] = 1;
        } else {
            sums[i] /= counts[i].max(1) as f32;
            counts[i] = 1;
        }
    }
    sums
}

/// Normalized correlation of two grids, -1..1 (0 when either is flat).
fn correlation(a: &[f32], b: &[f32]) -> f32 {
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let (ma, mb) = (mean(a), mean(b));
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (x - ma, y - mb);
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    if aa < 1e-3 || bb < 1e-3 {
        return 0.0;
    }
    ab / (aa * bb).sqrt()
}

/// The color of a bar's filled part: the average of its most colorful pixels.
pub fn bar_color(frame: &Frame, rect: [f32; 4]) -> [u8; 3] {
    let (x0, y0, x1, y1) = bounds(frame, rect);
    let mut pixels: Vec<([u8; 3], u8)> = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            let p = pixel(frame, x, y);
            let (max, min) = (*p.iter().max().unwrap(), *p.iter().min().unwrap());
            pixels.push((p, max - min));
        }
    }
    pixels.sort_by_key(|(_, chroma)| std::cmp::Reverse(*chroma));
    let top = &pixels[..(pixels.len() / 3).max(1)];
    let mut sum = [0u32; 3];
    for (p, _) in top {
        (0..3).for_each(|c| sum[c] += p[c] as u32);
    }
    sum.map(|s| (s / top.len() as u32) as u8)
}

/// What a zone reads on `frame`: the similarity with its reference (-1..1)
/// for a visible zone, how full it is (0..1) for a bar.
pub fn measure(zone: &Zone, frame: &Frame) -> f32 {
    match zone.kind {
        ZoneKind::Visible => {
            if zone.reference.len() != REF_WIDTH * REF_HEIGHT {
                return 0.0;
            }
            let reference: Vec<f32> = zone.reference.iter().map(|&v| v as f32).collect();
            correlation(&gray(frame, zone.rect), &reference)
        }
        ZoneKind::Bar => fill(zone, frame),
    }
}

/// Share of the bar's length whose middle line is in the bar's color.
fn fill(zone: &Zone, frame: &Frame) -> f32 {
    let (x0, y0, x1, y1) = bounds(frame, zone.rect);
    let horizontal = matches!(zone.direction, Direction::Right | Direction::Left);
    let (length, across) = if horizontal { (x1 - x0, y1 - y0) } else { (y1 - y0, x1 - x0) };
    let near = |p: [u8; 3]| {
        let d: f32 = (0..3).map(|c| (p[c] as f32 - zone.color[c] as f32).powi(2)).sum::<f32>().sqrt();
        d <= zone.tolerance
    };
    let mut filled = 0;
    for i in 0..length {
        // The middle third across the bar: borders and shadows stay out.
        let lines = (across / 3).max(1);
        let start = (across - lines) / 2;
        let hits = (start..start + lines)
            .filter(|&j| {
                let (x, y) = if horizontal { (x0 + i, y0 + j) } else { (x0 + j, y0 + i) };
                near(pixel(frame, x, y))
            })
            .count();
        if hits * 2 >= lines {
            filled += 1;
        }
    }
    filled as f32 / length.max(1) as f32
}

/// Reads a profile's zones frame after frame and tells which changed.
#[derive(Default)]
pub struct ZoneReader {
    values: BTreeMap<String, ZoneValue>,
    /// Raw measures of the last frame, for the GUI.
    pub measures: BTreeMap<String, f32>,
}

impl ZoneReader {
    pub fn update(&mut self, zones: &[Zone], frame: &Frame) -> Vec<(String, ZoneValue)> {
        let mut changed = Vec::new();
        self.measures.clear();
        self.values.retain(|name, _| zones.iter().any(|z| z.name == *name));
        for zone in zones {
            let measure = measure(zone, frame);
            self.measures.insert(zone.name.clone(), measure);
            let previous = self.values.get(&zone.name).copied();
            let value = match (zone.kind, previous) {
                (ZoneKind::Visible, Some(ZoneValue::Visible(true))) => ZoneValue::Visible(measure >= zone.threshold - HYSTERESIS),
                (ZoneKind::Visible, _) => ZoneValue::Visible(measure >= zone.threshold),
                (ZoneKind::Bar, Some(ZoneValue::Bar(old))) if (measure as f64 - old).abs() < BAR_STEP => ZoneValue::Bar(old),
                (ZoneKind::Bar, _) => ZoneValue::Bar((measure as f64 * 100.0).round() / 100.0),
            };
            if previous != Some(value) {
                self.values.insert(zone.name.clone(), value);
                changed.push((zone.name.clone(), value));
            }
        }
        changed
    }

    pub fn values(&self) -> &BTreeMap<String, ZoneValue> {
        &self.values
    }

    pub fn clear(&mut self) {
        self.values.clear();
        self.measures.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(pixel: impl Fn(u32, u32) -> [u8; 3]) -> Frame {
        let (w, h) = (160, 90);
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let p = pixel(x, y);
                pixels.extend([p[0], p[1], p[2], 255]);
            }
        }
        Frame { width: w, height: h, source_width: 1920, source_height: 1080, count: 1, pixels }
    }

    /// A battle interface: a ring in the bottom right corner over some scenery.
    fn with_hud(shown: bool, shift: u32) -> Frame {
        frame(|x, y| {
            let scenery = [((x + shift) * 3 % 200) as u8, 90, ((y * 5) % 160) as u8];
            let (dx, dy) = (x as f32 - 140.0, y as f32 - 75.0);
            let ring = (dx * dx + dy * dy).sqrt();
            if shown && (6.0..10.0).contains(&ring) {
                [250, 250, 250]
            } else {
                scenery
            }
        })
    }

    #[test]
    fn visible_zones_follow_the_element() {
        let rect = [120.0 / 160.0, 60.0 / 90.0, 40.0 / 160.0, 30.0 / 90.0];
        let zone = Zone { name: "battle_hud".into(), rect, reference: reference(&with_hud(true, 0), rect), ..Zone::default() };
        assert!(measure(&zone, &with_hud(true, 0)) > 0.99);
        // The scenery moved behind the element: still recognized; gone: not.
        let moved = measure(&zone, &with_hud(true, 37));
        let hidden = measure(&zone, &with_hud(false, 37));
        assert!(moved > zone.threshold, "{moved}");
        assert!(hidden < zone.threshold, "{hidden}");

        let mut reader = ZoneReader::default();
        let zones = [zone];
        assert_eq!(reader.update(&zones, &with_hud(true, 0)), vec![("battle_hud".to_owned(), ZoneValue::Visible(true))]);
        assert_eq!(reader.update(&zones, &with_hud(true, 10)), vec![], "no change, nothing reported");
        assert_eq!(reader.update(&zones, &with_hud(false, 10)), vec![("battle_hud".to_owned(), ZoneValue::Visible(false))]);
    }

    #[test]
    fn bars_are_measured_with_their_color() {
        // A red bar from x = 20 to 100, filled up to `level`, on a dark frame with a gray outline.
        let bar = |level: f32| {
            frame(move |x, y| match (x, y) {
                (20..=99, 10..=15) if (x - 20) as f32 / 80.0 < level => [200, 30, 40],
                (19..=100, 9..=16) => [120, 120, 120],
                _ => [20, 20, 30],
            })
        };
        let rect = [20.0 / 160.0, 10.0 / 90.0, 80.0 / 160.0, 6.0 / 90.0];
        let color = bar_color(&bar(1.0), rect);
        assert!(color[0] > 180 && color[1] < 60, "{color:?}");
        let zone = Zone { name: "hp".into(), kind: ZoneKind::Bar, rect, color, ..Zone::default() };
        for level in [1.0, 0.75, 0.3, 0.0] {
            let got = measure(&zone, &bar(level));
            assert!((got - level).abs() < 0.04, "{level}: {got}");
        }
        let left = Zone { direction: Direction::Left, ..zone.clone() };
        assert!((measure(&left, &bar(0.5)) - 0.5).abs() < 0.04, "the share filled does not depend on the side");

        let mut reader = ZoneReader::default();
        let zones = [zone];
        assert_eq!(reader.update(&zones, &bar(1.0)), vec![("hp".to_owned(), ZoneValue::Bar(1.0))]);
        assert_eq!(reader.update(&zones, &bar(0.99)), vec![], "small moves are not reported");
        assert_eq!(reader.update(&zones, &bar(0.5)), vec![("hp".to_owned(), ZoneValue::Bar(0.5))]);
    }
}
