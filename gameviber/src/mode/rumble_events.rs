//! Turns the per-tick rumble level into discrete events for modes:
//! changes, vibration start, vibration end (with peak and duration).

pub const DEFAULT_THRESHOLD: f64 = 0.05;
pub const DEFAULT_RELEASE: f64 = 0.08;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RumbleLevels {
    pub strong: f64,
    pub weak: f64,
}

impl RumbleLevels {
    pub fn level(&self) -> f64 {
        self.strong.max(self.weak)
    }

    pub fn avg(&self) -> f64 {
        (self.strong + self.weak) / 2.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RumbleEvent {
    Changed(RumbleLevels),
    Start(RumbleLevels),
    End { peak: f64, duration: f64 },
}

#[derive(Debug)]
pub struct RumbleTracker {
    threshold: f64,
    release: f64,
    last: RumbleLevels,
    active: Option<Vibration>,
    last_end: f64,
}

#[derive(Debug)]
struct Vibration {
    start: f64,
    peak: f64,
    below_since: Option<f64>,
}

impl RumbleTracker {
    pub fn new(threshold: f64, release: f64, time: f64) -> Self {
        Self { threshold, release, last: RumbleLevels::default(), active: None, last_end: time }
    }

    pub fn active(&self) -> bool {
        self.active.is_some()
    }

    /// Seconds since the last vibration ended (0 while one is active).
    pub fn idle(&self, time: f64) -> f64 {
        if self.active.is_some() {
            0.0
        } else {
            (time - self.last_end).max(0.0)
        }
    }

    pub fn update(&mut self, levels: RumbleLevels, time: f64) -> Vec<RumbleEvent> {
        let mut events = Vec::new();
        if levels != self.last {
            events.push(RumbleEvent::Changed(levels));
            self.last = levels;
        }
        let level = levels.level();
        match &mut self.active {
            None if level > self.threshold => {
                self.active = Some(Vibration { start: time, peak: level, below_since: None });
                events.push(RumbleEvent::Start(levels));
            }
            None => {}
            Some(v) => {
                v.peak = v.peak.max(level);
                if level > self.threshold {
                    v.below_since = None;
                } else {
                    let since = *v.below_since.get_or_insert(time);
                    if time - since >= self.release {
                        events.push(RumbleEvent::End { peak: v.peak, duration: since - v.start });
                        self.last_end = since;
                        self.active = None;
                    }
                }
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lv(strong: f64) -> RumbleLevels {
        RumbleLevels { strong, weak: 0.0 }
    }

    #[test]
    fn start_and_end_with_release_delay() {
        let mut t = RumbleTracker::new(0.05, 0.08, 0.0);
        let ev = t.update(lv(0.5), 1.0);
        assert_eq!(ev, vec![RumbleEvent::Changed(lv(0.5)), RumbleEvent::Start(lv(0.5))]);
        t.update(lv(0.8), 1.1);
        t.update(lv(0.0), 1.2);
        assert!(t.active());
        let ev = t.update(lv(0.0), 1.3);
        assert_eq!(ev.len(), 1);
        match &ev[0] {
            RumbleEvent::End { peak, duration } => {
                assert_eq!(*peak, 0.8);
                assert!((duration - 0.2).abs() < 1e-9);
            }
            other => panic!("{other:?}"),
        }
        assert!((t.idle(1.5) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn short_gap_does_not_split_vibration() {
        let mut t = RumbleTracker::new(0.05, 0.08, 0.0);
        t.update(lv(0.5), 0.0);
        t.update(lv(0.0), 0.02);
        let ev = t.update(lv(0.5), 0.04);
        assert!(!ev.iter().any(|e| matches!(e, RumbleEvent::Start(_) | RumbleEvent::End { .. })));
        assert!(t.active());
    }

    #[test]
    fn below_threshold_is_not_a_vibration() {
        let mut t = RumbleTracker::new(0.1, 0.08, 0.0);
        let ev = t.update(lv(0.05), 0.0);
        assert_eq!(ev, vec![RumbleEvent::Changed(lv(0.05))]);
        assert!(!t.active());
    }
}
