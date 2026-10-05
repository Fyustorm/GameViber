//! Modes shared with their game, as `.gameviber` files: a zip archive of
//! `gameviber.json` (how the game is set up: its scenes, zones, captures,
//! values from other programs, sound and executables), the mode's script
//! (`mode.luau`) and the captures (`captures/*.png`). A mode written for one
//! game reads that game's scenes and zones by name, so it travels with them.
//!
//! Importing one adds the mode to the game of the same name, completing its
//! signals with those it lacks (what the player set up stays), or adds the
//! game. Capture embeddings are left out: they are computed again with the
//! player's image model.

use std::fs;
use std::io::{Read, Seek, Write};
use std::path::Path;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

use crate::config::{self, ModeEntry, MODE_EXTENSION};
use crate::game::{self, Capture, Game, MAX_CAPTURES, MAX_SCENES};

pub const EXTENSION: &str = "gameviber";
/// Version of the layout; files of a newer one are refused.
const FORMAT: u32 = 1;
const MANIFEST: &str = "gameviber.json";
const MODE: &str = "mode.luau";
const CAPTURES: &str = "captures/";
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
/// What an imported file may hold: files in it, bytes per file, bytes in all.
const MAX_ENTRIES: usize = 2 + MAX_SCENES * MAX_CAPTURES;
const MAX_ENTRY_SIZE: u64 = 8 << 20;
const MAX_TOTAL_SIZE: u64 = 128 << 20;

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: u32,
    /// The GameViber version that wrote it.
    app_version: String,
    /// The mode file's name, without its extension.
    mode: String,
    /// The game, without its modes and capture embeddings.
    game: Game,
}

/// What an import did.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The game's id and name.
    pub game: String,
    pub name: String,
    /// The mode's id.
    pub mode: String,
    /// The game was added (else the mode joined the game of that name).
    pub new_game: bool,
    /// The same mode was there already.
    pub mode_existed: bool,
}

/// A file name for a mode of a game: "metaphor-refantazio-battles.gameviber".
pub fn file_name(game: &Game, mode: &str) -> String {
    format!("{}-{}.{EXTENSION}", game::slug(&game.name), ModeEntry::from_id(mode).key)
}

/// Writes `mode` with the game `game` (by id) to `path`.
pub fn export(game: &str, mode: &str, path: &Path) -> anyhow::Result<()> {
    let mut game = Game::load(game).context("this game is gone")?;
    let entry = ModeEntry::from_id(mode);
    if entry.builtin {
        bail!("built-in modes come with every GameViber: there is no need to share them");
    }
    let source = entry.source().with_context(|| format!("cannot read {}", entry.id))?;
    game.modes.clear();
    // Captures to sort are examples of no scene yet.
    game.captures.retain(|c| !c.scene.is_empty());
    let mut images = Vec::new();
    game.captures.retain_mut(|capture| {
        capture.embedding.clear();
        match fs::read(game::capture_path(&game.id, &capture.file)) {
            Ok(bytes) => {
                images.push((capture.file.clone(), bytes));
                true
            }
            Err(e) => {
                log::warn!("leaving out the capture {}: {e}", capture.file);
                false
            }
        }
    });
    let manifest = Manifest { format: FORMAT, app_version: env!("CARGO_PKG_VERSION").to_owned(), mode: entry.key.clone(), game };

    // Written beside the file, then renamed: a failure leaves no half file.
    let partial = path.with_extension(format!("{EXTENSION}.part"));
    let written = write_archive(fs::File::create(&partial)?, &manifest, &source, &images);
    match written.and_then(|()| Ok(fs::rename(&partial, path)?)) {
        Ok(()) => {
            log::info!("{} and {} exported to {}", manifest.game.name, entry.key, path.display());
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&partial);
            Err(e)
        }
    }
}

fn write_archive(file: fs::File, manifest: &Manifest, source: &str, images: &[(String, Vec<u8>)]) -> anyhow::Result<()> {
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let text = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    // PNG files are compressed already.
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(MANIFEST, text)?;
    zip.write_all(serde_json::to_string_pretty(manifest)?.as_bytes())?;
    zip.start_file(MODE, text)?;
    zip.write_all(source.as_bytes())?;
    for (file, bytes) in images {
        zip.start_file(format!("{CAPTURES}{file}"), stored)?;
        zip.write_all(bytes)?;
    }
    zip.finish()?.flush()?;
    Ok(())
}

/// Imports a `.gameviber` file: its mode joins the game of the same name, or a new game.
pub fn import(path: &Path) -> anyhow::Result<Imported> {
    import_into(path, &config::modes_dir())
}

fn import_into(path: &Path, modes_dir: &Path) -> anyhow::Result<Imported> {
    let file = fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut archive = Archive::open(file)?;
    let manifest: Manifest =
        serde_json::from_slice(&archive.read(MANIFEST)?).context("this file does not describe a game and its mode")?;
    if manifest.format > FORMAT {
        bail!("this mode was shared from a newer GameViber ({}): update GameViber to import it", manifest.app_version);
    }
    let source = String::from_utf8(archive.read(MODE)?).context("the mode's script is not text")?;
    let shared = manifest.game;
    if shared.name.trim().is_empty() {
        bail!("this file names no game");
    }
    // A mode that does not load would only show an error once imported.
    crate::mode::ModeRuntime::load(&format!("{}.{MODE_EXTENSION}", manifest.mode), &source, &Default::default(), None)
        .map_err(|e| anyhow::anyhow!("its mode does not load in this GameViber: {e}"))?;
    let mut images = Vec::new();
    for capture in shared.captures.iter().filter(|c| safe_file_name(&c.file)) {
        match archive.read(&format!("{CAPTURES}{}", capture.file)) {
            Ok(bytes) if bytes.starts_with(PNG_SIGNATURE) => images.push((capture.clone(), bytes)),
            Ok(_) => log::warn!("leaving out the capture {}: not a PNG image", capture.file),
            Err(e) => log::warn!("leaving out the capture {}: {e:#}", capture.file),
        }
    }

    let games = Game::list();
    let existing = games.iter().find(|g| g.name.trim().eq_ignore_ascii_case(shared.name.trim())).cloned();
    let new_game = existing.is_none();
    let mut game = existing.unwrap_or_else(|| Game::new(&shared.name));
    let taken: Vec<&String> = games.iter().filter(|g| g.id != game.id).flat_map(|g| &g.executables).collect();
    merge(&mut game, &shared, &taken);
    for (capture, bytes) in images {
        let of_scene = game.captures.iter().filter(|c| c.scene == capture.scene).count();
        if of_scene >= MAX_CAPTURES || game.captures.iter().any(|c| c.file == capture.file) {
            continue;
        }
        let path = game::capture_path(&game.id, &capture.file);
        if path.exists() {
            continue;
        }
        if let Some(dir) = path.parent() {
            config::create_dir(dir)?;
        }
        fs::write(&path, bytes)?;
        game.captures.push(Capture { file: capture.file, scene: capture.scene, embedding: Vec::new() });
    }
    // Zones drawn on a capture left out are edited without it.
    for zone in &mut game.zones {
        if zone.capture.as_ref().is_some_and(|file| !game.captures.iter().any(|c| c.file == *file)) {
            zone.capture = None;
        }
    }
    let (mode, mode_existed) = install_mode(modes_dir, &manifest.mode, &source)?;
    if !game.modes.contains(&mode) {
        game.modes.push(mode.clone());
    }
    game.save();
    log::info!("imported {} for {} ({})", manifest.mode, game.name, if new_game { "new game" } else { "existing game" });
    Ok(Imported { game: game.id, name: game.name, mode, new_game, mode_existed })
}

/// Adds to `game` what `shared` sets up and it lacks, by name: scenes, zones
/// (every place of each), values from other programs, executables no other
/// game runs as (`taken`), its sound if it has none.
fn merge(game: &mut Game, shared: &Game, taken: &[&String]) {
    for scene in &shared.scenes {
        if game.scenes.len() < MAX_SCENES && !game.scenes.iter().any(|s| s.name == scene.name) {
            game.scenes.push(scene.clone());
        }
    }
    let zones: Vec<String> = game.zones.iter().map(|z| z.name.clone()).collect();
    game.zones.extend(shared.zones.iter().filter(|z| !zones.contains(&z.name)).cloned());
    for input in &shared.inputs {
        if !game.inputs.iter().any(|i| i.name == input.name) {
            game.inputs.push(input.clone());
        }
    }
    for exe in &shared.executables {
        if !game.runs_as(exe) && !taken.iter().any(|t| t.eq_ignore_ascii_case(exe)) {
            game.executables.push(exe.clone());
        }
    }
    if game.audio.is_none() {
        game.audio = shared.audio.clone();
    }
}

/// Writes the mode in `dir`, unless the very same mode is there already; returns its id.
fn install_mode(dir: &Path, stem: &str, source: &str) -> anyhow::Result<(String, bool)> {
    let same = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == MODE_EXTENSION))
        .find(|p| fs::read_to_string(p).is_ok_and(|s| s == source));
    if let Some(path) = same {
        return Ok((path.to_string_lossy().into_owned(), true));
    }
    let stem: String = stem.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect();
    let path = config::unused_mode_path_in(dir, if stem.is_empty() { "shared-mode" } else { &stem });
    config::write_file(&path, source).with_context(|| format!("cannot write {}", path.display()))?;
    Ok((path.to_string_lossy().into_owned(), false))
}

/// A plain file name: no directory, nothing hidden.
fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A zip archive read within the limits.
struct Archive<R> {
    zip: zip::ZipArchive<R>,
    read: u64,
}

impl<R: Read + Seek> Archive<R> {
    fn open(reader: R) -> anyhow::Result<Self> {
        let zip = zip::ZipArchive::new(reader).context("this is not a file of a shared mode")?;
        if zip.len() > MAX_ENTRIES {
            bail!("this file holds too many files to be a shared mode");
        }
        Ok(Self { zip, read: 0 })
    }

    fn read(&mut self, name: &str) -> anyhow::Result<Vec<u8>> {
        let entry = self.zip.by_name(name).with_context(|| format!("{name} is missing"))?;
        // The sizes an archive states may lie: what is read counts.
        let limit = MAX_ENTRY_SIZE.min(MAX_TOTAL_SIZE.saturating_sub(self.read));
        let mut bytes = Vec::new();
        entry.take(limit + 1).read_to_end(&mut bytes).with_context(|| format!("cannot read {name}"))?;
        if bytes.len() as u64 > limit {
            bail!("{name} is too large");
        }
        self.read += bytes.len() as u64;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::game::{InputDecl, SceneDef, Zone, ZoneKind};
    use crate::screen::Frame;

    const MODE_SOURCE: &str = "mode { api = 1, name = \"Battles\" }\nfunction tick(dt, input) set(input.rumble.level) end\n";

    fn test_dirs(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("gameviber-sharing-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let games = root.join("games");
        game::TEST_DIR.with(|d| *d.borrow_mut() = Some(games));
        (root.clone(), root.join("modes"))
    }

    fn frame(v: u8) -> Frame {
        Frame { width: 4, height: 2, source_width: 4, source_height: 2, count: 0, pixels: vec![v; 32] }
    }

    #[test]
    fn a_mode_travels_with_its_game() {
        let (root, modes) = test_dirs("round-trip");
        let mode_path = modes.join("battles.luau");
        config::write_file(&mode_path, MODE_SOURCE).unwrap();
        let mode = mode_path.to_string_lossy().into_owned();

        let mut game = Game::new("Metaphor: ReFantazio");
        game.executables.push("METAPHOR.exe".into());
        game.scenes.push(SceneDef { name: "battle".into(), sound: Some("battle music".into()), ..SceneDef::default() });
        game.inputs.push(InputDecl { name: "hp".into(), ..InputDecl::default() });
        game.add_capture("battle", &frame(1)).unwrap();
        game.add_capture("", &frame(2)).unwrap();
        game.captures[0].embedding = vec![1.0, 0.0];
        let drawn_on = game.captures[0].file.clone();
        game.zones.push(Zone { name: "menu".into(), capture: Some(drawn_on.clone()), ..Zone::default() });
        game.modes.push(mode.clone());
        game.save();
        let file = root.join(file_name(&game, &mode));
        assert!(file.ends_with("metaphor-refantazio-battles.gameviber"));
        export(&game.id, &mode, &file).unwrap();
        assert!(export(&game.id, "builtin:simple", &root.join("x.gameviber")).is_err(), "built-in modes are not shared");

        // Another player, without the game nor the mode.
        game.delete();
        fs::remove_file(&mode_path).unwrap();
        let imported = import_into(&file, &modes).unwrap();
        assert!(imported.new_game && !imported.mode_existed);
        assert_eq!(imported.mode, mode, "the mode's file name");
        let got = Game::load(&imported.game).unwrap();
        assert_eq!(got.name, "Metaphor: ReFantazio");
        assert_eq!((got.executables.clone(), got.scenes.clone(), got.inputs.clone()), (game.executables.clone(), game.scenes.clone(), game.inputs.clone()));
        assert_eq!(got.modes, [mode.clone()]);
        assert_eq!(got.captures.len(), 1, "captures to sort stay home");
        assert!(got.captures[0].embedding.is_empty(), "embeddings are computed again");
        assert_eq!(game::load_capture(&got.id, &drawn_on).unwrap().pixels, vec![1; 32]);
        assert_eq!(got.zones[0].capture.as_deref(), Some(drawn_on.as_str()));
        assert_eq!(fs::read_to_string(&mode_path).unwrap(), MODE_SOURCE);

        // Imported again: nothing doubles.
        let again = import_into(&file, &modes).unwrap();
        assert!(!again.new_game && again.mode_existed);
        assert_eq!(Game::load(&got.id).unwrap(), got);
        assert_eq!(fs::read_dir(&modes).unwrap().count(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn signals_the_player_set_up_stay() {
        let mut mine = Game::new("Hades II");
        mine.scenes.push(SceneDef { name: "battle".into(), sound: Some("mine".into()), ..SceneDef::default() });
        mine.zones.push(Zone { name: "hp".into(), kind: ZoneKind::Bar, ..Zone::default() });
        let mut shared = mine.clone();
        shared.scenes = vec![
            SceneDef { name: "battle".into(), sound: Some("theirs".into()), ..SceneDef::default() },
            SceneDef { name: "boss".into(), ..SceneDef::default() },
        ];
        shared.zones = vec![Zone { name: "hp".into(), ..Zone::default() }, Zone { name: "boss_bar".into(), ..Zone::default() }, Zone { name: "boss_bar".into(), rect: [0.5; 4], ..Zone::default() }];
        shared.executables = vec!["Hades2.exe".into(), "other.exe".into()];
        let other = "other.exe".to_owned();
        merge(&mut mine, &shared, &[&other]);
        assert_eq!(mine.scenes.iter().map(|s| (s.name.as_str(), s.sound.as_deref())).collect::<Vec<_>>(), [("battle", Some("mine")), ("boss", None)]);
        assert_eq!(mine.zones.iter().map(|z| z.name.as_str()).collect::<Vec<_>>(), ["hp", "boss_bar", "boss_bar"], "every place of a new zone");
        assert_eq!(mine.zones[0].kind, ZoneKind::Bar);
        assert_eq!(mine.executables, ["Hades2.exe"], "an executable of another game stays there");
    }

    #[test]
    fn files_are_checked() {
        let (root, modes) = test_dirs("checks");
        config::create_dir(&root).unwrap();
        let write = |name: &str, entries: &[(&str, &[u8])]| {
            let path = root.join(name);
            let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
            for (entry, bytes) in entries {
                zip.start_file(*entry, zip::write::SimpleFileOptions::default()).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
            path
        };
        let manifest = |format: u32, mode: &str| {
            let game = Game { name: "Test".into(), captures: vec![Capture { file: "../../evil.png".into(), scene: "a".into(), embedding: Vec::new() }], ..Game::default() };
            serde_json::to_vec(&Manifest { format, app_version: "9.0.0".into(), mode: mode.into(), game }).unwrap()
        };
        let error = |path: PathBuf| format!("{:#}", import_into(&path, &modes).unwrap_err());
        fs::write(root.join("text.gameviber"), "hello").unwrap();
        assert!(error(root.join("text.gameviber")).contains("not a file of a shared mode"));
        assert!(error(write("newer.gameviber", &[(MANIFEST, &manifest(FORMAT + 1, "m")), (MODE, MODE_SOURCE.as_bytes())])).contains("newer GameViber (9.0.0)"));
        assert!(error(write("broken.gameviber", &[(MANIFEST, &manifest(FORMAT, "m")), (MODE, b"mode {")])).contains("does not load"));
        let big = vec![0u8; MAX_ENTRY_SIZE as usize + 1];
        assert!(error(write("big.gameviber", &[(MANIFEST, &manifest(FORMAT, "m")), (MODE, &big)])).contains("too large"));

        let imported = import_into(&write("ok.gameviber", &[(MANIFEST, &manifest(FORMAT, "../x y")), (MODE, MODE_SOURCE.as_bytes()), ("captures/../../evil.png", PNG_SIGNATURE)]), &modes).unwrap();
        assert!(imported.mode.ends_with("/modes/xy.luau"), "{}", imported.mode);
        assert!(Game::load(&imported.game).unwrap().captures.is_empty(), "paths are refused");
        assert!(!root.join("evil.png").exists());
        let _ = fs::remove_dir_all(root);
    }
}
