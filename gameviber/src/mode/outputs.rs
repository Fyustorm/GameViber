//! Output channels written by modes: a latched base level, temporary pulses
//! and keyframed patterns. A channel's value is the max of all three.

use std::collections::BTreeMap;

pub const ALL_CHANNELS: &str = "*";

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
}

impl Outputs {
    pub fn new(channels: Vec<String>) -> Self {
        let base = channels.iter().map(|c| (c.clone(), 0.0)).collect();
        Self { channels, base, pulses: Vec::new(), patterns: Vec::new(), next_id: 1 }
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
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        let id = self.next_id;
        self.next_id += 1;
        for c in self.targets(channel)? {
            let points = points.clone();
            self.patterns.push(PatternPlay { id, channel: c, points, start: now, loops, scale });
        }
        Ok(id)
    }

    pub fn stop_pattern(&mut self, id: u64) {
        self.patterns.retain(|p| p.id != id);
    }

    pub fn stop_all(&mut self) {
        self.base.values_mut().for_each(|v| *v = 0.0);
        self.pulses.clear();
        self.patterns.clear();
    }

    /// Final value per channel at `now`; drops finished pulses and patterns.
    pub fn evaluate(&mut self, now: f64) -> BTreeMap<String, f64> {
        self.pulses.retain(|p| p.until > now);
        let mut values = self.base.clone();
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
        o.stop_all();
        let v = o.evaluate(1.0);
        assert_eq!((v["main"], v["aux"]), (0.0, 0.0));
    }
}
