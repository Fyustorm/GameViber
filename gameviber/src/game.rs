//! Games (docs/spec-modes.md §6.3, §6.5): what the player set up for one
//! game, identified by its name, and shared by all its modes — its scenes,
//! captures of them (images the player took, which are also the examples
//! scenes are recognized with), zones of its screen drawn on them, the values
//! and events other programs send for it, which sound to listen to, and its
//! modes. A game may be linked to the executables it runs as, so that it
//! becomes the active game by itself. One JSON file per game in `games/`,
//! named after its id, and its captures as PNG files in a directory of the
//! same name.

use std::fs;
use std::io::BufReader;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::{self, AudioSource};
use crate::mode::scenes::SceneDecl;
use crate::screen::Frame;

/// A game keeps at most this many captures per scene.
pub const MAX_CAPTURES: usize = 40;
/// A game has at most this many scenes (as modes may declare).
pub const MAX_SCENES: usize = 8;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Game {
    /// File name stem, set when loaded: it stays when the game is renamed.
    #[serde(skip)]
    pub id: String,
    pub name: String,
    /// Executables it runs as, as the in-game overlay reports them
    /// (`METAPHOR.exe`); none: the player picks the game by hand.
    pub executables: Vec<String>,
    /// Its scenes (§6.3), shared by its modes.
    pub scenes: Vec<SceneDef>,
    pub zones: Vec<Zone>,
    /// Images of the game the player captured, by scene.
    pub captures: Vec<Capture>,
    /// Values and events other programs send while this game runs (§6.5), for
    /// the requests to AI assistants.
    pub inputs: Vec<InputDecl>,
    /// Its modes, by id (user mode files, built-in modes).
    pub modes: Vec<String>,
    /// Which sound to listen to while it is played; None: the default (Setup).
    pub audio: Option<AudioSource>,
}

/// A scene of a game: a name modes read (`input.scene`), and how it sounds
/// for the sound model; how it looks comes from its captures, and optionally
/// a description for the image model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneDef {
    pub name: String,
    pub sound: Option<String>,
    pub screen: Option<String>,
    /// A zone whose showing is a sure sign of the scene (its battle menu).
    pub zone: Option<String>,
    /// Seconds the scene is kept after its last sign: longer for signs that come and go.
    pub hold: f64,
}

impl Default for SceneDef {
    fn default() -> Self {
        Self { name: String::new(), sound: None, screen: None, zone: None, hold: DEFAULT_HOLD }
    }
}

/// Seconds a scene is kept after its last sign, unless set otherwise.
pub const DEFAULT_HOLD: f64 = 3.0;

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

/// A file name stem for a game's name: "Metaphor: ReFantazio" gives "metaphor-refantazio".
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() { "game".to_owned() } else { out }
}

fn path(id: &str) -> PathBuf {
    dir().join(format!("{id}.json"))
}

/// Where a game's captures are.
fn captures_dir(id: &str) -> PathBuf {
    dir().join(id)
}

/// Before games had names, profiles were files named after the executable
/// (`{"game": "METAPHOR.exe", ...}`): each becomes a game named after it.
fn migrate_legacy() {
    let Ok(entries) = fs::read_dir(dir()) else { return };
    for path in entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")) {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(serde_json::Value::Object(mut map)) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        if map.contains_key("name") {
            continue;
        }
        let Some(serde_json::Value::String(exe)) = map.remove("game") else { continue };
        let old_stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let name = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(&exe).to_owned();
        let Ok(mut game) = serde_json::from_value::<Game>(serde_json::Value::Object(map)) else { continue };
        game.name = name.clone();
        // Overlays used to name every Proton game after Wine's loader: that names no game.
        if !exe.starts_with("wine") {
            game.executables = vec![exe.clone()];
        }
        let mut scenes: Vec<String> = game.captures.iter().filter(|c| !c.scene.is_empty()).map(|c| c.scene.clone()).collect();
        scenes.sort();
        scenes.dedup();
        game.scenes = scenes.into_iter().map(|name| SceneDef { name, ..SceneDef::default() }).collect();
        // The old file goes first: the game may take its name.
        let _ = fs::remove_file(&path);
        game.id = unused_id(&slug(&name));
        if captures_dir(&old_stem).exists() && old_stem != game.id {
            let _ = fs::rename(captures_dir(&old_stem), captures_dir(&game.id));
        }
        game.save();
        log::info!("the profile of {exe} is now the game \"{name}\"");
    }
}

/// `id`, or `id-2`, `id-3`... if a game has it already.
fn unused_id(id: &str) -> String {
    let mut candidate = id.to_owned();
    let mut n = 2;
    while path(&candidate).exists() {
        candidate = format!("{id}-{n}");
        n += 1;
    }
    candidate
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

impl Game {
    /// A new game named `name`, with an id of its own (not saved yet).
    pub fn new(name: &str) -> Self {
        Game { id: unused_id(&slug(name)), name: name.trim().to_owned(), ..Game::default() }
    }

    /// Every game, sorted by name.
    pub fn list() -> Vec<Game> {
        migrate_legacy();
        let Ok(entries) = fs::read_dir(dir()) else { return Vec::new() };
        let mut games: Vec<Game> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .filter_map(|p| Self::load(&p.file_stem()?.to_string_lossy()))
            .collect();
        games.sort_by_key(|g| g.name.to_lowercase());
        games
    }

    pub fn load(id: &str) -> Option<Self> {
        let text = fs::read_to_string(path(id)).ok()?;
        match serde_json::from_str::<Game>(&text) {
            Ok(game) => Some(Game { id: id.to_owned(), ..game }),
            Err(e) => {
                log::warn!("ignoring the invalid game {id}: {e}");
                None
            }
        }
    }

    pub fn save(&self) {
        let result =
            serde_json::to_string(self).map_err(std::io::Error::other).and_then(|text| config::write_file(&path(&self.id), &text));
        if let Err(e) = result {
            log::warn!("cannot save the game {}: {e}", self.name);
        }
    }

    /// Deletes the game and its captures (its mode files stay).
    pub fn delete(&self) {
        let _ = fs::remove_file(path(&self.id));
        let _ = fs::remove_dir_all(captures_dir(&self.id));
    }

    pub fn runs_as(&self, exe: &str) -> bool {
        self.executables.iter().any(|e| e.eq_ignore_ascii_case(exe))
    }

    /// The scenes as modes see them.
    pub fn scene_decls(&self) -> Vec<SceneDecl> {
        self.scenes
            .iter()
            .map(|s| SceneDecl {
                name: s.name.clone(),
                sound: s.sound.clone(),
                screen: s.screen.clone(),
                // Only a zone the game still has.
                zone: s.zone.clone().filter(|z| self.zones.iter().any(|zone| zone.name == *z)),
                hold: s.hold,
            })
            .collect()
    }

    /// A copy for the GUI: the capture embeddings, large and of no use there,
    /// are left out (an analysed capture keeps a one-number placeholder).
    pub fn for_gui(&self) -> Game {
        let mut game = self.clone();
        for capture in &mut game.captures {
            if !capture.embedding.is_empty() {
                capture.embedding = vec![1.0];
            }
        }
        game
    }

    /// Captures `frame` as an example of `scene`; the oldest of the scene goes past `MAX_CAPTURES`.
    pub fn add_capture(&mut self, scene: &str, frame: &Frame) -> std::io::Result<()> {
        let file = save_capture(&self.id, scene, frame)?;
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
            delete_capture(&self.id, file);
        }
    }

    /// Names of its scenes, then of scenes only its captures name.
    pub fn scenes(&self) -> Vec<String> {
        let mut scenes: Vec<String> = self.scenes.iter().map(|s| s.name.clone()).collect();
        for capture in self.captures.iter().filter(|c| !c.scene.is_empty()) {
            if !scenes.contains(&capture.scene) {
                scenes.push(capture.scene.clone());
            }
        }
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

    /// How the game's signals read for an AI assistant writing a mode for it (§6.5).
    pub fn describe(&self) -> String {
        let mut out = String::new();
        if !self.scenes.is_empty() {
            let names: Vec<String> = self.scenes.iter().map(|s| format!("`{}`", s.name)).collect();
            out.push_str(&format!(
                "Scenes GameViber recognizes in this game (`input.scene`, `on_scene`; do not declare `scenes`): {}\n",
                names.join(", ")
            ));
        }
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

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gameviber-games-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.clone()));
        dir
    }

    #[test]
    fn games_are_named_saved_and_listed() {
        let dir = test_dir("list");
        assert_eq!(slug(" Metaphor: ReFantazio "), "metaphor-refantazio");
        let mut game = Game::new("Metaphor: ReFantazio");
        assert_eq!(game.id, "metaphor-refantazio");
        game.executables.push("METAPHOR.exe".into());
        game.scenes.push(SceneDef { name: "combat".into(), sound: Some("battle music".into()), screen: None, ..SceneDef::default() });
        game.save();
        assert_eq!(Game::new("Metaphor: ReFantazio").id, "metaphor-refantazio-2", "ids stay unique");
        let other = Game::new("Hades II");
        other.save();
        let games = Game::list();
        assert_eq!(games.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Hades II", "Metaphor: ReFantazio"]);
        assert!(games[1].runs_as("metaphor.exe"));
        assert_eq!(games[1].scene_decls()[0].sound.as_deref(), Some("battle music"));
        assert_eq!(games[1], game);
        other.delete();
        assert_eq!(Game::list().len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_profiles_become_games() {
        let dir = test_dir("legacy");
        fs::create_dir_all(dir.join("METAPHOR.exe")).unwrap();
        fs::write(dir.join("METAPHOR.exe").join("combat-1.png"), b"png").unwrap();
        fs::write(
            dir.join("METAPHOR.exe.json"),
            r#"{"game":"METAPHOR.exe","zones":[],"captures":[{"file":"combat-1.png","scene":"combat","embedding":[]}],"inputs":[]}"#,
        )
        .unwrap();
        fs::write(dir.join("wine64-preloader.json"), r#"{"game":"wine64-preloader","zones":[],"captures":[],"inputs":[]}"#).unwrap();
        let games = Game::list();
        let metaphor = games.iter().find(|g| g.name == "METAPHOR").unwrap();
        assert_eq!((metaphor.id.as_str(), &metaphor.executables[..]), ("metaphor", &["METAPHOR.exe".to_owned()][..]));
        assert_eq!(metaphor.scenes[0].name, "combat", "scenes from the captures");
        assert!(dir.join("metaphor").join("combat-1.png").exists(), "captures moved");
        let wine = games.iter().find(|g| g.name == "wine64-preloader").unwrap();
        assert!(wine.executables.is_empty(), "Wine's loader names no game");
        assert_eq!(wine.id, "wine64-preloader", "the id the old file had");
        assert!(!dir.join("METAPHOR.exe.json").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn captures_are_saved_capped_and_described() {
        let dir = test_dir("captures");
        let mut profile = Game::new("Test game");
        let game = profile.id.clone();
        let frame = |v: u8| Frame { width: 4, height: 2, source_width: 4, source_height: 2, count: 0, pixels: vec![v; 32] };
        for i in 0..MAX_CAPTURES + 2 {
            profile.add_capture("battle", &frame(i as u8)).unwrap();
        }
        profile.add_capture("story", &frame(200)).unwrap();
        assert_eq!(profile.captures.iter().filter(|c| c.scene == "battle").count(), MAX_CAPTURES);
        profile.scenes.push(SceneDef { name: "menu".into(), ..SceneDef::default() });
        assert_eq!(profile.scenes(), ["menu", "battle", "story"], "its scenes, then those only captures name");
        let first = &profile.captures[0];
        assert_eq!(load_capture(&game, &first.file).unwrap().pixels, vec![2; 32], "the two oldest went first");
        assert!(profile.example_centroids().is_empty(), "no embedding yet");
        profile.captures[0].embedding = vec![0.0, 2.0];
        assert_eq!(profile.example_centroids(), vec![("battle".to_owned(), crate::models::normalize(&[0.0, 1.0]))]);

        profile.zones.push(Zone { name: "hp".into(), kind: ZoneKind::Bar, ..Zone::default() });
        profile.inputs.push(InputDecl { name: "kill".into(), kind: InputKind::Event, description: "an enemy died".into() });
        let text = profile.describe();
        assert!(text.contains("`hp`: how full the bar is"), "{text}");
        assert!(text.contains("recognizes in this game (`input.scene`, `on_scene`; do not declare `scenes`): `menu`"), "{text}");
        assert!(text.contains("event `kill`: an enemy died"), "{text}");
        profile.save();
        assert_eq!(Game::load(&game).unwrap(), profile);
        let story = profile.captures.last().unwrap().file.clone();
        profile.remove_capture(&story);
        assert!(load_capture(&game, &story).is_none(), "its file is deleted");
        let _ = fs::remove_dir_all(dir);
    }
}
