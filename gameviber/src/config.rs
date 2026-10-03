//! Settings, per-mode parameter values and presets, and the mode catalog, stored under
//! `~/.config/gameviber` of the invoking user (even when run through sudo).

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::mode::ParamValue;

pub const BUILTIN_PREFIX: &str = "builtin:";
pub const MODE_EXTENSION: &str = "luau";

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
    /// The first-launch setup was completed or skipped.
    pub onboarded: bool,
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
            onboarded: false,
        }
    }
}

/// Home of the user who started us: the sudo caller rather than root.
fn user_home() -> PathBuf {
    if let Ok(user) = std::env::var("SUDO_USER") {
        if let Ok(name) = std::ffi::CString::new(user) {
            // SAFETY: getpwnam returns a pointer to static storage or null.
            let pw = unsafe { libc::getpwnam(name.as_ptr()) };
            if !pw.is_null() {
                let dir = unsafe { CStr::from_ptr((*pw).pw_dir) };
                return PathBuf::from(dir.to_string_lossy().into_owned());
            }
        }
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_dir() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if std::env::var_os("SUDO_USER").is_none() => PathBuf::from(dir).join("gameviber"),
        _ => user_home().join(".config").join("gameviber"),
    }
}

pub fn modes_dir() -> PathBuf {
    config_dir().join("modes")
}

/// Gives files created as root back to the sudo caller.
fn chown_to_caller(path: &Path) {
    let (Ok(uid), Ok(gid)) = (std::env::var("SUDO_UID"), std::env::var("SUDO_GID")) else { return };
    let (Ok(uid), Ok(gid)) = (uid.parse(), gid.parse()) else { return };
    let _ = std::os::unix::fs::chown(path, Some(uid), Some(gid));
}

fn create_dir(dir: &Path) -> io::Result<()> {
    if !dir.exists() {
        if let Some(parent) = dir.parent() {
            create_dir(parent)?;
        }
        fs::create_dir(dir)?;
        chown_to_caller(dir);
    }
    Ok(())
}

pub fn write_file(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        create_dir(dir)?;
    }
    fs::write(path, contents)?;
    chown_to_caller(path);
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

#[derive(Debug, Clone, PartialEq)]
pub struct ModeEntry {
    /// "builtin:<name>" or the absolute path of a user mode file.
    pub id: String,
    /// Built-in name or file stem; also the key of the saved parameters.
    pub key: String,
    pub builtin: bool,
}

impl ModeEntry {
    pub fn from_id(id: &str) -> Self {
        match id.strip_prefix(BUILTIN_PREFIX) {
            Some(name) => Self { id: id.to_owned(), key: name.to_owned(), builtin: true },
            None => {
                let key = Path::new(id).file_stem().map_or_else(|| id.to_owned(), |s| s.to_string_lossy().into_owned());
                Self { id: id.to_owned(), key, builtin: false }
            }
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        (!self.builtin).then(|| PathBuf::from(&self.id))
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
}

/// Built-in modes first, then `~/.config/gameviber/modes/*.luau` sorted by name.
pub fn list_modes() -> Vec<ModeEntry> {
    let mut modes: Vec<_> = BUILTIN_MODES.iter().map(|(name, _)| ModeEntry::from_id(&format!("{BUILTIN_PREFIX}{name}"))).collect();
    let mut user: Vec<_> = fs::read_dir(modes_dir())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == MODE_EXTENSION))
        .map(|p| ModeEntry::from_id(&p.to_string_lossy()))
        .collect();
    user.sort_by(|a, b| a.key.cmp(&b.key));
    modes.extend(user);
    modes
}

/// First free `<stem>.luau`, `<stem>-2.luau`... in the modes directory.
pub fn unused_mode_path(stem: &str) -> PathBuf {
    let dir = modes_dir();
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{stem}.{MODE_EXTENSION}") } else { format!("{stem}-{n}.{MODE_EXTENSION}") };
        let path = dir.join(name);
        if !path.exists() {
            return path;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_entries_resolve_source_and_key() {
        let e = ModeEntry::from_id("builtin:simple");
        assert!(e.builtin && e.key == "simple" && e.path().is_none());
        assert!(e.source().unwrap().contains("mode {"));
        assert!(ModeEntry::from_id("builtin:nope").source().is_err());
    }

    #[test]
    fn user_entries_use_file_stem() {
        let e = ModeEntry::from_id("/x/modes/combo.luau");
        assert_eq!((e.key.as_str(), e.builtin, e.chunk_name().as_str()), ("combo", false, "combo.luau"));
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
