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
//!
//! Motions (funscripts) are followed point by point: each move ends when the
//! motion gets to its point, the next one sent at the send period closest to
//! that; a turn sooner after the last than the toy can turn is skipped, and a
//! move too far for the time is shortened (the rhythm is kept).
//!
//! What a toy can do is found with the player (`Calibration`): it goes where the
//! player puts its range, then strokes faster, turns more often and strokes
//! slower in steps, until the player says it stopped following.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::funscript::Track;

/// Requested intensities below this stop the toy (as `ToySettings::SILENT`).
const SILENT: f64 = 0.01;
/// Shortest stroke at the lowest intensity, as a share of the range (Depth, Both).
const MIN_AMPLITUDE: f64 = 0.25;
/// Speed of a move from an unknown position, in full lengths per second (at most
/// the toy's fastest): the toy may be anywhere, so it gets there gently.
const APPROACH_SPEED: f64 = 1.0;
/// Half the output's send period (`intiface::SEND_PERIOD`): a motion's next move is
/// sent that early at most, so that it leaves on time rather than up to a period late.
const LEAD: f64 = 0.025;
/// A motion's point closer than this (s) is passed: it is too late to go there.
const MIN_MOVE: f64 = 0.02;
/// A rise of the intensity this large turns the toy before the end of its
/// half-stroke rather than at it.
const RETARGET: f64 = 0.25;
/// Closer positions are the same.
const EPSILON: f64 = 1e-3;
/// Shortest step of a calibration ramp, in seconds: a few strokes to watch.
const RAMP_STEP: f64 = 3.0;
/// Time players take to see the toy stop following and press the button, in
/// seconds: the result is the step tried that long before.
const REACTION: f64 = 1.0;

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

    /// What a motion played over this toy's whole range (depth 1) may do.
    pub fn motion_limits(&self) -> MotionLimits {
        let s = self.sanitized();
        // A motion's positions span the range: it moves that much less than the toy's length.
        let range = (s.top - s.bottom).max(EPSILON);
        MotionLimits { slowest: 1.0 / s.slowest / range, fastest: 1.0 / s.fastest / range, min_turn: s.min_turn }
    }

    /// Positions the strokes go between, and their speed (full lengths per second),
    /// for a 0..1 intensity, and a 0..1 length when another channel sets it (the
    /// intensity then sets the speed).
    fn strokes(&self, level: f64, length: Option<f64>) -> (f64, f64, f64) {
        let level = level.clamp(0.0, 1.0);
        let (fast, slow) = (1.0 / self.fastest, 1.0 / self.slowest);
        let (length, speed) = match (length.filter(|l| !l.is_nan()), self.style) {
            (Some(length), _) => (length, slow + (fast - slow) * level),
            (None, StrokeStyle::Speed) => (1.0, slow + (fast - slow) * level),
            (None, StrokeStyle::Depth) => (level, (slow + fast) / 2.0),
            (None, StrokeStyle::Both) => (level, slow + (fast - slow) * level),
        };
        let (low, high) = self.window(length);
        (low, high, speed)
    }

    /// Positions strokes of a 0..1 length go between: from the shortest ones to
    /// the whole range, in its middle.
    fn window(&self, length: f64) -> (f64, f64) {
        let amplitude = MIN_AMPLITUDE + (1.0 - MIN_AMPLITUDE) * length.clamp(0.0, 1.0);
        let center = (self.bottom + self.top) / 2.0;
        let half = (self.top - self.bottom) * amplitude / 2.0;
        (center - half, center + half)
    }
}

/// How fast a motion's positions (0..1 of the range) may move for a toy, per
/// second: slower moves jerk, faster ones are shortened (`Planner::follow`);
/// and how soon after a turn it can turn again (closer points are skipped).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionLimits {
    pub slowest: f64,
    pub fastest: f64,
    pub min_turn: f64,
}

impl MotionLimits {
    /// What suits both toys.
    pub fn strictest(self, other: Self) -> Self {
        Self { slowest: self.slowest.max(other.slowest), fastest: self.fastest.min(other.fastest), min_turn: self.min_turn.max(other.min_turn) }
    }
}

/// What a calibration step makes a stroker do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Calibration {
    /// Stay still (between steps: the mode does not drive the toy).
    Pause,
    /// Go to this position gently and stay there.
    Hold(f64),
    Ramp(Ramp),
}

/// Strokes tried one step after the other, until the player says the toy stopped
/// following, for one of its `StrokeSettings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ramp {
    /// Strokes over the range, faster and faster: `fastest`.
    Fastest,
    /// Short strokes, turning more and more often: `min_turn`.
    Turns,
    /// Strokes over the range, slower and slower: `slowest`.
    Slowest,
}

impl Ramp {
    #[cfg(test)]
    const ALL: [Ramp; 3] = [Ramp::Fastest, Ramp::Turns, Ramp::Slowest];

    /// First value tried, factor from one step to the next, last value (seconds).
    fn steps(self) -> (f64, f64, f64) {
        match self {
            Ramp::Fastest => (1.2, 0.88, *StrokeSettings::FASTEST_RANGE.start()),
            Ramp::Turns => (0.6, 0.85, *StrokeSettings::TURN_RANGE.start()),
            Ramp::Slowest => (2.0, 1.4, *StrokeSettings::SLOWEST_RANGE.end()),
        }
    }

    /// How long a value is tried: a few strokes, or one slow half-stroke.
    fn step_length(self, value: f64) -> f64 {
        match self {
            Ramp::Slowest => value.max(RAMP_STEP),
            Ramp::Fastest | Ramp::Turns => RAMP_STEP,
        }
    }

    /// Every value tried, with when it starts.
    fn schedule(self) -> Vec<(f64, f64)> {
        let (first, factor, last) = self.steps();
        let past = |v: f64| if factor < 1.0 { v < last } else { v > last };
        let mut schedule = Vec::new();
        let (mut start, mut value) = (0.0, first);
        loop {
            schedule.push((start, value));
            if value == last {
                return schedule;
            }
            start += self.step_length(value);
            value *= factor;
            if past(value) {
                value = last;
            }
        }
    }

    /// The value tried `elapsed` seconds after the start, None once the ramp is over.
    pub fn value(self, elapsed: f64) -> Option<f64> {
        if elapsed >= self.length() {
            return None;
        }
        self.schedule().into_iter().take_while(|(start, _)| *start <= elapsed).last().map(|(_, v)| v)
    }

    /// Seconds from the first step to the end of the last.
    pub fn length(self) -> f64 {
        let schedule = self.schedule();
        let (start, value) = schedule[schedule.len() - 1];
        start + self.step_length(value)
    }

    /// The setting to keep when the player says the toy stopped following
    /// `elapsed` seconds after the start (infinite: it followed every step): what
    /// was tried a reaction time before, with a margin.
    pub fn result(self, elapsed: f64) -> f64 {
        let (_, _, last) = self.steps();
        let tried = self.value((elapsed - REACTION).max(0.0)).unwrap_or(last);
        let (value, range) = match self {
            Ramp::Fastest => (tried * 1.15, StrokeSettings::FASTEST_RANGE),
            Ramp::Turns => (tried * 1.15, StrokeSettings::TURN_RANGE),
            Ramp::Slowest => (tried / 1.2, StrokeSettings::SLOWEST_RANGE),
        };
        value.clamp(*range.start(), *range.end())
    }
}

/// What a stroker is asked for on a send period.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drive {
    /// Intensity, 0..1: the speed of its strokes, and their length unless `length` says.
    pub level: f64,
    /// Length of its strokes, 0..1, when a channel or the mode sets it.
    pub length: Option<f64>,
    /// The last single stroke asked (`thrust()`), played once.
    pub thrust: Option<Thrust>,
    /// A motion playing: it takes the place of the strokes, whatever the intensity.
    pub motion: Option<MotionDrive>,
}

/// A motion playing (`play()` of a funscript), where it is now.
#[derive(Debug, Clone)]
pub struct MotionDrive {
    /// Tells this play from the others.
    pub id: u64,
    pub track: Arc<Track>,
    /// Where the motion is, in its own time (seconds, within one loop).
    pub at: f64,
    /// How fast it plays.
    pub rate: f64,
    /// Share of the range it uses (0..1), around `center` (0..1 of the range).
    pub depth: f64,
    pub center: f64,
}

impl PartialEq for MotionDrive {
    fn eq(&self, other: &Self) -> bool {
        (self.id, self.at, self.rate, self.depth, self.center) == (other.id, other.at, other.rate, other.depth, other.center)
            && Arc::ptr_eq(&self.track, &other.track)
    }
}

/// A motion being followed: its id, and the time (its own) of the point the last
/// move went to.
#[derive(Debug, Clone, Copy)]
struct Following {
    id: u64,
    sent_to: f64,
}

/// One stroke there and back, at once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thrust {
    /// Tells this one from the previous ones.
    pub id: u64,
    /// 0..1, as `Drive::length`.
    pub length: f64,
    /// For the whole stroke (never faster than the toy's fastest).
    pub seconds: f64,
}

/// A thrust being played: its window, its speed and the half-strokes left.
#[derive(Debug, Clone, Copy)]
struct ThrustPlay {
    window: (f64, f64),
    speed: f64,
    left: u8,
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
    /// When the toy last turned back.
    last_turn: f64,
    /// Intensity the current move was planned for.
    level: f64,
    /// The last move went up.
    up: bool,
    thrust: Option<ThrustPlay>,
    /// Id of the last thrust played.
    last_thrust: u64,
    following: Option<Following>,
}

impl Default for Planner {
    /// The toy may be anywhere.
    fn default() -> Self {
        Self { at: (0.0, 1.0), moving: None, last_move: f64::NEG_INFINITY, last_turn: f64::NEG_INFINITY, level: 0.0, up: false, thrust: None, last_thrust: 0, following: None }
    }
}

impl Planner {
    /// What to send at `time` (seconds, monotonic) for what the toy is asked, if anything.
    pub fn tick(&mut self, time: f64, drive: &Drive, settings: &StrokeSettings) -> Option<Motion> {
        let settings = settings.sanitized();
        let Drive { level, length, thrust, ref motion } = *drive;
        if let Some(motion) = motion {
            // Thrusts asked meanwhile are not played after it.
            if let Some(thrust) = thrust {
                self.last_thrust = thrust.id;
            }
            self.thrust = None;
            return self.follow(time, motion, &settings);
        }
        self.following = None;
        if level.is_nan() || level < SILENT {
            self.thrust = None;
            return self.stop(time);
        }
        if let Some(thrust) = thrust.filter(|t| t.id != self.last_thrust) {
            self.last_thrust = thrust.id;
            let window = settings.window(thrust.length);
            let speed = ((window.1 - window.0) / (thrust.seconds / 2.0).max(1e-3)).min(1.0 / settings.fastest);
            self.thrust = (window.1 - window.0 >= EPSILON).then_some(ThrustPlay { window, speed, left: 2 });
        }
        if let Some(play) = self.thrust {
            if play.left > 0 {
                // The first half-stroke cuts the current one short.
                let motion = self.plan(time, play.window, play.speed, settings.min_turn, play.left == 2);
                if motion.is_some() {
                    self.thrust = Some(ThrustPlay { left: play.left - 1, ..play });
                }
                return motion;
            }
            if self.moving.is_some_and(|m| time < m.end) {
                return None;
            }
            self.thrust = None;
        }
        let (low, high, speed) = settings.strokes(level, length);
        let motion = self.plan(time, (low, high), speed, settings.min_turn, level - self.level >= RETARGET);
        if motion.is_some() {
            self.level = level;
        }
        motion
    }

    /// What to send at `time` for a calibration step, `elapsed` seconds after it
    /// started. The settings being found are not obeyed, only the range and the
    /// widest limits (`StrokeSettings`' ranges); turns use the fastest found.
    pub fn calibrate(&mut self, time: f64, test: Calibration, elapsed: f64, settings: &StrokeSettings) -> Option<Motion> {
        let settings = settings.sanitized();
        let shortest_turn = *StrokeSettings::TURN_RANGE.start();
        let fastest = 1.0 / *StrokeSettings::FASTEST_RANGE.start();
        let (low, high) = (settings.bottom, settings.top);
        let ramp = match test {
            Calibration::Pause => return self.stop(time),
            Calibration::Hold(position) => {
                let position = position.clamp(0.0, 1.0);
                let moved = self.moving.is_some_and(|m| (m.to - position).abs() >= EPSILON);
                let speed = APPROACH_SPEED.min(1.0 / settings.fastest);
                return self.plan(time, (position, position), speed, shortest_turn, moved);
            }
            Calibration::Ramp(ramp) => ramp,
        };
        let Some(value) = ramp.value(elapsed) else { return self.stop(time) };
        match ramp {
            Ramp::Fastest => self.plan(time, (low, high), (1.0 / value).min(fastest), shortest_turn, false),
            Ramp::Slowest => self.plan(time, (low, high), 1.0 / value, shortest_turn, false),
            Ramp::Turns => {
                // As long as the toy can go in a turn's time, at most half the range.
                let length = ((high - low) / 2.0).min(value / settings.fastest);
                let center = (low + high) / 2.0;
                let window = (center - length / 2.0, center + length / 2.0);
                self.plan(time, window, length / value.max(shortest_turn), shortest_turn, false)
            }
        }
    }

    /// Stops a moving toy where it is.
    fn stop(&mut self, time: f64) -> Option<Motion> {
        let segment = self.moving.take()?;
        self.at = segment.bounds(time);
        (time < segment.end).then_some(Motion::Stop)
    }

    /// The next half-stroke between `low` and `high` at `speed` (full lengths per
    /// second), once the current one is over or at once if `interrupt`, never
    /// sooner than `min_turn` after the last one.
    fn plan(&mut self, time: f64, (low, high): (f64, f64), speed: f64, min_turn: f64, interrupt: bool) -> Option<Motion> {
        if time - self.last_move < min_turn {
            return None;
        }
        if let Some(segment) = self.moving {
            if time < segment.end && !interrupt {
                return None;
            }
            self.at = segment.bounds(time);
            self.moving = None;
        }
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
        Some(self.send(time, to, (distance / speed).max(min_turn)))
    }

    /// The next move of a motion: to its first point at least `min_turn` ahead, timed
    /// to get there with it, as far as the toy can go in that time.
    fn follow(&mut self, time: f64, motion: &MotionDrive, settings: &StrokeSettings) -> Option<Motion> {
        let mut following = match self.following {
            Some(f) if f.id == motion.id => f,
            _ => Following { id: motion.id, sent_to: f64::NEG_INFINITY },
        };
        let duration = motion.track.duration();
        // Its time went back: it looped.
        if following.sent_to > motion.at + duration / 2.0 {
            following.sent_to = f64::NEG_INFINITY;
        }
        self.following = Some(following);
        // A move of the motion's goes to its end, the next one sent at the send period
        // closest to it; the first one cuts the strokes short.
        let started = following.sent_to > f64::NEG_INFINITY;
        if self.moving.is_some_and(|m| time < m.end - LEAD) && started {
            return None;
        }
        if let Some(segment) = self.moving.take() {
            // About to get there from where it was known to be: there.
            let known = segment.from.1 - segment.from.0 < EPSILON;
            self.at = if known && time >= segment.end - LEAD { (segment.to, segment.to) } else { segment.bounds(time) };
        }
        let (a, b) = self.at;
        let here = (a + b) / 2.0;
        let fastest = 1.0 / settings.fastest;
        let rate = motion.rate.max(0.05);
        let points = motion.track.points();
        let map = |position: f64| {
            settings.bottom + (settings.top - settings.bottom) * (motion.center + (position - 0.5) * motion.depth).clamp(0.0, 1.0)
        };
        // The next point not passed yet; a turn sooner than the toy can turn is skipped.
        let mut next = points.partition_point(|p| p.0 <= motion.at.max(following.sent_to) + MIN_MOVE * rate);
        let (point, to) = loop {
            let &(point, position) = points.get(next)?;
            let to = map(position);
            let turns = self.last_move > f64::NEG_INFINITY && (to - here).abs() >= EPSILON && (to > here) != self.up;
            if !(turns && time - self.last_turn < settings.min_turn) {
                break (point, to);
            }
            next += 1;
        };
        let seconds = (point - motion.at) / rate;
        let (to, seconds) = if b - a < EPSILON {
            let reach = fastest * seconds;
            (a + (to - a).clamp(-reach, reach), seconds)
        } else {
            // Where it might be anywhere (never moved yet): there gently; else as fast as
            // it can from the farthest it might be. The motion goes on meanwhile.
            let distance = (to - a).abs().max((to - b).abs());
            let speed = if a < EPSILON && b > 1.0 - EPSILON { APPROACH_SPEED.min(fastest) } else { fastest };
            (to, (distance / speed).max(seconds))
        };
        self.following = Some(Following { sent_to: point, ..following });
        Some(self.send(time, to, seconds))
    }

    /// Sends the toy from where it may be to `to` in `seconds`.
    fn send(&mut self, time: f64, to: f64, seconds: f64) -> Motion {
        let ms = (seconds * 1000.0).ceil().max(1.0);
        let here = (self.at.0 + self.at.1) / 2.0;
        self.moving = Some(Segment { from: self.at, to, start: time, end: time + ms / 1000.0 });
        // A move of no length keeps the direction.
        if (to - here).abs() >= EPSILON {
            if self.last_move > f64::NEG_INFINITY && (to > here) != self.up {
                self.last_turn = time;
            }
            self.up = to > here;
        }
        self.last_move = time;
        Motion::Move { position: to, ms: ms as u32 }
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
            if let Some(motion) = planner.tick(time, &Drive { level: levels(time), ..Default::default() }, settings) {
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
    fn another_channel_sets_the_length() {
        let settings = StrokeSettings { style: StrokeStyle::Depth, ..Default::default() };
        let half_stroke = |length: f64| {
            let mut planner = Planner::default();
            planner.at = (0.5, 0.5);
            let mut sent = Vec::new();
            for i in 0..400 {
                let time = i as f64 * PERIOD;
                if let Some(Motion::Move { position, ms }) = planner.tick(time, &Drive { level: 0.2, length: Some(length), ..Default::default() }, &settings) {
                    sent.push((position, ms));
                }
            }
            let last = sent.len() - 1;
            ((sent[last].0 - sent[last - 1].0).abs(), sent[last].1)
        };
        let (short, short_ms) = half_stroke(0.0);
        let (long, long_ms) = half_stroke(1.0);
        assert!((short - MIN_AMPLITUDE).abs() < 1e-9 && (long - 1.0).abs() < 1e-9, "{short} {long}");
        // The intensity sets the speed, whatever the style.
        assert!(((short / short_ms as f64) - (long / long_ms as f64)).abs() < 1e-4);
    }

    #[test]
    fn a_thrust_plays_once_at_once_then_strokes_go_on() {
        let settings = StrokeSettings { min_turn: 0.1, ..Default::default() };
        let mut planner = Planner::default();
        planner.at = (0.5, 0.5);
        let mut toy = Toy { position: 0.5, moving: None };
        let thrust = Thrust { id: 1, length: 1.0, seconds: 0.4 };
        let mut sent = Vec::new();
        for i in 0..200 {
            let time = i as f64 * PERIOD;
            // Slow strokes; the thrust is asked at 3 s, then stays in what the toy is asked.
            let drive = Drive { level: 0.05, thrust: (time >= 3.0).then_some(thrust), ..Default::default() };
            if let Some(motion) = planner.tick(time, &drive, &settings) {
                sent.push((time, toy.at(time), motion));
                toy.apply(time, motion);
            }
        }
        let after: Vec<_> = sent.iter().filter(|(t, ..)| *t >= 3.0).collect();
        let (first, second) = (after[0], after[1]);
        assert!(first.0 < 3.0 + settings.min_turn + PERIOD, "{first:?}");
        // Over the whole range, as fast as the toy goes (0.2 s asked, 0.6 s its fastest).
        let (low, high) = settings.window(1.0);
        for &&(_, from, motion) in [first, second].iter() {
            let Motion::Move { position, ms } = motion else { panic!("{motion:?}") };
            assert!(position == low || position == high, "{position}");
            assert!((position - from).abs() / (ms as f64 / 1000.0) <= 1.0 / settings.fastest + 1e-6);
        }
        let Motion::Move { ms, .. } = second.2 else { panic!() };
        assert_eq!(ms, 600);
        // Then slow, short strokes again: the thrust is not played twice.
        let Motion::Move { position, .. } = after[2].2 else { panic!() };
        let (low, high) = settings.window(0.05);
        assert!(position == low || position == high, "{after:?}");
        assert!(after[3..].iter().all(|(.., m)| matches!(m, Motion::Move { position, .. } if *position == low || *position == high)));
    }

    /// Plays `track` from 0 (its time at `rate`) on a toy known at `start`, until `seconds`.
    fn follow(settings: &StrokeSettings, track: &Arc<Track>, rate: f64, start: f64, seconds: f64) -> Vec<(f64, f64, Motion)> {
        let mut planner = Planner::default();
        planner.at = (start, start);
        let mut toy = Toy { position: start, moving: None };
        let mut sent = Vec::new();
        let mut time = 0.0;
        while time < seconds {
            let at = (time * rate) % track.duration();
            let motion = MotionDrive { id: 7, track: track.clone(), at, rate, depth: 1.0, center: 0.5 };
            if let Some(m) = planner.tick(time, &Drive { motion: Some(motion), ..Default::default() }, settings) {
                sent.push((time, toy.at(time), m));
                toy.apply(time, m);
            }
            time += PERIOD;
        }
        sent
    }

    #[test]
    fn a_motion_is_followed_point_by_point() {
        let settings = StrokeSettings { fastest: 0.25, ..Default::default() };
        let track = Arc::new(Track::new(vec![(0.0, 0.0), (1.0, 1.0), (1.5, 0.2), (2.5, 0.8)]).unwrap());
        let sent = follow(&settings, &track, 1.0, 0.0, 2.4);
        let moves: Vec<(f64, u32)> = sent.iter().map(|&(_, _, m)| match m { Motion::Move { position, ms } => (position, ms), Motion::Stop => panic!() }).collect();
        // Each move ends when the motion gets to its point (within a send period), even at no intensity.
        assert_eq!(moves.len(), 3, "{sent:?}");
        assert_eq!(moves[0].0, 1.0);
        assert!((moves[0].1 as f64 - 1000.0).abs() <= 1.0);
        assert!((moves[1].0 - 0.2).abs() < 1e-9 && (sent[1].0 + moves[1].1 as f64 / 1000.0 - 1.5).abs() < 0.06);
        assert!((moves[2].0 - 0.8).abs() < 1e-9 && (sent[2].0 + moves[2].1 as f64 / 1000.0 - 2.5).abs() < 0.06);
    }

    #[test]
    fn a_motion_too_fast_is_shortened_and_too_close_points_skipped() {
        let settings = StrokeSettings { fastest: 1.0, min_turn: 0.25, ..Default::default() };
        // Whole-length strokes every 0.2 s: faster than the toy, closer than its turns.
        let points = (0..=40).map(|i| (i as f64 * 0.2, (i % 2) as f64)).collect();
        let track = Arc::new(Track::new(points).unwrap());
        let sent = follow(&settings, &track, 1.0, 0.5, 6.0);
        assert!(sent.len() > 10);
        let (mut last_turn, mut up) = (f64::NEG_INFINITY, None);
        for &(time, from, motion) in &sent {
            let Motion::Move { position, ms } = motion else { panic!() };
            assert!((position - from).abs() / (ms as f64 / 1000.0) <= 1.0 + 1e-6, "too fast at {time}");
            if (position - from).abs() > 1e-6 {
                if up.is_some_and(|up| up != (position > from)) {
                    assert!(time - last_turn >= settings.min_turn - 1e-9, "turned again after {}", time - last_turn);
                    last_turn = time;
                }
                up = Some(position > from);
            }
        }
    }

    #[test]
    fn a_motion_from_an_unknown_place_gets_there_gently_then_catches_up() {
        let settings = StrokeSettings::default();
        let track = Arc::new(Track::new((0..=20).map(|i| (i as f64 * 0.5, (i % 2) as f64)).collect()).unwrap());
        let mut planner = Planner::default();
        let motion = |at: f64| MotionDrive { id: 1, track: track.clone(), at, rate: 1.0, depth: 1.0, center: 0.5 };
        // To its first point (1 at 0.5 s), timed for the farthest it might be.
        let Some(Motion::Move { position, ms }) = planner.tick(0.0, &Drive { motion: Some(motion(0.0)), ..Default::default() }, &settings) else { panic!() };
        assert_eq!(position, 1.0);
        assert!(ms as f64 >= 1000.0 / APPROACH_SPEED - 1.0, "{ms}");
        // Nothing until it is there, then the points still ahead: 0 at 1.0 s went by, 1 at 1.5 s.
        assert_eq!(planner.tick(0.5, &Drive { motion: Some(motion(0.5)), ..Default::default() }, &settings), None);
        let Some(Motion::Move { position, ms }) = planner.tick(1.1, &Drive { motion: Some(motion(1.1)), ..Default::default() }, &settings) else { panic!() };
        assert_eq!((position, ms), (1.0, 400));
    }

    #[test]
    fn every_point_is_played_on_time_when_the_toy_can() {
        // game_over: 300 then 200 ms between points, turns at each, within what the toy does.
        let settings = StrokeSettings { fastest: 0.25, slowest: 4.5, min_turn: 0.15, ..Default::default() };
        let mut points = vec![(0.0, 0.0)];
        points.extend((1..=8).map(|i| (i as f64 * 0.3, if i % 2 == 1 { 1.0 } else { 0.0 })));
        points.extend((1..=14).map(|i| (2.4 + i as f64 * 0.2, if i % 2 == 1 { 0.56 } else { 1.0 })));
        let track = Arc::new(Track::new(points.clone()).unwrap());
        let mut planner = Planner::default();
        planner.at = (0.0, 0.0);
        let mut sent = Vec::new();
        // Sent every 50 ms, where the motion is known 0 to 60 ms late (the engine's tick, slower in debug builds).
        for i in 0..130 {
            let time = i as f64 * 0.05;
            let at = (time - (i % 4) as f64 * 0.02).max(0.0);
            let motion = MotionDrive { id: 1, track: track.clone(), at, rate: 1.0, depth: 1.0, center: 0.5 };
            if let Some(Motion::Move { position, ms }) = planner.tick(time, &Drive { motion: Some(motion), ..Default::default() }, &settings) {
                sent.push((time + ms as f64 / 1000.0, position));
            }
        }
        // Every point after the first, each reached within the lag and half a send period of its time.
        assert_eq!(sent.len(), points.len() - 1, "{sent:?}");
        for (&(arrives, position), &(t, p)) in sent.iter().zip(&points[1..]) {
            assert_eq!(position, p, "at {t}");
            assert!((arrives - t).abs() <= 0.09, "{t} reached at {arrives}");
        }
    }

    #[test]
    fn strokes_go_on_after_a_motion() {
        let settings = StrokeSettings::default();
        let track = Arc::new(Track::new(vec![(0.0, 0.0), (1.0, 1.0)]).unwrap());
        let mut planner = Planner::default();
        planner.at = (0.0, 0.0);
        let motion = MotionDrive { id: 1, track, at: 0.0, rate: 1.0, depth: 1.0, center: 0.5 };
        assert!(planner.tick(0.0, &Drive { level: 0.0, motion: Some(motion), ..Default::default() }, &settings).is_some());
        // Ended: the intensity drives again; at 0 it stops the toy.
        assert_eq!(planner.tick(0.5, &Drive::default(), &settings), Some(Motion::Stop));
        assert!(planner.tick(2.0, &Drive { level: 0.5, ..Default::default() }, &settings).is_some());
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
        let Some(Motion::Move { ms, .. }) = planner.tick(0.0, &Drive { level: 1.0, ..Default::default() }, &settings) else { panic!() };
        assert_eq!(ms, 200);
        assert_eq!(planner.tick(0.1, &Drive::default(), &settings), Some(Motion::Stop));
        let Some(Motion::Move { position, ms }) = planner.tick(1.0, &Drive { level: 1.0, ..Default::default() }, &settings) else { panic!() };
        // Somewhere between 0 and 0.5: timed for the far end of that.
        assert_eq!(position, 0.0);
        assert!(ms as f64 >= 500.0 / APPROACH_SPEED - 1.0, "{ms}");
    }

    #[test]
    fn ramps_step_to_their_limit_then_end() {
        for ramp in Ramp::ALL {
            let (first, _, last) = ramp.steps();
            assert_eq!(ramp.value(0.0), Some(first));
            assert_eq!(ramp.value(ramp.length() - 0.01), Some(last));
            assert_eq!(ramp.value(ramp.length()), None);
            let values: Vec<f64> = ramp.schedule().iter().map(|(_, v)| *v).collect();
            assert!(values.windows(2).all(|w| (w[1] - w[0]).signum() == (last - first).signum()), "{ramp:?} {values:?}");
            assert!(ramp.length() < 90.0, "{ramp:?} lasts {}", ramp.length());
        }
    }

    #[test]
    fn ramp_results_keep_a_margin_from_what_failed() {
        // Pressed during the fourth step: the third was tried a reaction time before.
        let third = Ramp::Fastest.schedule()[2];
        let pressed = Ramp::Fastest.schedule()[3].0 + 0.5;
        assert!((Ramp::Fastest.result(pressed) - third.1 * 1.15).abs() < 1e-9);
        assert!(Ramp::Slowest.result(20.0) < Ramp::Slowest.value(20.0 - REACTION).unwrap());
        // Followed everything: the limit, within the settings' range.
        assert_eq!(Ramp::Turns.result(f64::INFINITY), 0.1 * 1.15);
        assert!(StrokeSettings::SLOWEST_RANGE.contains(&Ramp::Slowest.result(f64::INFINITY)));
    }

    #[test]
    fn calibration_ramps_play_what_they_say() {
        let settings = StrokeSettings { bottom: 0.1, top: 0.9, fastest: 0.4, ..Default::default() };
        for ramp in Ramp::ALL {
            let mut planner = Planner::default();
            planner.at = (0.1, 0.1);
            let mut toy = Toy { position: 0.1, moving: None };
            let mut time = 0.0;
            let mut moves = Vec::new();
            while time < ramp.length() + 1.0 {
                if let Some(motion) = planner.calibrate(time, Calibration::Ramp(ramp), time, &settings) {
                    moves.push((time, toy.at(time), motion));
                    toy.apply(time, motion);
                }
                time += PERIOD;
            }
            for &(time, from, motion) in moves.iter().filter(|m| m.0 > 0.0) {
                let Motion::Move { position, ms } = motion else { continue };
                let tried = ramp.value(time).unwrap();
                let seconds = ms as f64 / 1000.0;
                assert!((0.1 - 1e-9..=0.9 + 1e-9).contains(&position), "{ramp:?} went to {position}");
                assert!((position - from).abs() / seconds <= 1.0 / 0.2 + 1e-6, "{ramp:?} too fast at {time}");
                match ramp {
                    Ramp::Fastest | Ramp::Slowest => {
                        assert!(((position - from).abs() / seconds - 1.0 / tried).abs() < 0.05, "{ramp:?} at {time}")
                    }
                    Ramp::Turns => {
                        // Exactly the turn tried, longer when the window just shrank around the toy.
                        assert!(seconds >= tried - 0.002, "{ramp:?} at {time}: {seconds} s");
                        assert!((position - from).abs() / seconds <= 1.0 / 0.4 + 1e-6);
                    }
                }
            }
            let after: Vec<_> = moves.iter().filter(|m| m.0 >= ramp.length()).collect();
            assert!(after.iter().all(|m| m.2 == Motion::Stop) && after.len() <= 1, "{ramp:?} after its end: {after:?}");
        }
    }

    #[test]
    fn hold_goes_where_the_slider_is() {
        let settings = StrokeSettings::default();
        let mut planner = Planner::default();
        let Some(Motion::Move { position, ms }) = planner.calibrate(0.0, Calibration::Hold(0.3), 0.0, &settings) else { panic!() };
        assert_eq!(position, 0.3);
        // From anywhere: 0.7 at most, gently.
        assert!(ms as f64 >= 700.0 / APPROACH_SPEED - 1.0);
        assert_eq!(planner.calibrate(0.5, Calibration::Hold(0.3), 0.5, &settings), None);
        // The slider moved: on its way at once.
        let Some(Motion::Move { position, .. }) = planner.calibrate(0.6, Calibration::Hold(0.8), 0.6, &settings) else { panic!() };
        assert_eq!(position, 0.8);
        assert_eq!(planner.calibrate(60.0, Calibration::Hold(0.8), 60.0, &settings), None);
    }

    #[test]
    fn motion_limits_follow_the_range() {
        let whole = StrokeSettings { fastest: 0.5, slowest: 4.0, min_turn: 0.2, ..Default::default() }.motion_limits();
        assert_eq!(whole, MotionLimits { slowest: 0.25, fastest: 2.0, min_turn: 0.2 });
        // Over half the length, a motion's positions may move twice as fast.
        let half = StrokeSettings { bottom: 0.25, top: 0.75, fastest: 0.5, slowest: 4.0, ..Default::default() }.motion_limits();
        assert_eq!((half.slowest, half.fastest), (0.5, 4.0));
        let both = whole.strictest(MotionLimits { slowest: 0.1, fastest: 1.5, min_turn: 0.3 });
        assert_eq!(both, MotionLimits { slowest: 0.25, fastest: 1.5, min_turn: 0.3 });
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
