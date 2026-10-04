//! Game profiles (docs/spec-modes.md §6.5): what the player taught GameViber
//! about one game, used by every mode while that game runs — zones of its
//! screen, example images of its scenes, and the values and events other
//! programs send for it. One JSON file per game in `games/`, named after the
//! game's executable.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config;

/// A profile keeps at most this many examples per scene (the oldest go first).
pub const MAX_EXAMPLES: usize = 40;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// Executable name of the game, as the in-game overlay reports it.
    pub game: String,
    pub zones: Vec<Zone>,
    /// Scene name -> embeddings of example images (`screen::clip`).
    pub examples: BTreeMap<String, Vec<Vec<f32>>>,
    /// Values and events other programs send while this game runs (§6.5), for
    /// the requests to AI assistants.
    pub inputs: Vec<InputDecl>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZoneKind {
    /// Is this element on screen? Compared with how it looked when the zone was drawn.
    #[default]
    Visible,
    /// How full is this bar? Measured with the bar's color.
    Bar,
}

impl ZoneKind {
    pub fn label(self) -> &'static str {
        match self {
            ZoneKind::Visible => "Shown or not",
            ZoneKind::Bar => "Bar (how full)",
        }
    }
}

/// Where a bar fills towards.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    #[default]
    Right,
    Left,
    Up,
    Down,
}

impl Direction {
    pub const ALL: [Direction; 4] = [Direction::Right, Direction::Left, Direction::Up, Direction::Down];

    pub fn label(self) -> &'static str {
        match self {
            Direction::Right => "Fills to the right",
            Direction::Left => "Fills to the left",
            Direction::Up => "Fills upwards",
            Direction::Down => "Fills downwards",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Zone {
    /// Name modes read it by (`input.zones.<name>`).
    pub name: String,
    pub kind: ZoneKind,
    /// Left, top, width, height, as fractions of the screen.
    pub rect: [f32; 4],
    /// Visible: grayscale look of the zone when it was drawn (`screen::zones`).
    pub reference: Vec<u8>,
    /// Visible: similarity with the reference (-1..1) above which the element is shown.
    pub threshold: f32,
    /// Bar: the color of its filled part.
    pub color: [u8; 3],
    pub direction: Direction,
    /// Bar: how far (0..255 per channel) a pixel may be from `color`.
    pub tolerance: f32,
}

impl Default for Zone {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: ZoneKind::Visible,
            rect: [0.0, 0.0, 0.1, 0.1],
            reference: Vec::new(),
            threshold: 0.45,
            color: [0, 0, 0],
            direction: Direction::Right,
            tolerance: 60.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    /// A value kept in `input.custom.<name>`.
    #[default]
    Value,
    /// An event passed to `on_event`.
    Event,
}

/// A value or an event another program sends, declared so that AI assistants know it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputDecl {
    pub name: String,
    pub kind: InputKind,
    /// What it means and its range ("health, 0 to 100").
    pub description: String,
}

/// Names modes use: letters, digits and underscores, starting with a letter.
pub fn valid_name(name: &str) -> bool {
    name.len() <= 32
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn dir() -> PathBuf {
    config::config_dir().join("games")
}

fn path(game: &str) -> PathBuf {
    let stem: String = game.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    dir().join(format!("{stem}.json"))
}

impl Profile {
    /// The profile of `game`, or an empty one.
    pub fn load(game: &str) -> Self {
        let profile = fs::read_to_string(path(game)).ok().and_then(|text| match serde_json::from_str::<Profile>(&text) {
            Ok(p) => Some(p),
            Err(e) => {
                log::warn!("ignoring invalid profile of {game}: {e}");
                None
            }
        });
        Profile { game: game.to_owned(), ..profile.unwrap_or_default() }
    }

    pub fn save(&self) {
        let result = serde_json::to_string(self).map_err(std::io::Error::other).and_then(|text| config::write_file(&path(&self.game), &text));
        if let Err(e) = result {
            log::warn!("cannot save the profile of {}: {e}", self.game);
        }
    }

    pub fn add_example(&mut self, scene: &str, embedding: &[f32]) {
        let examples = self.examples.entry(scene.to_owned()).or_default();
        examples.push(embedding.to_vec());
        if examples.len() > MAX_EXAMPLES {
            examples.remove(0);
        }
    }

    /// The mean of each scene's examples: what the image is compared with.
    pub fn example_centroids(&self) -> Vec<(String, crate::models::Embedding)> {
        self.examples
            .iter()
            .filter(|(_, examples)| !examples.is_empty())
            .map(|(scene, examples)| {
                let len = examples[0].len();
                let mut mean = vec![0f32; len];
                for e in examples.iter().filter(|e| e.len() == len) {
                    mean.iter_mut().zip(e).for_each(|(m, v)| *m += v);
                }
                (scene.clone(), crate::models::normalize(&mean))
            })
            .collect()
    }

    /// How the profile reads for an AI assistant writing a mode for this game (§6.5).
    pub fn describe(&self) -> String {
        let mut out = String::new();
        if !self.zones.is_empty() {
            out.push_str("Zones of the screen (`input.zones`, `on_zone`):\n");
            for z in &self.zones {
                let what = match z.kind {
                    ZoneKind::Visible => "true while shown, false otherwise".to_owned(),
                    ZoneKind::Bar => "how full the bar is, 0 to 1".to_owned(),
                };
                out.push_str(&format!("- `{}`: {what}\n", z.name));
            }
        }
        let scenes: Vec<String> =
            self.examples.iter().filter(|(_, e)| !e.is_empty()).map(|(name, e)| format!("`{name}` ({} images)", e.len())).collect();
        if !scenes.is_empty() {
            out.push_str(&format!(
                "Scenes with example images (declare them in `scenes` with these names): {}\n",
                scenes.join(", ")
            ));
        }
        if !self.inputs.is_empty() {
            out.push_str("Sent by another program (`input.custom`, `on_event`):\n");
            for i in &self.inputs {
                let kind = match i.kind {
                    InputKind::Value => format!("`input.custom.{}`", i.name),
                    InputKind::Event => format!("event `{}`", i.name),
                };
                out.push_str(&format!("- {kind}: {}\n", i.description));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked() {
        assert!(valid_name("battle_hud"));
        assert!(valid_name("hp2"));
        assert!(!valid_name("2hp"));
        assert!(!valid_name("health bar"));
        assert!(!valid_name(""));
    }

    #[test]
    fn examples_are_capped_and_described() {
        let mut profile = Profile { game: "game.exe".into(), ..Profile::default() };
        for i in 0..MAX_EXAMPLES + 3 {
            profile.add_example("battle", &[i as f32]);
        }
        assert_eq!(profile.examples["battle"].len(), MAX_EXAMPLES);
        assert_eq!(profile.examples["battle"][0], vec![3.0], "the oldest went first");
        profile.zones.push(Zone { name: "hp".into(), kind: ZoneKind::Bar, ..Zone::default() });
        profile.inputs.push(InputDecl { name: "kill".into(), kind: InputKind::Event, description: "an enemy died".into() });
        let text = profile.describe();
        assert!(text.contains("`hp`: how full the bar is"), "{text}");
        assert!(text.contains("`battle` (40 images)"), "{text}");
        assert!(text.contains("event `kill`: an enemy died"), "{text}");
        let json = serde_json::to_string(&profile).unwrap();
        assert_eq!(serde_json::from_str::<Profile>(&json).unwrap(), profile);
    }
}
