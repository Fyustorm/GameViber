//! Zones of the game's screen declared in its profile (`profile::Zone`):
//! whether an element is shown (its look compared with a reference taken when
//! the zone was drawn) and how full a bar is (the share of it in the bar's
//! full color rather than its empty one; with both colors known, the bar is
//! the longest run of them in the zone, wherever it is).

use std::collections::BTreeMap;

use super::Frame;
use crate::mode::ZoneValue;
use crate::game::{Direction, Zone, ZoneKind};

/// References are compared on a grayscale grid of this size.
pub const REF_WIDTH: usize = 32;
pub const REF_HEIGHT: usize = 24;
/// A shown element is hidden once its similarity drops this far below the threshold.
const HYSTERESIS: f32 = 0.05;
/// Bar values are reported when they move this much.
const BAR_STEP: f64 = 0.02;
/// A bar is not on screen when less than this share of its drawn length is
/// found, or more than `FOUND_MAX` (a background of its color, not the bar)...
const FOUND_SHARE: f32 = 0.6;
const FOUND_MAX: f32 = 1.5;
/// ...for this many copies in a row (0.3 s), so that a flash does not hide it.
const UNKNOWN_FRAMES: u32 = 3;

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

/// The color of the pixel at a point of `frame` (fractions of the image).
pub fn pick_color(frame: &Frame, x: f32, y: f32) -> [u8; 3] {
    let x = ((x * frame.width as f32) as usize).min(frame.width as usize - 1);
    let y = ((y * frame.height as f32) as usize).min(frame.height as usize - 1);
    pixel(frame, x, y)
}

/// A threshold telling the zone's scene apart from the others, from its
/// similarity on captures of that scene (`shown`) and of the others
/// (`hidden`): halfway between them when they do not overlap.
pub fn suggest_threshold(shown: &[f32], hidden: &[f32]) -> Option<f32> {
    let low = shown.iter().copied().fold(f32::INFINITY, f32::min);
    let high = hidden.iter().copied().fold(-1.0, f32::max);
    (!shown.is_empty() && low > high).then(|| ((low + high) / 2.0).clamp(0.1, 0.95))
}

/// What a zone reads on `frame`: the similarity with its reference (-1..1)
/// for a visible zone, how full it is (0..1) for a bar; None when the bar is
/// not on screen (too little of its colors in the zone, which needs its empty
/// color to be told from an empty bar).
pub fn measure(zone: &Zone, frame: &Frame) -> Option<f32> {
    match zone.kind {
        ZoneKind::Visible => {
            if zone.reference.len() != REF_WIDTH * REF_HEIGHT {
                return Some(0.0);
            }
            let reference: Vec<f32> = zone.reference.iter().map(|&v| v as f32).collect();
            Some(correlation(&gray(frame, zone.rect), &reference))
        }
        ZoneKind::Bar => bar_reading(zone, frame).map(|(fill, _)| fill),
    }
}

/// How full a bar is, and how much of its drawn length was found (1 without
/// the empty color); None when too little was found.
fn bar_reading(zone: &Zone, frame: &Frame) -> Option<(f32, f32)> {
    let bar = bar(zone, frame);
    let expected = if zone.length > 0.0 { zone.length } else { 0.1 };
    let found = if zone.empty_color.is_some() { bar.length() / expected } else { 1.0 };
    let too_long = zone.length > 0.0 && found > FOUND_MAX;
    (found >= FOUND_SHARE && !too_long).then_some((bar.fill, found.min(1.0)))
}

/// A zone drawn in several places (zones sharing its name: the health bar in
/// battle and out of it) is shown when one of them shows it; `was_shown`
/// lowers the thresholds a little (hysteresis). Also returns the best similarity.
pub fn shown_anywhere(places: &[&Zone], frame: &Frame, was_shown: bool) -> (bool, f32) {
    let margin = if was_shown { HYSTERESIS } else { 0.0 };
    places.iter().fold((false, -1.0), |(shown, best), zone| {
        let m = measure(zone, frame).unwrap_or(0.0);
        (shown || m >= zone.threshold - margin, best.max(m))
    })
}

/// A bar drawn in several places reads where it is found (the most complete
/// when several find it); None when no place finds it.
pub fn fill_anywhere(places: &[&Zone], frame: &Frame) -> Option<f32> {
    places
        .iter()
        .filter_map(|zone| bar_reading(zone, frame))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(fill, _)| fill)
}

/// Share of the zone's length a bar covers on `frame` (to save with a zone being drawn).
pub fn bar_length(zone: &Zone, frame: &Frame) -> f32 {
    bar(zone, frame).length()
}

/// Where the bar was found in the zone, as fractions of the zone's length
/// along its axis (left to right, top to bottom): its full part, then its
/// empty part. For the editor.
pub fn bar_extent(zone: &Zone, frame: &Frame) -> ((f32, f32), (f32, f32)) {
    let bar = bar(zone, frame);
    let full = (bar.end - bar.start) * bar.fill;
    match zone.direction {
        Direction::Left | Direction::Up => ((bar.end - full, bar.end), (bar.start, bar.end - full)),
        Direction::Right | Direction::Down => ((bar.start, bar.start + full), (bar.start + full, bar.end)),
    }
}

/// A bar found in its zone.
struct Bar {
    fill: f32,
    /// Fractions of the zone's length along its axis.
    start: f32,
    end: f32,
}

impl Bar {
    fn length(&self) -> f32 {
        self.end - self.start
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Part {
    Full,
    Empty,
    /// Neither color: outside the bar.
    Other,
}

/// The share of the bar whose middle line is in its full color, nearer to it
/// than to its empty color. With both colors, the bar is the longest run of
/// full and empty columns in the zone; with the full color only, the whole zone.
fn bar(zone: &Zone, frame: &Frame) -> Bar {
    let parts = columns(zone, frame);
    let len = parts.len().max(1) as f32;
    // Which end the bar fills from: its full part is read from there.
    let reversed = matches!(zone.direction, Direction::Left | Direction::Up);
    if zone.empty_color.is_some() {
        // The longest run of bar columns, one stray column allowed inside.
        let (mut best, mut best_full, mut best_start) = (0, 0, 0);
        let (mut start, mut gap) = (0, 0);
        for i in 0..=parts.len() {
            let inside = parts.get(i).is_some_and(|p| *p != Part::Other);
            if inside {
                gap = 0;
                continue;
            }
            gap += 1;
            if gap > 1 || i == parts.len() {
                let end = i + 1 - gap;
                if end > start && end - start > best {
                    best = end - start;
                    best_full = parts[start..end].iter().filter(|p| **p == Part::Full).count();
                    best_start = start;
                }
                start = i + 1;
                gap = 0;
            }
        }
        let (start, end) = (best_start as f32 / len, (best_start + best) as f32 / len);
        let (start, end) = if reversed { (1.0 - end, 1.0 - start) } else { (start, end) };
        return Bar { fill: best_full as f32 / best.max(1) as f32, start, end };
    }
    let filled = parts.iter().filter(|p| **p == Part::Full).count();
    Bar { fill: filled as f32 / len, start: 0.0, end: 1.0 }
}

/// Each column along the bar. The bar's lines are found first: those with
/// the most pixels near its colors, so that text or icons in a zone drawn
/// larger than the bar stay out. A column is then full (or empty) when one of
/// its pixels on those lines is near the full color (bars are often shaded, a
/// lighter line over a darker one, and the color was picked on one), empty
/// when most of them are near the empty color.
fn columns(zone: &Zone, frame: &Frame) -> Vec<Part> {
    let (x0, y0, x1, y1) = bounds(frame, zone.rect);
    let horizontal = matches!(zone.direction, Direction::Right | Direction::Left);
    let (length, across) = if horizontal { (x1 - x0, y1 - y0) } else { (y1 - y0, x1 - x0) };
    let at = |i: usize, j: usize| if horizontal { pixel(frame, x0 + i, y0 + j) } else { pixel(frame, x0 + j, y0 + i) };
    let distance = |p: [u8; 3], color: [u8; 3]| (0..3).map(|c| (p[c] as f32 - color[c] as f32).powi(2)).sum::<f32>().sqrt();
    // Distances of a pixel to the nearest full and empty colors (a bar may have several shades).
    let nearest = |p: [u8; 3], colors: &mut dyn Iterator<Item = &[u8; 3]>| colors.map(|c| distance(p, *c)).fold(f32::INFINITY, f32::min);
    let distances = |p: [u8; 3]| {
        let full = nearest(p, &mut std::iter::once(&zone.color).chain(&zone.more_colors));
        let empty = match &zone.empty_color {
            Some(e) => nearest(p, &mut std::iter::once(e).chain(&zone.more_empty)),
            None => f32::INFINITY,
        };
        (full, empty)
    };
    let near = |p: [u8; 3]| {
        let (full, empty) = distances(p);
        full.min(empty) <= zone.tolerance
    };
    let counts: Vec<usize> = (0..across).map(|j| (0..length).filter(|&i| near(at(i, j))).count()).collect();
    let most = counts.iter().copied().max().unwrap_or(0);
    let lines: Vec<usize> = (0..across).filter(|&j| most > 0 && counts[j] * 2 >= most).collect();
    let mut parts: Vec<Part> = (0..length)
        .map(|i| {
            let pixels: Vec<(f32, f32)> = lines.iter().map(|&j| distances(at(i, j))).collect();
            // Full when one of the bar's lines has the full color (shaded bars)...
            let full = pixels.iter().any(|&(f, e)| f <= zone.tolerance && f <= e);
            // ...empty only when most of them have the empty color: a line under
            // the bars in that color (a decoration) does not make one.
            let empty = pixels.iter().filter(|&&(f, e)| e <= zone.tolerance && e < f).count();
            if full {
                Part::Full
            } else if empty > 0 && empty * 2 >= pixels.len() {
                Part::Empty
            } else {
                Part::Other
            }
        })
        .collect();
    if matches!(zone.direction, Direction::Left | Direction::Up) {
        parts.reverse();
    }
    parts
}

/// Reads a profile's zones frame after frame and tells which changed.
#[derive(Default)]
pub struct ZoneReader {
    values: BTreeMap<String, ZoneValue>,
    /// Raw measures of the last frame, for the GUI (None: bar not on screen).
    pub measures: BTreeMap<String, Option<f32>>,
    /// Copies in a row each bar was not found on.
    missing: BTreeMap<String, u32>,
}

impl ZoneReader {
    pub fn update(&mut self, zones: &[Zone], frame: &Frame) -> Vec<(String, ZoneValue)> {
        let mut changed = Vec::new();
        self.measures.clear();
        self.values.retain(|name, _| zones.iter().any(|z| z.name == *name));
        // Zones sharing a name are places of one zone.
        let mut names: Vec<&str> = Vec::new();
        for zone in zones {
            if !names.contains(&zone.name.as_str()) {
                names.push(&zone.name);
            }
        }
        for name in names {
            let places: Vec<&Zone> = zones.iter().filter(|z| z.name == name).collect();
            let previous = self.values.get(name).copied();
            let (measure, value) = match places[0].kind {
                ZoneKind::Visible => {
                    let (shown, best) = shown_anywhere(&places, frame, previous == Some(ZoneValue::Visible(true)));
                    (Some(best), ZoneValue::Visible(shown))
                }
                ZoneKind::Bar => {
                    let measure = fill_anywhere(&places, frame);
                    let missing = self.missing.entry(name.to_owned()).or_default();
                    *missing = if measure.is_none() { *missing + 1 } else { 0 };
                    let value = match (measure, previous) {
                        // Briefly lost (a flash, an effect over it): keep the last value.
                        (None, Some(old)) if *missing < UNKNOWN_FRAMES => old,
                        (None, _) => ZoneValue::Unknown,
                        (Some(m), Some(ZoneValue::Bar(old))) if (m as f64 - old).abs() < BAR_STEP => ZoneValue::Bar(old),
                        (Some(m), _) => ZoneValue::Bar((m as f64 * 100.0).round() / 100.0),
                    };
                    (measure, value)
                }
            };
            self.measures.insert(name.to_owned(), measure);
            if previous != Some(value) {
                self.values.insert(name.to_owned(), value);
                changed.push((name.to_owned(), value));
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
        self.missing.clear();
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
        assert!(measure(&zone, &with_hud(true, 0)).unwrap() > 0.99);
        // The scenery moved behind the element: still recognized; gone: not.
        let moved = measure(&zone, &with_hud(true, 37)).unwrap();
        let hidden = measure(&zone, &with_hud(false, 37)).unwrap();
        assert!(moved > zone.threshold, "{moved}");
        assert!(hidden < zone.threshold, "{hidden}");

        let mut reader = ZoneReader::default();
        let zones = [zone];
        assert_eq!(reader.update(&zones, &with_hud(true, 0)), vec![("battle_hud".to_owned(), ZoneValue::Visible(true))]);
        assert_eq!(reader.update(&zones, &with_hud(true, 10)), vec![], "no change, nothing reported");
        assert_eq!(reader.update(&zones, &with_hud(false, 10)), vec![("battle_hud".to_owned(), ZoneValue::Visible(false))]);
    }

    /// Low on health the bar blinks: its full part turns lighter, then back.
    #[test]
    fn bars_with_several_shades() {
        let bar = |full: [u8; 3]| {
            frame(move |x, y| match (x, y) {
                (10..=69, 10..=15) => if x < 25 { full } else { [190, 30, 30] },
                _ => [25, 25, 35],
            })
        };
        let rect = [5.0 / 160.0, 9.0 / 90.0, 150.0 / 160.0, 8.0 / 90.0];
        let zone = Zone {
            name: "hp".into(),
            kind: ZoneKind::Bar,
            rect,
            color: [40, 200, 60],
            empty_color: Some([190, 30, 30]),
            tolerance: 40.0,
            ..Zone::default()
        };
        let lit = [170, 250, 170];
        assert!((measure(&zone, &bar([40, 200, 60])).unwrap() - 0.25).abs() < 0.04);
        assert!(measure(&zone, &bar(lit)).unwrap() < 0.05, "the lit shade is not the full color");
        let shades = Zone { more_colors: vec![lit], ..zone };
        for full in [[40, 200, 60], lit] {
            let got = measure(&shades, &bar(full)).unwrap();
            assert!((got - 0.25).abs() < 0.04, "{full:?}: {got}");
        }
    }

    /// Metaphor: a character's stance shifts its health bar sideways, and the
    /// zone may run over the mana bar next to it.
    #[test]
    fn bars_are_found_in_their_zone() {
        // Health (green) then wounds (red), 60 px long, starting at `left`, among other colors.
        let bar = |left: u32, level: f32| {
            frame(move |x, y| match (x, y) {
                (_, 10..=15) if (left..left + 60).contains(&x) => {
                    if ((x - left) as f32) < level * 60.0 { [40, 200, 60] } else { [190, 30, 30] }
                }
                (_, 10..=15) if x % 7 == 0 => [230, 230, 230],
                _ => [25, 25, 35],
            })
        };
        let rect = [5.0 / 160.0, 9.0 / 90.0, 150.0 / 160.0, 8.0 / 90.0];
        let zone = Zone {
            name: "hp".into(),
            kind: ZoneKind::Bar,
            rect,
            color: [40, 200, 60],
            empty_color: Some([190, 30, 30]),
            ..Zone::default()
        };
        for (left, level) in [(10, 1.0), (30, 0.5), (85, 0.25), (60, 0.0)] {
            let got = measure(&zone, &bar(left, level)).unwrap();
            assert!((got - level).abs() < 0.04, "bar at {left}, {level}: {got}");
        }
        // In a menu the bar is gone: unknown, not empty.
        let menu = frame(|x, _| if x % 7 == 0 { [230, 230, 230] } else { [25, 25, 35] });
        let drawn = Zone { length: bar_length(&zone, &bar(30, 0.5)), ..zone.clone() };
        assert!((drawn.length - 0.4).abs() < 0.02, "60 px of 150: {}", drawn.length);
        assert_eq!(measure(&drawn, &menu), None);
        assert_eq!(measure(&drawn, &bar(85, 0.0)), Some(0.0), "an empty bar is still a bar");
        let mut reader = ZoneReader::default();
        let zones = [drawn];
        let first = reader.update(&zones, &bar(30, 0.5));
        assert!(matches!(first[..], [(_, ZoneValue::Bar(v))] if (v - 0.5).abs() < 0.02), "{first:?}");
        assert_eq!(reader.update(&zones, &menu), vec![], "briefly lost: the last value stays");
        reader.update(&zones, &menu);
        assert_eq!(reader.update(&zones, &menu), vec![("hp".to_owned(), ZoneValue::Unknown)]);
        // Where it was found, for the editor: 60 px from x = 30 in a zone from x = 5, 150 px long.
        let ((start, full_end), (_, end)) = bar_extent(&zone, &bar(30, 0.5));
        let near = |a: f32, px: f32| (a - px / 150.0).abs() < 0.02;
        assert!(near(start, 25.0) && near(full_end, 55.0) && near(end, 85.0), "{start} {full_end} {end}");
        // Without the empty color the whole zone is the bar: 30 px full of 150.
        let full_only = Zone { empty_color: None, ..zone };
        assert!((measure(&full_only, &bar(30, 0.5)).unwrap() - 0.2).abs() < 0.03);
    }

    /// Metaphor's bars are 3 pixels high and shaded, with numbers above them.
    #[test]
    fn shaded_bars_under_text_are_read() {
        let shades = [[62, 143, 113], [82, 188, 149], [48, 112, 89]];
        let bar = |level: f32| {
            frame(move |x, y| match (x, y) {
                (20..=79, 20..=22) if ((x - 20) as f32) < level * 60.0 => shades[(y - 20) as usize],
                (20..=79, 20..=22) => [117, 23, 44],
                // The health number above, in the bar's green.
                (24..=40, 12..=17) if x % 3 != 0 => [80, 182, 144],
                _ => [17, 17, 17],
            })
        };
        // Drawn exactly on the bar, and larger with the number in it; the color picked on the darkest line.
        for rect in [[20.0 / 160.0, 20.0 / 90.0, 60.0 / 160.0, 3.0 / 90.0], [18.0 / 160.0, 10.0 / 90.0, 70.0 / 160.0, 14.0 / 90.0]] {
            let zone = Zone {
                name: "hp".into(),
                kind: ZoneKind::Bar,
                rect,
                color: pick_color(&bar(1.0), 30.0 / 160.0, 22.5 / 90.0),
                empty_color: Some([117, 23, 44]),
                tolerance: 50.0,
                ..Zone::default()
            };
            assert_eq!(zone.color, [48, 112, 89]);
            for level in [1.0, 0.7, 0.3] {
                let got = measure(&zone, &bar(level)).unwrap();
                assert!((got - level).abs() < 0.04, "{rect:?} {level}: {got}");
            }
        }
    }

    /// The health bar is in one place in battle and in another out of it.
    #[test]
    fn a_zone_drawn_in_two_places_reads_where_it_is_found() {
        let bar_at = |top: u32, level: f32| {
            frame(move |x, y| match (x, y) {
                (20..=79, _) if (top..top + 6).contains(&y) => {
                    if ((x - 20) as f32) < level * 60.0 { [40, 200, 60] } else { [190, 30, 30] }
                }
                _ => [25, 25, 35],
            })
        };
        let place = |top: u32| Zone {
            name: "hp".into(),
            kind: ZoneKind::Bar,
            rect: [15.0 / 160.0, (top as f32 - 1.0) / 90.0, 70.0 / 160.0, 8.0 / 90.0],
            color: [40, 200, 60],
            empty_color: Some([190, 30, 30]),
            length: 60.0 / 70.0,
            ..Zone::default()
        };
        let (battle, field) = (place(10), place(60));
        assert!((fill_anywhere(&[&battle, &field], &bar_at(10, 0.75)).unwrap() - 0.75).abs() < 0.04);
        assert!((fill_anywhere(&[&battle, &field], &bar_at(60, 0.25)).unwrap() - 0.25).abs() < 0.04);
        assert_eq!(fill_anywhere(&[&battle, &field], &frame(|_, _| [25, 25, 35])), None, "in no place: unknown");

        let mut reader = ZoneReader::default();
        let zones = [battle, field];
        let first = reader.update(&zones, &bar_at(60, 0.25));
        assert!(matches!(first[..], [(ref n, ZoneValue::Bar(v))] if n == "hp" && (v - 0.25).abs() < 0.04), "one value for both places: {first:?}");
    }

    #[test]
    fn thresholds_are_suggested_from_captures() {
        assert_eq!(suggest_threshold(&[0.9, 0.8], &[0.1, 0.3]), Some(0.55));
        assert_eq!(suggest_threshold(&[0.9, 0.2], &[0.1, 0.3]), None, "they overlap");
        assert_eq!(suggest_threshold(&[], &[0.1]), None);
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
            let got = measure(&zone, &bar(level)).unwrap();
            assert!((got - level).abs() < 0.04, "{level}: {got}");
        }
        let left = Zone { direction: Direction::Left, ..zone.clone() };
        assert!((measure(&left, &bar(0.5)).unwrap() - 0.5).abs() < 0.04, "the share filled does not depend on the side");

        // An empty part close to the full color (a darker red) is told apart once picked.
        let close = |level: f32| {
            frame(move |x, y| match (x, y) {
                (20..=99, 10..=15) if (x - 20) as f32 / 80.0 < level => [200, 30, 40],
                (20..=99, 10..=15) => [150, 40, 50],
                _ => [20, 20, 30],
            })
        };
        let empty = pick_color(&close(0.0), 0.5, 12.0 / 90.0);
        assert_eq!(empty, [150, 40, 50]);
        assert!(measure(&zone, &close(0.5)).unwrap() > 0.9, "without the empty color both look full");
        let picked = Zone { empty_color: Some(empty), ..zone.clone() };
        assert!((measure(&picked, &close(0.5)).unwrap() - 0.5).abs() < 0.04);

        let mut reader = ZoneReader::default();
        let zones = [zone];
        assert_eq!(reader.update(&zones, &bar(1.0)), vec![("hp".to_owned(), ZoneValue::Bar(1.0))]);
        assert_eq!(reader.update(&zones, &bar(0.99)), vec![], "small moves are not reported");
        assert_eq!(reader.update(&zones, &bar(0.5)), vec![("hp".to_owned(), ZoneValue::Bar(0.5))]);
    }
}
