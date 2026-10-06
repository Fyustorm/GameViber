//! Games: a game the player plays, identified by its name, with its modes
//! (each a package of its own, `package.rs`, with the inputs it reads), which
//! sound to listen to while it is played, and the executables it runs as, so
//! that it becomes the active game by itself. One JSON file per game in
//! `games/`, named after its id.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::{self, AudioSource};

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
    /// Its modes, by id (user mode files, built-in modes).
    pub modes: Vec<String>,
    /// Which sound to listen to while it is played; None: the default (Setup).
    pub audio: Option<AudioSource>,
}

#[cfg(test)]
thread_local! {
    /// Tests keep their games out of the player's.
    pub(crate) static TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Where the games are.
pub(crate) fn games_dir() -> PathBuf {
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
    games_dir().join(format!("{id}.json"))
}

/// Before games had names, profiles were files named after the executable
/// (`{"game": "METAPHOR.exe", ...}`): each becomes a game named after it, its
/// captures in a directory named after its id (`package::migrate` then gives
/// them to its modes).
pub(crate) fn migrate_legacy() {
    let Ok(entries) = fs::read_dir(games_dir()) else { return };
    for path in entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")) {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(serde_json::Value::Object(mut map)) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        if map.contains_key("name") {
            continue;
        }
        let Some(serde_json::Value::String(exe)) = map.remove("game") else { continue };
        let old_stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let name = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(&exe).to_owned();
        map.insert("name".into(), name.clone().into());
        // Overlays used to name every Proton game after Wine's loader: that names no game.
        if !exe.starts_with("wine") {
            map.insert("executables".into(), serde_json::json!([exe]));
        }
        let mut scenes: Vec<String> = map
            .get("captures")
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten()
            .filter_map(|c| c.get("scene").and_then(|s| s.as_str()))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        scenes.sort();
        scenes.dedup();
        map.insert("scenes".into(), scenes.into_iter().map(|name| serde_json::json!({ "name": name })).collect());
        // The old file goes first: the game may take its name.
        let _ = fs::remove_file(&path);
        let id = unused_id(&slug(&name));
        let captures = |id: &str| games_dir().join(id);
        if captures(&old_stem).exists() && old_stem != id {
            let _ = fs::rename(captures(&old_stem), captures(&id));
        }
        let result = serde_json::to_string(&map).map_err(std::io::Error::other).and_then(|text| config::write_file(&self::path(&id), &text));
        match result {
            Ok(()) => log::info!("the profile of {exe} is now the game \"{name}\""),
            Err(e) => log::error!("cannot save the game {name}: {e}"),
        }
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

impl Game {
    /// A new game named `name`, with an id of its own (not saved yet).
    pub fn new(name: &str) -> Self {
        Game { id: unused_id(&slug(name)), name: name.trim().to_owned(), ..Game::default() }
    }

    /// Every game, sorted by name.
    pub fn list() -> Vec<Game> {
        let Ok(entries) = fs::read_dir(games_dir()) else { return Vec::new() };
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

    /// Deletes the game, and what it kept from before modes had packages
    /// (its modes stay).
    pub fn delete(&self) {
        let _ = fs::remove_file(path(&self.id));
        let _ = fs::remove_file(path(&self.id).with_extension("json.v1"));
        let _ = fs::remove_dir_all(games_dir().join(&self.id));
    }

    pub fn runs_as(&self, exe: &str) -> bool {
        self.executables.iter().any(|e| e.eq_ignore_ascii_case(exe))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        game.save();
        assert_eq!(Game::new("Metaphor: ReFantazio").id, "metaphor-refantazio-2", "ids stay unique");
        let other = Game::new("Hades II");
        other.save();
        let games = Game::list();
        assert_eq!(games.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Hades II", "Metaphor: ReFantazio"]);
        assert!(games[1].runs_as("metaphor.exe"));
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
        migrate_legacy();
        let games = Game::list();
        let metaphor = games.iter().find(|g| g.name == "METAPHOR").unwrap();
        assert_eq!((metaphor.id.as_str(), &metaphor.executables[..]), ("metaphor", &["METAPHOR.exe".to_owned()][..]));
        let text = fs::read_to_string(dir.join("metaphor.json")).unwrap();
        assert!(text.contains(r#""scenes":[{"name":"combat"}]"#), "scenes from the captures: {text}");
        assert!(dir.join("metaphor").join("combat-1.png").exists(), "captures moved");
        let wine = games.iter().find(|g| g.name == "wine64-preloader").unwrap();
        assert!(wine.executables.is_empty(), "Wine's loader names no game");
        assert_eq!(wine.id, "wine64-preloader", "the id the old file had");
        assert!(!dir.join("METAPHOR.exe.json").exists());
        let _ = fs::remove_dir_all(dir);
    }
}
