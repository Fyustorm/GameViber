//! Phases (§6.3): what each sense says about the phases a mode declares —
//! the game's sound against their sound descriptions, its image against
//! their screen descriptions and against example images set up for the
//! mode — fused, averaged over a few seconds, and settled on with enough
//! hysteresis that a few ambiguous seconds do not flip the phase. A phase
//! set up for the mode can also be tied to an indicator (its battle menu): the indicator shown is a
//! sure sign of it, ahead of the guesses. Each phase can be held a while after
//! its last sign, for signs that come and go.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::models::{self, Embedding};

pub const DEFAULT_WINDOW: f64 = 10.0;
/// A phase is entered when its average probability reaches this...
pub const ENTER: f64 = 0.5;
/// ...and, when another phase is current, beats it by this much.
pub const MARGIN: f64 = 0.1;
/// Sharpness of the comparison of an image with example images.
const EXAMPLE_SCALE: f64 = 50.0;
/// An indicator must be shown this long to be a sign of its phase (not a frame of a transition).
const INDICATOR_CONFIRM: f64 = 0.5;

/// A phase a mode declares: a name and how it sounds and looks.
#[derive(Debug, Clone, PartialEq)]
pub struct PhaseDecl {
    pub name: String,
    pub sound: Option<String>,
    pub screen: Option<String>,
    /// An indicator of the game whose showing is a sure sign of the phase.
    pub indicator: Option<String>,
    /// Seconds the phase is kept after its last sign.
    pub hold: f64,
    /// Events the mode does not get during the phase.
    pub ignore: Ignored,
}

/// Guessed events a phase keeps from the mode, set up by the player (a menu's
/// clicks heard as hits): the script never sees them while the phase is current.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ignored {
    /// Hits of the sound: `on_audio_hit`, and `on_impact` from the sound.
    pub sound_hits: bool,
    /// Flashes of the image: `on_impact` from the screen.
    pub flashes: bool,
}

impl Ignored {
    pub fn is_none(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sense {
    /// The sound against the sound descriptions.
    Sound,
    /// The image against the screen descriptions.
    Screen,
    /// The image against the example images set up for the mode.
    Examples,
}

impl Sense {
    const ALL: [Sense; 3] = [Sense::Sound, Sense::Screen, Sense::Examples];

    /// Evidence older than this is not used.
    fn fresh_secs(self) -> f64 {
        match self {
            // A sound clip covers 10 s and comes every 2 s.
            Sense::Sound => 15.0,
            Sense::Screen | Sense::Examples => 5.0,
        }
    }

    fn scale(self) -> f64 {
        match self {
            Sense::Sound => crate::audio::clap::LOGIT_SCALE,
            Sense::Screen => crate::screen::clip::LOGIT_SCALE,
            Sense::Examples => EXAMPLE_SCALE,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhaseChange {
    pub phase: Option<String>,
    pub previous: Option<String>,
    pub confidence: f64,
}

/// What one sense compares with: the phases it can speak about and a reference vector for each.
#[derive(Default)]
struct References {
    phases: Vec<usize>,
    vectors: Vec<Embedding>,
    /// The last evidence: when, and a likelihood per phase (1 = says nothing).
    latest: Option<(f64, Vec<f64>)>,
}

pub struct PhaseTracker {
    names: Vec<String>,
    senses: [References; 3],
    window: f64,
    history: VecDeque<(f64, Vec<f64>)>,
    average: Vec<f64>,
    current: Option<usize>,
    /// Per phase: its indicator, how long it is held, since when its indicator is shown,
    /// and the last time something said it (its indicator, or the best average).
    indicators: Vec<Option<String>>,
    holds: Vec<f64>,
    shown_since: Vec<Option<f64>>,
    last_sign: Vec<f64>,
    /// Indicators are being read (the image is there): phases tied to one are only
    /// entered through it.
    indicators_read: bool,
}

impl PhaseTracker {
    pub fn new(phases: &[PhaseDecl], window: f64) -> Self {
        let n = phases.len();
        Self {
            names: phases.iter().map(|s| s.name.clone()).collect(),
            senses: Default::default(),
            window,
            history: VecDeque::new(),
            average: vec![0.0; n],
            current: None,
            indicators: phases.iter().map(|s| s.indicator.clone()).collect(),
            holds: phases.iter().map(|s| s.hold.max(0.0)).collect(),
            shown_since: vec![None; n],
            last_sign: vec![f64::NEG_INFINITY; n],
            indicators_read: false,
        }
    }

    fn index(sense: Sense) -> usize {
        Sense::ALL.iter().position(|s| *s == sense).unwrap()
    }

    /// Reference vectors of `sense`: one per named phase (unknown names are ignored).
    pub fn set_references(&mut self, sense: Sense, references: Vec<(String, Embedding)>) {
        let mut r = References::default();
        for (name, vector) in references {
            if let Some(i) = self.names.iter().position(|n| *n == name) {
                r.phases.push(i);
                r.vectors.push(vector);
            }
        }
        self.senses[Self::index(sense)] = r;
    }

    pub fn has(&self, sense: Sense) -> bool {
        !self.senses[Self::index(sense)].phases.is_empty()
    }

    /// Some sense can recognize phases.
    #[cfg(test)]
    pub fn ready(&self) -> bool {
        Sense::ALL.iter().any(|s| self.has(*s))
    }

    /// New evidence from `sense` at `time`: an embedding of the sound or the image.
    pub fn update(&mut self, time: f64, sense: Sense, vector: &[f32]) -> Option<PhaseChange> {
        let n = self.names.len();
        let r = &mut self.senses[Self::index(sense)];
        if r.phases.is_empty() {
            return None;
        }
        // Among the phases it knows, a sense shares out the likelihood; it says
        // nothing (1) about the others.
        let p = models::probabilities(vector, &r.vectors, sense.scale());
        let mut likelihood = vec![1.0; n];
        for (k, &i) in r.phases.iter().enumerate() {
            likelihood[i] = p[k] * r.phases.len() as f64;
        }
        r.latest = Some((time, likelihood));

        let mut fused = vec![1.0; n];
        for (s, r) in Sense::ALL.iter().zip(&self.senses) {
            if let Some((t, l)) = &r.latest {
                if time - t <= s.fresh_secs() {
                    fused.iter_mut().zip(l).for_each(|(f, l)| *f *= l);
                }
            }
        }
        let sum: f64 = fused.iter().sum();
        fused.iter_mut().for_each(|f| *f /= sum.max(1e-12));
        self.settle(time, fused)
    }

    /// An indicator's new state: shown (a bar on screen counts) or not.
    pub fn indicator(&mut self, time: f64, name: &str, shown: bool) -> Option<PhaseChange> {
        self.indicators_read = true;
        for i in 0..self.names.len() {
            if self.indicators[i].as_deref() == Some(name) {
                if !shown && self.shown_since[i].is_some_and(|t| time - t >= INDICATOR_CONFIRM) {
                    // The hold counts from when it went.
                    self.last_sign[i] = time;
                }
                self.shown_since[i] = if shown { self.shown_since[i].or(Some(time)) } else { None };
            }
        }
        self.decide(time)
    }

    /// Time passes: an indicator shown long enough enters its phase, a hold runs out.
    pub fn tick(&mut self, time: f64) -> Option<PhaseChange> {
        if self.indicators.iter().all(Option::is_none) && self.holds.iter().all(|h| *h <= 0.0) {
            return None;
        }
        self.decide(time)
    }

    /// A phase is tied to an indicator, and indicators are being read.
    fn by_indicator_only(&self, i: usize) -> bool {
        self.indicators_read && self.indicators[i].is_some()
    }

    fn settle(&mut self, time: f64, probabilities: Vec<f64>) -> Option<PhaseChange> {
        self.history.push_back((time, probabilities));
        while self.history.front().is_some_and(|(t, _)| time - t >= self.window) {
            self.history.pop_front();
        }
        let n = self.history.len() as f64;
        self.average = (0..self.names.len()).map(|i| self.history.iter().map(|(_, p)| p[i]).sum::<f64>() / n).collect();
        self.decide(time)
    }

    fn decide(&mut self, time: f64) -> Option<PhaseChange> {
        // Indicators shown long enough are sure signs.
        let mut by_indicator = None;
        for i in 0..self.names.len() {
            if self.shown_since[i].is_some_and(|t| time - t >= INDICATOR_CONFIRM) {
                self.last_sign[i] = time;
                if by_indicator.is_none() || self.current == Some(i) {
                    by_indicator = Some(i);
                }
            }
        }
        // Then the best guess among the phases not tied to an indicator being read.
        let best = (0..self.names.len())
            .filter(|&i| !self.by_indicator_only(i))
            .max_by(|&a, &b| self.average[a].total_cmp(&self.average[b]))
            .filter(|&i| self.average[i] > 0.0);
        if let Some(best) = best.filter(|&b| by_indicator.is_none() && self.average[b] >= ENTER) {
            self.last_sign[best] = time;
        }
        let held = self.current.is_some_and(|c| time - self.last_sign[c] < self.holds[c]);
        let next = match (by_indicator, self.current) {
            (Some(z), _) => Some(z),
            (None, Some(_)) if held => return None,
            // The current phase's indicator is gone (and its hold over): the best guess, or no phase.
            (None, Some(c)) if self.by_indicator_only(c) => best.filter(|&b| self.average[b] >= ENTER),
            (None, Some(c)) => best.filter(|&b| b != c && self.average[b] >= ENTER && self.average[b] - self.average[c] >= MARGIN).or(Some(c)),
            (None, None) => best.filter(|&b| self.average[b] >= ENTER),
        };
        if next == self.current {
            return None;
        }
        let previous = std::mem::replace(&mut self.current, next);
        Some(PhaseChange {
            phase: next.map(|i| self.names[i].clone()),
            previous: previous.map(|i| self.names[i].clone()),
            confidence: self.confidence(),
        })
    }

    /// A sense stopped (the sound or the image is gone). When no sense is
    /// left, the phase is forgotten: returns the change when one was current.
    pub fn forget(&mut self, sense: Sense) -> Option<PhaseChange> {
        self.senses[Self::index(sense)].latest = None;
        if sense == Sense::Screen {
            // The indicators are read on the image.
            self.indicators_read = false;
            self.shown_since.iter_mut().for_each(|s| *s = None);
        }
        if self.senses.iter().any(|r| r.latest.is_some()) {
            return None;
        }
        self.reset()
    }

    pub fn reset(&mut self) -> Option<PhaseChange> {
        self.history.clear();
        self.senses.iter_mut().for_each(|r| r.latest = None);
        self.indicators_read = false;
        self.shown_since.iter_mut().for_each(|s| *s = None);
        self.last_sign.iter_mut().for_each(|t| *t = f64::NEG_INFINITY);
        self.average.iter_mut().for_each(|p| *p = 0.0);
        let previous = self.current.take()?;
        Some(PhaseChange { phase: None, previous: Some(self.names[previous].clone()), confidence: 0.0 })
    }

    pub fn current(&self) -> Option<&str> {
        self.current.map(|i| self.names[i].as_str())
    }

    /// Probability of each phase, in the order of the names given: 1 for a
    /// phase whose indicator is shown, else its average.
    pub fn averages(&self) -> impl Iterator<Item = (&str, f64)> {
        let shown = self.shown_since.iter().map(Option::is_some);
        self.names.iter().map(String::as_str).zip(self.average.iter().zip(shown).map(|(p, shown)| if shown { 1.0 } else { *p }))
    }

    pub fn confidence(&self) -> f64 {
        self.current.map_or(0.0, |i| if self.shown_since[i].is_some() { 1.0 } else { self.average[i] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(i: usize) -> Embedding {
        (0..3).map(|k| if k == i { 1.0 } else { 0.0 }).collect()
    }

    /// A vector between the directions 0 and 1, `towards_one` 0..1.
    fn between(towards_one: f32) -> Vec<f32> {
        let (a, b) = (1.0 - towards_one, towards_one);
        let norm = (a * a + b * b).sqrt();
        vec![a / norm, b / norm, 0.0]
    }

    fn decls(names: &[&str]) -> Vec<PhaseDecl> {
        names.iter().map(|n| PhaseDecl { name: (*n).to_owned(), sound: None, screen: None, indicator: None, hold: 0.0, ignore: Ignored::default() }).collect()
    }

    fn names() -> Vec<PhaseDecl> {
        decls(&["battle", "calm"])
    }

    #[test]
    fn phases_settle_and_change_with_hysteresis() {
        let mut tracker = PhaseTracker::new(&names(), 10.0);
        assert_eq!(tracker.update(0.0, Sense::Sound, &between(0.0)), None, "no references yet");
        tracker.set_references(Sense::Sound, vec![("battle".into(), unit(0)), ("calm".into(), unit(1))]);
        let change = tracker.update(2.0, Sense::Sound, &between(0.0)).unwrap();
        assert_eq!(change.phase.as_deref(), Some("battle"));
        assert_eq!(change.previous, None);
        assert!(change.confidence > 0.99);

        // Ambiguous sound for a while: battle stays.
        for t in 2..5 {
            assert_eq!(tracker.update(2.0 + 2.0 * t as f64, Sense::Sound, &between(0.5)), None);
        }
        // Calm for good: it takes over once it dominates the window.
        let mut changed_at = None;
        for t in 0..10 {
            let time = 12.0 + 2.0 * t as f64;
            if let Some(change) = tracker.update(time, Sense::Sound, &between(1.0)) {
                assert_eq!(change.phase.as_deref(), Some("calm"));
                assert_eq!(change.previous.as_deref(), Some("battle"));
                changed_at = Some(time);
                break;
            }
        }
        let changed_at = changed_at.expect("calm is entered");
        assert!((12.0..=18.0).contains(&changed_at), "{changed_at}");
        assert_eq!(tracker.current(), Some("calm"));

        let reset = tracker.forget(Sense::Sound).unwrap();
        assert_eq!((reset.phase, reset.previous.as_deref()), (None, Some("calm")));
        assert_eq!(tracker.current(), None);
    }

    /// The sound cannot tell two phases apart; the image can.
    #[test]
    fn senses_are_fused() {
        let mut tracker = PhaseTracker::new(&decls(&["battle", "dungeon", "story"]), 4.0);
        // One sound description for both action phases (the same music), one for story.
        tracker.set_references(Sense::Sound, vec![("dungeon".into(), unit(0)), ("story".into(), unit(1))]);
        tracker.set_references(Sense::Examples, vec![("battle".into(), unit(2)), ("dungeon".into(), unit(0))]);
        let action_music = [1.0, 0.0, 0.0];
        // Sound alone: dungeon versus story only; battle is not described, so it gets a neutral share.
        tracker.update(0.0, Sense::Sound, &action_music);
        let averages: Vec<f64> = tracker.averages().map(|(_, p)| p).collect();
        assert!(averages[1] > 0.6 && averages[0] > 0.2 && averages[2] < 0.01, "{averages:?}");
        // The image looks like the battle examples: battle wins with the same music.
        let mut last = None;
        for t in 1..6 {
            if let Some(change) = tracker.update(t as f64, Sense::Examples, &[0.0, 0.0, 1.0]) {
                last = Some(change);
            }
            tracker.update(t as f64 + 0.5, Sense::Sound, &action_music);
        }
        assert_eq!(tracker.current(), Some("battle"), "{last:?}");
        // The image goes away: the sound alone remains, the phase is kept.
        assert_eq!(tracker.forget(Sense::Examples), None);
        assert_eq!(tracker.current(), Some("battle"));
    }

    /// The battle menu is a sure sign of battles, held a few seconds when it
    /// hides (an attack); the sound settles the rest.
    #[test]
    fn indicators_are_sure_signs_held_a_while() {
        let mut phases = decls(&["battle", "explore"]);
        phases[0].indicator = Some("battle_menu".into());
        phases[0].hold = 4.0;
        let mut tracker = PhaseTracker::new(&phases, 4.0);
        // Exploring music, no menu: explore.
        tracker.set_references(Sense::Sound, vec![("battle".into(), unit(0)), ("explore".into(), unit(1))]);
        assert_eq!(tracker.indicator(0.0, "battle_menu", false), None);
        assert_eq!(tracker.update(0.0, Sense::Sound, &between(1.0)).unwrap().phase.as_deref(), Some("explore"));
        // The menu shows, with the same music: battle, once it stayed half a second.
        assert_eq!(tracker.indicator(1.0, "battle_menu", true), None);
        let change = tracker.tick(1.6).unwrap();
        assert_eq!((change.phase.as_deref(), change.confidence), (Some("battle"), 1.0));
        // Battle music alone never enters battle while indicators are read.
        tracker.indicator(2.0, "battle_menu", false);
        for t in 0..3 {
            tracker.update(2.0 + t as f64, Sense::Sound, &between(0.0));
        }
        // It hides during an attack: battle is held...
        assert_eq!(tracker.tick(5.0), None);
        assert_eq!(tracker.current(), Some("battle"));
        // ...then left for the best guess, here none (battle music, but battle needs its menu).
        let change = tracker.tick(6.5).unwrap();
        assert_eq!((change.phase, change.previous.as_deref()), (None, Some("battle")));
        // Without the image, the sound alone can say battle again.
        tracker.forget(Sense::Screen);
        let change = tracker.update(7.0, Sense::Sound, &between(0.0)).unwrap();
        assert_eq!(change.phase.as_deref(), Some("battle"));
    }
}
