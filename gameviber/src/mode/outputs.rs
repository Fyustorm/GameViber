//! Output channels written by modes: a latched base level, temporary pulses
//! and keyframed patterns. A channel's value is the max of all three, and of
//! the speed of its strokes: strokes (`stroke()`, held), single ones
//! (`thrust()`, also a pulse) and motions (`play()` of a funscript) are for
//! strokers, other toys play their intensity.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::funscript::Track;
use crate::stroke::MotionDrive;

pub const ALL_CHANNELS: &str = "*";
/// Pulses and patterns going on at once, on all channels (docs/spec-modes.md §10).
const MAX_PULSES: usize = 64;
const MAX_PATTERNS: usize = 16;
const MAX_MOTIONS: usize = 16;

/// Strokes asked with `stroke()`, held until changed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeIntent {
    /// 0..1, from the toy's slowest to its fastest.
    pub speed: f64,
    /// 0..1, from its shortest strokes to its whole range.
    pub length: f64,
}

/// One stroke asked with `thrust()`, there and back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThrustIntent {
    /// 0..1, as `StrokeIntent::length`.
    pub length: f64,
    pub seconds: f64,
}

/// How a motion is played (`play()`'s options).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionOptions {
    /// 0 = loop forever.
    pub loops: u32,
    /// Playback speed: 2 plays it twice as fast.
    pub rate: f64,
    /// Share of the toy's range it uses (0..1), around `center` (0..1 of the range).
    pub depth: f64,
    pub center: f64,
}

impl Default for MotionOptions {
    fn default() -> Self {
        Self { loops: 1, rate: 1.0, depth: 1.0, center: 0.5 }
    }
}

#[derive(Debug, Clone)]
struct MotionPlay {
    id: u64,
    channel: String,
    track: Arc<Track>,
    start: f64,
    options: MotionOptions,
}

impl MotionPlay {
    /// Where it is in its own time, or None once finished.
    fn at(&self, time: f64) -> Option<f64> {
        let duration = self.track.duration();
        let elapsed = (time - self.start).max(0.0) * self.options.rate;
        if self.options.loops > 0 && elapsed >= duration * self.options.loops as f64 {
            return None;
        }
        Some(elapsed % duration)
    }
}

#[derive(Debug, Clone)]
struct Pulse {
    channel: String,
    level: f64,
    until: f64,
}

#[derive(Debug, Clone)]
struct PatternPlay {
    id: u64,
    channel: String,
    /// (time in s, intensity), sorted by time.
    points: Vec<(f64, f64)>,
    start: f64,
    /// 0 = loop forever.
    loops: u32,
    scale: f64,
}

impl PatternPlay {
    fn duration(&self) -> f64 {
        self.points.last().map_or(0.0, |p| p.0)
    }

    /// Value at `time`, or None once finished.
    fn value(&self, time: f64) -> Option<f64> {
        let duration = self.duration();
        let elapsed = (time - self.start).max(0.0);
        if duration <= 0.0 {
            return None;
        }
        if self.loops > 0 && elapsed >= duration * self.loops as f64 {
            return None;
        }
        Some(interpolate(&self.points, elapsed % duration) * self.scale)
    }
}

fn interpolate(points: &[(f64, f64)], t: f64) -> f64 {
    let Some(first) = points.first() else { return 0.0 };
    if t <= first.0 {
        return first.1;
    }
    for pair in points.windows(2) {
        let ((t0, v0), (t1, v1)) = (pair[0], pair[1]);
        if t <= t1 {
            return if t1 > t0 { v0 + (v1 - v0) * (t - t0) / (t1 - t0) } else { v1 };
        }
    }
    points.last().map_or(0.0, |p| p.1)
}

#[derive(Debug)]
pub struct Outputs {
    channels: Vec<String>,
    base: BTreeMap<String, f64>,
    pulses: Vec<Pulse>,
    patterns: Vec<PatternPlay>,
    next_id: u64,
    strokes: BTreeMap<String, StrokeIntent>,
    /// Asked since the last `take_thrusts`.
    thrusts: Vec<(String, ThrustIntent)>,
    /// One per channel at most: a new one replaces it.
    motions: Vec<MotionPlay>,
}

impl Outputs {
    pub fn new(channels: Vec<String>) -> Self {
        let base = channels.iter().map(|c| (c.clone(), 0.0)).collect();
        Self {
            channels,
            base,
            pulses: Vec::new(),
            patterns: Vec::new(),
            next_id: 1,
            strokes: BTreeMap::new(),
            thrusts: Vec::new(),
            motions: Vec::new(),
        }
    }

    /// Resolves a channel argument ("*" = every channel) or fails with a script-facing message.
    fn targets(&self, channel: &str) -> Result<Vec<String>, String> {
        if channel == ALL_CHANNELS {
            Ok(self.channels.clone())
        } else if self.base.contains_key(channel) {
            Ok(vec![channel.to_owned()])
        } else {
            Err(format!("unknown channel '{channel}' (declared: {})", self.channels.join(", ")))
        }
    }

    pub fn set(&mut self, channel: &str, level: f64) -> Result<(), String> {
        for c in self.targets(channel)? {
            self.base.insert(c, level.clamp(0.0, 1.0));
        }
        Ok(())
    }

    pub fn pulse(&mut self, channel: &str, level: f64, seconds: f64, now: f64) -> Result<(), String> {
        self.pulses.retain(|p| p.until > now);
        if self.pulses.len() >= MAX_PULSES {
            return Err(format!("pulse: at most {MAX_PULSES} at once"));
        }
        for c in self.targets(channel)? {
            self.pulses.push(Pulse { channel: c, level: level.clamp(0.0, 1.0), until: now + seconds.max(0.0) });
        }
        Ok(())
    }

    pub fn play(
        &mut self,
        channel: &str,
        mut points: Vec<(f64, f64)>,
        loops: u32,
        scale: f64,
        now: f64,
    ) -> Result<u64, String> {
        if points.is_empty() {
            return Err("pattern has no points".into());
        }
        if self.patterns.len() >= MAX_PATTERNS {
            return Err(format!("play: at most {MAX_PATTERNS} patterns at once"));
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        let id = self.next_id;
        self.next_id += 1;
        for c in self.targets(channel)? {
            let points = points.clone();
            self.patterns.push(PatternPlay { id, channel: c, points, start: now, loops, scale });
        }
        Ok(id)
    }

    /// Strokes held on the channel until changed; None goes back to its intensity.
    pub fn stroke(&mut self, channel: &str, stroke: Option<StrokeIntent>) -> Result<(), String> {
        for c in self.targets(channel)? {
            match stroke {
                Some(s) => {
                    let s = StrokeIntent { speed: s.speed.clamp(0.0, 1.0), length: s.length.clamp(0.0, 1.0) };
                    self.strokes.insert(c, s)
                }
                None => self.strokes.remove(&c),
            };
        }
        Ok(())
    }

    /// One stroke at once; other toys feel it as a pulse of its length.
    pub fn thrust(&mut self, channel: &str, length: f64, seconds: f64, now: f64) -> Result<(), String> {
        let thrust = ThrustIntent { length: length.clamp(0.0, 1.0), seconds: seconds.max(0.0) };
        self.pulse(channel, thrust.length, thrust.seconds, now)?;
        for c in self.targets(channel)? {
            self.thrusts.push((c, thrust));
        }
        Ok(())
    }

    /// Strokes held, by channel.
    pub fn strokes(&self) -> BTreeMap<String, StrokeIntent> {
        self.strokes.clone()
    }

    /// Single strokes asked since the last call.
    pub fn take_thrusts(&mut self) -> Vec<(String, ThrustIntent)> {
        std::mem::take(&mut self.thrusts)
    }

    /// Plays a motion on the channel, in place of the one it was playing.
    pub fn play_motion(&mut self, channel: &str, track: Arc<Track>, options: MotionOptions, now: f64) -> Result<u64, String> {
        let targets = self.targets(channel)?;
        self.motions.retain(|m| !targets.contains(&m.channel));
        if self.motions.len() + targets.len() > MAX_MOTIONS {
            return Err(format!("play: at most {MAX_MOTIONS} motions at once"));
        }
        let options = MotionOptions {
            loops: options.loops,
            rate: if options.rate.is_finite() { options.rate.clamp(0.1, 10.0) } else { 1.0 },
            depth: options.depth.clamp(0.0, 1.0),
            center: options.center.clamp(0.0, 1.0),
        };
        let id = self.next_id;
        self.next_id += 1;
        for c in targets {
            self.motions.push(MotionPlay { id, channel: c, track: track.clone(), start: now, options });
        }
        Ok(id)
    }

    /// The motions playing at `now`, by channel.
    pub fn motions(&self, now: f64) -> BTreeMap<String, MotionDrive> {
        self.motions
            .iter()
            .filter_map(|m| {
                let at = m.at(now)?;
                let MotionOptions { rate, depth, center, .. } = m.options;
                Some((m.channel.clone(), MotionDrive { id: m.id, track: m.track.clone(), at, rate, depth, center }))
            })
            .collect()
    }

    /// Stops a pattern or a motion.
    pub fn stop_pattern(&mut self, id: u64) {
        self.patterns.retain(|p| p.id != id);
        self.motions.retain(|m| m.id != id);
    }

    pub fn stop_all(&mut self) {
        self.base.values_mut().for_each(|v| *v = 0.0);
        self.pulses.clear();
        self.patterns.clear();
        self.strokes.clear();
        self.thrusts.clear();
        self.motions.clear();
    }

    /// Final value per channel at `now`; drops finished pulses and patterns.
    pub fn evaluate(&mut self, now: f64) -> BTreeMap<String, f64> {
        self.pulses.retain(|p| p.until > now);
        let mut values = self.base.clone();
        for (c, s) in &self.strokes {
            let v = values.entry(c.clone()).or_default();
            *v = v.max(s.speed);
        }
        for p in &self.pulses {
            let v = values.entry(p.channel.clone()).or_default();
            *v = v.max(p.level);
        }
        self.patterns.retain(|p| {
            let Some(level) = p.value(now) else { return false };
            let v = values.entry(p.channel.clone()).or_default();
            *v = v.max(level.clamp(0.0, 1.0));
            true
        });
        // Other toys feel how fast a motion moves.
        self.motions.retain(|m| {
            let Some(at) = m.at(now) else { return false };
            let v = values.entry(m.channel.clone()).or_default();
            *v = v.max(m.track.intensity(at) * m.options.rate * m.options.depth);
            true
        });
        values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outputs() -> Outputs {
        Outputs::new(vec!["main".into(), "aux".into()])
    }

    #[test]
    fn set_is_latched_and_clamped() {
        let mut o = outputs();
        o.set("main", 1.5).unwrap();
        assert_eq!(o.evaluate(0.0)["main"], 1.0);
        assert_eq!(o.evaluate(10.0)["main"], 1.0);
        assert_eq!(o.evaluate(10.0)["aux"], 0.0);
    }

    #[test]
    fn star_targets_every_channel_and_unknown_fails() {
        let mut o = outputs();
        o.set("*", 0.3).unwrap();
        let v = o.evaluate(0.0);
        assert_eq!((v["main"], v["aux"]), (0.3, 0.3));
        assert!(o.set("nope", 0.1).unwrap_err().contains("unknown channel"));
    }

    #[test]
    fn pulse_overrides_base_until_expiry() {
        let mut o = outputs();
        o.set("main", 0.2).unwrap();
        o.pulse("main", 0.9, 0.5, 1.0).unwrap();
        assert_eq!(o.evaluate(1.2)["main"], 0.9);
        assert_eq!(o.evaluate(1.5)["main"], 0.2);
    }

    #[test]
    fn pattern_interpolates_loops_and_stops() {
        let mut o = outputs();
        let id = o.play("main", vec![(0.0, 0.0), (1.0, 1.0)], 2, 0.5, 0.0).unwrap();
        assert!((o.evaluate(0.5)["main"] - 0.25).abs() < 1e-9);
        assert!((o.evaluate(1.5)["main"] - 0.25).abs() < 1e-9);
        assert_eq!(o.evaluate(2.0)["main"], 0.0);
        let id2 = o.play("main", vec![(0.0, 1.0), (1.0, 1.0)], 0, 1.0, 0.0).unwrap();
        assert_ne!(id, id2);
        assert_eq!(o.evaluate(100.0)["main"], 1.0);
        o.stop_pattern(id2);
        assert_eq!(o.evaluate(100.0)["main"], 0.0);
    }

    #[test]
    fn stop_all_clears_everything() {
        let mut o = outputs();
        o.set("main", 0.5).unwrap();
        o.pulse("aux", 1.0, 10.0, 0.0).unwrap();
        o.stroke("main", Some(StrokeIntent { speed: 0.7, length: 1.0 })).unwrap();
        o.thrust("aux", 1.0, 0.5, 0.0).unwrap();
        o.stop_all();
        let v = o.evaluate(1.0);
        assert_eq!((v["main"], v["aux"]), (0.0, 0.0));
        assert!(o.strokes().is_empty() && o.take_thrusts().is_empty());
    }

    #[test]
    fn strokes_are_held_and_felt_as_their_speed() {
        let mut o = outputs();
        o.set("main", 0.2).unwrap();
        o.stroke("*", Some(StrokeIntent { speed: 1.5, length: 0.4 })).unwrap();
        assert_eq!(o.evaluate(5.0)["aux"], 1.0);
        assert_eq!(o.strokes()["main"], StrokeIntent { speed: 1.0, length: 0.4 });
        o.stroke("main", None).unwrap();
        assert_eq!(o.evaluate(5.0)["main"], 0.2);
        assert!(o.stroke("nope", None).is_err());
    }

    #[test]
    fn motions_play_replace_each_other_and_end() {
        let mut o = outputs();
        let track = Arc::new(Track::new(vec![(0.0, 0.0), (1.0, 1.0)]).unwrap());
        let first = o.play_motion("main", track.clone(), MotionOptions::default(), 0.0).unwrap();
        let options = MotionOptions { loops: 2, rate: 2.0, depth: 0.5, ..Default::default() };
        let second = o.play_motion("main", track.clone(), options, 1.0).unwrap();
        assert_ne!(first, second);
        let motions = o.motions(1.25);
        assert_eq!((motions.len(), motions["main"].id), (1, second));
        assert!((motions["main"].at - 0.5).abs() < 1e-9);
        // Felt by other toys: 1 length per second, twice as fast, half as deep.
        let felt = o.evaluate(1.4)["main"];
        assert!((felt - 1.0 / 3.0).abs() < 1e-9, "{felt}");
        // Looped once (1.5 s), over after the second loop (2 s).
        assert!((o.motions(1.75)["main"].at - 0.5).abs() < 1e-9);
        assert_eq!(o.evaluate(2.0)["main"], 0.0);
        assert!(o.motions(2.0).is_empty());
        let third = o.play_motion("*", track, MotionOptions::default(), 3.0).unwrap();
        o.stop_pattern(third);
        assert!(o.motions(3.1).is_empty());
    }

    #[test]
    fn a_thrust_is_taken_once_and_felt_as_a_pulse() {
        let mut o = outputs();
        o.thrust("main", 0.8, 0.4, 1.0).unwrap();
        assert_eq!(o.evaluate(1.2)["main"], 0.8);
        assert_eq!(o.evaluate(1.5)["main"], 0.0);
        assert_eq!(o.take_thrusts(), vec![("main".to_owned(), ThrustIntent { length: 0.8, seconds: 0.4 })]);
        assert!(o.take_thrusts().is_empty());
    }
}
