//! Motions for strokers: positions over time, from funscripts (the usual format of
//! scripted strokes, `{"actions": [{"at": ms, "pos": 0..100}]}`) in a mode's
//! package (`funscripts/<name>.funscript`) or written in its script (`motion {}`),
//! played by modes (docs/spec-modes.md §8.5). Strokers follow them as their
//! planner can (`stroke`); other toys feel how fast they move.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

pub const EXTENSION: &str = "funscript";
/// Points of a motion, at most.
pub const MAX_POINTS: usize = 20_000;
/// Bytes of a funscript file, at most.
pub const MAX_FILE_SIZE: u64 = 2 << 20;
/// Funscripts in a mode's package, at most.
pub const MAX_FUNSCRIPTS: usize = 32;
/// Speed other toys feel at full intensity, in full lengths per second.
const FULL_SPEED: f64 = 3.0;
/// Time the speed other toys feel is measured over, in seconds.
const SPEED_WINDOW: f64 = 0.25;

/// A mode's funscripts, by name.
pub type Funscripts = BTreeMap<String, Arc<Track>>;

/// Positions (0..1) at times (seconds), linearly between them.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Sorted by time, one per time.
    points: Vec<(f64, f64)>,
}

#[derive(Deserialize)]
struct File {
    actions: Vec<Action>,
    #[serde(default)]
    inverted: bool,
}

#[derive(Deserialize)]
struct Action {
    /// Milliseconds.
    at: f64,
    /// 0..100.
    pos: f64,
}

impl Track {
    /// From (time in s, position 0..1) points, in any order: positions are clamped,
    /// a time given twice keeps its last position.
    pub fn new(mut points: Vec<(f64, f64)>) -> Result<Self, String> {
        if points.iter().any(|(t, p)| !t.is_finite() || !p.is_finite() || *t < 0.0) {
            return Err("motion times must be numbers >= 0 and positions numbers".into());
        }
        if points.len() > MAX_POINTS {
            return Err(format!("a motion has at most {MAX_POINTS} points"));
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut kept: Vec<(f64, f64)> = Vec::with_capacity(points.len());
        for (t, p) in points {
            let p = p.clamp(0.0, 1.0);
            match kept.last_mut() {
                Some(last) if last.0 == t => last.1 = p,
                _ => kept.push((t, p)),
            }
        }
        if kept.len() < 2 {
            return Err("a motion needs at least two points at different times".into());
        }
        Ok(Self { points: kept })
    }

    /// A funscript file's content.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let file: File = serde_json::from_slice(bytes).map_err(|e| format!("not a funscript: {e}"))?;
        let points = file
            .actions
            .iter()
            .map(|a| (a.at / 1000.0, if file.inverted { 100.0 - a.pos } else { a.pos } / 100.0))
            .collect();
        Self::new(points)
    }

    /// Seconds from 0 to its last point.
    pub fn duration(&self) -> f64 {
        self.points.last().map_or(0.0, |p| p.0)
    }

    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }

    /// Position at `time` (s): its first one before it, its last one after.
    pub fn position(&self, time: f64) -> f64 {
        position_in(&self.points, time)
    }

    /// As a funscript file: times in whole milliseconds, positions in whole percents.
    pub fn to_json(&self) -> String {
        let actions: Vec<_> =
            self.points.iter().map(|&(t, p)| json!({ "at": (t * 1000.0).round() as u64, "pos": (p * 100.0).round() as u8 })).collect();
        let file = json!({ "version": "1.0", "inverted": false, "range": 100, "actions": actions });
        serde_json::to_string(&file).unwrap_or_default()
    }

    /// How fast it moves around `time`, as an intensity (0..1) for toys that do
    /// not move: the way travelled over the last moment.
    pub fn intensity(&self, time: f64) -> f64 {
        let start = (time - SPEED_WINDOW).max(0.0);
        if time <= start {
            return 0.0;
        }
        let first = self.points.partition_point(|p| p.0 <= start);
        let last = self.points.partition_point(|p| p.0 < time);
        let mut previous = self.position(start);
        let mut way = 0.0;
        for &(_, p) in &self.points[first..last] {
            way += (p - previous).abs();
            previous = p;
        }
        way += (self.position(time) - previous).abs();
        (way / (time - start) / FULL_SPEED).clamp(0.0, 1.0)
    }
}

/// A funscript's name in a package and in scripts: 1 to 48 of `a-z`, `0-9`, `_`, `-`.
pub fn valid_name(name: &str) -> bool {
    (1..=48).contains(&name.len()) && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A name for a file's funscript: lowercase, other characters as `_`, room left
/// for a number (`_2`).
pub fn name_of(stem: &str) -> String {
    let name: String = stem.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).take(44).collect();
    if name.is_empty() { "motion".into() } else { name }
}

/// Points of `points` (sorted) between `from` and `to`, with where the motion is
/// at both, moved to start at 0: a part of a motion as a motion of its own.
pub fn slice(points: &[(f64, f64)], from: f64, to: f64) -> Vec<(f64, f64)> {
    let (from, to) = (from.min(to), from.max(to));
    let at = |t: f64| position_in(points, t);
    let mut part = vec![(0.0, at(from))];
    part.extend(points.iter().filter(|p| p.0 > from && p.0 < to).map(|&(t, p)| (t - from, p)));
    part.push((to - from, at(to)));
    part
}

/// `points` without those between `from` and `to`; with `close`, the later ones
/// come that much earlier (the gap is cut out), else they stay where they are.
pub fn remove(points: &[(f64, f64)], from: f64, to: f64, close: bool) -> Vec<(f64, f64)> {
    let (from, to) = (from.min(to), from.max(to));
    points
        .iter()
        .filter(|p| p.0 < from || p.0 > to)
        .map(|&(t, p)| if close && t > to { (t - (to - from), p) } else { (t, p) })
        .collect()
}

/// `points` with `count` strokes between `low` and `high` from `at`, each half-stroke
/// lasting `half` seconds: those already there after `at` come later, by the strokes'
/// length and a half-stroke to get back to them.
pub fn insert_strokes(points: &[(f64, f64)], at: f64, count: usize, half: f64, low: f64, high: f64) -> Vec<(f64, f64)> {
    let half = half.max(0.01);
    let length = 2.0 * count as f64 * half;
    let mut out: Vec<(f64, f64)> = points.iter().filter(|p| p.0 < at).copied().collect();
    out.extend((0..=2 * count).map(|i| (at + i as f64 * half, if i % 2 == 0 { low } else { high })));
    out.extend(points.iter().filter(|p| p.0 >= at).map(|&(t, p)| (t + length + half, p)));
    out
}

/// Where the motion of `points` (sorted) is at `time`.
fn position_in(points: &[(f64, f64)], time: f64) -> f64 {
    let i = points.partition_point(|p| p.0 < time);
    match (i.checked_sub(1).map(|j| points[j]), points.get(i)) {
        (Some((t0, p0)), Some(&(t1, p1))) if t1 > t0 => p0 + (p1 - p0) * (time - t0) / (t1 - t0),
        (_, Some(&(_, p))) | (Some((_, p)), None) => p,
        (None, None) => 0.0,
    }
}

/// The funscripts of a package's directory (`funscripts/`), by file name; those
/// that cannot be read are left out (logged).
pub fn load_dir(dir: &Path) -> Funscripts {
    let mut funscripts = Funscripts::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return funscripts };
    let mut paths: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let Some(name) = path.file_stem().and_then(|n| n.to_str()) else { continue };
        if path.extension().is_none_or(|e| e != EXTENSION) {
            continue;
        }
        if funscripts.len() >= MAX_FUNSCRIPTS {
            log::warn!("{}: a mode has at most {MAX_FUNSCRIPTS} funscripts", dir.display());
            break;
        }
        match read(&path) {
            Ok(track) => {
                funscripts.insert(name.to_owned(), Arc::new(track));
            }
            Err(e) => log::warn!("leaving out {}: {e}", path.display()),
        }
    }
    funscripts
}

/// One funscript file.
pub fn read(path: &Path) -> Result<Track, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_FILE_SIZE {
        return Err(format!("larger than {} MB", MAX_FILE_SIZE >> 20));
    }
    Track::parse(&std::fs::read(path).map_err(|e| e.to_string())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_funscript() {
        let track = Track::parse(br#"{"version": "1.0", "inverted": false, "range": 90,
            "actions": [{"at": 500, "pos": 100}, {"at": 0, "pos": 0}, {"at": 1000, "pos": 50}, {"at": 1000, "pos": 20}]}"#)
        .unwrap();
        assert_eq!(track.points(), &[(0.0, 0.0), (0.5, 1.0), (1.0, 0.2)]);
        assert_eq!(track.duration(), 1.0);
        let inverted = Track::parse(br#"{"inverted": true, "actions": [{"at": 0, "pos": 10}, {"at": 100, "pos": 90}]}"#).unwrap();
        assert!((inverted.points()[0].1 - 0.9).abs() < 1e-9);
        assert!(Track::parse(b"{}").is_err());
        assert!(Track::parse(br#"{"actions": [{"at": 0, "pos": 10}]}"#).is_err());
        assert!(Track::parse(br#"{"actions": [{"at": -5, "pos": 10}, {"at": 100, "pos": 90}]}"#).is_err());
    }

    #[test]
    fn written_as_funscripts_read_back() {
        let track = Track::new(vec![(0.0, 0.0), (0.2504, 0.333), (1.0, 1.0)]).unwrap();
        let again = Track::parse(track.to_json().as_bytes()).unwrap();
        assert_eq!(again.points(), &[(0.0, 0.0), (0.25, 0.33), (1.0, 1.0)]);
    }

    #[test]
    fn parts_of_a_motion() {
        let points = [(0.0, 0.0), (1.0, 1.0), (2.0, 0.0), (3.0, 1.0)];
        // A part starts at 0, with where the motion is at both ends.
        assert_eq!(slice(&points, 2.5, 0.5), vec![(0.0, 0.5), (0.5, 1.0), (1.5, 0.0), (2.0, 0.5)]);
        assert_eq!(remove(&points, 0.5, 2.5, false), vec![(0.0, 0.0), (3.0, 1.0)]);
        assert_eq!(remove(&points, 0.5, 2.5, true), vec![(0.0, 0.0), (1.0, 1.0)]);
        let strokes = insert_strokes(&points[..2], 0.5, 2, 0.25, 0.1, 0.9);
        assert_eq!(strokes, vec![(0.0, 0.0), (0.5, 0.1), (0.75, 0.9), (1.0, 0.1), (1.25, 0.9), (1.5, 0.1), (2.25, 1.0)]);
        assert!(Track::new(strokes).is_ok());
        assert!(valid_name("boss_hit-2") && !valid_name("Boss") && !valid_name("") && !valid_name("a b"));
        assert_eq!(name_of("Boss Hit (v2)"), "boss_hit__v2_");
    }

    #[test]
    fn positions_and_felt_speed() {
        let track = Track::new(vec![(0.0, 0.0), (1.0, 1.0), (2.0, 1.0), (2.1, 0.0), (2.2, 1.0)]).unwrap();
        assert_eq!(track.position(-1.0), 0.0);
        assert!((track.position(0.25) - 0.25).abs() < 1e-9);
        assert_eq!(track.position(5.0), 1.0);
        // 1 length per second: a third of full intensity; still: nothing; a fast back and forth: all.
        assert!((track.intensity(0.6) - 1.0 / FULL_SPEED).abs() < 1e-9);
        assert_eq!(track.intensity(1.9), 0.0);
        assert_eq!(track.intensity(2.2), 1.0);
        assert_eq!(track.intensity(0.0), 0.0);
    }
}
