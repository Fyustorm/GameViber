//! Game profiles (docs/spec-modes.md §6.5): what the player taught GameViber
//! about one game, used by every mode while that game runs — captures of its
//! scenes (images the player took, which are also the examples scenes are
//! recognized with), zones of its screen drawn on them, and the values and
//! events other programs send for it. One JSON file per game in `games/`,
//! named after the game's executable, and its captures as PNG files in a
//! directory of the same name.

use std::fs;
use std::io::BufReader;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config;
use crate::screen::Frame;

/// A profile keeps at most this many captures per scene.
pub const MAX_CAPTURES: usize = 40;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// Executable name of the game, as the in-game overlay reports it.
    pub game: String,
    pub zones: Vec<Zone>,
    /// Images of the game the player captured, by scene.
    pub captures: Vec<Capture>,
    /// Values and events other programs send while this game runs (§6.5), for
    /// the requests to AI assistants.
    pub inputs: Vec<InputDecl>,
}

/// An image of the game the player captured as an example of a scene.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Capture {
    /// PNG file name in the profile's directory.
    pub file: String,
    /// Empty for a capture to sort (taken in game, filed later).
    pub scene: String,
    /// Its embedding (`screen::clip`); empty until the image model computed it.
    pub embedding: Vec<f32>,
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
    /// Name modes read it by (`input.zones.<name>`). Zones sharing a name are
    /// places of one zone (a bar shown elsewhere out of battles): its value
    /// comes from the place where it is found.
    pub name: String,
    pub kind: ZoneKind,
    /// Left, top, width, height, as fractions of the screen.
    pub rect: [f32; 4],
    /// Visible: grayscale look of the zone when it was drawn (`screen::zones`).
    pub reference: Vec<u8>,
    /// Visible: similarity with the reference (-1..1) above which the element is shown.
    pub threshold: f32,
    /// Bar: the color of its filled part...
    pub color: [u8; 3],
    /// ...and of its empty part, when the player picked it. With it, the bar is
    /// found in the zone as the longest run of its two colors, so the zone may
    /// be larger than the bar, or cover every place a moving bar can be.
    pub empty_color: Option<[u8; 3]>,
    /// Bar: share of the zone's length it covered when drawn. Much less of its
    /// colors found means it is not on screen (menus): its value is unknown.
    pub length: f32,
    /// The scene of the capture it was drawn on (shown there, for a visible zone).
    pub scene: Option<String>,
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
            empty_color: None,
            length: 0.0,
            scene: None,
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

#[cfg(test)]
thread_local! {
    /// Tests keep their profiles out of the player's.
    static TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

fn dir() -> PathBuf {
    #[cfg(test)]
    if let Some(dir) = TEST_DIR.with(|d| d.borrow().clone()) {
        return dir;
    }
    config::config_dir().join("games")
}

fn stem(game: &str) -> String {
    game.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect()
}

fn path(game: &str) -> PathBuf {
    dir().join(format!("{}.json", stem(game)))
}

/// Where a game's captures are.
fn captures_dir(game: &str) -> PathBuf {
    dir().join(stem(game))
}

/// Overlays before 0.1 named every Proton game after Wine's loader: the
/// first Windows game seen afterwards takes that profile over.
fn migrate_wine_profile(game: &str) {
    const WINE: &str = "wine64-preloader";
    if !game.to_lowercase().ends_with(".exe") || path(game).exists() || !path(WINE).exists() {
        return;
    }
    let Ok(text) = fs::read_to_string(path(WINE)) else { return };
    let text = text.replacen(&format!("\"game\":\"{WINE}\""), &format!("\"game\":{}", serde_json::json!(game)), 1);
    if config::write_file(&path(game), &text).is_ok() {
        let _ = fs::remove_file(path(WINE));
        if captures_dir(WINE).exists() {
            let _ = fs::rename(captures_dir(WINE), captures_dir(game));
        }
        log::info!("the profile of {WINE} is now the profile of {game}");
    }
}

/// Saves `frame` as a PNG capture of `game`; returns its file name.
pub fn save_capture(game: &str, scene: &str, frame: &Frame) -> std::io::Result<String> {
    let dir = captures_dir(game);
    config::create_dir(&dir)?;
    let stamp = crate::session::local_time().replace([' ', ':'], "-");
    let label = if scene.is_empty() { "capture" } else { scene };
    let mut file = format!("{label}-{stamp}.png");
    let mut n = 2;
    while dir.join(&file).exists() {
        file = format!("{label}-{stamp}-{n}.png");
        n += 1;
    }
    let out = fs::File::create(dir.join(&file))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(out), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(&frame.pixels).map_err(std::io::Error::other)?;
    writer.finish().map_err(std::io::Error::other)?;
    Ok(file)
}

/// A capture of `game`, as a frame.
pub fn load_capture(game: &str, file: &str) -> Option<Frame> {
    let reader = BufReader::new(fs::File::open(captures_dir(game).join(file)).ok()?);
    let mut decoder = png::Decoder::new(reader);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buffer.truncate(info.buffer_size());
    Some(Frame { width: info.width, height: info.height, source_width: info.width, source_height: info.height, count: 0, pixels: buffer })
}

fn delete_capture(game: &str, file: &str) {
    if let Err(e) = fs::remove_file(captures_dir(game).join(file)) {
        log::warn!("cannot delete the capture {file}: {e}");
    }
}

impl Profile {
    /// The profile of `game`, or an empty one.
    pub fn load(game: &str) -> Self {
        migrate_wine_profile(game);
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

    /// Captures `frame` as an example of `scene`; the oldest of the scene goes past `MAX_CAPTURES`.
    pub fn add_capture(&mut self, scene: &str, frame: &Frame) -> std::io::Result<()> {
        let file = save_capture(&self.game, scene, frame)?;
        self.captures.push(Capture { file, scene: scene.to_owned(), embedding: Vec::new() });
        let of_scene: Vec<usize> = self.captures.iter().enumerate().filter(|(_, c)| c.scene == scene).map(|(i, _)| i).collect();
        if of_scene.len() > MAX_CAPTURES {
            self.remove_capture(&self.captures[of_scene[0]].file.clone());
        }
        Ok(())
    }

    /// Forgets a capture and deletes its file.
    pub fn remove_capture(&mut self, file: &str) {
        if let Some(i) = self.captures.iter().position(|c| c.file == file) {
            self.captures.remove(i);
            delete_capture(&self.game, file);
        }
    }

    /// Scene names of the captures, sorted (captures to sort left out).
    pub fn scenes(&self) -> Vec<String> {
        let mut scenes: Vec<String> = self.captures.iter().filter(|c| !c.scene.is_empty()).map(|c| c.scene.clone()).collect();
        scenes.sort();
        scenes.dedup();
        scenes
    }

    /// The mean of each scene's capture embeddings: what the image is compared with.
    pub fn example_centroids(&self) -> Vec<(String, crate::models::Embedding)> {
        self.scenes()
            .into_iter()
            .filter_map(|scene| {
                let embeddings: Vec<&Vec<f32>> = self.captures.iter().filter(|c| c.scene == scene && !c.embedding.is_empty()).map(|c| &c.embedding).collect();
                let len = embeddings.first()?.len();
                let mut mean = vec![0f32; len];
                for e in embeddings.iter().filter(|e| e.len() == len) {
                    mean.iter_mut().zip(e.iter()).for_each(|(m, v)| *m += v);
                }
                Some((scene, crate::models::normalize(&mean)))
            })
            .collect()
    }

    /// How the profile reads for an AI assistant writing a mode for this game (§6.5).
    pub fn describe(&self) -> String {
        let mut out = String::new();
        if !self.zones.is_empty() {
            out.push_str("Zones of the screen (`input.zones`, `on_zone`):\n");
            let mut seen = Vec::new();
            for z in &self.zones {
                // A zone drawn in several places is one input.
                if seen.contains(&&z.name) {
                    continue;
                }
                seen.push(&z.name);
                let what = match z.kind {
                    ZoneKind::Visible => "true while shown, false otherwise".to_owned(),
                    ZoneKind::Bar => "how full the bar is, 0 to 1, or nil while it is not on screen (menus, cutscenes)".to_owned(),
                };
                out.push_str(&format!("- `{}`: {what}\n", z.name));
            }
        }
        let scenes: Vec<String> = self
            .scenes()
            .iter()
            .map(|name| format!("`{name}` ({} images)", self.captures.iter().filter(|c| c.scene == *name).count()))
            .collect();
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
    fn captures_are_saved_capped_and_described() {
        let game = format!("test-game-{}.exe", std::process::id());
        let dir = std::env::temp_dir().join(format!("gameviber-profile-{}", std::process::id()));
        TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.clone()));
        let mut profile = Profile::load(&game);
        let frame = |v: u8| Frame { width: 4, height: 2, source_width: 4, source_height: 2, count: 0, pixels: vec![v; 32] };
        for i in 0..MAX_CAPTURES + 2 {
            profile.add_capture("battle", &frame(i as u8)).unwrap();
        }
        profile.add_capture("story", &frame(200)).unwrap();
        assert_eq!(profile.captures.iter().filter(|c| c.scene == "battle").count(), MAX_CAPTURES);
        assert_eq!(profile.scenes(), ["battle", "story"]);
        let first = &profile.captures[0];
        assert_eq!(load_capture(&game, &first.file).unwrap().pixels, vec![2; 32], "the two oldest went first");
        assert!(profile.example_centroids().is_empty(), "no embedding yet");
        profile.captures[0].embedding = vec![0.0, 2.0];
        assert_eq!(profile.example_centroids(), vec![("battle".to_owned(), crate::models::normalize(&[0.0, 1.0]))]);

        profile.zones.push(Zone { name: "hp".into(), kind: ZoneKind::Bar, ..Zone::default() });
        profile.inputs.push(InputDecl { name: "kill".into(), kind: InputKind::Event, description: "an enemy died".into() });
        let text = profile.describe();
        assert!(text.contains("`hp`: how full the bar is"), "{text}");
        assert!(text.contains("`battle` (40 images)"), "{text}");
        assert!(text.contains("event `kill`: an enemy died"), "{text}");
        profile.save();
        assert_eq!(Profile::load(&game), profile);
        let story = profile.captures.last().unwrap().file.clone();
        profile.remove_capture(&story);
        assert!(load_capture(&game, &story).is_none(), "its file is deleted");
        let _ = fs::remove_dir_all(dir);
    }
}
