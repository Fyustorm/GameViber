//! Settings, per-mode parameter values and presets, and the mode catalog, stored under
//! `platform::config_dir()` (`~/.config/gameviber` of the invoking user on Linux, even
//! when run through sudo).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub use gameviber_common::overlay::Corner;

use crate::mode::ParamValue;
use crate::platform;

pub const BUILTIN_PREFIX: &str = "builtin:";
pub const MODE_EXTENSION: &str = "luau";
/// A user mode's script in its package (`modes/<name>/mode.luau`, `package.rs`).
pub const MODE_FILE: &str = "mode.luau";
/// Its variants, other scripts reading the same inputs (`variants/<variant>.luau`).
pub const VARIANTS_DIR: &str = "variants";

const BUILTIN_MODES: [(&str, &str); 10] = [
    ("simple", include_str!("../modes/simple.luau")),
    ("accumulation", include_str!("../modes/accumulation.luau")),
    ("combo", include_str!("../modes/combo.luau")),
    ("overheat", include_str!("../modes/overheat.luau")),
    ("tension", include_str!("../modes/tension.luau")),
    ("engine", include_str!("../modes/engine.luau")),
    ("heartbeat", include_str!("../modes/heartbeat.luau")),
    ("all_or_nothing", include_str!("../modes/all_or_nothing.luau")),
    ("ambient", include_str!("../modes/ambient.luau")),
    ("surge", include_str!("../modes/surge.luau")),
];

pub const DEFAULT_MODE: &str = "builtin:simple";
pub const DEFAULT_LANGUAGE: &str = "English";

/// Template for modes created from the GUI.
pub const NEW_MODE_TEMPLATE: &str = r#"mode {
  api = 1,
  name = "NAME",
  description = "",
  params = {
    gain = number(1, 0, 3, "Gain", 0.1),
  },
}

function tick(dt, input)
  set(input.rumble.level * P.gain)
end
"#;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum SourceChoice {
    /// uinput virtual gamepad (root only needed to hide the real one)
    #[default]
    Proxy,
    /// Passive observation through an eBPF probe (root, through the helper; games see the real gamepad)
    Ebpf,
    /// No interception, simulator only
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub source: SourceChoice,
    /// Proxy source: hide the real gamepad from games.
    pub hide: bool,
    pub url: String,
    pub active_mode: String,
    /// Global intensity ceiling applied after the mode (safety layer).
    pub global_cap: f64,
    /// Channel -> toy names. A missing "main" entry means every toy.
    pub routing: BTreeMap<String, Vec<String>>,
    /// Gamepad buttons held together for the panic stop (at least two).
    pub panic_combo: Vec<String>,
    /// Gamepad buttons held together to mark a moment that felt wrong (at least two).
    pub mark_combo: Vec<String>,
    /// Gamepad buttons held together to capture the game's image into the active mode's captures.
    pub capture_combo: Vec<String>,
    /// Toy name -> how it renders intensities (missing: `ToySettings::default()`).
    pub toys: BTreeMap<String, ToySettings>,
    /// The first-launch setup was completed or skipped.
    pub onboarded: bool,
    pub overlay: OverlaySettings,
    /// Where the game's sound is captured from (docs/spec-modes.md §6.3).
    pub audio: AudioSource,
    /// Modes see the game's image, copied by the in-game overlay (§6.3, §6.4).
    pub screen: bool,
    /// Local port other programs send values and events to (§6.5); 0 turns it off.
    #[serde(alias = "inputs_port")]
    pub external_port: u16,
    /// Keyboard shortcuts for the combos' actions, through the desktop's portal.
    pub keyboard_shortcuts: bool,
    /// New GameViber versions are looked for on GitHub (`update`).
    pub check_updates: bool,
    /// The game last played, by id (`game.rs`).
    pub active_game: Option<String>,
    /// Language AI assistants answer in, and write the texts players see in a mode
    /// (the GUI itself is in English for now).
    pub language: String,
}

/// Port of the local server other programs send values and events to.
pub const DEFAULT_EXTERNAL_PORT: u16 = 12350;

/// Which sound the audio analysis listens to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioSource {
    Off,
    /// The game showing the in-game overlay, or else everything the computer plays.
    #[default]
    Auto,
    /// Everything the computer plays (the default output).
    Everything,
    /// One application, by the name it gives PipeWire.
    App(String),
}

/// How one toy renders the 0..1 intensity it is asked for (safety layer, after the mode).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToySettings {
    /// Intensity the weakest felt vibration is raised to (motors often do nothing below it).
    pub min: f64,
    /// Intensity the strongest vibration is lowered to.
    pub max: f64,
    /// Response curve exponent: below 1 low levels feel stronger, above 1 softer.
    pub curve: f64,
}

impl Default for ToySettings {
    fn default() -> Self {
        Self { min: 0.0, max: 1.0, curve: 1.0 }
    }
}

impl ToySettings {
    pub const CURVE_RANGE: std::ops::RangeInclusive<f64> = 0.4..=2.5;
    /// Requested intensities below this stay off, whatever `min`.
    pub const SILENT: f64 = 0.01;

    /// Toy intensity for a requested 0..1 intensity: 0 stays 0, anything else is
    /// shaped by the curve then spread between `min` and `max`.
    pub fn shape(&self, level: f64) -> f64 {
        if level < Self::SILENT {
            return 0.0;
        }
        let curve = self.curve.clamp(*Self::CURVE_RANGE.start(), *Self::CURVE_RANGE.end());
        let max = self.max.clamp(0.0, 1.0);
        let min = self.min.clamp(0.0, max);
        min + (max - min) * level.min(1.0).powf(curve)
    }
}

/// In-game overlay preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlaySettings {
    /// Show the overlay in games where the layer is active.
    pub visible: bool,
    pub corner: Corner,
    pub scale: f32,
    /// Panel background opacity, 0..1.
    pub opacity: f32,
    /// The layer is installed for every Vulkan game rather than per game.
    pub all_games: bool,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self { visible: true, corner: Corner::TopLeft, scale: 1.0, opacity: 0.75, all_games: false }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            source: SourceChoice::Proxy,
            hide: false,
            url: "ws://127.0.0.1:12345".into(),
            active_mode: DEFAULT_MODE.into(),
            global_cap: 1.0,
            routing: BTreeMap::new(),
            panic_combo: crate::gamepad::DEFAULT_PANIC_COMBO.map(str::to_owned).to_vec(),
            mark_combo: crate::gamepad::DEFAULT_MARK_COMBO.map(str::to_owned).to_vec(),
            capture_combo: crate::gamepad::DEFAULT_CAPTURE_COMBO.map(str::to_owned).to_vec(),
            toys: BTreeMap::new(),
            onboarded: false,
            overlay: OverlaySettings::default(),
            audio: AudioSource::Auto,
            screen: true,
            external_port: DEFAULT_EXTERNAL_PORT,
            keyboard_shortcuts: false,
            check_updates: true,
            active_game: None,
            language: DEFAULT_LANGUAGE.into(),
        }
    }
}

pub use crate::platform::{config_dir, data_dir};

#[cfg(test)]
thread_local! {
    /// Tests keep their modes out of the player's.
    pub(crate) static MODES_TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

pub fn modes_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(dir) = MODES_TEST_DIR.with(|d| d.borrow().clone()) {
        return dir;
    }
    config_dir().join("modes")
}

pub fn create_dir(dir: &Path) -> io::Result<()> {
    if !dir.exists() {
        if let Some(parent) = dir.parent() {
            create_dir(parent)?;
        }
        fs::create_dir(dir)?;
        platform::chown_to_caller(dir);
    }
    Ok(())
}

pub fn write_file(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        create_dir(dir)?;
    }
    fs::write(path, contents)?;
    platform::chown_to_caller(path);
    Ok(())
}

fn read_toml<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            log::warn!("ignoring invalid {}: {e}", path.display());
            T::default()
        }),
        Err(_) => T::default(),
    }
}

fn write_toml<T: Serialize>(path: &Path, value: &T) {
    let result = toml::to_string_pretty(value).map_err(io::Error::other).and_then(|text| write_file(path, &text));
    if let Err(e) = result {
        log::warn!("cannot save {}: {e}", path.display());
    }
}

impl Settings {
    fn path() -> PathBuf {
        config_dir().join("settings.toml")
    }

    pub fn load() -> Self {
        read_toml(&Self::path())
    }

    pub fn save(&self) {
        write_toml(&Self::path(), self);
    }
}

/// Named parameter sets of a mode, stored in `presets/<key>.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Presets {
    /// Preset last loaded or saved, shown as the current one in the GUI.
    pub active: Option<String>,
    pub presets: BTreeMap<String, BTreeMap<String, ParamValue>>,
}

/// The rounds of fixing a mode that did not feel right, stored in `feedback/<key>.toml`,
/// so that the next request tells the AI assistant what was already tried.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedbackHistory {
    pub rounds: Vec<FeedbackRound>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeedbackRound {
    /// A request was sent: what the player answered.
    Request { date: String, version: String, answers: Vec<String>, words: String },
    /// A question's quick fix changed a setting.
    QuickFix { date: String, version: String, question: String, answer: String, setting: String, from: f64, to: f64 },
    /// The assistant's corrected mode was applied.
    Fixed { date: String, from_version: String, to_version: String },
}

impl FeedbackHistory {
    /// Rounds kept; older ones are dropped.
    pub const MAX_ROUNDS: usize = 30;

    pub fn push(&mut self, round: FeedbackRound) {
        // Copying the request again for the same version replaces the previous copy.
        if let (FeedbackRound::Request { version, .. }, Some(FeedbackRound::Request { version: last, .. })) =
            (&round, self.rounds.last())
        {
            if version == last {
                self.rounds.pop();
            }
        }
        self.rounds.push(round);
        let excess = self.rounds.len().saturating_sub(Self::MAX_ROUNDS);
        self.rounds.drain(..excess);
    }

    /// One line per round, oldest first, for the AI assistant.
    pub fn lines(&self) -> Vec<String> {
        let version = |v: &str| if v.is_empty() { String::new() } else { format!(" (version {v})") };
        self.rounds
            .iter()
            .map(|round| match round {
                FeedbackRound::Request { date, version: v, answers, words } => {
                    let mut said = answers.join("; ");
                    if !words.trim().is_empty() {
                        if !said.is_empty() {
                            said.push_str("; ");
                        }
                        said.push_str(&format!("\"{}\"", words.trim()));
                    }
                    if said.is_empty() {
                        said = "nothing specific".into();
                    }
                    format!("{date}{}: the player reported: {said}", version(v))
                }
                FeedbackRound::QuickFix { date, version: v, question, answer, setting, from, to } => {
                    format!("{date}{}: quick fix for \"{question}: {answer}\": {setting} {from} -> {to}", version(v))
                }
                FeedbackRound::Fixed { date, from_version, to_version } => {
                    format!("{date}: the assistant's corrected mode was applied ({from_version} -> {to_version})")
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModeEntry {
    /// "builtin:<name>" or the absolute path of a user mode file.
    pub id: String,
    /// Built-in name, package name (`<package>.<variant>` for a variant), or
    /// file stem; also the key of the saved parameters.
    pub key: String,
    pub builtin: bool,
    /// The variant's name, for a variant of a package.
    pub variant: Option<String>,
}

impl ModeEntry {
    pub fn from_id(id: &str) -> Self {
        match id.strip_prefix(BUILTIN_PREFIX) {
            Some(name) => Self { id: id.to_owned(), key: name.to_owned(), builtin: true, variant: None },
            None => {
                let path = Path::new(id);
                let text = |name: Option<&std::ffi::OsStr>| name.map(|s| s.to_string_lossy().into_owned());
                let parent = path.parent();
                if path.file_name().is_some_and(|f| f == MODE_FILE) {
                    let key = text(parent.and_then(Path::file_name)).unwrap_or_else(|| id.to_owned());
                    return Self { id: id.to_owned(), key, builtin: false, variant: None };
                }
                let package = parent.filter(|p| p.file_name().is_some_and(|f| f == VARIANTS_DIR)).and_then(Path::parent);
                match (text(package.and_then(Path::file_name)), text(path.file_stem())) {
                    (Some(package), Some(variant)) => {
                        Self { id: id.to_owned(), key: format!("{package}.{variant}"), builtin: false, variant: Some(variant) }
                    }
                    (None, stem) => Self { id: id.to_owned(), key: stem.unwrap_or_else(|| id.to_owned()), builtin: false, variant: None },
                    (Some(_), None) => Self { id: id.to_owned(), key: id.to_owned(), builtin: false, variant: None },
                }
            }
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        (!self.builtin).then(|| PathBuf::from(&self.id))
    }

    /// The mode's package (`package.rs`), its variant's too: None for a
    /// built-in mode, or a file run from elsewhere.
    pub fn dir(&self) -> Option<PathBuf> {
        let path = self.path()?;
        let parent = path.parent()?;
        match &self.variant {
            Some(_) => parent.parent().map(Path::to_path_buf),
            None => path.file_name().is_some_and(|f| f == MODE_FILE).then(|| parent.to_path_buf()),
        }
    }

    /// The id of the mode a variant belongs to (its own for a mode).
    pub fn main_id(&self) -> String {
        match (&self.variant, self.dir()) {
            (Some(_), Some(dir)) => dir.join(MODE_FILE).to_string_lossy().into_owned(),
            _ => self.id.clone(),
        }
    }

    /// The variants of the mode's package, sorted by name.
    pub fn variants(&self) -> Vec<ModeEntry> {
        let Some(dir) = self.dir() else { return Vec::new() };
        let mut variants: Vec<ModeEntry> = fs::read_dir(dir.join(VARIANTS_DIR))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == MODE_EXTENSION))
            .map(|p| ModeEntry::from_id(&p.to_string_lossy()))
            .collect();
        variants.sort_by(|a, b| a.key.cmp(&b.key));
        variants
    }

    pub fn chunk_name(&self) -> String {
        format!("{}.{MODE_EXTENSION}", self.key)
    }

    pub fn source(&self) -> io::Result<String> {
        match self.id.strip_prefix(BUILTIN_PREFIX) {
            Some(name) => BUILTIN_MODES
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, src)| (*src).to_owned())
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no built-in mode '{name}'"))),
            None => fs::read_to_string(&self.id),
        }
    }

    pub fn modified(&self) -> Option<SystemTime> {
        fs::metadata(self.path()?).and_then(|m| m.modified()).ok()
    }

    fn params_path(&self) -> PathBuf {
        config_dir().join("params").join(format!("{}.toml", self.key))
    }

    pub fn load_params(&self) -> BTreeMap<String, ParamValue> {
        read_toml(&self.params_path())
    }

    pub fn save_params(&self, values: &BTreeMap<String, ParamValue>) {
        write_toml(&self.params_path(), values);
    }

    fn presets_path(&self) -> PathBuf {
        config_dir().join("presets").join(format!("{}.toml", self.key))
    }

    pub fn load_presets(&self) -> Presets {
        read_toml(&self.presets_path())
    }

    pub fn save_presets(&self, presets: &Presets) {
        write_toml(&self.presets_path(), presets);
    }

    fn feedback_path(&self) -> PathBuf {
        config_dir().join("feedback").join(format!("{}.toml", self.key))
    }

    pub fn load_feedback(&self) -> FeedbackHistory {
        read_toml(&self.feedback_path())
    }

    pub fn save_feedback(&self, history: &FeedbackHistory) {
        write_toml(&self.feedback_path(), history);
    }

    /// Deletes a user mode's package (or file and backup), and its saved
    /// parameters, presets and feedback history, so a new mode with the same
    /// name starts afresh.
    pub fn delete(&self) -> io::Result<()> {
        let Some(path) = self.path() else {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "built-in modes cannot be deleted"));
        };
        match self.dir() {
            Some(dir) if self.variant.is_none() => fs::remove_dir_all(&dir)?,
            _ => fs::remove_file(&path)?,
        }
        for file in [path.with_extension("luau.bak"), self.params_path(), self.presets_path(), self.feedback_path()] {
            if let Err(e) = fs::remove_file(&file) {
                if e.kind() != io::ErrorKind::NotFound {
                    log::warn!("cannot delete {}: {e}", file.display());
                }
            }
        }
        Ok(())
    }
}

/// Built-in modes first, then the packages `~/.config/gameviber/modes/*/mode.luau` sorted by name.
pub fn list_modes() -> Vec<ModeEntry> {
    let mut modes: Vec<_> = BUILTIN_MODES.iter().map(|(name, _)| ModeEntry::from_id(&format!("{BUILTIN_PREFIX}{name}"))).collect();
    let mut user: Vec<_> = fs::read_dir(modes_dir())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path().join(MODE_FILE))
        .filter(|p| p.is_file())
        .map(|p| ModeEntry::from_id(&p.to_string_lossy()))
        .collect();
    user.sort_by(|a, b| a.key.cmp(&b.key));
    for mode in user {
        let variants = mode.variants();
        modes.push(mode);
        modes.extend(variants);
    }
    modes
}

/// The script of the first free package `<stem>/`, `<stem>-2/`... in the modes directory.
pub fn unused_mode_path(stem: &str) -> PathBuf {
    unused_mode_path_in(&modes_dir(), stem)
}

/// The first free variant `<stem>`, `<stem>-2`... of the package `dir`.
pub fn unused_variant_path(dir: &Path, stem: &str) -> PathBuf {
    let mut n = 1;
    loop {
        let name = if n == 1 { stem.to_owned() } else { format!("{stem}-{n}") };
        let path = dir.join(VARIANTS_DIR).join(format!("{name}.{MODE_EXTENSION}"));
        if !path.exists() {
            return path;
        }
        n += 1;
    }
}

/// The script of the first free package `<stem>/`, `<stem>-2/`... in `dir`.
pub fn unused_mode_path_in(dir: &Path, stem: &str) -> PathBuf {
    let mut n = 1;
    loop {
        let name = if n == 1 { stem.to_owned() } else { format!("{stem}-{n}") };
        let package = dir.join(&name);
        if !package.exists() {
            return package.join(MODE_FILE);
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_modes_are_under_the_mit_license() {
        for (name, source) in BUILTIN_MODES {
            assert!(source.starts_with("-- SPDX-License-Identifier: MIT\n"), "{name}: first line (AGENTS.md, Licenses)");
        }
    }

    #[test]
    fn audio_source_round_trips_in_the_settings_file() {
        for audio in [AudioSource::Off, AudioSource::Auto, AudioSource::Everything, AudioSource::App("METAPHOR.exe".into())] {
            let settings = Settings { audio: audio.clone(), ..Settings::default() };
            let text = toml::to_string(&settings).unwrap();
            let back: Settings = toml::from_str(&text).unwrap();
            assert_eq!(back.audio, audio, "{text}");
        }
        let old: Settings = toml::from_str("url = 'ws://x'").unwrap();
        assert_eq!(old.audio, AudioSource::Auto, "settings saved before audio existed");
    }

    #[test]
    fn builtin_entries_resolve_source_and_key() {
        let e = ModeEntry::from_id("builtin:simple");
        assert!(e.builtin && e.key == "simple" && e.path().is_none());
        assert!(e.source().unwrap().contains("mode {"));
        assert!(ModeEntry::from_id("builtin:nope").source().is_err());
    }

    #[test]
    fn user_entries_use_their_package_or_file_stem() {
        let e = ModeEntry::from_id("/x/modes/combo/mode.luau");
        assert_eq!((e.key.as_str(), e.builtin, e.chunk_name().as_str()), ("combo", false, "combo.luau"));
        assert_eq!(e.dir(), Some(PathBuf::from("/x/modes/combo")));
        let e = ModeEntry::from_id("/x/modes/combo/variants/boss.luau");
        assert_eq!((e.key.as_str(), e.variant.as_deref(), e.chunk_name().as_str()), ("combo.boss", Some("boss"), "combo.boss.luau"));
        assert_eq!(e.dir(), Some(PathBuf::from("/x/modes/combo")), "the package's inputs");
        assert_eq!(e.main_id(), "/x/modes/combo/mode.luau");
        let e = ModeEntry::from_id("/elsewhere/combo.luau");
        assert_eq!((e.key.as_str(), e.dir()), ("combo", None), "a file run from elsewhere has no package");
        assert_eq!(ModeEntry::from_id("builtin:combo").dir(), None);
    }

    #[test]
    fn feedback_history_round_trips_and_merges_requests() {
        let mut history = FeedbackHistory::default();
        let request = |answers: &[&str]| FeedbackRound::Request {
            date: "2026-10-03 20:00".into(),
            version: "1.0".into(),
            answers: answers.iter().map(|a| a.to_string()).collect(),
            words: String::new(),
        };
        history.push(request(&["Dash: Too short"]));
        history.push(request(&["Dash: Too long"]));
        history.push(FeedbackRound::QuickFix {
            date: "d".into(),
            version: "1.0".into(),
            question: "Dash".into(),
            answer: "Too long".into(),
            setting: "Dash length (s)".into(),
            from: 0.4,
            to: 0.3,
        });
        history.push(FeedbackRound::Fixed { date: "d".into(), from_version: "1.0".into(), to_version: "1.1".into() });
        assert_eq!(history.rounds.len(), 3, "the second request replaced the first");
        let text = toml::to_string(&history).unwrap();
        assert_eq!(toml::from_str::<FeedbackHistory>(&text).unwrap(), history);
        let lines = history.lines();
        assert_eq!(lines[0], "2026-10-03 20:00 (version 1.0): the player reported: Dash: Too long");
        assert_eq!(lines[1], "d (version 1.0): quick fix for \"Dash: Too long\": Dash length (s) 0.4 -> 0.3");
        assert_eq!(lines[2], "d: the assistant's corrected mode was applied (1.0 -> 1.1)");
    }

    #[test]
    fn toy_settings_shape_levels() {
        let default = ToySettings::default();
        assert_eq!(default.shape(0.0), 0.0);
        assert_eq!(default.shape(0.42), 0.42);
        let toy = ToySettings { min: 0.2, max: 0.8, curve: 2.0 };
        assert_eq!(toy.shape(0.0), 0.0);
        assert_eq!(toy.shape(0.005), 0.0);
        assert!((toy.shape(0.5) - (0.2 + 0.6 * 0.25)).abs() < 1e-9);
        assert!((toy.shape(1.0) - 0.8).abs() < 1e-9);
        // An inverted range never goes above max.
        let odd = ToySettings { min: 0.9, max: 0.5, curve: 1.0 };
        assert!(odd.shape(0.1) <= 0.5);
    }

    #[test]
    fn template_is_a_valid_mode() {
        let rt = crate::mode::ModeRuntime::load("t", NEW_MODE_TEMPLATE, &BTreeMap::new(), None);
        assert!(rt.is_ok(), "{:?}", rt.err());
    }

    #[test]
    fn params_round_trip_through_toml() {
        let values = BTreeMap::from([
            ("a".to_owned(), ParamValue::Number(1.5)),
            ("b".to_owned(), ParamValue::Bool(true)),
            ("c".to_owned(), ParamValue::Text("A".into())),
        ]);
        let text = toml::to_string_pretty(&values).unwrap();
        let back: BTreeMap<String, ParamValue> = toml::from_str(&text).unwrap();
        assert_eq!(back, values);
    }

    #[test]
    fn presets_round_trip_through_toml() {
        let values = BTreeMap::from([("window".to_owned(), ParamValue::Number(0.4))]);
        let presets = Presets {
            active: Some("Tekken 8".into()),
            presets: BTreeMap::from([("Tekken 8".to_owned(), values.clone()), ("soft.v2".to_owned(), values)]),
        };
        let text = toml::to_string_pretty(&presets).unwrap();
        assert_eq!(toml::from_str::<Presets>(&text).unwrap(), presets);
        let none = Presets { active: None, ..presets };
        let text = toml::to_string_pretty(&none).unwrap();
        assert_eq!(toml::from_str::<Presets>(&text).unwrap(), none);
    }
}
