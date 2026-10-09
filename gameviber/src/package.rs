//! Mode packages: a user mode is a directory of `modes/`, holding its script
//! (`mode.luau`), its variants (other scripts reading the same inputs,
//! `variants/<variant>.luau`), the inputs the player set up for it (`mode.json`, docs/spec-modes.md
//! §6.3, §6.5) — its phases, captures of them (images the player took, which
//! are also the examples phases are recognized with, in `captures/`), indicators
//! of the game's screen drawn on them, the values and events other programs send —
//! so that a mode travels with everything it reads. Built-in modes have no package:
//! they read no input set up by the player.

use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{self, ModeEntry};
use crate::game;
use crate::mode::phases::{Ignored, PhaseDecl};
use crate::screen::Frame;

/// The inputs of a mode, in its package.
pub const INPUTS_FILE: &str = "mode.json";
pub const CAPTURES_DIR: &str = "captures";
/// Motions for strokers the script plays (`funscript()`, docs/spec-modes.md §8.5).
pub const FUNSCRIPTS_DIR: &str = "funscripts";

/// The funscripts of the package in `dir` (none without a package).
pub fn funscripts(dir: &Path) -> crate::funscript::Funscripts {
    if dir.as_os_str().is_empty() {
        return Default::default();
    }
    crate::funscript::load_dir(&dir.join(FUNSCRIPTS_DIR))
}
/// A mode keeps at most this many captures per phase.
pub const MAX_CAPTURES: usize = 40;
/// A mode has at most this many phases (as modes may declare).
pub const MAX_PHASES: usize = 8;

/// A phase of a game: a name modes read (`input.phase`), and how it sounds
/// for the sound model; how it looks comes from its captures, and optionally
/// a description for the image model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhaseDef {
    pub name: String,
    pub sound: Option<String>,
    pub screen: Option<String>,
    /// Indicators whose showing, all together, is a sure sign of the phase (its
    /// battle menu; a gauge and a menu); `indicator` (one) before a sign could
    /// be several, `zone` before the terms changed.
    #[serde(alias = "indicator", alias = "zone", deserialize_with = "one_or_many", skip_serializing_if = "Vec::is_empty")]
    pub indicators: Vec<String>,
    /// The phase while indicators are read and no phase's sign is shown (story:
    /// none of the menus); one per mode.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub otherwise: bool,
    /// Seconds the phase is kept after its last sign: longer for signs that come and go.
    pub hold: f64,
    /// Events the mode does not get during the phase.
    #[serde(skip_serializing_if = "Ignored::is_none")]
    pub ignore: Ignored,
}

impl Default for PhaseDef {
    fn default() -> Self {
        Self { name: String::new(), sound: None, screen: None, indicators: Vec::new(), otherwise: false, hold: DEFAULT_HOLD, ignore: Ignored::default() }
    }
}

/// A list, or one indicator (or none) as stored before a sign could be several.
fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(Option<String>),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(one) => one.into_iter().collect(),
        OneOrMany::Many(many) => many,
    })
}

/// Seconds a phase is kept after its last sign, unless set otherwise.
pub const DEFAULT_HOLD: f64 = 3.0;

/// An image of the game the player captured as an example of a phase.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Capture {
    /// PNG file name in the package's `captures/`.
    pub file: String,
    /// Its phase (`phase` before the terms changed); empty for a capture to
    /// sort (taken in game, filed later).
    #[serde(alias = "scene")]
    pub phase: String,
    /// Its embedding (`screen::clip`); empty until the image model computed it.
    pub embedding: Vec<f32>,
}

/// What an indicator reads (stored `visible` and `bar` before the terms changed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndicatorKind {
    /// Is this element on screen? Compared with how it looked when the zone was drawn.
    #[default]
    #[serde(alias = "visible")]
    Visibility,
    /// How full is this bar? Measured with the bar's color, or its look.
    #[serde(alias = "bar")]
    Gauge,
}

impl IndicatorKind {
    pub fn label(self) -> &'static str {
        match self {
            IndicatorKind::Visibility => "Visibility (shown or not)",
            IndicatorKind::Gauge => "Gauge (how full)",
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
    /// The indicator it is a zone of, by the name modes read it by
    /// (`input.indicators.<name>`; `name` before the terms changed). An
    /// indicator shown in several places (a bar shown elsewhere out of
    /// battles) has a zone for each: its value comes from the zone where it is found.
    #[serde(alias = "name")]
    pub indicator: String,
    pub kind: IndicatorKind,
    /// Left, top, width, height, as fractions of the screen.
    pub rect: [f32; 4],
    /// Visibility: grayscale look of the zone when it was drawn (`screen::indicators`).
    pub reference: Vec<u8>,
    /// Visibility: similarity with the reference (-1..1) above which the element is shown.
    pub threshold: f32,
    /// Gauge: the color of its filled part...
    pub color: [u8; 3],
    /// ...and of its empty part, when the player picked it. With it, the bar is
    /// found in the zone as the longest run of its two colors, so the zone may
    /// be larger than the bar, or cover every place a moving bar can be.
    pub empty_color: Option<[u8; 3]>,
    /// Gauge: other shades of its filled part (a bar blinking when low)...
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub more_colors: Vec<[u8; 3]>,
    /// ...and of its empty part.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub more_empty: Vec<[u8; 3]>,
    /// Gauge: the colors (shades) of its next tiers, for a bar filled again
    /// over itself in another color once full (Prince of Persia's Athra: green
    /// up to half, then yellow over the green). Each tier is an equal share of
    /// the value: with one more, the first color reads 0..0.5, the second 0.5..1.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<Vec<[u8; 3]>>,
    /// Gauge: read only while these visibility indicators are shown (the frame
    /// around the bar) or hidden (a menu over it), unknown otherwise: a bar
    /// whose empty color is also the menus' background reads empty there.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub read_when: Vec<Condition>,
    /// Gauge: share of the zone's length it covered when drawn. Much less of its
    /// colors found means it is not on screen (menus): its value is unknown.
    pub length: f32,
    /// Bar read by its look rather than its colors (gradients, segments,
    /// hearts): the zone full, a grid of colors along it taken from a
    /// capture (`screen::indicators`, left to right and top to bottom)...
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub full_look: Vec<[u8; 3]>,
    /// ...and empty, as far as captures showed it so (None: not seen empty yet).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub empty_look: Vec<Option<[u8; 3]>>,
    /// The phase of the capture it was drawn on (shown there, for a visible
    /// zone; `phase` before the terms changed)...
    #[serde(alias = "scene")]
    pub phase: Option<String>,
    /// ...and that capture's file, to show it again when the zone is edited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture: Option<String>,
    pub direction: Direction,
    /// Gauge: how far (0..255 per channel) a pixel may be from `color`.
    pub tolerance: f32,
}

impl Default for Zone {
    fn default() -> Self {
        Self {
            indicator: String::new(),
            kind: IndicatorKind::Visibility,
            rect: [0.0, 0.0, 0.1, 0.1],
            reference: Vec::new(),
            threshold: 0.6,
            color: [0, 0, 0],
            empty_color: None,
            more_colors: Vec::new(),
            more_empty: Vec::new(),
            tiers: Vec::new(),
            read_when: Vec::new(),
            length: 0.0,
            full_look: Vec::new(),
            empty_look: Vec::new(),
            phase: None,
            capture: None,
            direction: Direction::Right,
            tolerance: 60.0,
        }
    }
}

/// A condition a gauge is read under (`Zone::read_when`): a visibility
/// indicator shown, or hidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    pub indicator: String,
    pub shown: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalKind {
    /// A value kept in `input.external.<name>`.
    #[default]
    Value,
    /// An event passed to `on_event`.
    Event,
}

/// A value or an event another program sends, declared so that AI assistants know it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalInput {
    pub name: String,
    pub kind: ExternalKind,
    /// What it means and its range ("health, 0 to 100").
    pub description: String,
}

/// Names modes use: letters, digits and underscores, starting with a letter.
pub fn valid_name(name: &str) -> bool {
    name.len() <= 32
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The inputs the player set up for a mode (`mode.json` in its package).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Inputs {
    /// The package's directory, set when loaded; empty for a mode without
    /// one (built-in, or a file outside `modes/`).
    #[serde(skip)]
    pub dir: PathBuf,
    /// Its phases (§6.3), `phases` before the terms changed.
    #[serde(alias = "scenes")]
    pub phases: Vec<PhaseDef>,
    /// The zones of the screen its indicators are read in: zones sharing a
    /// name are one indicator.
    pub zones: Vec<Zone>,
    /// Images of the game the player captured, by phase.
    pub captures: Vec<Capture>,
    /// Values and events other programs send while the mode is played (§6.5),
    /// for the requests to AI assistants; `inputs` before the terms changed.
    #[serde(alias = "inputs")]
    pub external: Vec<ExternalInput>,
    /// Indicators an assistant's analysis proposed, to draw (those drawn are left out).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub planned: Vec<PlannedIndicator>,
    /// The player's own instructions, written at the end of the requests for the mode's script.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub instructions: String,
}

/// An indicator an assistant proposed: what it reads and where to draw it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlannedIndicator {
    pub name: String,
    pub kind: IndicatorKind,
    /// Where it is on the screen, and when to capture it.
    pub place: String,
}

/// What an analysis's setup changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SetupApplied {
    pub phases_added: usize,
    pub phases_completed: usize,
    pub indicators_to_draw: usize,
}

/// Where a capture of the package in `dir` is (its PNG file).
pub fn capture_path(dir: &Path, file: &str) -> PathBuf {
    dir.join(CAPTURES_DIR).join(file)
}

/// Saves `frame` as a PNG capture in the package `dir`; returns its file name.
pub fn save_capture(dir: &Path, phase: &str, frame: &Frame) -> std::io::Result<String> {
    let dir = dir.join(CAPTURES_DIR);
    config::create_dir(&dir)?;
    let stamp = crate::platform::local_time().replace([' ', ':'], "-");
    let label = if phase.is_empty() { "capture" } else { phase };
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

/// Image files `read_image` reads.
pub const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "bmp"];

/// An image file (a screenshot, an image found online) as a frame to add as
/// a capture: no wider or higher than the overlay's copies can be.
pub fn read_image(path: &Path) -> anyhow::Result<Frame> {
    use gameviber_common::overlay::frames::{MAX_HEIGHT, MAX_WIDTH};
    let mut image = image::ImageReader::open(path)?.with_guessed_format()?.decode()?;
    if image.width() > MAX_WIDTH || image.height() > MAX_HEIGHT {
        image = image.resize(MAX_WIDTH, MAX_HEIGHT, image::imageops::FilterType::Triangle);
    }
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(Frame { width, height, source_width: width, source_height: height, count: 0, pixels: rgba.into_raw() })
}

/// A capture of the package in `dir`, as a frame.
pub fn load_capture(dir: &Path, file: &str) -> Option<Frame> {
    let reader = BufReader::new(fs::File::open(capture_path(dir, file)).ok()?);
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

fn delete_capture(dir: &Path, file: &str) {
    if let Err(e) = fs::remove_file(capture_path(dir, file)) {
        log::warn!("cannot delete the capture {file}: {e}");
    }
}

impl Inputs {
    /// The inputs of `mode`: none set up yet, or none at all for a mode without a package.
    pub fn of(mode: &ModeEntry) -> Self {
        mode.dir().map(|dir| Self::load(&dir)).unwrap_or_default()
    }

    /// The inputs of the package in `dir`.
    pub fn load(dir: &Path) -> Self {
        let inputs = match fs::read_to_string(dir.join(INPUTS_FILE)) {
            Ok(text) => serde_json::from_str::<Inputs>(&text).unwrap_or_else(|e| {
                log::warn!("ignoring the invalid inputs of {}: {e}", dir.display());
                Inputs::default()
            }),
            Err(_) => Inputs::default(),
        };
        Inputs { dir: dir.to_owned(), ..inputs }
    }

    pub fn save(&self) {
        if !self.has_package() {
            return log::warn!("this mode has no package to keep inputs in");
        }
        let result = serde_json::to_string(self)
            .map_err(std::io::Error::other)
            .and_then(|text| config::write_file(&self.dir.join(INPUTS_FILE), &text));
        if let Err(e) = result {
            log::warn!("cannot save the inputs of {}: {e}", self.dir.display());
        }
    }

    /// The mode has a package to keep inputs in (it is not built-in).
    pub fn has_package(&self) -> bool {
        !self.dir.as_os_str().is_empty()
    }

    /// Nothing set up.
    pub fn is_empty(&self) -> bool {
        self.phases.is_empty() && self.zones.is_empty() && self.captures.is_empty() && self.external.is_empty()
    }

    /// A copy in the package `dir`, captures included (a new mode starting
    /// from these inputs). Captures that cannot be copied are left out.
    pub fn copy_to(&self, dir: &Path) -> Inputs {
        let mut copy = Inputs { dir: dir.to_owned(), ..self.clone() };
        copy.captures.retain(|capture| {
            let to = capture_path(dir, &capture.file);
            let copied = to.parent().map_or(Ok(()), config::create_dir).and_then(|()| fs::copy(capture_path(&self.dir, &capture.file), &to));
            if let Err(e) = &copied {
                log::warn!("leaving out the capture {}: {e}", capture.file);
            }
            copied.is_ok()
        });
        copy
    }

    /// Indicators proposed and not drawn yet.
    pub fn to_draw(&self) -> impl Iterator<Item = &PlannedIndicator> {
        self.planned.iter().filter(|p| !self.zones.iter().any(|z| z.indicator == p.name))
    }

    /// Takes in what an assistant's analysis proposes: its phases are added (up
    /// to `MAX_PHASES`), those already set up only get what they lack (a sound,
    /// a sure sign), and its indicators not drawn yet are listed to draw.
    pub fn apply_setup(&mut self, setup: &crate::mode::prompt::Setup) -> SetupApplied {
        let mut applied = SetupApplied::default();
        let mut otherwise = self.phases.iter().any(|p| p.otherwise);
        for proposed in setup.phases.iter().filter(|p| valid_name(&p.name)) {
            let sound = proposed.sound.clone().filter(|s| !s.trim().is_empty());
            let sign: Vec<String> = proposed.indicators.iter().filter(|z| valid_name(z)).cloned().collect();
            let wants_otherwise = proposed.otherwise && sign.is_empty() && !otherwise;
            let (phase, added) = match self.phases.iter().position(|p| p.name == proposed.name) {
                Some(i) => (&mut self.phases[i], false),
                None if self.phases.len() < MAX_PHASES => {
                    self.phases.push(PhaseDef { name: proposed.name.clone(), ..PhaseDef::default() });
                    (self.phases.last_mut().unwrap(), true)
                }
                None => continue,
            };
            let before = phase.clone();
            if phase.sound.is_none() {
                phase.sound = sound;
            }
            if phase.indicators.is_empty() && !phase.otherwise {
                phase.indicators = sign;
                phase.otherwise = wants_otherwise;
                otherwise |= wants_otherwise;
            }
            if added {
                applied.phases_added += 1;
            } else if *phase != before {
                applied.phases_completed += 1;
            }
        }
        for proposed in setup.indicators.iter().filter(|z| valid_name(&z.name)) {
            let planned = PlannedIndicator { name: proposed.name.clone(), kind: proposed.kind, place: proposed.place.trim().to_owned() };
            match self.planned.iter_mut().find(|p| p.name == planned.name) {
                Some(p) => *p = planned,
                None => self.planned.push(planned),
            }
        }
        applied.indicators_to_draw = self.to_draw().count();
        applied
    }

    /// The phases as modes see them.
    pub fn phase_decls(&self) -> Vec<PhaseDecl> {
        self.phases
            .iter()
            .map(|s| PhaseDecl {
                name: s.name.clone(),
                sound: s.sound.clone(),
                screen: s.screen.clone(),
                // Only indicators the mode still has.
                indicators: s.indicators.iter().filter(|z| self.zones.iter().any(|zone| zone.indicator == **z)).cloned().collect(),
                otherwise: s.otherwise && s.indicators.is_empty(),
                hold: s.hold,
                ignore: s.ignore,
            })
            .collect()
    }

    /// A copy for the GUI: the capture embeddings, large and of no use there,
    /// are left out (an analysed capture keeps a one-number placeholder).
    pub fn for_gui(&self) -> Inputs {
        let mut inputs = self.clone();
        for capture in &mut inputs.captures {
            if !capture.embedding.is_empty() {
                capture.embedding = vec![1.0];
            }
        }
        inputs
    }

    /// Captures `frame` as an example of `phase`; the oldest of the phase goes past `MAX_CAPTURES`.
    pub fn add_capture(&mut self, phase: &str, frame: &Frame) -> std::io::Result<()> {
        if !self.has_package() {
            return Err(std::io::Error::other("built-in modes keep no captures"));
        }
        let file = save_capture(&self.dir, phase, frame)?;
        self.captures.push(Capture { file, phase: phase.to_owned(), embedding: Vec::new() });
        let of_phase: Vec<usize> = self.captures.iter().enumerate().filter(|(_, c)| c.phase == phase).map(|(i, _)| i).collect();
        if of_phase.len() > MAX_CAPTURES {
            self.remove_capture(&self.captures[of_phase[0]].file.clone());
        }
        Ok(())
    }

    /// Forgets a capture and deletes its file.
    pub fn remove_capture(&mut self, file: &str) {
        if let Some(i) = self.captures.iter().position(|c| c.file == file) {
            self.captures.remove(i);
            delete_capture(&self.dir, file);
        }
    }

    /// Names of its phases, then of phases only its captures name.
    pub fn phase_names(&self) -> Vec<String> {
        let mut phases: Vec<String> = self.phases.iter().map(|s| s.name.clone()).collect();
        for capture in self.captures.iter().filter(|c| !c.phase.is_empty()) {
            if !phases.contains(&capture.phase) {
                phases.push(capture.phase.clone());
            }
        }
        phases
    }

    /// The mean of each phase's capture embeddings: what the image is compared with.
    pub fn example_centroids(&self) -> Vec<(String, crate::models::Embedding)> {
        self.phase_names()
            .into_iter()
            .filter_map(|phase| {
                let embeddings: Vec<&Vec<f32>> = self.captures.iter().filter(|c| c.phase == phase && !c.embedding.is_empty()).map(|c| &c.embedding).collect();
                let len = embeddings.first()?.len();
                let mut mean = vec![0f32; len];
                for e in embeddings.iter().filter(|e| e.len() == len) {
                    mean.iter_mut().zip(e.iter()).for_each(|(m, v)| *m += v);
                }
                Some((phase, crate::models::normalize(&mean)))
            })
            .collect()
    }

    /// How the inputs read for an AI assistant writing a mode with them (§6.5).
    pub fn describe(&self) -> String {
        let mut out = String::new();
        if !self.phases.is_empty() {
            let names: Vec<String> = self.phases.iter().map(|s| format!("`{}`", s.name)).collect();
            out.push_str(&format!(
                "Phases GameViber recognizes in this game (`input.phase`, `on_phase`; do not declare `phases`): {}\n",
                names.join(", ")
            ));
        }
        if !self.zones.is_empty() {
            out.push_str("Indicators of the screen (`input.indicators`, `on_indicator`):\n");
            let mut seen = Vec::new();
            for z in &self.zones {
                // An indicator read in several zones is one input.
                if seen.contains(&&z.indicator) {
                    continue;
                }
                seen.push(&z.indicator);
                let what = match z.kind {
                    IndicatorKind::Visibility => "true while shown, false otherwise".to_owned(),
                    IndicatorKind::Gauge => "how full the gauge is, 0 to 1, or nil while it is not on screen (menus, cutscenes)".to_owned(),
                };
                out.push_str(&format!("- `{}`: {what}\n", z.indicator));
            }
        }
        if !self.external.is_empty() {
            out.push_str("Sent by another program (`input.external`, `on_event`):\n");
            for i in &self.external {
                let kind = match i.kind {
                    ExternalKind::Value => format!("`input.external.{}`", i.name),
                    ExternalKind::Event => format!("event `{}`", i.name),
                };
                out.push_str(&format!("- {kind}: {}\n", i.description));
            }
        }
        out
    }
}

/// Keys of what games held for all their modes before modes had packages.
const LEGACY_INPUTS: [&str; 4] = ["scenes", "zones", "captures", "inputs"];

/// Up to 0.1.0-alpha.2, a user mode was a file `modes/<name>.luau` and its game
/// held the inputs all its modes read. Each file becomes a package (the active
/// mode and the games follow it), and each user mode of a game gets a copy of
/// the game's inputs; the game's file is kept as `<id>.json.v1`, its captures
/// where they were. Built-in modes read no inputs any more.
pub fn migrate() {
    game::migrate_legacy();
    let moved = migrate_files(&config::modes_dir());
    let mut settings = config::Settings::load();
    if let Some((_, new)) = moved.iter().find(|(old, _)| *old == settings.active_mode) {
        settings.active_mode = new.clone();
        settings.save();
    }
    migrate_games(&game::games_dir(), &moved);
}

/// Moves each `<name>.luau` of `dir` (and its backup) into a package; returns the old and new ids.
fn migrate_files(dir: &Path) -> Vec<(String, String)> {
    let mut moved = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else { return moved };
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        if !path.is_file() || path.extension().is_none_or(|e| e != config::MODE_EXTENSION) {
            continue;
        }
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let script = config::unused_mode_path_in(dir, &stem);
        let result = script.parent().map_or(Ok(()), config::create_dir).and_then(|()| fs::rename(&path, &script));
        if let Err(e) = result {
            log::error!("cannot move the mode {} into its package: {e}", path.display());
            continue;
        }
        let backup = path.with_extension(format!("{}.bak", config::MODE_EXTENSION));
        if backup.exists() {
            let _ = fs::rename(&backup, script.with_extension(format!("{}.bak", config::MODE_EXTENSION)));
        }
        log::info!("the mode {stem} is now a package");
        moved.push((path.to_string_lossy().into_owned(), script.to_string_lossy().into_owned()));
    }
    moved
}

/// Follows the modes that `moved`, and copies each game's inputs into its user modes.
fn migrate_games(dir: &Path, moved: &[(String, String)]) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for path in entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")) {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(serde_json::Value::Object(mut map)) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let mut changed = false;
        if let Some(serde_json::Value::Array(modes)) = map.get_mut("modes") {
            for mode in modes.iter_mut() {
                if let Some(new) = mode.as_str().and_then(|m| moved.iter().find(|(old, _)| old == m)).map(|(_, new)| new.clone()) {
                    *mode = serde_json::Value::String(new);
                    changed = true;
                }
            }
        }
        let legacy: serde_json::Map<String, serde_json::Value> =
            LEGACY_INPUTS.iter().filter_map(|key| map.remove(*key).map(|value| ((*key).to_owned(), value))).collect();
        if !legacy.is_empty() {
            changed = true;
            let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let shared = serde_json::from_value::<Inputs>(serde_json::Value::Object(legacy)).unwrap_or_else(|e| {
                log::warn!("cannot read the inputs of the game {id}: {e}");
                Inputs::default()
            });
            if !shared.is_empty() {
                let modes = map.get("modes").and_then(|m| m.as_array()).cloned().unwrap_or_default();
                for mode in modes.iter().filter_map(|m| m.as_str()).map(ModeEntry::from_id) {
                    let Some(package) = mode.dir().filter(|d| d.exists() && !d.join(INPUTS_FILE).exists()) else { continue };
                    legacy_copy(&shared, &dir.join(&id), &package).save();
                    log::info!("the mode {} now has the inputs of the game {id}", mode.key);
                }
                if let Err(e) = config::write_file(&path.with_extension("json.v1"), &text) {
                    log::warn!("cannot keep a copy of the game {id}: {e}");
                }
            }
        }
        if changed {
            let result = serde_json::to_string(&map).map_err(std::io::Error::other).and_then(|text| config::write_file(&path, &text));
            if let Err(e) = result {
                log::error!("cannot update the game {}: {e}", path.display());
            }
        }
    }
}

/// A game's inputs copied into a package: its captures were in a directory
/// named after the game, `captures`.
fn legacy_copy(inputs: &Inputs, captures: &Path, package: &Path) -> Inputs {
    let mut copy = Inputs { dir: package.to_owned(), ..inputs.clone() };
    copy.captures.retain(|capture| {
        let from = captures.join(&capture.file);
        let to = capture_path(package, &capture.file);
        let copied = to.parent().map_or(Ok(()), config::create_dir).and_then(|()| fs::copy(&from, &to));
        if let Err(e) = &copied {
            log::warn!("leaving out the capture {}: {e}", capture.file);
        }
        copied.is_ok()
    });
    copy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("gameviber-package-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        game::TEST_DIR.with(|d| *d.borrow_mut() = Some(root.join("games")));
        config::MODES_TEST_DIR.with(|d| *d.borrow_mut() = Some(root.join("modes")));
        root
    }

    fn frame(v: u8) -> Frame {
        Frame { width: 4, height: 2, source_width: 4, source_height: 2, count: 0, pixels: vec![v; 32] }
    }

    #[test]
    fn names_are_checked() {
        assert!(valid_name("battle_hud"));
        assert!(valid_name("hp2"));
        assert!(!valid_name("2hp"));
        assert!(!valid_name("health bar"));
        assert!(!valid_name(""));
    }

    #[test]
    fn signs_of_older_files_are_read() {
        let phase = |json: serde_json::Value| serde_json::from_value::<PhaseDef>(json).unwrap();
        assert_eq!(phase(serde_json::json!({ "name": "battle", "zone": "menu" })).indicators, ["menu"]);
        assert_eq!(phase(serde_json::json!({ "name": "battle", "indicator": "menu" })).indicators, ["menu"]);
        assert!(phase(serde_json::json!({ "name": "battle", "indicator": null })).indicators.is_empty());
        let both = PhaseDef { name: "battle".into(), indicators: vec!["hp".into(), "menu".into()], ..PhaseDef::default() };
        assert_eq!(phase(serde_json::to_value(&both).unwrap()), both);
        let story = PhaseDef { name: "story".into(), otherwise: true, ..PhaseDef::default() };
        assert_eq!(phase(serde_json::to_value(&story).unwrap()), story);
    }

    #[test]
    fn captures_are_saved_capped_and_described() {
        let root = test_root("captures");
        let mode = ModeEntry::from_id(&config::unused_mode_path("test").to_string_lossy());
        let mut inputs = Inputs::of(&mode);
        assert_eq!(inputs.dir, root.join("modes").join("test"));
        for i in 0..MAX_CAPTURES + 2 {
            inputs.add_capture("battle", &frame(i as u8)).unwrap();
        }
        inputs.add_capture("story", &frame(200)).unwrap();
        assert_eq!(inputs.captures.iter().filter(|c| c.phase == "battle").count(), MAX_CAPTURES);
        inputs.phases.push(PhaseDef { name: "menu".into(), ..PhaseDef::default() });
        assert_eq!(inputs.phase_names(), ["menu", "battle", "story"], "its phases, then those only captures name");
        let first = &inputs.captures[0];
        assert_eq!(load_capture(&inputs.dir, &first.file).unwrap().pixels, vec![2; 32], "the two oldest went first");
        assert!(inputs.example_centroids().is_empty(), "no embedding yet");
        inputs.captures[0].embedding = vec![0.0, 2.0];
        assert_eq!(inputs.example_centroids(), vec![("battle".to_owned(), crate::models::normalize(&[0.0, 1.0]))]);

        inputs.zones.push(Zone { indicator: "hp".into(), kind: IndicatorKind::Gauge, ..Zone::default() });
        inputs.external.push(ExternalInput { name: "kill".into(), kind: ExternalKind::Event, description: "an enemy died".into() });
        let text = inputs.describe();
        assert!(text.contains("`hp`: how full the gauge is"), "{text}");
        assert!(text.contains("recognizes in this game (`input.phase`, `on_phase`; do not declare `phases`): `menu`"), "{text}");
        assert!(text.contains("event `kill`: an enemy died"), "{text}");
        inputs.save();
        assert_eq!(Inputs::of(&mode), inputs);

        let copy = inputs.copy_to(&root.join("modes").join("copy"));
        assert_eq!(copy.captures.len(), inputs.captures.len());
        assert!(load_capture(&copy.dir, &copy.captures[0].file).is_some(), "captures copied");
        let story = inputs.captures.last().unwrap().file.clone();
        inputs.remove_capture(&story);
        assert!(load_capture(&inputs.dir, &story).is_none(), "its file is deleted");
        assert!(load_capture(&copy.dir, &story).is_some(), "not the copy's");

        assert!(!Inputs::of(&ModeEntry::from_id("builtin:simple")).has_package());
        let mut builtin = Inputs::default();
        assert!(builtin.add_capture("battle", &frame(1)).is_err(), "built-in modes keep no captures");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mode_files_become_packages_with_their_game_inputs() {
        let root = test_root("migrate");
        let (modes, games) = (root.join("modes"), root.join("games"));
        config::write_file(&modes.join("battles.luau"), "-- battles").unwrap();
        config::write_file(&modes.join("battles.luau.bak"), "-- before").unwrap();
        config::write_file(&modes.join("other.luau"), "-- other").unwrap();
        config::write_file(&games.join("metaphor").join("battle-1.png"), "png").unwrap();
        let old = |stem: &str| modes.join(format!("{stem}.luau")).to_string_lossy().into_owned();
        let game = serde_json::json!({
            "name": "Metaphor", "executables": ["METAPHOR.exe"],
            "scenes": [{ "name": "battle" }], "zones": [{ "name": "hp", "kind": "bar" }],
            "captures": [{ "file": "battle-1.png", "scene": "battle", "embedding": [0.5] }],
            "inputs": [{ "name": "kill", "kind": "event", "description": "an enemy died" }],
            "modes": [old("battles"), "builtin:simple"],
        });
        config::write_file(&games.join("metaphor.json"), &game.to_string()).unwrap();

        let moved = migrate_files(&modes);
        migrate_games(&games, &moved);
        let battles = modes.join("battles").join(config::MODE_FILE);
        assert_eq!(fs::read_to_string(&battles).unwrap(), "-- battles");
        assert_eq!(fs::read_to_string(battles.with_extension("luau.bak")).unwrap(), "-- before");
        assert!(modes.join("other").join(config::MODE_FILE).exists() && !modes.join("other.luau").exists());
        let id = battles.to_string_lossy().into_owned();
        assert!(moved.contains(&(old("battles"), id.clone())));

        let game = game::Game::load("metaphor").unwrap();
        assert_eq!((game.name.as_str(), &game.modes[..]), ("Metaphor", &[id.clone(), "builtin:simple".to_owned()][..]));
        let inputs = Inputs::of(&ModeEntry::from_id(&id));
        assert_eq!((inputs.phases.len(), inputs.zones.len(), inputs.external.len()), (1, 1, 1));
        assert_eq!(inputs.captures[0].embedding, [0.5], "embeddings kept");
        assert!(capture_path(&inputs.dir, "battle-1.png").exists(), "captures copied");
        assert!(Inputs::of(&ModeEntry::from_id(&modes.join("other").join(config::MODE_FILE).to_string_lossy())).is_empty(), "a mode of no game");
        assert!(games.join("metaphor.json.v1").exists(), "the game's old file is kept");
        assert!(!fs::read_to_string(games.join("metaphor.json")).unwrap().contains("scenes"));

        // Done once: running it again changes nothing.
        assert!(migrate_files(&modes).is_empty());
        migrate_games(&games, &[]);
        assert_eq!(Inputs::of(&ModeEntry::from_id(&id)), inputs);
        let _ = fs::remove_dir_all(root);
    }
}
