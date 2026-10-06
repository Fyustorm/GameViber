//! Modes shared with their game, as `.gameviber` files: a zip archive of
//! `gameviber.json` (the game — its name and the executables it runs as — and
//! the inputs the mode reads: its phases, indicators, captures, values from other
//! programs), the mode's script (`mode.luau`), its variants
//! (`variants/*.luau`) and the captures (`captures/*.png`): a mode's package
//! (`package.rs`), with its game.
//!
//! Importing one makes a package of the mode, joined to the same game (the
//! same Steam app id, or the same name: `Game::same_as`), or a new game. Files of the first format (0.1.0-alpha.2) held the
//! inputs in the game: they become the mode's. Capture embeddings are left
//! out: they are computed again with the player's image model.

use std::fs;
use std::io::{Read, Seek, Write};
use std::path::Path;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

use crate::config::{self, ModeEntry, MODE_EXTENSION};
use crate::game::{self, Game};
use crate::package::{self, Inputs, MAX_CAPTURES, MAX_PHASES};

pub const EXTENSION: &str = "gameviber";
/// Version of the layout; files of a newer one are refused.
const FORMAT: u32 = 2;
const MANIFEST: &str = "gameviber.json";
const MODE: &str = "mode.luau";
const CAPTURES: &str = "captures/";
const VARIANTS: &str = "variants/";
/// A shared mode has at most this many variants.
const MAX_VARIANTS: usize = 16;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
/// What an imported file may hold: files in it, bytes per file, bytes in all.
const MAX_ENTRIES: usize = 2 + MAX_VARIANTS + MAX_PHASES * MAX_CAPTURES;
const MAX_ENTRY_SIZE: u64 = 8 << 20;
const MAX_TOTAL_SIZE: u64 = 128 << 20;

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: u32,
    /// The GameViber version that wrote it.
    app_version: String,
    /// The mode's name in files, without extension.
    mode: String,
    /// The game: its name and executables (format 1: also the inputs).
    game: serde_json::Value,
    /// The mode's inputs, without capture embeddings (format 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inputs: Option<Inputs>,
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
    /// The same mode was there already (its inputs stay as they are).
    pub mode_existed: bool,
}

/// A file name for a mode of a game: "metaphor-refantazio-battles.gameviber".
pub fn file_name(game: &Game, mode: &str) -> String {
    format!("{}-{}.{EXTENSION}", game::slug(&game.name), ModeEntry::from_id(&ModeEntry::from_id(mode).main_id()).key)
}

/// Writes `mode` (or the mode it is a variant of, with all its variants) with
/// the game `game` (by id) to `path`, without the captures `left_out` (file names).
pub fn export(game: &str, mode: &str, path: &Path, left_out: &[String]) -> anyhow::Result<()> {
    let game = Game::load(game).context("this game is gone")?;
    let entry = ModeEntry::from_id(&ModeEntry::from_id(mode).main_id());
    if entry.builtin {
        bail!("built-in modes come with every GameViber: there is no need to share them");
    }
    let source = entry.source().with_context(|| format!("cannot read {}", entry.id))?;
    let mut variants = Vec::new();
    for variant in entry.variants() {
        let text = variant.source().with_context(|| format!("cannot read {}", variant.id))?;
        variants.push((variant.variant.unwrap_or_default(), text));
    }
    let mut inputs = Inputs::of(&entry);
    // Captures to sort are examples of no phase yet.
    inputs.captures.retain(|c| !c.phase.is_empty() && !left_out.contains(&c.file));
    for zone in &mut inputs.zones {
        if zone.capture.as_ref().is_some_and(|file| left_out.contains(file)) {
            zone.capture = None;
        }
    }
    let mut images = Vec::new();
    inputs.captures.retain_mut(|capture| {
        capture.embedding.clear();
        match fs::read(package::capture_path(&inputs.dir, &capture.file)) {
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
    // The sound to listen to is set for this computer.
    let shared_game = serde_json::json!({ "name": game.name, "executables": game.executables, "steam_app_id": game.steam_app_id });
    let manifest =
        Manifest { format: FORMAT, app_version: env!("CARGO_PKG_VERSION").to_owned(), mode: entry.key.clone(), game: shared_game, inputs: Some(inputs) };

    // Written beside the file, then renamed: a failure leaves no half file.
    let partial = path.with_extension(format!("{EXTENSION}.part"));
    let written = write_archive(fs::File::create(&partial)?, &manifest, &source, &variants, &images);
    match written.and_then(|()| Ok(fs::rename(&partial, path)?)) {
        Ok(()) => {
            log::info!("{} for {} exported to {}", entry.key, game.name, path.display());
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&partial);
            Err(e)
        }
    }
}

fn write_archive(file: fs::File, manifest: &Manifest, source: &str, variants: &[(String, String)], images: &[(String, Vec<u8>)]) -> anyhow::Result<()> {
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let text = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    // PNG files are compressed already.
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(MANIFEST, text)?;
    zip.write_all(serde_json::to_string_pretty(manifest)?.as_bytes())?;
    zip.start_file(MODE, text)?;
    zip.write_all(source.as_bytes())?;
    for (name, source) in variants {
        zip.start_file(format!("{VARIANTS}{name}.{MODE_EXTENSION}"), text)?;
        zip.write_all(source.as_bytes())?;
    }
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

/// What a `.gameviber` file holds, checked: every script loads.
struct Shared {
    /// The mode's name in files.
    stem: String,
    game: Game,
    source: String,
    variants: Vec<(String, String)>,
    inputs: Inputs,
    images: Vec<(package::Capture, Vec<u8>)>,
}

fn read_shared(path: &Path) -> anyhow::Result<Shared> {
    let file = fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut archive = Archive::open(file)?;
    let manifest: Manifest =
        serde_json::from_slice(&archive.read(MANIFEST)?).context("this file does not describe a game and its mode")?;
    if manifest.format > FORMAT {
        bail!("this mode was shared from a newer GameViber ({}): update GameViber to import it", manifest.app_version);
    }
    let source = String::from_utf8(archive.read(MODE)?).context("the mode's script is not text")?;
    let shared: Game = serde_json::from_value(manifest.game.clone()).context("this file names no game")?;
    if shared.name.trim().is_empty() {
        bail!("this file names no game");
    }
    // The first format kept the inputs in the game.
    let inputs = match manifest.inputs {
        Some(inputs) => inputs,
        None => serde_json::from_value(manifest.game).unwrap_or_default(),
    };
    // A mode that does not load would only show an error once imported.
    crate::mode::ModeRuntime::load(&format!("{}.{MODE_EXTENSION}", manifest.mode), &source, &Default::default(), None)
        .map_err(|e| anyhow::anyhow!("its mode does not load in this GameViber: {e}"))?;
    let mut variants = Vec::new();
    for file in archive.names().into_iter().filter_map(|n| n.strip_prefix(VARIANTS).map(str::to_owned)) {
        let Some(name) = file.strip_suffix(&format!(".{MODE_EXTENSION}")).filter(|n| safe_file_name(n) && !n.contains('.')) else { continue };
        if variants.len() >= MAX_VARIANTS {
            break;
        }
        let text = String::from_utf8(archive.read(&format!("{VARIANTS}{file}"))?).with_context(|| format!("the variant {name} is not text"))?;
        crate::mode::ModeRuntime::load(&format!("{}.{name}.{MODE_EXTENSION}", manifest.mode), &text, &Default::default(), None)
            .map_err(|e| anyhow::anyhow!("its variant {name} does not load in this GameViber: {e}"))?;
        variants.push((name.to_owned(), text));
    }
    let mut images = Vec::new();
    for capture in inputs.captures.iter().filter(|c| safe_file_name(&c.file)) {
        match archive.read(&format!("{CAPTURES}{}", capture.file)) {
            Ok(bytes) if bytes.starts_with(PNG_SIGNATURE) => images.push((capture.clone(), bytes)),
            Ok(_) => log::warn!("leaving out the capture {}: not a PNG image", capture.file),
            Err(e) => log::warn!("leaving out the capture {}: {e:#}", capture.file),
        }
    }
    Ok(Shared { stem: manifest.mode, game: shared, source, variants, inputs, images })
}

fn import_into(path: &Path, modes_dir: &Path) -> anyhow::Result<Imported> {
    let Shared { stem, game: shared, source, variants, inputs, images } = read_shared(path)?;
    let (mode, mode_existed) = match same_mode(modes_dir, &source) {
        Some(mode) => (mode, true),
        None => (install_mode(modes_dir, &stem, &source, &variants, inputs, images)?, false),
    };
    let games = Game::list();
    let existing = games.iter().find(|g| g.same_as(&shared.name, shared.steam_app_id)).cloned();
    let new_game = existing.is_none();
    let mut game = existing.unwrap_or_else(|| Game::new(&shared.name));
    let taken: Vec<&String> = games.iter().filter(|g| g.id != game.id).flat_map(|g| &g.executables).collect();
    game.steam_app_id = game.steam_app_id.or(shared.steam_app_id);
    for exe in &shared.executables {
        if !game.runs_as(exe) && !taken.iter().any(|t| t.eq_ignore_ascii_case(exe)) {
            game.executables.push(exe.clone());
        }
    }
    if !game.modes.contains(&mode) {
        game.modes.push(mode.clone());
    }
    game.save();
    log::info!("imported {} for {} ({})", stem, game.name, if new_game { "new game" } else { "existing game" });
    Ok(Imported { game: game.id, name: game.name, mode, new_game, mode_existed })
}

/// Replaces the package of `mode` (a user mode, by id) with what the
/// `.gameviber` file `path` holds: its script, variants, inputs and
/// captures (a new version of a mode installed from the community).
pub fn update(path: &Path, mode: &str) -> anyhow::Result<()> {
    let Shared { source, variants, inputs, images, .. } = read_shared(path)?;
    let entry = ModeEntry::from_id(mode);
    let script = entry.path().filter(|_| entry.dir().is_some()).context("only a package can be updated")?;
    let dir = entry.dir().context("only a package can be updated")?;
    for old in [config::VARIANTS_DIR, package::CAPTURES_DIR] {
        match fs::remove_dir_all(dir.join(old)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    fill_package(&script, &source, &variants, inputs, images)?;
    log::info!("{} updated", entry.key);
    Ok(())
}

/// The id of a package of `dir` with the very same script.
fn same_mode(dir: &Path, source: &str) -> Option<String> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path().join(config::MODE_FILE))
        .find(|p| fs::read_to_string(p).is_ok_and(|s| s == source))
        .map(|p| p.to_string_lossy().into_owned())
}

/// Makes a package of the mode in `dir`, with its inputs and the captures
/// `images` holds (those it lacks are left out); returns its id.
fn install_mode(
    dir: &Path,
    stem: &str,
    source: &str,
    variants: &[(String, String)],
    inputs: Inputs,
    images: Vec<(package::Capture, Vec<u8>)>,
) -> anyhow::Result<String> {
    let stem: String = stem.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect();
    let path = config::unused_mode_path_in(dir, if stem.is_empty() { "shared-mode" } else { &stem });
    fill_package(&path, source, variants, inputs, images)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Writes a package: its script at `path`, its variants, captures and inputs.
fn fill_package(
    path: &Path,
    source: &str,
    variants: &[(String, String)],
    mut inputs: Inputs,
    images: Vec<(package::Capture, Vec<u8>)>,
) -> anyhow::Result<()> {
    config::write_file(path, source).with_context(|| format!("cannot write {}", path.display()))?;
    inputs.dir = ModeEntry::from_id(&path.to_string_lossy()).dir().context("no package for the mode")?;
    for (name, text) in variants {
        let file = inputs.dir.join(config::VARIANTS_DIR).join(format!("{name}.{MODE_EXTENSION}"));
        config::write_file(&file, text).with_context(|| format!("cannot write {}", file.display()))?;
    }
    inputs.phases.truncate(MAX_PHASES);
    inputs.captures.clear();
    for (capture, bytes) in images {
        let of_phase = inputs.captures.iter().filter(|c| c.phase == capture.phase).count();
        if of_phase >= MAX_CAPTURES || inputs.captures.iter().any(|c| c.file == capture.file) {
            continue;
        }
        let file = package::capture_path(&inputs.dir, &capture.file);
        if let Some(captures) = file.parent() {
            config::create_dir(captures)?;
        }
        fs::write(&file, bytes)?;
        inputs.captures.push(package::Capture { embedding: Vec::new(), ..capture });
    }
    // Zones drawn on a capture left out are edited without it.
    for zone in &mut inputs.zones {
        if zone.capture.as_ref().is_some_and(|file| !inputs.captures.iter().any(|c| c.file == *file)) {
            zone.capture = None;
        }
    }
    inputs.save();
    Ok(())
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

    /// The names of the files it holds.
    fn names(&self) -> Vec<String> {
        self.zip.file_names().map(str::to_owned).collect()
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
    use crate::package::{Capture, ExternalInput, PhaseDef, Zone};
    use crate::screen::Frame;

    const MODE_SOURCE: &str = "mode { api = 1, name = \"Battles\" }\nfunction tick(dt, input) set(input.rumble.level) end\n";

    fn test_dirs(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("gameviber-sharing-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        game::TEST_DIR.with(|d| *d.borrow_mut() = Some(root.join("games")));
        (root.clone(), root.join("modes"))
    }

    fn frame(v: u8) -> Frame {
        Frame { width: 4, height: 2, source_width: 4, source_height: 2, count: 0, pixels: vec![v; 32] }
    }

    #[test]
    fn a_mode_travels_with_its_inputs_and_game() {
        let (root, modes) = test_dirs("round-trip");
        let mode_path = config::unused_mode_path_in(&modes, "battles");
        config::write_file(&mode_path, MODE_SOURCE).unwrap();
        let boss = MODE_SOURCE.replace("Battles", "Boss");
        let variant = config::unused_variant_path(mode_path.parent().unwrap(), "boss");
        config::write_file(&variant, &boss).unwrap();
        let mode = mode_path.to_string_lossy().into_owned();

        let mut inputs = Inputs::of(&ModeEntry::from_id(&mode));
        inputs.phases.push(PhaseDef { name: "battle".into(), sound: Some("battle music".into()), ..PhaseDef::default() });
        inputs.external.push(ExternalInput { name: "hp".into(), ..ExternalInput::default() });
        inputs.add_capture("battle", &frame(1)).unwrap();
        inputs.add_capture("", &frame(2)).unwrap();
        inputs.captures[0].embedding = vec![1.0, 0.0];
        let drawn_on = inputs.captures[0].file.clone();
        inputs.zones.push(Zone { indicator: "menu".into(), capture: Some(drawn_on.clone()), ..Zone::default() });
        inputs.save();
        let mut game = Game::new("Metaphor: ReFantazio");
        game.executables.push("METAPHOR.exe".into());
        game.steam_app_id = Some(2679460);
        game.audio = Some(config::AudioSource::Everything);
        game.modes.push(mode.clone());
        game.save();
        let file = root.join(file_name(&game, &mode));
        assert!(file.ends_with("metaphor-refantazio-battles.gameviber"));
        export(&game.id, &variant.to_string_lossy(), &file, &[]).unwrap();
        assert!(export(&game.id, "builtin:simple", &root.join("x.gameviber"), &[]).is_err(), "built-in modes are not shared");

        // Another player, without the game nor the mode.
        game.delete();
        fs::remove_dir_all(mode_path.parent().unwrap()).unwrap();
        let imported = import_into(&file, &modes).unwrap();
        assert!(imported.new_game && !imported.mode_existed);
        assert_eq!(imported.mode, mode, "the mode's package name");
        let got = Game::load(&imported.game).unwrap();
        assert_eq!((got.name.as_str(), &got.executables, &got.modes), ("Metaphor: ReFantazio", &game.executables, &vec![mode.clone()]));
        assert_eq!(got.audio, None, "the sound to listen to stays on each computer");
        assert_eq!(got.steam_app_id, Some(2679460));
        let got = Inputs::of(&ModeEntry::from_id(&mode));
        assert_eq!((&got.phases, &got.external), (&inputs.phases, &inputs.external));
        assert_eq!(got.captures.len(), 1, "captures to sort stay home");
        assert!(got.captures[0].embedding.is_empty(), "embeddings are computed again");
        assert_eq!(package::load_capture(&got.dir, &drawn_on).unwrap().pixels, vec![1; 32]);
        assert_eq!(got.zones[0].capture.as_deref(), Some(drawn_on.as_str()));
        assert_eq!(fs::read_to_string(&mode_path).unwrap(), MODE_SOURCE);
        assert_eq!(fs::read_to_string(&variant).unwrap(), boss, "exported from its variant, the mode comes with all of them");

        // Imported again: nothing doubles.
        let again = import_into(&file, &modes).unwrap();
        assert!(!again.new_game && again.mode_existed);
        assert_eq!(Game::load(&again.game).unwrap().modes, [mode.clone()]);
        assert_eq!(fs::read_dir(&modes).unwrap().count(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn files_of_the_first_format_give_their_game_inputs_to_the_mode() {
        let (root, modes) = test_dirs("format-1");
        config::create_dir(&root).unwrap();
        let path = root.join("old.gameviber");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        let game = serde_json::json!({
            "name": "Hades II", "executables": ["Hades2.exe"],
            "scenes": [{ "name": "boss" }], "zones": [{ "name": "hp", "kind": "bar" }],
            "captures": [{ "file": "boss-1.png", "scene": "boss", "embedding": [] }], "inputs": [], "modes": [],
        });
        let manifest = serde_json::json!({ "format": 1, "app_version": "0.1.0-alpha.2", "mode": "boss", "game": game });
        zip.start_file(MANIFEST, options).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.start_file(MODE, options).unwrap();
        zip.write_all(MODE_SOURCE.as_bytes()).unwrap();
        zip.start_file("captures/boss-1.png", options).unwrap();
        zip.write_all(PNG_SIGNATURE).unwrap();
        zip.finish().unwrap();
        let imported = import_into(&path, &modes).unwrap();
        let inputs = Inputs::of(&ModeEntry::from_id(&imported.mode));
        assert_eq!((inputs.phases[0].name.as_str(), inputs.zones[0].indicator.as_str(), inputs.captures.len()), ("boss", "hp", 1));
        assert!(package::capture_path(&inputs.dir, "boss-1.png").exists());
        assert_eq!(Game::load(&imported.game).unwrap().executables, ["Hades2.exe"]);
        let _ = fs::remove_dir_all(root);
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
            let inputs = Inputs { captures: vec![Capture { file: "../../evil.png".into(), phase: "a".into(), embedding: Vec::new() }], ..Inputs::default() };
            let game = serde_json::json!({ "name": "Test" });
            serde_json::to_vec(&Manifest { format, app_version: "9.0.0".into(), mode: mode.into(), game, inputs: Some(inputs) }).unwrap()
        };
        let error = |path: PathBuf| format!("{:#}", import_into(&path, &modes).unwrap_err());
        fs::write(root.join("text.gameviber"), "hello").unwrap();
        assert!(error(root.join("text.gameviber")).contains("not a file of a shared mode"));
        assert!(error(write("newer.gameviber", &[(MANIFEST, &manifest(FORMAT + 1, "m")), (MODE, MODE_SOURCE.as_bytes())])).contains("newer GameViber (9.0.0)"));
        assert!(error(write("broken.gameviber", &[(MANIFEST, &manifest(FORMAT, "m")), (MODE, b"mode {")])).contains("does not load"));
        let big = vec![0u8; MAX_ENTRY_SIZE as usize + 1];
        assert!(error(write("big.gameviber", &[(MANIFEST, &manifest(FORMAT, "m")), (MODE, &big)])).contains("too large"));

        let imported = import_into(&write("ok.gameviber", &[(MANIFEST, &manifest(FORMAT, "../x y")), (MODE, MODE_SOURCE.as_bytes()), ("captures/../../evil.png", PNG_SIGNATURE)]), &modes).unwrap();
        assert!(imported.mode.ends_with("/modes/xy/mode.luau"), "{}", imported.mode);
        assert!(Inputs::of(&ModeEntry::from_id(&imported.mode)).captures.is_empty(), "paths are refused");
        assert!(!root.join("evil.png").exists());
        let _ = fs::remove_dir_all(root);
    }
}
