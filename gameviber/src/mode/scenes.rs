//! Scenes (§6.3): what each sense says about the scenes a mode declares —
//! the game's sound against their sound descriptions, its image against
//! their screen descriptions and against example images from the game's
//! profile — fused, averaged over a few seconds, and settled on with enough
//! hysteresis that a few ambiguous seconds do not flip the scene. A scene of
//! the game can also be tied to a zone (its battle menu): the zone shown is a
//! sure sign of it, ahead of the guesses. Each scene can be held a while after
//! its last sign, for signs that come and go.

use std::collections::VecDeque;

use crate::models::{self, Embedding};

pub const DEFAULT_WINDOW: f64 = 10.0;
/// A scene is entered when its average probability reaches this...
pub const ENTER: f64 = 0.5;
/// ...and, when another scene is current, beats it by this much.
pub const MARGIN: f64 = 0.1;
/// Sharpness of the comparison of an image with example images.
const EXAMPLE_SCALE: f64 = 50.0;
/// A zone must be shown this long to be a sign of its scene (not a frame of a transition).
const ZONE_CONFIRM: f64 = 0.5;

/// A scene a mode declares: a name and how it sounds and looks.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneDecl {
    pub name: String,
    pub sound: Option<String>,
    pub screen: Option<String>,
    /// A zone of the game whose showing is a sure sign of the scene.
    pub zone: Option<String>,
    /// Seconds the scene is kept after its last sign.
    pub hold: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sense {
    /// The sound against the sound descriptions.
    Sound,
    /// The image against the screen descriptions.
    Screen,
    /// The image against the profile's example images.
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
pub struct SceneChange {
    pub scene: Option<String>,
    pub previous: Option<String>,
    pub confidence: f64,
}

/// What one sense compares with: the scenes it can speak about and a reference vector for each.
#[derive(Default)]
struct References {
    scenes: Vec<usize>,
    vectors: Vec<Embedding>,
    /// The last evidence: when, and a likelihood per scene (1 = says nothing).
    latest: Option<(f64, Vec<f64>)>,
}

pub struct SceneTracker {
    names: Vec<String>,
    senses: [References; 3],
    window: f64,
    history: VecDeque<(f64, Vec<f64>)>,
    average: Vec<f64>,
    current: Option<usize>,
    /// Per scene: its zone, how long it is held, since when its zone is shown,
    /// and the last time something said it (its zone, or the best average).
    zones: Vec<Option<String>>,
    holds: Vec<f64>,
    shown_since: Vec<Option<f64>>,
    last_sign: Vec<f64>,
    /// Zones are being read (the image is there): scenes tied to one are only
    /// entered through it.
    zones_read: bool,
}

impl SceneTracker {
    pub fn new(scenes: &[SceneDecl], window: f64) -> Self {
        let n = scenes.len();
        Self {
            names: scenes.iter().map(|s| s.name.clone()).collect(),
            senses: Default::default(),
            window,
            history: VecDeque::new(),
            average: vec![0.0; n],
            current: None,
            zones: scenes.iter().map(|s| s.zone.clone()).collect(),
            holds: scenes.iter().map(|s| s.hold.max(0.0)).collect(),
            shown_since: vec![None; n],
            last_sign: vec![f64::NEG_INFINITY; n],
            zones_read: false,
        }
    }

    fn index(sense: Sense) -> usize {
        Sense::ALL.iter().position(|s| *s == sense).unwrap()
    }

    /// Reference vectors of `sense`: one per named scene (unknown names are ignored).
    pub fn set_references(&mut self, sense: Sense, references: Vec<(String, Embedding)>) {
        let mut r = References::default();
        for (name, vector) in references {
            if let Some(i) = self.names.iter().position(|n| *n == name) {
                r.scenes.push(i);
                r.vectors.push(vector);
            }
        }
        self.senses[Self::index(sense)] = r;
    }

    pub fn has(&self, sense: Sense) -> bool {
        !self.senses[Self::index(sense)].scenes.is_empty()
    }

    /// Some sense can recognize scenes.
    #[cfg(test)]
    pub fn ready(&self) -> bool {
        Sense::ALL.iter().any(|s| self.has(*s))
    }

    /// New evidence from `sense` at `time`: an embedding of the sound or the image.
    pub fn update(&mut self, time: f64, sense: Sense, vector: &[f32]) -> Option<SceneChange> {
        let n = self.names.len();
        let r = &mut self.senses[Self::index(sense)];
        if r.scenes.is_empty() {
            return None;
        }
        // Among the scenes it knows, a sense shares out the likelihood; it says
        // nothing (1) about the others.
        let p = models::probabilities(vector, &r.vectors, sense.scale());
        let mut likelihood = vec![1.0; n];
        for (k, &i) in r.scenes.iter().enumerate() {
            likelihood[i] = p[k] * r.scenes.len() as f64;
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

    /// A zone's new state: shown (a bar on screen counts) or not.
    pub fn zone(&mut self, time: f64, name: &str, shown: bool) -> Option<SceneChange> {
        self.zones_read = true;
        for i in 0..self.names.len() {
            if self.zones[i].as_deref() == Some(name) {
                if !shown && self.shown_since[i].is_some_and(|t| time - t >= ZONE_CONFIRM) {
                    // The hold counts from when it went.
                    self.last_sign[i] = time;
                }
                self.shown_since[i] = if shown { self.shown_since[i].or(Some(time)) } else { None };
            }
        }
        self.decide(time)
    }

    /// Time passes: a zone shown long enough enters its scene, a hold runs out.
    pub fn tick(&mut self, time: f64) -> Option<SceneChange> {
        if self.zones.iter().all(Option::is_none) && self.holds.iter().all(|h| *h <= 0.0) {
            return None;
        }
        self.decide(time)
    }

    /// A scene is tied to a zone, and zones are being read.
    fn by_zone_only(&self, i: usize) -> bool {
        self.zones_read && self.zones[i].is_some()
    }

    fn settle(&mut self, time: f64, probabilities: Vec<f64>) -> Option<SceneChange> {
        self.history.push_back((time, probabilities));
        while self.history.front().is_some_and(|(t, _)| time - t >= self.window) {
            self.history.pop_front();
        }
        let n = self.history.len() as f64;
        self.average = (0..self.names.len()).map(|i| self.history.iter().map(|(_, p)| p[i]).sum::<f64>() / n).collect();
        self.decide(time)
    }

    fn decide(&mut self, time: f64) -> Option<SceneChange> {
        // Zones shown long enough are sure signs.
        let mut by_zone = None;
        for i in 0..self.names.len() {
            if self.shown_since[i].is_some_and(|t| time - t >= ZONE_CONFIRM) {
                self.last_sign[i] = time;
                if by_zone.is_none() || self.current == Some(i) {
                    by_zone = Some(i);
                }
            }
        }
        // Then the best guess among the scenes not tied to a zone being read.
        let best = (0..self.names.len())
            .filter(|&i| !self.by_zone_only(i))
            .max_by(|&a, &b| self.average[a].total_cmp(&self.average[b]))
            .filter(|&i| self.average[i] > 0.0);
        if let Some(best) = best.filter(|&b| by_zone.is_none() && self.average[b] >= ENTER) {
            self.last_sign[best] = time;
        }
        let held = self.current.is_some_and(|c| time - self.last_sign[c] < self.holds[c]);
        let next = match (by_zone, self.current) {
            (Some(z), _) => Some(z),
            (None, Some(_)) if held => return None,
            // The current scene's zone is gone (and its hold over): the best guess, or no scene.
            (None, Some(c)) if self.by_zone_only(c) => best.filter(|&b| self.average[b] >= ENTER),
            (None, Some(c)) => best.filter(|&b| b != c && self.average[b] >= ENTER && self.average[b] - self.average[c] >= MARGIN).or(Some(c)),
            (None, None) => best.filter(|&b| self.average[b] >= ENTER),
        };
        if next == self.current {
            return None;
        }
        let previous = std::mem::replace(&mut self.current, next);
        Some(SceneChange {
            scene: next.map(|i| self.names[i].clone()),
            previous: previous.map(|i| self.names[i].clone()),
            confidence: self.confidence(),
        })
    }

    /// A sense stopped (the sound or the image is gone). When no sense is
    /// left, the scene is forgotten: returns the change when one was current.
    pub fn forget(&mut self, sense: Sense) -> Option<SceneChange> {
        self.senses[Self::index(sense)].latest = None;
        if sense == Sense::Screen {
            // The zones are read on the image.
            self.zones_read = false;
            self.shown_since.iter_mut().for_each(|s| *s = None);
        }
        if self.senses.iter().any(|r| r.latest.is_some()) {
            return None;
        }
        self.reset()
    }

    pub fn reset(&mut self) -> Option<SceneChange> {
        self.history.clear();
        self.senses.iter_mut().for_each(|r| r.latest = None);
        self.zones_read = false;
        self.shown_since.iter_mut().for_each(|s| *s = None);
        self.last_sign.iter_mut().for_each(|t| *t = f64::NEG_INFINITY);
        self.average.iter_mut().for_each(|p| *p = 0.0);
        let previous = self.current.take()?;
        Some(SceneChange { scene: None, previous: Some(self.names[previous].clone()), confidence: 0.0 })
    }

    pub fn current(&self) -> Option<&str> {
        self.current.map(|i| self.names[i].as_str())
    }

    /// Probability of each scene, in the order of the names given: 1 for a
    /// scene whose zone is shown, else its average.
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

    fn decls(names: &[&str]) -> Vec<SceneDecl> {
        names.iter().map(|n| SceneDecl { name: (*n).to_owned(), sound: None, screen: None, zone: None, hold: 0.0 }).collect()
    }

    fn names() -> Vec<SceneDecl> {
        decls(&["battle", "calm"])
    }

    #[test]
    fn scenes_settle_and_change_with_hysteresis() {
        let mut tracker = SceneTracker::new(&names(), 10.0);
        assert_eq!(tracker.update(0.0, Sense::Sound, &between(0.0)), None, "no references yet");
        tracker.set_references(Sense::Sound, vec![("battle".into(), unit(0)), ("calm".into(), unit(1))]);
        let change = tracker.update(2.0, Sense::Sound, &between(0.0)).unwrap();
        assert_eq!(change.scene.as_deref(), Some("battle"));
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
                assert_eq!(change.scene.as_deref(), Some("calm"));
                assert_eq!(change.previous.as_deref(), Some("battle"));
                changed_at = Some(time);
                break;
            }
        }
        let changed_at = changed_at.expect("calm is entered");
        assert!((12.0..=18.0).contains(&changed_at), "{changed_at}");
        assert_eq!(tracker.current(), Some("calm"));

        let reset = tracker.forget(Sense::Sound).unwrap();
        assert_eq!((reset.scene, reset.previous.as_deref()), (None, Some("calm")));
        assert_eq!(tracker.current(), None);
    }

    /// The sound cannot tell two scenes apart; the image can.
    #[test]
    fn senses_are_fused() {
        let mut tracker = SceneTracker::new(&decls(&["battle", "dungeon", "story"]), 4.0);
        // One sound description for both action scenes (the same music), one for story.
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
        // The image goes away: the sound alone remains, the scene is kept.
        assert_eq!(tracker.forget(Sense::Examples), None);
        assert_eq!(tracker.current(), Some("battle"));
    }

    /// The battle menu is a sure sign of battles, held a few seconds when it
    /// hides (an attack); the sound settles the rest.
    #[test]
    fn zones_are_sure_signs_held_a_while() {
        let mut scenes = decls(&["battle", "explore"]);
        scenes[0].zone = Some("battle_menu".into());
        scenes[0].hold = 4.0;
        let mut tracker = SceneTracker::new(&scenes, 4.0);
        // Exploring music, no menu: explore.
        tracker.set_references(Sense::Sound, vec![("battle".into(), unit(0)), ("explore".into(), unit(1))]);
        assert_eq!(tracker.zone(0.0, "battle_menu", false), None);
        assert_eq!(tracker.update(0.0, Sense::Sound, &between(1.0)).unwrap().scene.as_deref(), Some("explore"));
        // The menu shows, with the same music: battle, once it stayed half a second.
        assert_eq!(tracker.zone(1.0, "battle_menu", true), None);
        let change = tracker.tick(1.6).unwrap();
        assert_eq!((change.scene.as_deref(), change.confidence), (Some("battle"), 1.0));
        // Battle music alone never enters battle while zones are read.
        tracker.zone(2.0, "battle_menu", false);
        for t in 0..3 {
            tracker.update(2.0 + t as f64, Sense::Sound, &between(0.0));
        }
        // It hides during an attack: battle is held...
        assert_eq!(tracker.tick(5.0), None);
        assert_eq!(tracker.current(), Some("battle"));
        // ...then left for the best guess, here none (battle music, but battle needs its menu).
        let change = tracker.tick(6.5).unwrap();
        assert_eq!((change.scene, change.previous.as_deref()), (None, Some("battle")));
        // Without the image, the sound alone can say battle again.
        tracker.forget(Sense::Screen);
        let change = tracker.update(7.0, Sense::Sound, &between(0.0)).unwrap();
        assert_eq!(change.scene.as_deref(), Some("battle"));
    }
}
