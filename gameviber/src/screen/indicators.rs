//! Indicators of the game's screen set up for a mode (`package::Zone`), each
//! read in one or more zones: whether an element is shown (its look compared
//! with a reference taken when the zone was drawn; for an element whose inside
//! changes, a minimap, only on the parts of it that stay the same, learned
//! from captures) and how full a gauge's bar
//! is (the share of it in the bar's full color rather than its empty one;
//! with both colors known, the bar is the longest run of them in the zone,
//! wherever it is; a bar filled again over itself in other colors reads
//! each of them as a tier of its value). A bar whose colors
//! do not tell (gradients, segments, hearts) is read by its look instead: the
//! bar full, and empty, along its length, taken from captures. A curved bar
//! (an arc, a ring) is read along a path clicked on it rather than across a
//! rectangle (`Strip`).

use std::collections::BTreeMap;

use super::Frame;
use crate::mode::IndicatorValue;
use crate::package::{Condition, Direction, Zone, IndicatorKind};

/// References are compared on a grayscale grid of this size.
pub const REF_WIDTH: usize = 32;
pub const REF_HEIGHT: usize = 24;
/// A shown element is hidden once its similarity drops this far below the threshold.
const HYSTERESIS: f32 = 0.05;
/// A cell matches the reference less the further its luma is from it, not at all from this far.
const CELL_MATCH: f32 = 32.0;
/// Cells mattering less than this (0..1) are left out of a zone's weights.
const LEAST_WEIGHT: f32 = 0.2;
/// Captures of the zone's phase needed to learn its weights.
pub const WEIGHT_CAPTURES: usize = 3;
/// Bar values are reported when they move this much.
const BAR_STEP: f64 = 0.02;
/// A bar is not on screen when less than this share of its drawn length is
/// found, or more than `FOUND_MAX` (a background of its color, not the bar)...
const FOUND_SHARE: f32 = 0.6;
const FOUND_MAX: f32 = 1.5;
/// ...for this many copies in a row (0.3 s), so that a flash does not hide it.
const UNKNOWN_FRAMES: u32 = 3;
/// A bar's look is a grid of this many cells along it, by this many across.
pub const LOOK_LENGTH: usize = 96;
pub const LOOK_ACROSS: usize = 4;
/// A bar read by its look is not on screen when its columns are further from
/// the look they were given than this share of the tolerance, on average.
const LOOK_FOUND: f32 = 0.5;
/// Pixels a bar read by its look may have moved, each way.
const LOOK_SHIFT: i32 = 2;
/// A bar's higher tier counts once this share of it (or 2 columns) has its
/// color, so that a few stray columns do not make one.
const TIER_SHARE: f32 = 0.02;

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

/// How far each cell of `grid` is from the reference's, 0 (the same) to 1 (`CELL_MATCH` or more).
fn cell_distances<'a>(grid: &'a [f32], reference: &'a [u8]) -> impl Iterator<Item = f32> + 'a {
    grid.iter().zip(reference).map(|(g, &r)| ((g - r as f32).abs() / CELL_MATCH).min(1.0))
}

/// Which cells of a visibility zone tell whether its element is shown
/// (`Zone::weights`), from captures where it is (`shown`) and where it is
/// not (`hidden`): those staying near the reference while it is shown (its
/// frame, not a minimap's map turning inside) and away from it otherwise
/// (not a background of its color). Medians, so that a few captures filed
/// in the wrong phase do not count. None with too few captures, or when no
/// cell tells.
pub fn learn_weights(zone: &Zone, shown: &[&Frame], hidden: &[&Frame]) -> Option<Vec<u8>> {
    if zone.reference.len() != REF_WIDTH * REF_HEIGHT || shown.len() < WEIGHT_CAPTURES {
        return None;
    }
    let distances = |frames: &[&Frame]| -> Vec<Vec<f32>> {
        frames.iter().map(|f| cell_distances(&gray(f, zone.rect), &zone.reference).collect()).collect()
    };
    let median = |all: &[Vec<f32>], cell: usize| {
        let mut v: Vec<f32> = all.iter().map(|d| d[cell]).collect();
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let (shown, hidden) = (distances(shown), distances(hidden));
    let weights: Vec<u8> = (0..REF_WIDTH * REF_HEIGHT)
        .map(|cell| {
            // Without captures where it is hidden, any cell would differ there.
            let away = if hidden.is_empty() { 1.0 } else { median(&hidden, cell) };
            let w = (away - median(&shown, cell)).max(0.0);
            if w < LEAST_WEIGHT { 0 } else { (w * 255.0).round() as u8 }
        })
        .collect();
    weights.iter().any(|&w| w > 0).then_some(weights)
}

/// Whether the zone is compared on the cells its weights keep.
pub fn has_weights(zone: &Zone) -> bool {
    zone.weights.len() == REF_WIDTH * REF_HEIGHT
}

/// Share of a zone's cells its weights keep.
pub fn weighted_share(zone: &Zone) -> f32 {
    zone.weights.iter().filter(|&&w| w > 0).count() as f32 / (REF_WIDTH * REF_HEIGHT) as f32
}

/// The color of a bar's filled part: the average of its most colorful pixels.
pub fn bar_color(zone: &Zone, frame: &Frame) -> [u8; 3] {
    let strip = Strip::new(zone, frame, (0, 0));
    let mut pixels: Vec<([u8; 3], u8)> = Vec::new();
    for i in 0..strip.length {
        for j in 0..strip.across {
            let p = strip.at(i, j);
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

/// A threshold telling the zone's phase apart from the others, from its
/// similarity on captures of that phase (`shown`) and of the others
/// (`hidden`): halfway between them when they do not overlap.
pub fn suggest_threshold(shown: &[f32], hidden: &[f32]) -> Option<f32> {
    let low = shown.iter().copied().fold(f32::INFINITY, f32::min);
    let high = hidden.iter().copied().fold(-1.0, f32::max);
    (!shown.is_empty() && low > high).then(|| ((low + high) / 2.0).clamp(0.1, 0.95))
}

/// What a zone reads on `frame`: the similarity with its reference for a
/// visibility indicator's zone (-1..1; with weights, 0..1: how near the cells
/// they keep are, weighed), how full its bar is (0..1) for a gauge's; None when the bar is
/// not on screen (too little of its colors in the zone, which needs its empty
/// color to be told from an empty bar).
pub fn measure(zone: &Zone, frame: &Frame) -> Option<f32> {
    match zone.kind {
        IndicatorKind::Visibility => {
            if zone.reference.len() != REF_WIDTH * REF_HEIGHT {
                return Some(0.0);
            }
            let grid = gray(frame, zone.rect);
            if has_weights(zone) {
                let (mut near, mut total) = (0.0, 0.0);
                for (d, &w) in cell_distances(&grid, &zone.reference).zip(&zone.weights) {
                    near += (1.0 - d) * w as f32;
                    total += w as f32;
                }
                return Some(if total > 0.0 { near / total } else { 0.0 });
            }
            let reference: Vec<f32> = zone.reference.iter().map(|&v| v as f32).collect();
            Some(correlation(&grid, &reference))
        }
        IndicatorKind::Gauge => bar_reading(zone, frame).map(|(fill, _)| fill),
    }
}

/// How full a bar is, and how much of its drawn length was found (1 without
/// the empty color); None when too little was found.
fn bar_reading(zone: &Zone, frame: &Frame) -> Option<(f32, f32)> {
    if has_look(zone) {
        let (fill, cost, _) = look_fill(zone, frame);
        return (cost <= zone.tolerance * LOOK_FOUND).then_some((fill, 1.0 - cost / zone.tolerance));
    }
    let bar = bar(zone, frame);
    let expected = if zone.length > 0.0 { zone.length } else { 0.1 };
    let found = if zone.empty_color.is_some() { bar.length() / expected } else { 1.0 };
    let too_long = zone.length > 0.0 && found > FOUND_MAX;
    (found >= FOUND_SHARE && !too_long).then_some((bar.fill, found.min(1.0)))
}

/// An indicator read in several zones (the health bar in battle and out of
/// it) is shown when one of them shows it; `was_shown`
/// lowers the thresholds a little (hysteresis). Also returns the best similarity.
pub fn shown_anywhere(zones: &[&Zone], frame: &Frame, was_shown: bool) -> (bool, f32) {
    let margin = if was_shown { HYSTERESIS } else { 0.0 };
    zones.iter().fold((false, -1.0), |(shown, best), zone| {
        let m = measure(zone, frame).unwrap_or(0.0);
        (shown || m >= zone.threshold - margin, best.max(m))
    })
}

/// A gauge read in several zones reads where its bar is found (the most
/// complete when several find it); None when no zone finds it.
pub fn fill_anywhere(zones: &[&Zone], frame: &Frame) -> Option<f32> {
    zones
        .iter()
        .filter_map(|zone| bar_reading(zone, frame))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(fill, _)| fill)
}

/// The first of a gauge's conditions (`Zone::read_when`) that does not hold,
/// given whether each visibility indicator is shown (None: no such
/// indicator, its condition left out); None when they all hold.
pub fn unmet(zone: &Zone, shown: impl Fn(&str) -> Option<bool>) -> Option<&Condition> {
    zone.read_when.iter().find(|c| shown(&c.indicator).is_some_and(|s| s != c.shown))
}

/// Whether the visibility indicator `name` is shown on `frame` (None: no such indicator in `zones`).
pub fn shown_on(name: &str, zones: &[Zone], frame: &Frame) -> Option<bool> {
    let of: Vec<&Zone> = zones.iter().filter(|z| z.indicator == name && z.kind == IndicatorKind::Visibility).collect();
    (!of.is_empty()).then(|| shown_anywhere(&of, frame, false).0)
}

/// Share of the zone's length a bar covers on `frame` (to save with a zone being drawn).
pub fn bar_length(zone: &Zone, frame: &Frame) -> f32 {
    bar(zone, frame).length()
}

/// Where the bar was found in the zone, as fractions of the zone's length
/// along its axis (left to right, top to bottom; along its path from its
/// first point): its full part, then its empty part. For the editor.
pub fn bar_extent(zone: &Zone, frame: &Frame) -> ((f32, f32), (f32, f32)) {
    let bar = bar(zone, frame);
    let full = (bar.end - bar.start) * bar.shown;
    if reversed(zone) {
        ((bar.end - full, bar.end), (bar.start, bar.end - full))
    } else {
        ((bar.start, bar.start + full), (bar.start + full, bar.end))
    }
}

/// The zone is read along its path rather than across its rectangle.
pub fn has_path(zone: &Zone) -> bool {
    zone.path.len() >= 2
}

/// The bar fills from the end of its strip (a rectangle filling to the left or upwards).
fn reversed(zone: &Zone) -> bool {
    !has_path(zone) && matches!(zone.direction, Direction::Left | Direction::Up)
}

/// A zone's pixels as a strip along its bar: `length` steps, `across`
/// pixels each. A rectangle's steps are its columns, left to right (its rows,
/// top to bottom, for a bar filling up or down); a path's are along its line
/// from its first point, each across its thickness. `shift` moves it by whole pixels.
struct Strip<'a> {
    frame: &'a Frame,
    length: usize,
    across: usize,
    /// A path's pixels, `across` for each step; empty for a rectangle...
    points: Vec<(usize, usize)>,
    /// ...whose left top and axis are these.
    origin: (usize, usize),
    horizontal: bool,
}

impl<'a> Strip<'a> {
    fn new(zone: &Zone, frame: &'a Frame, shift: (i32, i32)) -> Self {
        let (w, h) = (frame.width as f32, frame.height as f32);
        if !has_path(zone) {
            let [x, y, width, height] = zone.rect;
            let (x0, y0, x1, y1) = bounds(frame, [x + shift.0 as f32 / w.max(1.0), y + shift.1 as f32 / h.max(1.0), width, height]);
            let horizontal = horizontal(zone.direction);
            let (length, across) = if horizontal { (x1 - x0, y1 - y0) } else { (y1 - y0, x1 - x0) };
            return Self { frame, length, across, points: Vec::new(), origin: (x0, y0), horizontal };
        }
        // The path in pixels, without points on top of each other.
        let mut line: Vec<(f32, f32)> = Vec::new();
        for p in &zone.path {
            let q = (p[0] * w + shift.0 as f32, p[1] * h + shift.1 as f32);
            if line.last().is_none_or(|l| (l.0 - q.0).hypot(l.1 - q.1) >= 0.5) {
                line.push(q);
            }
        }
        let segments: Vec<f32> = line.windows(2).map(|s| (s[1].0 - s[0].0).hypot(s[1].1 - s[0].1)).collect();
        let total: f32 = segments.iter().sum();
        let length = (total.round() as usize).max(2);
        let across = ((zone.thickness * h).round() as usize).max(1);
        let mut points = Vec::with_capacity(length * across);
        let (mut k, mut before) = (0, 0.0);
        for i in 0..length {
            let at = (i as f32 + 0.5) / length as f32 * total;
            while k + 1 < segments.len() && at > before + segments[k] {
                before += segments[k];
                k += 1;
            }
            let (a, b, len) = (line[k.min(line.len() - 1)], line[(k + 1).min(line.len() - 1)], segments.get(k).copied().unwrap_or(0.0));
            let (dx, dy) = if len > 0.0 { ((b.0 - a.0) / len, (b.1 - a.1) / len) } else { (1.0, 0.0) };
            let t = if len > 0.0 { ((at - before) / len).clamp(0.0, 1.0) } else { 0.0 };
            let (px, py) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            for j in 0..across {
                // Across the line, its middle on the path.
                let off = j as f32 - (across - 1) as f32 / 2.0;
                let (x, y) = (px - dy * off, py + dx * off);
                points.push((x.round().clamp(0.0, w - 1.0) as usize, y.round().clamp(0.0, h - 1.0) as usize));
            }
        }
        Self { frame, length, across, points, origin: (0, 0), horizontal: true }
    }

    /// The pixel of step `i`, `j` across.
    fn at(&self, i: usize, j: usize) -> [u8; 3] {
        if !self.points.is_empty() {
            let (x, y) = self.points[i * self.across + j];
            return pixel(self.frame, x, y);
        }
        let (x0, y0) = self.origin;
        if self.horizontal { pixel(self.frame, x0 + i, y0 + j) } else { pixel(self.frame, x0 + j, y0 + i) }
    }
}

/// A bar found in its zone.
struct Bar {
    /// Its value: how full it is, its tiers counted.
    fill: f32,
    /// The share of it in its highest tier's colors.
    shown: f32,
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
    /// In a full color, of this tier (0 without tiers).
    Full(usize),
    Empty,
    /// Neither color: outside the bar.
    Other,
}

/// The share of the bar whose middle line is in its full color, nearer to it
/// than to its empty color. With both colors, the bar is the longest run of
/// full and empty columns in the zone; with the full color only, the whole zone.
fn bar(zone: &Zone, frame: &Frame) -> Bar {
    if has_look(zone) {
        let fill = look_fill(zone, frame).0;
        return Bar { fill, shown: fill, start: 0.0, end: 1.0 };
    }
    let parts = columns(zone, frame);
    let len = parts.len().max(1) as f32;
    // Which end the bar fills from: its full part is read from there.
    let reversed = reversed(zone);
    if zone.empty_color.is_some() {
        // The longest run of bar columns, one stray column allowed inside.
        let (mut best, mut best_start) = (0, 0);
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
                    best_start = start;
                }
                start = i + 1;
                gap = 0;
            }
        }
        let (start, end) = (best_start as f32 / len, (best_start + best) as f32 / len);
        let (start, end) = if reversed { (1.0 - end, 1.0 - start) } else { (start, end) };
        let (fill, shown) = tiered_fill(zone, &parts[best_start..best_start + best]);
        return Bar { fill, shown, start, end };
    }
    let (fill, shown) = tiered_fill(zone, &parts);
    Bar { fill, shown, start: 0.0, end: 1.0 }
}

/// The value of a bar's columns, and the share of them in its highest tier:
/// its tiers below are full, that one fills as far as its color goes.
fn tiered_fill(zone: &Zone, parts: &[Part]) -> (f32, f32) {
    let len = parts.len().max(1) as f32;
    let at_least = |tier: usize| parts.iter().filter(|p| matches!(p, Part::Full(t) if *t >= tier)).count();
    let tiers = zone.tiers.iter().filter(|t| !t.is_empty()).count();
    let least = (len * TIER_SHARE).max(2.0);
    let top = (1..=tiers).rev().find(|&t| at_least(t) as f32 >= least).unwrap_or(0);
    let shown = at_least(top) as f32 / len;
    ((top as f32 + shown) / (tiers + 1) as f32, shown)
}

/// Each column along the bar. The bar's lines are found first: those with
/// the most pixels near its colors, so that text or icons in a zone drawn
/// larger than the bar stay out. A column is then full (or empty) when one of
/// its pixels on those lines is near the full color (bars are often shaded, a
/// lighter line over a darker one, and the color was picked on one), empty
/// when most of them are near the empty color.
fn columns(zone: &Zone, frame: &Frame) -> Vec<Part> {
    let strip = Strip::new(zone, frame, (0, 0));
    let (length, across) = (strip.length, strip.across);
    let at = |i: usize, j: usize| strip.at(i, j);
    let distance = |p: [u8; 3], color: [u8; 3]| (0..3).map(|c| (p[c] as f32 - color[c] as f32).powi(2)).sum::<f32>().sqrt();
    // Distances of a pixel to the nearest full and empty colors (a bar may
    // have several shades), and the tier of the full one.
    let nearest = |p: [u8; 3], colors: &mut dyn Iterator<Item = &[u8; 3]>| colors.map(|c| distance(p, *c)).fold(f32::INFINITY, f32::min);
    let full_colors: Vec<(usize, [u8; 3])> = std::iter::once(&zone.color)
        .chain(&zone.more_colors)
        .map(|c| (0, *c))
        .chain(zone.tiers.iter().filter(|t| !t.is_empty()).enumerate().flat_map(|(k, t)| t.iter().map(move |c| (k + 1, *c))))
        .collect();
    let distances = |p: [u8; 3]| {
        let (tier, full) = full_colors.iter().map(|&(t, c)| (t, distance(p, c))).fold((0, f32::INFINITY), |a, b| if b.1 < a.1 { b } else { a });
        let empty = match &zone.empty_color {
            Some(e) => nearest(p, &mut std::iter::once(e).chain(&zone.more_empty)),
            None => f32::INFINITY,
        };
        (full, empty, tier)
    };
    let near = |p: [u8; 3]| {
        let (full, empty, _) = distances(p);
        full.min(empty) <= zone.tolerance
    };
    let counts: Vec<usize> = (0..across).map(|j| (0..length).filter(|&i| near(at(i, j))).count()).collect();
    let most = counts.iter().copied().max().unwrap_or(0);
    let lines: Vec<usize> = (0..across).filter(|&j| most > 0 && counts[j] * 2 >= most).collect();
    let mut parts: Vec<Part> = (0..length)
        .map(|i| {
            let pixels: Vec<(f32, f32, usize)> = lines.iter().map(|&j| distances(at(i, j))).collect();
            // Full when one of the bar's lines has a full color (shaded bars), of
            // the tier nearest to its color...
            let full = pixels.iter().filter(|&&(f, e, _)| f <= zone.tolerance && f <= e).min_by(|a, b| a.0.total_cmp(&b.0));
            // ...empty only when most of them have the empty color: a line under
            // the bars in that color (a decoration) does not make one.
            let empty = pixels.iter().filter(|&&(f, e, _)| e <= zone.tolerance && e < f).count();
            if let Some(&(_, _, tier)) = full {
                Part::Full(tier)
            } else if empty > 0 && empty * 2 >= pixels.len() {
                Part::Empty
            } else {
                Part::Other
            }
        })
        .collect();
    if reversed(zone) {
        parts.reverse();
    }
    parts
}

/// The zone read by its look (its full look taken).
pub fn has_look(zone: &Zone) -> bool {
    zone.full_look.len() == LOOK_LENGTH * LOOK_ACROSS
}

fn horizontal(direction: Direction) -> bool {
    matches!(direction, Direction::Right | Direction::Left)
}

/// The zone's colors on the look grid: `LOOK_ACROSS` cells for each step
/// along its strip, each the average of its pixels.
fn look_cells(zone: &Zone, frame: &Frame, shift: (i32, i32)) -> Vec<[f32; 3]> {
    let strip = Strip::new(zone, frame, shift);
    let (length, across) = (strip.length, strip.across);
    // The pixels of cell `k` of `cells` over `n` pixels: at least one.
    let span = |k: usize, cells: usize, n: usize| {
        let start = (k * n / cells).min(n - 1);
        start..((k + 1) * n / cells).max(start + 1)
    };
    let mut out = Vec::with_capacity(LOOK_LENGTH * LOOK_ACROSS);
    for i in 0..LOOK_LENGTH {
        for j in 0..LOOK_ACROSS {
            let mut sum = [0f32; 3];
            let mut count = 0.0;
            for a in span(i, LOOK_LENGTH, length) {
                for b in span(j, LOOK_ACROSS, across) {
                    let p = strip.at(a, b);
                    (0..3).for_each(|c| sum[c] += p[c] as f32);
                    count += 1.0;
                }
            }
            out.push(sum.map(|s| s / count));
        }
    }
    out
}

/// The zone's look on `frame` (taken on a capture where the bar is full).
pub fn look(zone: &Zone, frame: &Frame) -> Vec<[u8; 3]> {
    look_at(zone, frame, (0, 0))
}

fn look_at(zone: &Zone, frame: &Frame, shift: (i32, i32)) -> Vec<[u8; 3]> {
    look_cells(zone, frame, shift).iter().map(|c| c.map(|v| v.round() as u8)).collect()
}

/// The zone's empty look with what `frame` shows of the bar empty added: the
/// cells past its full part (the column at the boundary left out).
pub fn add_empty_look(zone: &Zone, frame: &Frame) -> Vec<Option<[u8; 3]>> {
    let mut empty = zone.empty_look.clone();
    empty.resize(LOOK_LENGTH * LOOK_ACROSS, None);
    if !has_look(zone) {
        return empty;
    }
    let (fill, _, shift) = look_fill(zone, frame);
    let cells = look_at(zone, frame, shift);
    let full = (fill * LOOK_LENGTH as f32).round() as usize;
    let reversed = reversed(zone);
    for k in (if full > 0 { full + 1 } else { 0 })..LOOK_LENGTH {
        let i = if reversed { LOOK_LENGTH - 1 - k } else { k };
        for j in 0..LOOK_ACROSS {
            empty[i * LOOK_ACROSS + j] = Some(cells[i * LOOK_ACROSS + j]);
        }
    }
    empty
}

/// Share of the bar's columns its empty look covers.
pub fn empty_seen(zone: &Zone) -> f32 {
    zone.empty_look.iter().step_by(LOOK_ACROSS).filter(|c| c.is_some()).count() as f32 / LOOK_LENGTH as f32
}

/// A bar read by its look: its full part ends where the columns before
/// look most like the bar full and those after like the bar empty, so that
/// columns alike either way (the gaps between segments, an icon) do not
/// move it, nor a few columns hidden by an effect. A column not seen empty
/// yet counts as empty when further than the tolerance from full. Also
/// returns how far the columns are, on average, from the look they were
/// given (those not seen empty left out): too far (`LOOK_FOUND`), the bar
/// is not on screen.
fn look_fill(zone: &Zone, frame: &Frame) -> (f32, f32, (i32, i32)) {
    // The bar may have moved a pixel or two since its look was taken: read where it fits best (also returned).
    let shifts = (-LOOK_SHIFT..=LOOK_SHIFT).flat_map(|dx| (-LOOK_SHIFT..=LOOK_SHIFT).map(move |dy| (dx, dy)));
    shifts
        .map(|shift| (look_fill_at(zone, frame, shift), shift))
        .min_by(|a, b| a.0 .2.total_cmp(&b.0 .2))
        .map(|((fill, far, _), shift)| (fill, far, shift))
        .unwrap_or((0.0, f32::INFINITY, (0, 0)))
}

/// `look_fill` with the strip moved by `shift`, and what the boundary found costs (to compare places).
fn look_fill_at(zone: &Zone, frame: &Frame, shift: (i32, i32)) -> (f32, f32, f32) {
    let cells = look_cells(zone, frame, shift);
    let cap = 2.0 * zone.tolerance;
    let distance = |p: [f32; 3], c: [u8; 3]| (0..3).map(|k| (p[k] - c[k] as f32).powi(2)).sum::<f32>().sqrt();
    let mut costs: Vec<(f32, Option<f32>)> = (0..LOOK_LENGTH)
        .map(|i| {
            let column = i * LOOK_ACROSS..(i + 1) * LOOK_ACROSS;
            let mean = |look: &dyn Fn(usize) -> Option<[u8; 3]>| -> Option<f32> {
                let sum = column.clone().map(|k| look(k).map(|c| distance(cells[k], c))).sum::<Option<f32>>()?;
                Some((sum / LOOK_ACROSS as f32).min(cap))
            };
            (mean(&|k| zone.full_look.get(k).copied()).unwrap_or(cap), mean(&|k| zone.empty_look.get(k).copied().flatten()))
        })
        .collect();
    if reversed(zone) {
        costs.reverse();
    }
    // Full up to `k`, empty after: the `k` that costs least.
    let empty = |c: Option<f32>| c.unwrap_or(zone.tolerance);
    let mut before = 0.0;
    let mut after: f32 = costs.iter().map(|c| empty(c.1)).sum();
    let (mut best, mut full) = (after, 0);
    for (k, &(f, e)) in costs.iter().enumerate() {
        before += f;
        after -= empty(e);
        if before + after < best - 1e-3 {
            best = before + after;
            full = k + 1;
        }
    }
    let known: Vec<f32> = costs[..full].iter().map(|c| c.0).chain(costs[full..].iter().filter_map(|c| c.1)).collect();
    let far = known.iter().sum::<f32>() / known.len().max(1) as f32;
    (full as f32 / LOOK_LENGTH as f32, far, best)
}

/// Reads a mode's indicators frame after frame and tells which changed.
#[derive(Default)]
pub struct IndicatorReader {
    values: BTreeMap<String, IndicatorValue>,
    /// Raw measures of the last frame, for the GUI (None: bar not on screen).
    pub measures: BTreeMap<String, Option<f32>>,
    /// Copies in a row each bar was not found on.
    missing: BTreeMap<String, u32>,
}

impl IndicatorReader {
    pub fn update(&mut self, zones: &[Zone], frame: &Frame) -> Vec<(String, IndicatorValue)> {
        let mut changed = Vec::new();
        self.measures.clear();
        self.values.retain(|name, _| zones.iter().any(|z| z.indicator == *name));
        // Zones naming the same indicator are read together, visibility first:
        // gauges may be read only while one is shown, or hidden.
        let mut names: Vec<&str> = Vec::new();
        for zone in zones {
            if !names.contains(&zone.indicator.as_str()) {
                names.push(&zone.indicator);
            }
        }
        names.sort_by_key(|name| zones.iter().any(|z| z.indicator == *name && z.kind == IndicatorKind::Gauge));
        for name in names {
            let of_indicator: Vec<&Zone> = zones.iter().filter(|z| z.indicator == name).collect();
            let previous = self.values.get(name).copied();
            let (measure, value) = match of_indicator[0].kind {
                IndicatorKind::Visibility => {
                    let (shown, best) = shown_anywhere(&of_indicator, frame, previous == Some(IndicatorValue::Visibility(true)));
                    (Some(best), IndicatorValue::Visibility(shown))
                }
                IndicatorKind::Gauge => {
                    let shown = |n: &str| match self.values.get(n) {
                        Some(IndicatorValue::Visibility(v)) => Some(*v),
                        _ => None,
                    };
                    let measure = if unmet(of_indicator[0], shown).is_none() { fill_anywhere(&of_indicator, frame) } else { None };
                    let missing = self.missing.entry(name.to_owned()).or_default();
                    *missing = if measure.is_none() { *missing + 1 } else { 0 };
                    let value = match (measure, previous) {
                        // Briefly lost (a flash, an effect over it): keep the last value.
                        (None, Some(old)) if *missing < UNKNOWN_FRAMES => old,
                        (None, _) => IndicatorValue::Unknown,
                        (Some(m), Some(IndicatorValue::Gauge(old))) if (m as f64 - old).abs() < BAR_STEP => IndicatorValue::Gauge(old),
                        (Some(m), _) => IndicatorValue::Gauge((m as f64 * 100.0).round() / 100.0),
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

    pub fn values(&self) -> &BTreeMap<String, IndicatorValue> {
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
    fn visibility_follows_the_element() {
        let rect = [120.0 / 160.0, 60.0 / 90.0, 40.0 / 160.0, 30.0 / 90.0];
        let zone = Zone { indicator: "battle_hud".into(), rect, reference: reference(&with_hud(true, 0), rect), ..Zone::default() };
        assert!(measure(&zone, &with_hud(true, 0)).unwrap() > 0.99);
        // The scenery moved behind the element: still recognized; gone: not.
        let moved = measure(&zone, &with_hud(true, 37)).unwrap();
        let hidden = measure(&zone, &with_hud(false, 37)).unwrap();
        assert!(moved > zone.threshold, "{moved}");
        assert!(hidden < zone.threshold, "{hidden}");

        let mut reader = IndicatorReader::default();
        let zones = [zone];
        assert_eq!(reader.update(&zones, &with_hud(true, 0)), vec![("battle_hud".to_owned(), IndicatorValue::Visibility(true))]);
        assert_eq!(reader.update(&zones, &with_hud(true, 10)), vec![], "no change, nothing reported");
        assert_eq!(reader.update(&zones, &with_hud(false, 10)), vec![("battle_hud".to_owned(), IndicatorValue::Visibility(false))]);
    }

    /// A minimap: a ring whose inside, the map, changes all the time, over
    /// scenery that changes too.
    fn minimap(shown: bool, seed: u32) -> Frame {
        // Blocks of 4 px in colors changing with `seed`.
        let noise = move |x: u32, y: u32, salt: u32| {
            let mut h = (x / 4).wrapping_mul(73_856_093) ^ (y / 4).wrapping_mul(19_349_663) ^ seed.wrapping_mul(83_492_791) ^ salt;
            h ^= h >> 13;
            h = h.wrapping_mul(0x5bd1_e995);
            h ^= h >> 15;
            [(h & 0xff) as u8, ((h >> 8) & 0xff) as u8, ((h >> 16) & 0xff) as u8]
        };
        frame(move |x, y| {
            let (dx, dy) = (x as f32 - 140.0, y as f32 - 70.0);
            let r = (dx * dx + dy * dy).sqrt();
            match shown {
                true if r < 12.0 => noise(x, y, 1),
                true if r < 14.5 => [200, 190, 160],
                _ => noise(x, y, 2),
            }
        })
    }

    #[test]
    fn elements_whose_inside_changes_are_told_by_what_stays() {
        let rect = [124.0 / 160.0, 54.0 / 90.0, 32.0 / 160.0, 32.0 / 90.0];
        let zone = Zone { indicator: "minimap".into(), rect, reference: reference(&minimap(true, 0), rect), ..Zone::default() };
        let measures = |zone: &Zone, shown: bool, seeds: std::ops::Range<u32>| -> Vec<f32> {
            seeds.map(|seed| measure(zone, &minimap(shown, seed)).unwrap()).collect()
        };
        // Compared everywhere, the map drowns the ring.
        assert_eq!(suggest_threshold(&measures(&zone, true, 20..40), &measures(&zone, false, 20..40)), None);

        let shown: Vec<Frame> = (1..6).map(|seed| minimap(true, seed)).collect();
        let hidden: Vec<Frame> = (10..15).map(|seed| minimap(false, seed)).collect();
        let (shown, hidden): (Vec<&Frame>, Vec<&Frame>) = (shown.iter().collect(), hidden.iter().collect());
        assert_eq!(learn_weights(&zone, &shown[..2], &hidden), None, "too few captures");
        let weights = learn_weights(&zone, &shown, &hidden).unwrap();
        let learned = Zone { weights, ..zone.clone() };
        assert!(has_weights(&learned));
        // Only the ring is kept: a fraction of the zone.
        let share = weighted_share(&learned);
        assert!((0.1..0.4).contains(&share), "{share}");
        let (on, off) = (measures(&learned, true, 20..60), measures(&learned, false, 20..60));
        let threshold = suggest_threshold(&on, &off).expect("told apart once learned");
        let lowest = on.iter().copied().fold(f32::INFINITY, f32::min);
        let highest = off.iter().copied().fold(-1.0, f32::max);
        assert!(lowest - highest > 0.3, "shown from {lowest}, hidden up to {highest}, threshold {threshold}");
        // Even without captures where it is hidden.
        let alone = Zone { weights: learn_weights(&zone, &shown, &[]).unwrap(), ..zone };
        let (on, off) = (measures(&alone, true, 20..60), measures(&alone, false, 20..60));
        assert!(suggest_threshold(&on, &off).is_some());
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
            indicator: "hp".into(),
            kind: IndicatorKind::Gauge,
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

    /// Prince of Persia's Athra: green up to half its value, then yellow over
    /// the green, a lighter "MAX" over its end once full, on a dark track.
    #[test]
    fn bars_filled_again_in_another_color_read_as_tiers() {
        let (green, yellow, track) = ([40, 170, 130], [240, 200, 40], [20, 20, 45]);
        let bar = |green_to: f32, yellow_to: f32| {
            frame(move |x, y| match (x, y) {
                (60..=69, 9..=13) if yellow_to >= 1.0 => [255, 250, 220],
                (10..=69, 10..=15) => {
                    let at = (x - 10) as f32 / 60.0;
                    if at < yellow_to { yellow } else if at < green_to { green } else { track }
                }
                _ => [60, 50, 40],
            })
        };
        let rect = [5.0 / 160.0, 9.0 / 90.0, 70.0 / 160.0, 8.0 / 90.0];
        let zone = Zone {
            indicator: "athra".into(),
            kind: IndicatorKind::Gauge,
            rect,
            color: green,
            empty_color: Some(track),
            tiers: vec![vec![yellow]],
            tolerance: 40.0,
            ..Zone::default()
        };
        for (green_to, yellow_to, value) in [(0.0, 0.0, 0.0), (0.4, 0.0, 0.2), (1.0, 0.0, 0.5), (1.0, 0.5, 0.75), (1.0, 1.0, 1.0)] {
            let got = measure(&zone, &bar(green_to, yellow_to)).unwrap();
            assert!((got - value).abs() < 0.03, "green {green_to}, yellow {yellow_to}: {got}");
        }
        // Refilled in yellow over the empty track rather than the green: the same.
        assert!((measure(&zone, &bar(0.5, 0.5)).unwrap() - 0.75).abs() < 0.03);
        // The editor shows the highest tier's part as full.
        let ((start, full_end), _) = bar_extent(&zone, &bar(1.0, 0.5));
        assert!(((full_end - start) - 30.0 / 70.0).abs() < 0.03, "{start} {full_end}");
        // Without its tiers, the yellow is not the bar.
        let one = Zone { tiers: Vec::new(), ..zone };
        assert!((measure(&one, &bar(1.0, 0.0)).unwrap() - 1.0).abs() < 0.03);
    }

    /// A bar's empty color is dark, and so is the menu's background over it:
    /// read only while the menu's button is hidden, it is unknown in menus.
    #[test]
    fn gauges_are_read_only_under_their_conditions() {
        let (green, dark) = ([40, 170, 130], [20, 20, 25]);
        let game = |level: f32| {
            frame(move |x, y| match (x, y) {
                (10..=69, 10..=15) => if ((x - 10) as f32) < level * 60.0 { green } else { dark },
                _ => [60, 50, 40],
            })
        };
        let menu = frame(|x, y| match (x, y) {
            (140..=149, 75..=84) if (x + y) % 3 != 0 => [240, 240, 240],
            _ => dark,
        });
        let button = [135.0 / 160.0, 70.0 / 90.0, 20.0 / 160.0, 18.0 / 90.0];
        let rect = [10.0 / 160.0, 10.0 / 90.0, 60.0 / 160.0, 6.0 / 90.0];
        let hp = Zone { indicator: "hp".into(), kind: IndicatorKind::Gauge, rect, color: green, empty_color: Some(dark), tolerance: 30.0, ..Zone::default() };
        let hp = Zone { length: bar_length(&hp, &game(0.5)), ..hp };
        assert_eq!(measure(&hp, &menu), Some(0.0), "the menu looks like an empty bar");
        let in_menu = Zone { indicator: "menu".into(), rect: button, reference: reference(&menu, button), ..Zone::default() };
        let read_when = vec![Condition { indicator: "menu".into(), shown: false }];
        let zones = [Zone { read_when, ..hp }, in_menu];
        let mut reader = IndicatorReader::default();
        let first = reader.update(&zones, &game(0.5));
        assert!(first.contains(&("menu".to_owned(), IndicatorValue::Visibility(false))), "{first:?}");
        assert!(first.contains(&("hp".to_owned(), IndicatorValue::Gauge(0.5))), "{first:?}");
        reader.update(&zones, &menu);
        reader.update(&zones, &menu);
        let later = reader.update(&zones, &menu);
        assert!(later.contains(&("hp".to_owned(), IndicatorValue::Unknown)), "{later:?}");
        let back = reader.update(&zones, &game(0.25));
        assert!(back.contains(&("hp".to_owned(), IndicatorValue::Gauge(0.25))), "{back:?}");
        // A condition on an indicator no longer set up is left out.
        let alone = [zones[0].clone()];
        let mut reader = IndicatorReader::default();
        assert_eq!(reader.update(&alone, &menu), vec![("hp".to_owned(), IndicatorValue::Gauge(0.0))]);
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
            indicator: "hp".into(),
            kind: IndicatorKind::Gauge,
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
        let mut reader = IndicatorReader::default();
        let zones = [drawn];
        let first = reader.update(&zones, &bar(30, 0.5));
        assert!(matches!(first[..], [(_, IndicatorValue::Gauge(v))] if (v - 0.5).abs() < 0.02), "{first:?}");
        assert_eq!(reader.update(&zones, &menu), vec![], "briefly lost: the last value stays");
        reader.update(&zones, &menu);
        assert_eq!(reader.update(&zones, &menu), vec![("hp".to_owned(), IndicatorValue::Unknown)]);
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
                indicator: "hp".into(),
                kind: IndicatorKind::Gauge,
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
    fn an_indicator_read_in_two_zones_reads_where_it_is_found() {
        let bar_at = |top: u32, level: f32| {
            frame(move |x, y| match (x, y) {
                (20..=79, _) if (top..top + 6).contains(&y) => {
                    if ((x - 20) as f32) < level * 60.0 { [40, 200, 60] } else { [190, 30, 30] }
                }
                _ => [25, 25, 35],
            })
        };
        let zone_at = |top: u32| Zone {
            indicator: "hp".into(),
            kind: IndicatorKind::Gauge,
            rect: [15.0 / 160.0, (top as f32 - 1.0) / 90.0, 70.0 / 160.0, 8.0 / 90.0],
            color: [40, 200, 60],
            empty_color: Some([190, 30, 30]),
            length: 60.0 / 70.0,
            ..Zone::default()
        };
        let (battle, field) = (zone_at(10), zone_at(60));
        assert!((fill_anywhere(&[&battle, &field], &bar_at(10, 0.75)).unwrap() - 0.75).abs() < 0.04);
        assert!((fill_anywhere(&[&battle, &field], &bar_at(60, 0.25)).unwrap() - 0.25).abs() < 0.04);
        assert_eq!(fill_anywhere(&[&battle, &field], &frame(|_, _| [25, 25, 35])), None, "in no place: unknown");

        let mut reader = IndicatorReader::default();
        let zones = [battle, field];
        let first = reader.update(&zones, &bar_at(60, 0.25));
        assert!(matches!(first[..], [(ref n, IndicatorValue::Gauge(v))] if n == "hp" && (v - 0.25).abs() < 0.04), "one value for both places: {first:?}");
    }

    /// A bar in `rect` filled up to `level` from its left end (its right end: `from_right`).
    fn look_bar(
        rect: [f32; 4],
        from_right: bool,
        full: impl Fn(u32, u32) -> [u8; 3] + Copy,
        empty: impl Fn(u32, u32) -> [u8; 3] + Copy,
    ) -> impl Fn(f32) -> Frame {
        move |level| {
            frame(move |x, y| {
                let (x0, y0) = ((rect[0] * 160.0).round() as u32, (rect[1] * 90.0).round() as u32);
                let (x1, y1) = (x0 + (rect[2] * 160.0).round() as u32, y0 + (rect[3] * 90.0).round() as u32);
                if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
                    let along = if from_right { x1 - 1 - x } else { x - x0 };
                    if (along as f32) < level * (x1 - x0) as f32 { full(x, y) } else { empty(x, y) }
                } else {
                    [25, 25, 35]
                }
            })
        }
    }

    /// A bar from red to green along its length, in segments with gaps the
    /// color of the background: no color tells its full part.
    #[test]
    fn gradient_bars_in_segments_are_read_by_their_look() {
        let rect = [16.0 / 160.0, 10.0 / 90.0, 96.0 / 160.0, 6.0 / 90.0];
        let gap = |x: u32| x % 8 >= 6;
        let full = move |x: u32, _| if gap(x) { [25, 25, 35] } else { [(220 - (x - 16) * 2) as u8, (40 + (x - 16) * 2) as u8, 40] };
        let empty = move |x: u32, _| if gap(x) { [25, 25, 35] } else { [30, 40, 90] };
        let bar = look_bar(rect, false, full, empty);
        let mut zone = Zone { indicator: "hp".into(), kind: IndicatorKind::Gauge, rect, full_look: look(&Zone { rect, ..Zone::default() }, &bar(1.0)), ..Zone::default() };
        assert!(has_look(&zone));
        for level in [1.0, 0.75, 0.5, 0.25, 0.0] {
            let got = measure(&zone, &bar(level)).unwrap();
            assert!((got - level).abs() < 0.04, "full look only, {level}: {got}");
        }
        // Without the empty look a bar gone reads empty; with it, unknown.
        let menu = frame(|_, _| [25, 25, 35]);
        assert_eq!(measure(&zone, &menu), Some(0.0));
        zone.empty_look = add_empty_look(&zone, &bar(0.5));
        assert!((empty_seen(&zone) - 0.5).abs() < 0.03, "{}", empty_seen(&zone));
        zone.empty_look = add_empty_look(&zone, &bar(0.0));
        assert_eq!(empty_seen(&zone), 1.0);
        for level in [1.0, 0.6, 0.3, 0.0] {
            let got = measure(&zone, &bar(level)).unwrap();
            assert!((got - level).abs() < 0.04, "with the empty look, {level}: {got}");
        }
        assert_eq!(measure(&zone, &menu), None);
        // A few columns hidden by an effect do not move it.
        let lit = bar(0.75);
        let flash = frame(|x, y| if (40..46).contains(&x) { [255, 255, 255] } else { pixel(&lit, x as usize, y as usize) });
        assert!((measure(&zone, &flash).unwrap() - 0.75).abs() < 0.04);
    }

    /// Hearts: full ones red, empty ones a dark outline, a half heart at the boundary.
    #[test]
    fn hearts_are_read_by_their_look() {
        let rect = [10.0 / 160.0, 20.0 / 90.0, 100.0 / 160.0, 10.0 / 90.0];
        // Ten hearts of 10 x 10 px: a diamond inside each.
        let inside = |x: u32, y: u32| {
            let (dx, dy) = ((x - 10) % 10, y - 20);
            (dx as i32 - 5).abs() + (dy as i32 - 5).abs() < 5
        };
        let full = move |x, y| if inside(x, y) { [220, 30, 40] } else { [25, 25, 35] };
        let empty = move |x, y| if inside(x, y) { [50, 50, 50] } else { [25, 25, 35] };
        let bar = look_bar(rect, false, full, empty);
        let mut zone = Zone { indicator: "hp".into(), kind: IndicatorKind::Gauge, rect, full_look: look(&Zone { rect, ..Zone::default() }, &bar(1.0)), ..Zone::default() };
        // Without the empty look, the edges of an empty heart (mostly
        // background) look full too: the reading is within a heart.
        for level in [1.0, 0.75, 0.45, 0.1] {
            let got = measure(&zone, &bar(level)).unwrap();
            assert!((got - level).abs() < 0.1, "full look only, {level}: {got}");
        }
        zone.empty_look = add_empty_look(&zone, &bar(0.0));
        for level in [1.0, 0.75, 0.45, 0.1] {
            let got = measure(&zone, &bar(level)).unwrap();
            assert!((got - level).abs() < 0.04, "{level}: {got}");
        }
        // Filling to the left: the same hearts read from the other end.
        let left = Zone { direction: Direction::Left, ..zone.clone() };
        let got = measure(&left, &look_bar(rect, true, full, empty)(0.3)).unwrap();
        assert!((got - 0.3).abs() < 0.04, "{got}");
    }

    /// The Witcher 3's stamina: an arc 3 px thick, filling from its left end,
    /// over scenery that crosses the rectangle around it.
    #[test]
    fn curved_bars_are_read_along_their_path() {
        let (yellow, track) = ([230, 190, 40], [70, 30, 30]);
        // An arc of a circle around (80, 100), from 200° to 340°.
        let (from, to) = (200f32.to_radians(), 340f32.to_radians());
        let bar = |level: f32| {
            frame(move |x, y| {
                let (dx, dy) = (x as f32 - 80.0, y as f32 - 100.0);
                let mut angle = dy.atan2(dx);
                if angle < 0.0 {
                    angle += std::f32::consts::TAU;
                }
                if ((dx * dx + dy * dy).sqrt() - 60.0).abs() <= 1.5 && (from..=to).contains(&angle) {
                    if angle - from < level * (to - from) { yellow } else { track }
                } else if (x / 3 + y / 5) % 4 == 0 {
                    [225, 185, 45]
                } else {
                    [40, 50, 30]
                }
            })
        };
        let path: Vec<[f32; 2]> = (0..=14)
            .map(|k| {
                let a = from + (to - from) * k as f32 / 14.0;
                [(80.0 + 60.0 * a.cos()) / 160.0, (100.0 + 60.0 * a.sin()) / 90.0]
            })
            .collect();
        let zone = Zone {
            indicator: "stamina".into(),
            kind: IndicatorKind::Gauge,
            rect: [20.0 / 160.0, 35.0 / 90.0, 120.0 / 160.0, 50.0 / 90.0],
            path,
            thickness: 3.0 / 90.0,
            color: yellow,
            empty_color: Some(track),
            tolerance: 40.0,
            ..Zone::default()
        };
        assert!(has_path(&zone));
        let zone = Zone { length: bar_length(&zone, &bar(0.5)), ..zone };
        assert!(zone.length > 0.9, "{}", zone.length);
        for level in [1.0, 0.8, 0.5, 0.2, 0.0] {
            let got = measure(&zone, &bar(level)).unwrap();
            assert!((got - level).abs() < 0.05, "colors, {level}: {got}");
        }
        // Its middle line lost: unknown.
        assert_eq!(measure(&zone, &frame(|_, _| [20, 80, 120])), None);
        // Along the path, the scenery's yellow stripes are not the bar; across the rectangle they are.
        let across = Zone { path: Vec::new(), empty_color: None, ..zone.clone() };
        assert!(measure(&across, &bar(0.0)).unwrap() > 0.2);
        // Read by its look as well.
        let mut by_look = Zone { full_look: look(&zone, &bar(1.0)), empty_color: None, ..zone };
        by_look.empty_look = add_empty_look(&by_look, &bar(0.0));
        for level in [1.0, 0.7, 0.3, 0.0] {
            let got = measure(&by_look, &bar(level)).unwrap();
            assert!((got - level).abs() < 0.05, "look, {level}: {got}");
        }
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
        let color = bar_color(&Zone { kind: IndicatorKind::Gauge, rect, ..Zone::default() }, &bar(1.0));
        assert!(color[0] > 180 && color[1] < 60, "{color:?}");
        let zone = Zone { indicator: "hp".into(), kind: IndicatorKind::Gauge, rect, color, ..Zone::default() };
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

        let mut reader = IndicatorReader::default();
        let zones = [zone];
        assert_eq!(reader.update(&zones, &bar(1.0)), vec![("hp".to_owned(), IndicatorValue::Gauge(1.0))]);
        assert_eq!(reader.update(&zones, &bar(0.99)), vec![], "small moves are not reported");
        assert_eq!(reader.update(&zones, &bar(0.5)), vec![("hp".to_owned(), IndicatorValue::Gauge(0.5))]);
    }
}
