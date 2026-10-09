//! Strokers: toys that move to a position rather than vibrate (Intiface's
//! `HwPositionWithDuration`, "go to x in d ms", or `Position`). The planner turns
//! the 0..1 intensity a toy is asked for into half-strokes it can follow, as set
//! in its `StrokeSettings`: faster and longer strokes as the intensity rises.
//!
//! GameViber never knows where a stroker really is. Positions are absolute, so an
//! error does not add up, and as long as the toy is never asked for more than it
//! can do, it is where it was sent at the end of every half-stroke. Otherwise
//! (first move, a move interrupted) where it may be is kept as an interval, and
//! the next move is timed for the farthest point of it.

use serde::{Deserialize, Serialize};

/// Requested intensities below this stop the toy (as `ToySettings::SILENT`).
const SILENT: f64 = 0.01;
/// Shortest stroke at the lowest intensity, as a share of the range (Depth, Both).
const MIN_AMPLITUDE: f64 = 0.25;
/// Speed of a move from an unknown position, in full lengths per second (at most
/// the toy's fastest): the toy may be anywhere, so it gets there gently.
const APPROACH_SPEED: f64 = 0.5;
/// A rise of the intensity this large turns the toy before the end of its
/// half-stroke rather than at it.
const RETARGET: f64 = 0.25;
/// Closer positions are the same.
const EPSILON: f64 = 1e-3;

/// What the intensity changes in the strokes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrokeStyle {
    /// Faster strokes, always over the whole range.
    Speed,
    /// Longer strokes, at a medium speed.
    Depth,
    /// Faster and longer strokes.
    #[default]
    Both,
}

impl StrokeStyle {
    pub const ALL: [StrokeStyle; 3] = [StrokeStyle::Both, StrokeStyle::Speed, StrokeStyle::Depth];

    pub fn label(self) -> &'static str {
        match self {
            StrokeStyle::Speed => "Speed",
            StrokeStyle::Depth => "Length",
            StrokeStyle::Both => "Both",
        }
    }
}

/// What a stroker can do and what the player wants of it (safety layer). Positions
/// are 0..1 of the toy's whole length, times are in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StrokeSettings {
    /// Lowest position the toy goes to.
    pub bottom: f64,
    /// Highest position the toy goes to.
    pub top: f64,
    /// Time of the fastest move over the toy's whole length: faster ones are not asked.
    pub fastest: f64,
    /// Time of the slowest move over the toy's whole length: some toys jerk below it.
    pub slowest: f64,
    /// Shortest time between two changes of direction.
    pub min_turn: f64,
    pub style: StrokeStyle,
}

impl Default for StrokeSettings {
    /// Careful values most strokers follow (The Handy, Kiiroo Keon, OSR2).
    fn default() -> Self {
        Self { bottom: 0.0, top: 1.0, fastest: 0.6, slowest: 4.0, min_turn: 0.25, style: StrokeStyle::Both }
    }
}

impl StrokeSettings {
    pub const FASTEST_RANGE: std::ops::RangeInclusive<f64> = 0.2..=3.0;
    pub const SLOWEST_RANGE: std::ops::RangeInclusive<f64> = 1.0..=20.0;
    pub const TURN_RANGE: std::ops::RangeInclusive<f64> = 0.1..=1.5;

    /// Within their ranges, `bottom` <= `top` and `fastest` <= `slowest`.
    fn sanitized(&self) -> Self {
        let clamp = |v: f64, r: &std::ops::RangeInclusive<f64>, fallback: f64| {
            if v.is_finite() { v.clamp(*r.start(), *r.end()) } else { fallback }
        };
        let defaults = Self::default();
        let bottom = clamp(self.bottom, &(0.0..=1.0), defaults.bottom);
        let top = clamp(self.top, &(0.0..=1.0), defaults.top).max(bottom);
        let fastest = clamp(self.fastest, &Self::FASTEST_RANGE, defaults.fastest);
        let slowest = clamp(self.slowest, &Self::SLOWEST_RANGE, defaults.slowest).max(fastest);
        let min_turn = clamp(self.min_turn, &Self::TURN_RANGE, defaults.min_turn);
        Self { bottom, top, fastest, slowest, min_turn, style: self.style }
    }

    /// Positions the strokes go between, and their speed (full lengths per second),
    /// for a 0..1 intensity.
    fn strokes(&self, level: f64) -> (f64, f64, f64) {
        let level = level.clamp(0.0, 1.0);
        let (fast, slow) = (1.0 / self.fastest, 1.0 / self.slowest);
        let (amplitude, speed) = match self.style {
            StrokeStyle::Speed => (1.0, slow + (fast - slow) * level),
            StrokeStyle::Depth => (MIN_AMPLITUDE + (1.0 - MIN_AMPLITUDE) * level, (slow + fast) / 2.0),
            StrokeStyle::Both => (MIN_AMPLITUDE + (1.0 - MIN_AMPLITUDE) * level, slow + (fast - slow) * level),
        };
        let center = (self.bottom + self.top) / 2.0;
        let half = (self.top - self.bottom) * amplitude / 2.0;
        (center - half, center + half, speed)
    }
}

/// What to send to a stroker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Motion {
    /// Go to `position` (0..1) in `ms` milliseconds.
    Move { position: f64, ms: u32 },
    /// Stop where it is.
    Stop,
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    /// Where the toy may have been when it was sent.
    from: (f64, f64),
    to: f64,
    start: f64,
    end: f64,
}

impl Segment {
    /// Where the toy may be at `time`: there once the move's time is over (it was
    /// never asked more than it can do), else on its way from anywhere it may have
    /// started from, late but never early.
    fn bounds(&self, time: f64) -> (f64, f64) {
        if time >= self.end {
            return (self.to, self.to);
        }
        let done = ((time - self.start) / (self.end - self.start)).clamp(0.0, 1.0);
        let (a, b) = self.from;
        let (a2, b2) = (a + (self.to - a) * done, b + (self.to - b) * done);
        (a.min(b).min(a2).min(b2), a.max(b).max(a2).max(b2))
    }
}

/// One stroker's moves, fed the toy's intensity on every send period.
#[derive(Debug, Clone)]
pub struct Planner {
    /// Where the toy may be when no move is going on (one point once a move ended).
    at: (f64, f64),
    moving: Option<Segment>,
    /// When the last move was sent.
    last_move: f64,
    /// Intensity the current move was planned for.
    level: f64,
    /// The last move went up.
    up: bool,
}

impl Default for Planner {
    /// The toy may be anywhere.
    fn default() -> Self {
        Self { at: (0.0, 1.0), moving: None, last_move: f64::NEG_INFINITY, level: 0.0, up: false }
    }
}

impl Planner {
    /// What to send at `time` (seconds, monotonic) for the toy's 0..1 intensity, if anything.
    pub fn tick(&mut self, time: f64, level: f64, settings: &StrokeSettings) -> Option<Motion> {
        let settings = settings.sanitized();
        if level.is_nan() || level < SILENT {
            let segment = self.moving.take()?;
            self.at = segment.bounds(time);
            return (time < segment.end).then_some(Motion::Stop);
        }
        if time - self.last_move < settings.min_turn {
            return None;
        }
        if let Some(segment) = self.moving {
            if time < segment.end && level - self.level < RETARGET {
                return None;
            }
            self.at = segment.bounds(time);
            self.moving = None;
        }
        let (low, high, speed) = settings.strokes(level);
        let (a, b) = self.at;
        let known = b - a < EPSILON;
        // Back the other way, unless the toy is already past that end.
        let here = (a + b) / 2.0;
        let to = if (self.up && here > low + EPSILON) || here >= high - EPSILON { low } else { high };
        let distance = (to - a).abs().max((to - b).abs());
        if distance < EPSILON {
            return None;
        }
        let speed = if known { speed } else { speed.min(APPROACH_SPEED) };
        let seconds = (distance / speed).max(settings.min_turn);
        let ms = (seconds * 1000.0).ceil();
        self.moving = Some(Segment { from: self.at, to, start: time, end: time + ms / 1000.0 });
        self.last_move = time;
        self.level = level;
        self.up = to > here;
        Some(Motion::Move { position: to, ms: ms as u32 })
    }

    /// Where the toy should be at `time`, for toys sent positions to go to at once
    /// (`Position`) rather than with a duration.
    pub fn position(&self, time: f64) -> f64 {
        match self.moving {
            Some(s) => {
                let done = ((time - s.start) / (s.end - s.start)).clamp(0.0, 1.0);
                let from = (s.from.0 + s.from.1) / 2.0;
                from + (s.to - from) * done
            }
            None => (self.at.0 + self.at.1) / 2.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERIOD: f64 = 0.05;

    /// A toy that does what it is told: from where it is to the position in the time.
    struct Toy {
        position: f64,
        moving: Option<(f64, f64, f64, f64)>,
    }

    impl Toy {
        fn at(&self, time: f64) -> f64 {
            match self.moving {
                Some((from, to, start, end)) => from + (to - from) * ((time - start) / (end - start)).clamp(0.0, 1.0),
                None => self.position,
            }
        }

        fn apply(&mut self, time: f64, motion: Motion) {
            self.position = self.at(time);
            self.moving = match motion {
                Motion::Move { position, ms } => Some((self.position, position, time, time + ms as f64 / 1000.0)),
                Motion::Stop => None,
            };
        }
    }

    /// Small deterministic random numbers.
    struct Random(u64);

    impl Random {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    fn run(settings: &StrokeSettings, levels: impl Fn(f64) -> f64, seconds: f64, start: f64) -> Vec<(f64, f64, Motion)> {
        let mut planner = Planner::default();
        let mut toy = Toy { position: start, moving: None };
        let mut sent = Vec::new();
        let mut time = 0.0;
        while time < seconds {
            if let Some(motion) = planner.tick(time, levels(time), settings) {
                sent.push((time, toy.at(time), motion));
                toy.apply(time, motion);
            }
            time += PERIOD;
        }
        sent
    }

    #[test]
    fn never_asks_more_than_the_toy_can_do() {
        let mut random = Random(0x9e37_79b9_7f4a_7c15);
        for case in 0..200 {
            let settings = StrokeSettings {
                bottom: random.next() * 0.4,
                top: 0.6 + random.next() * 0.4,
                fastest: 0.2 + random.next(),
                slowest: 2.0 + random.next() * 6.0,
                min_turn: 0.1 + random.next() * 0.4,
                style: StrokeStyle::ALL[case % 3],
            };
            // Intensities jumping around, with silences.
            let steps: Vec<f64> = (0..40).map(|_| if random.next() < 0.2 { 0.0 } else { random.next() }).collect();
            let start = random.next();
            let sent = run(&settings, |t| steps[(t / 0.7) as usize % steps.len()], 28.0, start);
            assert!(sent.len() > 20, "case {case}: {} moves", sent.len());
            let mut last_move = f64::NEG_INFINITY;
            for &(time, position, motion) in &sent {
                let Motion::Move { position: to, ms } = motion else { continue };
                assert!(to >= settings.bottom - 1e-9 && to <= settings.top + 1e-9, "case {case}: {to} out of range");
                let speed = (to - position).abs() / (ms as f64 / 1000.0);
                assert!(speed <= 1.0 / settings.fastest + 1e-6, "case {case}: {speed}/s at {time}");
                assert!(time - last_move >= settings.min_turn - 1e-9, "case {case}: turned after {}", time - last_move);
                last_move = time;
            }
        }
    }

    #[test]
    fn first_move_is_gentle_wherever_the_toy_is() {
        let settings = StrokeSettings { fastest: 0.2, ..Default::default() };
        for start in [0.0, 0.5, 1.0] {
            let sent = run(&settings, |_| 1.0, 1.0, start);
            let Some(&(_, _, Motion::Move { ms, .. })) = sent.first() else { panic!("no move") };
            // Timed for the farthest end, whatever the toy's real distance.
            assert!(ms as f64 >= 1000.0 / APPROACH_SPEED, "{ms} ms from {start}");
        }
    }

    #[test]
    fn stronger_means_faster_and_longer() {
        let settings = StrokeSettings::default();
        let strokes = |level: f64| {
            let sent = run(&settings, |_| level, 20.0, 0.5);
            // Once on its way, the half-strokes repeat.
            let &(_, from, Motion::Move { position, ms }) = &sent[sent.len() - 1] else { panic!() };
            ((position - from).abs(), ms)
        };
        let (soft_length, soft_ms) = strokes(0.1);
        let (hard_length, hard_ms) = strokes(1.0);
        assert!(hard_length > soft_length * 2.0, "{soft_length} vs {hard_length}");
        assert!((hard_length / hard_ms as f64) > (soft_length / soft_ms as f64) * 3.0);
        assert!((hard_length - 1.0).abs() < 1e-6);
        assert_eq!(hard_ms, 600);
    }

    #[test]
    fn silence_stops_a_moving_toy_and_stays_quiet() {
        let settings = StrokeSettings::default();
        let sent = run(&settings, |t| if t < 3.0 { 0.5 } else { 0.0 }, 6.0, 0.5);
        let after: Vec<_> = sent.iter().filter(|(t, ..)| *t >= 3.0).collect();
        assert!(after.len() <= 1, "{after:?}");
        assert!(after.iter().all(|(.., m)| *m == Motion::Stop));
        // Nothing while silent from the start.
        assert!(run(&settings, |_| 0.0, 3.0, 0.5).is_empty());
    }

    #[test]
    fn a_strong_rise_turns_before_the_end_of_a_slow_stroke() {
        let settings = StrokeSettings { slowest: 10.0, ..Default::default() };
        let sent = run(&settings, |t| if t < 8.0 { 0.02 } else { 1.0 }, 9.0, 0.5);
        let first_after = sent.iter().find(|(t, ..)| *t >= 8.0).expect("a move after the rise");
        assert!(first_after.0 < 8.0 + settings.min_turn + PERIOD, "{first_after:?}");
    }

    #[test]
    fn a_toy_stopped_halfway_is_moved_gently() {
        let settings = StrokeSettings { fastest: 0.2, min_turn: 0.1, ..Default::default() };
        let mut planner = Planner::default();
        // Known at the bottom, then sent up and stopped halfway.
        planner.at = (0.0, 0.0);
        let Some(Motion::Move { ms, .. }) = planner.tick(0.0, 1.0, &settings) else { panic!() };
        assert_eq!(ms, 200);
        assert_eq!(planner.tick(0.1, 0.0, &settings), Some(Motion::Stop));
        let Some(Motion::Move { position, ms }) = planner.tick(1.0, 1.0, &settings) else { panic!() };
        // Somewhere between 0 and 0.5: timed for the far end of that.
        assert_eq!(position, 0.0);
        assert!(ms as f64 >= 500.0 / APPROACH_SPEED - 1.0, "{ms}");
    }

    #[test]
    fn settings_out_of_range_are_made_safe() {
        let s = StrokeSettings { bottom: 0.8, top: 0.2, fastest: 0.0, slowest: f64::NAN, min_turn: 9.0, ..Default::default() };
        let s = s.sanitized();
        assert_eq!((s.bottom, s.top), (0.8, 0.8));
        assert_eq!(s.fastest, *StrokeSettings::FASTEST_RANGE.start());
        assert_eq!(s.slowest, StrokeSettings::default().slowest);
        assert_eq!(s.min_turn, *StrokeSettings::TURN_RANGE.end());
        // A range of nothing does not move.
        assert!(run(&s, |_| 1.0, 3.0, 0.5).iter().all(|(.., m)| matches!(m, Motion::Move { position, .. } if *position == 0.8)));
    }
}
