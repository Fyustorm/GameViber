//! Rebuilds the motor state from force-feedback semantics (upload / play /
//! stop / erase / gain, as Linux's evdev defines them, which every source
//! translates to), like the kernel's `ff-memless`: every active effect
//! contributes to the strong and weak motors, contributions are summed then
//! clamped.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gameviber_common::{FF_CONSTANT, FF_EFFECT_SIZE, FF_PERIODIC, FF_RAMP, FF_RUMBLE};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Envelope {
    pub attack_length: u16,
    pub attack_level: u16,
    pub fade_length: u16,
    pub fade_level: u16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EffectKind {
    Rumble { strong: u16, weak: u16 },
    Periodic { magnitude: i16, envelope: Envelope },
    Constant { level: i16, envelope: Envelope },
    Ramp { start: i16, end: i16, envelope: Envelope },
    /// Condition effects (spring, damper...): no vibration equivalent.
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effect {
    pub kind: EffectKind,
    pub length_ms: u16,
    pub delay_ms: u16,
}

impl Effect {
    /// Decodes a `struct ff_effect` as the kernel lays it out (the eBPF probe
    /// sends it raw): 0 type, 2 id, 4 direction, 6 trigger, 10 replay length,
    /// 12 replay delay, 16 the union of the effect types.
    pub fn from_kernel(raw: &[u8; FF_EFFECT_SIZE]) -> Self {
        let word = |at: usize| u16::from_ne_bytes([raw[at], raw[at + 1]]);
        let signed = |at: usize| word(at) as i16;
        // struct ff_envelope: attack length and level, fade length and level.
        let envelope = |at: usize| Envelope {
            attack_length: word(at),
            attack_level: word(at + 2),
            fade_length: word(at + 4),
            fade_level: word(at + 6),
        };
        let kind = match word(0) {
            // strong_magnitude, weak_magnitude
            FF_RUMBLE => EffectKind::Rumble { strong: word(16), weak: word(18) },
            // waveform, period, magnitude, offset, phase, envelope
            FF_PERIODIC => EffectKind::Periodic { magnitude: signed(20), envelope: envelope(26) },
            // level, envelope
            FF_CONSTANT => EffectKind::Constant { level: signed(16), envelope: envelope(18) },
            // start_level, end_level, envelope
            FF_RAMP => EffectKind::Ramp { start: signed(16), end: signed(18), envelope: envelope(20) },
            _ => EffectKind::Unsupported,
        };
        Self { kind, length_ms: word(10), delay_ms: word(12) }
    }

    /// (strong, weak) in 0..0xFFFF, `elapsed` since the effect actually started.
    fn motors_at(&self, elapsed: Duration) -> (u32, u32) {
        let t = elapsed.as_millis() as u32;
        let length = self.length_ms as u32;
        match self.kind {
            EffectKind::Rumble { strong, weak } => (strong as u32, weak as u32),
            EffectKind::Periodic { magnitude, envelope } => both(apply_envelope(magnitude.unsigned_abs(), t, length, envelope)),
            EffectKind::Constant { level, envelope } => both(apply_envelope(level.unsigned_abs(), t, length, envelope)),
            EffectKind::Ramp { start, end, envelope } => {
                let progress = if length == 0 { 0.0 } else { (t as f32 / length as f32).min(1.0) };
                let level = start as f32 + (end as f32 - start as f32) * progress;
                both(apply_envelope(level.abs() as u16, t, length, envelope))
            }
            EffectKind::Unsupported => (0, 0),
        }
    }
}

/// Single-level effects (0..0x7FFF): ff-memless scales them to 0..0xFFFF on both motors.
fn both(level: u16) -> (u32, u32) {
    let v = (level as u32 * 2).min(0xFFFF);
    (v, v)
}

/// Same computation as `apply_envelope` in drivers/input/ff-memless.c.
fn apply_envelope(value: u16, t: u32, length: u32, env: Envelope) -> u16 {
    let value = value as i64;
    if env.attack_length > 0 && t < env.attack_length as u32 {
        let attack = env.attack_level.min(0x7FFF) as i64;
        return (attack + (value - attack) * t as i64 / env.attack_length as i64) as u16;
    }
    if env.fade_length > 0 && length > 0 {
        let fade_start = length.saturating_sub(env.fade_length as u32);
        if t >= fade_start {
            let fade = env.fade_level.min(0x7FFF) as i64;
            let dt = (t - fade_start) as i64;
            return (value + (fade - value) * dt / env.fade_length as i64).max(0) as u16;
        }
    }
    value as u16
}

#[derive(Debug)]
struct Playing {
    since: Instant,
    count: i32,
}

/// How long a play may wait for its effect's upload (eBPF source: both
/// arrive through different paths).
const PENDING_PLAY_TIMEOUT: Duration = Duration::from_secs(1);

/// Force-feedback state of one gamepad.
#[derive(Debug)]
pub struct RumbleState {
    effects: HashMap<i16, Effect>,
    playing: HashMap<i16, Playing>,
    gain: u16,
}

impl Default for RumbleState {
    fn default() -> Self {
        Self { effects: HashMap::new(), playing: HashMap::new(), gain: 0xFFFF }
    }
}

impl RumbleState {
    pub fn upload(&mut self, id: i16, effect: Effect) {
        self.effects.insert(id, effect);
    }

    pub fn erase(&mut self, id: i16) {
        self.effects.remove(&id);
        self.playing.remove(&id);
    }

    /// `count` = repetitions requested by EV_FF (0 = stop).
    pub fn play(&mut self, id: i16, count: i32, now: Instant) {
        if count <= 0 {
            self.playing.remove(&id);
        } else {
            self.playing.insert(id, Playing { since: now, count });
        }
    }

    pub fn set_gain(&mut self, gain: u16) {
        self.gain = gain;
    }

    /// (strong, weak) in 0..0xFFFF at `now`. Drops finished effects.
    pub fn motors(&mut self, now: Instant) -> (u16, u16) {
        let (mut strong, mut weak) = (0u32, 0u32);
        self.playing.retain(|id, p| {
            let Some(effect) = self.effects.get(id) else {
                // Play received before its upload: wait a little.
                return now.duration_since(p.since) < PENDING_PLAY_TIMEOUT;
            };
            let start = p.since + Duration::from_millis(effect.delay_ms as u64);
            if now < start {
                return true;
            }
            let elapsed = now - start;
            let (s, w) = if effect.length_ms == 0 {
                effect.motors_at(elapsed) // infinite length
            } else {
                let length = Duration::from_millis(effect.length_ms as u64);
                if elapsed >= length * p.count as u32 {
                    return false;
                }
                let in_loop = Duration::from_nanos((elapsed.as_nanos() % length.as_nanos()) as u64);
                effect.motors_at(in_loop)
            };
            strong += s;
            weak += w;
            true
        });
        let gain = self.gain as u32;
        (
            (strong.min(0xFFFF) * gain / 0xFFFF) as u16,
            (weak.min(0xFFFF) * gain / 0xFFFF) as u16,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rumble(strong: u16, weak: u16, length_ms: u16) -> Effect {
        Effect { kind: EffectKind::Rumble { strong, weak }, length_ms, delay_ms: 0 }
    }

    #[test]
    fn rumble_plays_for_its_length_then_stops() {
        let t0 = Instant::now();
        let mut s = RumbleState::default();
        s.upload(0, rumble(0x4000, 0x2000, 300));
        s.play(0, 1, t0);
        assert_eq!(s.motors(t0 + Duration::from_millis(100)), (0x4000, 0x2000));
        assert_eq!(s.motors(t0 + Duration::from_millis(300)), (0, 0));
    }

    #[test]
    fn repeat_count_extends_duration() {
        let t0 = Instant::now();
        let mut s = RumbleState::default();
        s.upload(0, rumble(0x4000, 0, 100));
        s.play(0, 3, t0);
        assert_eq!(s.motors(t0 + Duration::from_millis(250)).0, 0x4000);
        assert_eq!(s.motors(t0 + Duration::from_millis(300)).0, 0);
    }

    #[test]
    fn stop_and_zero_length_infinite() {
        let t0 = Instant::now();
        let mut s = RumbleState::default();
        s.upload(1, rumble(0xFFFF, 0, 0));
        s.play(1, 1, t0);
        assert_eq!(s.motors(t0 + Duration::from_secs(60)).0, 0xFFFF);
        s.play(1, 0, t0);
        assert_eq!(s.motors(t0 + Duration::from_secs(60)), (0, 0));
    }

    #[test]
    fn effect_replays_after_stop_without_reupload() {
        let t0 = Instant::now();
        let mut s = RumbleState::default();
        s.upload(0, rumble(0x1000, 0, 0));
        s.play(0, 1, t0);
        s.play(0, 0, t0);
        s.play(0, 1, t0);
        assert_eq!(s.motors(t0).0, 0x1000);
    }

    #[test]
    fn concurrent_effects_are_summed_and_clamped() {
        let t0 = Instant::now();
        let mut s = RumbleState::default();
        s.upload(0, rumble(0xC000, 0x1000, 0));
        s.upload(1, rumble(0xC000, 0x1000, 0));
        s.play(0, 1, t0);
        s.play(1, 1, t0);
        assert_eq!(s.motors(t0), (0xFFFF, 0x2000));
    }

    #[test]
    fn play_before_upload_is_held() {
        let t0 = Instant::now();
        let mut s = RumbleState::default();
        s.play(2, 1, t0);
        assert_eq!(s.motors(t0), (0, 0));
        s.upload(2, rumble(0x3000, 0, 0));
        assert_eq!(s.motors(t0 + Duration::from_millis(10)).0, 0x3000);
    }

    #[test]
    fn orphan_play_expires() {
        let t0 = Instant::now();
        let mut s = RumbleState::default();
        s.play(2, 1, t0);
        s.motors(t0 + PENDING_PLAY_TIMEOUT);
        s.upload(2, rumble(0x3000, 0, 0));
        assert_eq!(s.motors(t0 + PENDING_PLAY_TIMEOUT).0, 0);
    }

    #[test]
    fn gain_scales_output() {
        let t0 = Instant::now();
        let mut s = RumbleState::default();
        s.upload(0, rumble(0xFFFF, 0x8000, 0));
        s.play(0, 1, t0);
        s.set_gain(0x8000);
        let (strong, weak) = s.motors(t0);
        assert!((0x7FFF..=0x8000).contains(&strong));
        assert!((0x3FFF..=0x4000).contains(&weak));
    }

    #[test]
    fn periodic_uses_magnitude_on_both_motors_with_attack() {
        let t0 = Instant::now();
        let mut s = RumbleState::default();
        let envelope = Envelope { attack_length: 100, attack_level: 0, ..Default::default() };
        s.upload(0, Effect { kind: EffectKind::Periodic { magnitude: 0x4000, envelope }, length_ms: 0, delay_ms: 0 });
        s.play(0, 1, t0);
        assert_eq!(s.motors(t0), (0, 0));
        assert_eq!(s.motors(t0 + Duration::from_millis(50)), (0x4000, 0x4000));
        assert_eq!(s.motors(t0 + Duration::from_millis(200)), (0x8000, 0x8000));
    }

    #[test]
    fn delay_postpones_start() {
        let t0 = Instant::now();
        let mut s = RumbleState::default();
        s.upload(0, Effect { delay_ms: 100, ..rumble(0x5000, 0, 100) });
        s.play(0, 1, t0);
        assert_eq!(s.motors(t0 + Duration::from_millis(50)).0, 0);
        assert_eq!(s.motors(t0 + Duration::from_millis(150)).0, 0x5000);
        assert_eq!(s.motors(t0 + Duration::from_millis(200)).0, 0);
    }

    #[test]
    fn a_kernel_effect_is_decoded() {
        let mut raw = [0u8; FF_EFFECT_SIZE];
        let mut put = |at: usize, value: u16| raw[at..at + 2].copy_from_slice(&value.to_ne_bytes());
        put(0, FF_PERIODIC);
        put(10, 500); // replay length
        put(20, (-1000i16) as u16); // magnitude
        put(26, 10); // attack length
        put(32, 0x1234); // fade level
        let e = Effect::from_kernel(&raw);
        assert_eq!(e.length_ms, 500);
        match e.kind {
            EffectKind::Periodic { magnitude, envelope } => {
                assert_eq!(magnitude, -1000);
                assert_eq!(envelope.attack_length, 10);
                assert_eq!(envelope.fade_level, 0x1234);
            }
            other => panic!("{other:?}"),
        }
    }
}
