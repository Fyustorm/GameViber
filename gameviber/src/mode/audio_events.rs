//! Audio scenes (§6.3): the probabilities of each scene a mode declares,
//! averaged over a few seconds, and the scene they settle on, with enough
//! hysteresis that a few ambiguous seconds do not flip it.

use std::collections::VecDeque;

use crate::audio::clap::{self, Embedding};

pub const DEFAULT_WINDOW: f64 = 10.0;
/// A scene is entered when its average probability reaches this...
pub const ENTER: f64 = 0.5;
/// ...and, when another scene is current, beats it by this much.
pub const MARGIN: f64 = 0.1;

#[derive(Debug, Clone, PartialEq)]
pub struct SceneChange {
    pub scene: Option<String>,
    pub previous: Option<String>,
    pub confidence: f64,
}

pub struct SceneTracker {
    names: Vec<String>,
    /// Text embeddings of the descriptions, once computed.
    texts: Option<Vec<Embedding>>,
    window: f64,
    history: VecDeque<(f64, Vec<f64>)>,
    average: Vec<f64>,
    current: Option<usize>,
}

impl SceneTracker {
    pub fn new(names: Vec<String>, window: f64) -> Self {
        let average = vec![0.0; names.len()];
        Self { names, texts: None, window, history: VecDeque::new(), average, current: None }
    }

    pub fn set_texts(&mut self, texts: Vec<Embedding>) {
        if texts.len() == self.names.len() {
            self.texts = Some(texts);
        }
    }

    pub fn ready(&self) -> bool {
        self.texts.is_some()
    }

    /// A new embedding of the game's sound at `time`.
    pub fn update(&mut self, time: f64, sound: &[f32]) -> Option<SceneChange> {
        let texts = self.texts.as_ref()?;
        self.history.push_back((time, clap::probabilities(sound, texts)));
        while self.history.front().is_some_and(|(t, _)| time - t >= self.window) {
            self.history.pop_front();
        }
        let n = self.history.len() as f64;
        self.average = (0..self.names.len()).map(|i| self.history.iter().map(|(_, p)| p[i]).sum::<f64>() / n).collect();

        let best = (0..self.names.len()).max_by(|&a, &b| self.average[a].total_cmp(&self.average[b]))?;
        let switch = match self.current {
            Some(current) if current == best => false,
            Some(current) => self.average[best] >= ENTER && self.average[best] - self.average[current] >= MARGIN,
            None => self.average[best] >= ENTER,
        };
        if !switch {
            return None;
        }
        let previous = self.current.replace(best);
        Some(SceneChange {
            scene: Some(self.names[best].clone()),
            previous: previous.map(|i| self.names[i].clone()),
            confidence: self.average[best],
        })
    }

    /// The sound stopped: forget it. Returns the change when a scene was current.
    pub fn reset(&mut self) -> Option<SceneChange> {
        self.history.clear();
        self.average.iter_mut().for_each(|p| *p = 0.0);
        let previous = self.current.take()?;
        Some(SceneChange { scene: None, previous: Some(self.names[previous].clone()), confidence: 0.0 })
    }

    pub fn current(&self) -> Option<&str> {
        self.current.map(|i| self.names[i].as_str())
    }

    /// Average probability of each scene, in the order of the names given.
    pub fn averages(&self) -> impl Iterator<Item = (&str, f64)> {
        self.names.iter().map(String::as_str).zip(self.average.iter().copied())
    }

    pub fn confidence(&self) -> f64 {
        self.current.map_or(0.0, |i| self.average[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(i: usize) -> Embedding {
        (0..3).map(|k| if k == i { 1.0 } else { 0.0 }).collect()
    }

    /// A sound between the scenes 0 and 1, `towards_one` 0..1.
    fn sound(towards_one: f32) -> Vec<f32> {
        let (a, b) = (1.0 - towards_one, towards_one);
        let norm = (a * a + b * b).sqrt();
        vec![a / norm, b / norm, 0.0]
    }

    #[test]
    fn scenes_settle_and_change_with_hysteresis() {
        let mut tracker = SceneTracker::new(vec!["battle".into(), "calm".into()], 10.0);
        assert_eq!(tracker.update(0.0, &sound(0.0)), None, "no texts yet");
        tracker.set_texts(vec![unit(0), unit(1)]);
        let change = tracker.update(2.0, &sound(0.0)).unwrap();
        assert_eq!(change.scene.as_deref(), Some("battle"));
        assert_eq!(change.previous, None);
        assert!(change.confidence > 0.99);

        // Ambiguous sound for a while: battle stays.
        for t in 2..5 {
            assert_eq!(tracker.update(2.0 + 2.0 * t as f64, &sound(0.5)), None);
        }
        // Calm for good: it takes over once it dominates the window.
        let mut changed_at = None;
        for t in 0..10 {
            let time = 12.0 + 2.0 * t as f64;
            if let Some(change) = tracker.update(time, &sound(1.0)) {
                assert_eq!(change.scene.as_deref(), Some("calm"));
                assert_eq!(change.previous.as_deref(), Some("battle"));
                changed_at = Some(time);
                break;
            }
        }
        let changed_at = changed_at.expect("calm is entered");
        assert!((12.0..=18.0).contains(&changed_at), "{changed_at}");
        assert_eq!(tracker.current(), Some("calm"));

        let reset = tracker.reset().unwrap();
        assert_eq!((reset.scene, reset.previous.as_deref()), (None, Some("calm")));
        assert_eq!(tracker.current(), None);
    }
}
