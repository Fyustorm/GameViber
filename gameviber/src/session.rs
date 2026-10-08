//! Recorded play sessions: the game's rumble, the player's buttons and axes,
//! what was heard of the game's sound and seen of its image (levels, hits,
//! flashes, indicators, phase embeddings — never the sound), about two images
//! of the game a second, and the values other programs sent, as the mode saw
//! them, so that a mode can be tuned on a real session without playing it
//! again, and its images used as captures.
//!
//! A recording is a JSON Lines file in `~/.config/gameviber/recordings/`: a
//! header line, then one line per change (`[time, change]`); its images are
//! JPEG files beside it, in a directory of the same name ending in `.frames`,
//! each named after its time in milliseconds.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::audio::{AudioHit, AudioLevels, Band, Embedding};
use crate::config;
use crate::gamepad::{ButtonEvent, PadState, AXES, BUTTONS};
use crate::mode::rumble_events::RumbleLevels;
use crate::mode::{ModeEvent, IndicatorValue};
use crate::package::Zone;
use crate::platform::local_time;
use crate::screen::{Frame, ScreenLevels};

/// 2: audio changes. 3: image, indicators, values from other programs. 4: the
/// names of the modes' Inputs (indicator, external values and events); the
/// older names are still read.
const FORMAT_VERSION: u32 = 4;
const EXTENSION: &str = "jsonl";
/// Recording stops by itself after this long.
pub const MAX_SECS: f64 = 60.0 * 60.0;
/// Changes smaller than this are not recorded (sensor noise).
const AXIS_STEP: f64 = 0.01;
const RUMBLE_STEP: f64 = 0.001;
const AUDIO_STEP: f64 = 0.02;
/// Images of the game recorded per second, unless set otherwise (`Recorder::images`).
pub const FRAME_RATE: f64 = 2.0;
/// Images per second a recording can keep: the overlay copies about 10.
pub const FRAME_RATES: [f64; 4] = [1.0, 2.0, 5.0, 10.0];
const FRAMES_EXTENSION: &str = "frames";
const JPEG_QUALITY: u8 = 80;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub version: u32,
    /// Local date and time the recording started ("2026-10-03 21:14").
    pub started: String,
    /// Game whose overlay was connected, if any.
    pub game: Option<String>,
    /// Name of the mode active while recording.
    pub mode: String,
    pub duration: f64,
    /// Moments the player marked as feeling wrong.
    #[serde(default)]
    pub marks: u32,
    /// Images of the game recorded.
    #[serde(default)]
    pub frames: u32,
    /// The zones its indicators were read in, when they were read in the same
    /// ones throughout; else (older recordings too) they are read again from
    /// its images when it is replayed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zones: Option<Vec<Zone>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Rumble { strong: f64, weak: f64 },
    Button { name: String, pressed: bool },
    Axis { name: String, value: f64 },
    /// The player marked this moment as feeling wrong.
    Mark,
    /// Levels of the game's sound.
    Audio { level: f64, low: f64, mid: f64, high: f64, intensity: f64 },
    /// The sound stopped being captured.
    NoAudio,
    AudioHit { strength: f64, band: Band },
    /// Embedding of the last 10 s of sound, for the mode's phases.
    AudioClip { embedding: Vec<f32> },
    /// Measures of the game's image.
    Screen { brightness: f64, motion: f64, action: f64 },
    /// The image stopped being copied.
    NoScreen,
    Flash { strength: f64 },
    /// Embedding of the game's image, for the mode's phases.
    ScreenClip { embedding: Vec<f32> },
    #[serde(alias = "zone")]
    Indicator { name: String, value: IndicatorValue },
    /// A value another program sent (`input.external`).
    #[serde(alias = "custom")]
    ExternalValue { name: String, value: serde_json::Value },
    /// An event another program sent (`on_event`).
    #[serde(alias = "external")]
    ExternalEvent { name: String, data: serde_json::Value },
}

/// The game's sound and image during one tick; their events (hits, clips,
/// flashes, indicators) and the values other programs sent come with the tick's events.
#[derive(Debug, Clone, Copy, Default)]
pub struct Senses {
    pub audio: Option<AudioLevels>,
    pub screen: Option<ScreenLevels>,
}

/// A recording file as listed in the GUI.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordingInfo {
    pub path: PathBuf,
    pub header: Header,
    /// Bytes it takes on disk, its images included.
    pub size: u64,
}

pub fn recordings_dir() -> PathBuf {
    config::config_dir().join("recordings")
}

/// Where the images of the recording at `path` are.
pub fn frames_dir(path: &Path) -> PathBuf {
    path.with_extension(FRAMES_EXTENSION)
}

/// Where the images of the recording in progress are written until it is saved.
fn spool_dir() -> PathBuf {
    recordings_dir().join(format!(".recording.{FRAMES_EXTENSION}"))
}

/// Deletes a recording and its images.
pub fn delete(path: &Path) -> io::Result<()> {
    fs::remove_file(path)?;
    let frames = frames_dir(path);
    if frames.is_dir() {
        fs::remove_dir_all(frames)?;
    }
    Ok(())
}

/// An image of the game, JPEG encoded: in memory, or in a file.
#[derive(Clone, PartialEq)]
pub enum FrameData {
    Jpeg(Arc<[u8]>),
    File(PathBuf),
}

impl fmt::Debug for FrameData {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FrameData::Jpeg(bytes) => write!(f, "Jpeg({} bytes)", bytes.len()),
            FrameData::File(path) => write!(f, "File({})", path.display()),
        }
    }
}

impl FrameData {
    /// The image, decoded.
    pub fn load(&self) -> anyhow::Result<Frame> {
        let image = match self {
            FrameData::Jpeg(bytes) => image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)?,
            FrameData::File(path) => image::ImageReader::open(path)?.with_guessed_format()?.decode()?,
        };
        let rgba = image.into_rgba8();
        let (width, height) = rgba.dimensions();
        Ok(Frame { width, height, source_width: width, source_height: height, count: 0, pixels: rgba.into_raw() })
    }
}

/// An image of the game recorded `t` seconds into its session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionFrame {
    pub t: f64,
    pub data: FrameData,
}

/// `frame` as a JPEG image.
pub fn encode_frame(frame: &Frame) -> anyhow::Result<Arc<[u8]>> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    anyhow::ensure!(w > 0 && h > 0 && frame.pixels.len() >= w * h * 4, "empty image");
    let rgb: Vec<u8> = frame.pixels[..w * h * 4].chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY).encode(
        &rgb,
        frame.width,
        frame.height,
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(bytes.into())
}

/// File name of an image `t` seconds into its session.
fn frame_file(t: f64) -> String {
    format!("{:09}.jpg", (t * 1000.0).round().max(0.0) as u64)
}

/// The images of a recording, by time.
fn read_frames(dir: &Path) -> Vec<SessionFrame> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut frames: Vec<SessionFrame> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter_map(|path| {
            let ms: u64 = path.file_stem()?.to_str()?.parse().ok()?;
            Some(SessionFrame { t: ms as f64 / 1000.0, data: FrameData::File(path) })
        })
        .collect();
    frames.sort_by(|a, b| a.t.total_cmp(&b.t));
    frames
}

/// Recordings, newest first.
pub fn list() -> Vec<RecordingInfo> {
    list_in(&recordings_dir())
}

/// Only the header line of each file is read.
fn list_in(dir: &Path) -> Vec<RecordingInfo> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<RecordingInfo> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == EXTENSION))
        .filter_map(|path| {
            let mut line = String::new();
            BufReader::new(fs::File::open(&path).ok()?).read_line(&mut line).ok()?;
            let header = serde_json::from_str(&line).ok()?;
            let images: u64 = fs::read_dir(frames_dir(&path))
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok()?.metadata().ok())
                .map(|m| m.len())
                .sum();
            let size = fs::metadata(&path).map_or(0, |m| m.len()) + images;
            Some(RecordingInfo { path, header, size })
        })
        .collect();
    found.sort_by(|a, b| b.path.cmp(&a.path));
    found
}

/// Work on a session under way (a replay, its images embedded): how far it
/// got, and whether to give up.
#[derive(Debug, Default)]
pub struct Progress {
    stopped: AtomicBool,
    /// Thousandths done.
    done: AtomicU32,
}

impl Progress {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    /// Share done, 0 to 1.
    pub fn done(&self) -> f32 {
        self.done.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn set(&self, share: f64) {
        self.done.store((share.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
    }
}

/// A session in memory: its header, its changes and its images, in seconds from its start.
#[derive(Debug, Clone)]
pub struct Session {
    pub header: Header,
    pub changes: Vec<(f64, Change)>,
    pub frames: Vec<SessionFrame>,
}

impl Session {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut lines = text.lines();
        let header: Header = serde_json::from_str(lines.next().unwrap_or_default())?;
        anyhow::ensure!(header.version <= FORMAT_VERSION, "recording made by a newer GameViber");
        let changes = lines.filter(|l| !l.trim().is_empty()).map(serde_json::from_str).collect::<Result<_, _>>()?;
        Ok(Self { header, changes, frames: read_frames(&frames_dir(path)) })
    }

    /// It has images of the game but no embeddings of them for the phases (its
    /// mode had no phases to compare them with when it was recorded).
    pub fn lacks_image_embeddings(&self) -> bool {
        !self.frames.is_empty() && !self.changes.iter().any(|(_, c)| matches!(c, Change::ScreenClip { .. }))
    }

    /// Embeddings of its images for the phases, one a second as the engine
    /// makes them (`IMAGE_PHASE_STEP`); gives up once stopped.
    pub fn embed_images(&self, progress: &Progress) -> anyhow::Result<Vec<(f64, Change)>> {
        let mut encoder = crate::screen::clip::ImageEncoder::load()?;
        let mut last = f64::NEG_INFINITY;
        let mut embeddings = Vec::new();
        for (i, frame) in self.frames.iter().enumerate() {
            if frame.t - last < crate::engine::IMAGE_PHASE_STEP * 0.9 {
                continue;
            }
            anyhow::ensure!(!progress.stopped(), "stopped");
            progress.set(i as f64 / self.frames.len() as f64);
            last = frame.t;
            match frame.data.load() {
                Ok(image) => embeddings.push((frame.t, Change::ScreenClip { embedding: encoder.embed(&image)?.to_vec() })),
                Err(e) => log::warn!("cannot read an image of the session: {e:#}"),
            }
        }
        Ok(embeddings)
    }

    /// Adds `changes` among its own, in time order.
    pub fn insert(&mut self, changes: Vec<(f64, Change)>) {
        self.changes.extend(changes);
        self.changes.sort_by(|a, b| a.0.total_cmp(&b.0));
    }

    /// Writes the session to the recordings directory; returns its path.
    pub fn save(&self) -> io::Result<PathBuf> {
        self.save_in(&recordings_dir())
    }

    fn save_in(&self, dir: &Path) -> io::Result<PathBuf> {
        let label = self.header.game.as_deref().unwrap_or(&self.header.mode);
        let stem: String = format!("{} {label}", self.header.started.replace(':', "-"))
            .chars()
            .map(|c| if c.is_alphanumeric() || " -_.".contains(c) { c } else { '_' })
            .collect();
        let mut path = dir.join(format!("{stem}.{EXTENSION}"));
        let mut n = 2;
        while path.exists() || frames_dir(&path).exists() {
            path = dir.join(format!("{stem} ({n}).{EXTENSION}"));
            n += 1;
        }
        let mut text = serde_json::to_string(&self.header).map_err(io::Error::other)?;
        text.push('\n');
        for change in &self.changes {
            text.push_str(&serde_json::to_string(change).map_err(io::Error::other)?);
            text.push('\n');
        }
        if !self.frames.is_empty() {
            let dir = frames_dir(&path);
            config::create_dir(&dir)?;
            for frame in &self.frames {
                let to = dir.join(frame_file(frame.t));
                match &frame.data {
                    FrameData::Jpeg(bytes) => fs::write(&to, bytes)?,
                    // Moved out of the spool, else copied.
                    FrameData::File(from) => {
                        if fs::rename(from, &to).is_err() {
                            fs::copy(from, &to)?;
                        }
                    }
                }
            }
        }
        config::write_file(&path, &text)?;
        Ok(path)
    }
}

/// Accumulates the changes of a session: all of them, or only the last
/// `window` seconds (a rolling memory of what just happened).
pub struct Recorder {
    started: String,
    game: Option<String>,
    mode: String,
    start: f64,
    window: Option<f64>,
    rumble: RumbleLevels,
    axes: BTreeMap<&'static str, f64>,
    audio: Option<AudioLevels>,
    screen: Option<ScreenLevels>,
    changes: VecDeque<(f64, Change)>,
    /// State at the start of the window: changes dropped from it, latest per key.
    base: BTreeMap<String, Change>,
    /// Images of the game, when the last one came, and the seconds between two.
    frames: VecDeque<SessionFrame>,
    last_frame: f64,
    frame_step: f64,
    /// Where images are written as they come (long recordings); None: kept in memory.
    spool: Option<PathBuf>,
    /// The zones indicators are read in (None: not read), and since when (seconds in).
    zones: (Option<Vec<Zone>>, f64),
}

impl Recorder {
    pub fn new(time: f64, mode: String, game: Option<String>) -> Self {
        Self {
            started: local_time(),
            game,
            mode,
            start: time,
            window: None,
            rumble: RumbleLevels::default(),
            axes: BTreeMap::new(),
            audio: None,
            screen: None,
            changes: VecDeque::new(),
            base: BTreeMap::new(),
            frames: VecDeque::new(),
            last_frame: f64::NEG_INFINITY,
            frame_step: 1.0 / FRAME_RATE,
            spool: None,
            zones: (None, 0.0),
        }
    }

    /// The indicators are now read in `zones` (None: they are not read).
    pub fn reading(&mut self, time: f64, zones: Option<&[Zone]>) {
        if self.zones.0.as_deref() != zones {
            self.zones = (zones.map(<[Zone]>::to_vec), self.elapsed(time).max(0.0));
        }
    }

    /// A recording whose images are written to disk as they come, not kept in memory.
    pub fn spooled(mut self) -> Self {
        let dir = spool_dir();
        let _ = fs::remove_dir_all(&dir);
        match config::create_dir(&dir) {
            Ok(()) => self.spool = Some(dir),
            Err(e) => log::warn!("images of the game kept in memory: cannot create {}: {e}", dir.display()),
        }
        self
    }

    /// What is already true when the recording starts (indicators, values from
    /// other programs, buttons held), else missing until it changes.
    pub fn starting_from(mut self, state: impl IntoIterator<Item = Change>) -> Self {
        self.changes.extend(state.into_iter().map(|c| (0.0, c)));
        self
    }

    /// Keeps `per_second` images of the game a second (at most what the overlay copies).
    pub fn images(mut self, per_second: f64) -> Self {
        self.frame_step = 1.0 / per_second.clamp(0.1, 30.0);
        self
    }

    /// An image of the game is due (images come about every 0.1 s).
    pub fn wants_frame(&self, time: f64) -> bool {
        self.elapsed(time) - self.last_frame >= self.frame_step * 0.9
    }

    /// Records an image of the game, JPEG encoded (`encode_frame`).
    pub fn frame(&mut self, time: f64, jpeg: Arc<[u8]>) {
        let t = round(self.elapsed(time), 1000.0);
        self.last_frame = t;
        let data = match &self.spool {
            Some(dir) => {
                let path = dir.join(frame_file(t));
                if let Err(e) = fs::write(&path, &jpeg) {
                    return log::warn!("cannot write an image of the game: {e}");
                }
                FrameData::File(path)
            }
            None => FrameData::Jpeg(jpeg),
        };
        self.frames.push_back(SessionFrame { t, data });
    }

    /// Keeps only the last `secs` seconds.
    pub fn rolling(time: f64, secs: f64) -> Self {
        Self { window: Some(secs), ..Self::new(time, String::new(), None) }
    }

    pub fn elapsed(&self, time: f64) -> f64 {
        time - self.start
    }

    /// The player marked this moment.
    pub fn mark(&mut self, time: f64) {
        self.changes.push_back((round(self.elapsed(time), 1000.0), Change::Mark));
    }

    /// Records what changed since the previous tick; `events` are the mode's
    /// events of the tick (buttons are recorded from `buttons`).
    pub fn tick(&mut self, time: f64, rumble: RumbleLevels, buttons: &[ButtonEvent], pad: &PadState, senses: Senses, events: &[ModeEvent]) {
        let t = round(self.elapsed(time), 1000.0);
        if (rumble.strong - self.rumble.strong).abs() >= RUMBLE_STEP || (rumble.weak - self.rumble.weak).abs() >= RUMBLE_STEP {
            self.rumble = rumble;
            let (strong, weak) = (round(rumble.strong, 1000.0), round(rumble.weak, 1000.0));
            self.changes.push_back((t, Change::Rumble { strong, weak }));
        }
        for b in buttons {
            self.changes.push_back((t, Change::Button { name: b.name.to_owned(), pressed: b.pressed }));
        }
        for (&name, &value) in pad.axes() {
            let last = self.axes.get(name).copied().unwrap_or(0.0);
            if (value - last).abs() >= AXIS_STEP || (value == 0.0 && last != 0.0) {
                self.axes.insert(name, value);
                self.changes.push_back((t, Change::Axis { name: name.to_owned(), value: round(value, 100.0) }));
            }
        }
        self.record_senses(t, senses, events);
        if let Some(window) = self.window {
            while self.changes.front().is_some_and(|(at, _)| *at < t - window) {
                let (_, change) = self.changes.pop_front().unwrap();
                let key = match &change {
                    Change::Rumble { .. } => "rumble".to_owned(),
                    Change::Button { name, .. } => format!("button {name}"),
                    Change::Axis { name, .. } => format!("axis {name}"),
                    Change::Audio { .. } | Change::NoAudio => "audio".to_owned(),
                    Change::Screen { .. } | Change::NoScreen => "screen".to_owned(),
                    Change::Indicator { name, .. } => format!("zone {name}"),
                    Change::ExternalValue { name, .. } => format!("external {name}"),
                    Change::Mark
                    | Change::AudioHit { .. }
                    | Change::AudioClip { .. }
                    | Change::Flash { .. }
                    | Change::ScreenClip { .. }
                    | Change::ExternalEvent { .. } => continue,
                };
                self.base.insert(key, change);
            }
            while self.frames.front().is_some_and(|f| f.t < t - window) {
                self.frames.pop_front();
            }
        }
    }

    fn record_senses(&mut self, t: f64, senses: Senses, events: &[ModeEvent]) {
        let moved = match (self.audio, senses.audio) {
            (None, None) => false,
            (Some(a), Some(b)) => [a.level - b.level, a.low - b.low, a.mid - b.mid, a.high - b.high, a.intensity - b.intensity]
                .iter()
                .any(|d| d.abs() >= AUDIO_STEP),
            _ => true,
        };
        if moved {
            self.audio = senses.audio;
            self.changes.push_back((
                t,
                match senses.audio {
                    Some(l) => Change::Audio {
                        level: round(l.level, 100.0),
                        low: round(l.low, 100.0),
                        mid: round(l.mid, 100.0),
                        high: round(l.high, 100.0),
                        intensity: round(l.intensity, 100.0),
                    },
                    None => Change::NoAudio,
                },
            ));
        }
        let moved = match (self.screen, senses.screen) {
            (None, None) => false,
            (Some(a), Some(b)) => {
                [a.brightness - b.brightness, a.motion - b.motion, a.action - b.action].iter().any(|d| d.abs() as f64 >= AUDIO_STEP)
            }
            _ => true,
        };
        if moved {
            self.screen = senses.screen;
            self.changes.push_back((
                t,
                match senses.screen {
                    Some(l) => Change::Screen {
                        brightness: round(l.brightness as f64, 100.0),
                        motion: round(l.motion as f64, 100.0),
                        action: round(l.action as f64, 100.0),
                    },
                    None => Change::NoScreen,
                },
            ));
        }
        let embedding = |e: &Embedding| e.iter().map(|x| (x * 1e4).round() / 1e4).collect();
        for event in events {
            let change = match event {
                ModeEvent::AudioHit(hit) => Change::AudioHit { strength: round(hit.strength, 100.0), band: hit.band },
                ModeEvent::AudioClip(clip) => Change::AudioClip { embedding: embedding(clip) },
                ModeEvent::ScreenClip(image) => Change::ScreenClip { embedding: embedding(image) },
                ModeEvent::ScreenFlash(strength) => Change::Flash { strength: round(*strength, 100.0) },
                ModeEvent::Indicator { name, value } => Change::Indicator { name: name.clone(), value: *value },
                ModeEvent::ExternalValue { name, value } => Change::ExternalValue { name: name.clone(), value: value.clone() },
                ModeEvent::ExternalEvent { name, data } => Change::ExternalEvent { name: name.clone(), data: data.clone() },
                ModeEvent::Button(_) | ModeEvent::Device { .. } => continue,
            };
            self.changes.push_back((t, change));
        }
    }

    /// The session so far (or its window), with `mode` and `game` given when
    /// the recorder did not know them.
    pub fn session(&self, time: f64, mode: Option<&str>, game: Option<&str>) -> Session {
        let elapsed = self.elapsed(time);
        let from = self.window.map_or(0.0, |w| (elapsed - w).max(0.0));
        // The window's starting state, released buttons and centered axes left out.
        let base = self.base.values().filter(|c| match c {
            Change::Button { pressed, .. } => *pressed,
            Change::Axis { value, .. } => *value != 0.0,
            Change::Rumble { strong, weak } => *strong != 0.0 || *weak != 0.0,
            Change::Audio { .. } | Change::Screen { .. } | Change::Indicator { .. } => true,
            Change::ExternalValue { value, .. } => !value.is_null(),
            Change::Mark
            | Change::NoAudio
            | Change::NoScreen
            | Change::AudioHit { .. }
            | Change::AudioClip { .. }
            | Change::Flash { .. }
            | Change::ScreenClip { .. }
            | Change::ExternalEvent { .. } => false,
        });
        let changes: Vec<(f64, Change)> = base
            .cloned()
            .map(|c| (0.0, c))
            .chain(self.changes.iter().map(|(t, c)| (round(t - from, 1000.0).max(0.0), c.clone())))
            .collect();
        let frames: Vec<SessionFrame> = self
            .frames
            .iter()
            .filter(|f| f.t >= from)
            .map(|f| SessionFrame { t: round(f.t - from, 1000.0), data: f.data.clone() })
            .collect();
        let marks = changes.iter().filter(|(_, c)| *c == Change::Mark).count() as u32;
        let started = if self.window.is_some() { local_time() } else { self.started.clone() };
        Session {
            header: Header {
                version: FORMAT_VERSION,
                started,
                game: game.map(str::to_owned).or_else(|| self.game.clone()),
                mode: mode.map(str::to_owned).unwrap_or_else(|| self.mode.clone()),
                duration: round(elapsed - from, 1000.0),
                marks,
                frames: frames.len() as u32,
                zones: self.zones.0.clone().filter(|_| self.zones.1 <= from),
            },
            changes,
            frames,
        }
    }
}

/// Plays a session back into a mode (`mode::report`, offline).
pub struct Player {
    changes: Vec<(f64, Change)>,
    next: usize,
    start: f64,
    pub rumble: RumbleLevels,
    pub audio: Option<AudioLevels>,
    pub screen: Option<ScreenLevels>,
    /// Events of the sound, the image and other programs due, until `take_events`.
    events: Vec<ModeEvent>,
}

impl Player {
    /// Plays `session` from `time`.
    pub fn new(session: Session, time: f64) -> Self {
        Self {
            changes: session.changes,
            next: 0,
            start: time,
            rumble: RumbleLevels::default(),
            audio: None,
            screen: None,
            events: Vec::new(),
        }
    }

    /// Events of the sound, the image and other programs replayed since the last call.
    pub fn take_events(&mut self) -> Vec<ModeEvent> {
        std::mem::take(&mut self.events)
    }

    /// Applies the changes due by `time` to the rumble and to `pad`; returns the
    /// button events for the mode.
    pub fn advance(&mut self, time: f64, pad: &mut PadState) -> Vec<ButtonEvent> {
        let position = time - self.start;
        let mut events = Vec::new();
        while let Some((t, change)) = self.changes.get(self.next) {
            if *t > position {
                break;
            }
            match change {
                Change::Rumble { strong, weak } => self.rumble = RumbleLevels { strong: *strong, weak: *weak },
                Change::Button { name, pressed } => {
                    if let Some(name) = BUTTONS.iter().find(|b| *b == name) {
                        events.extend(pad.button(name, *pressed, time));
                    }
                }
                Change::Axis { name, value } => {
                    if let Some(name) = AXES.iter().find(|a| *a == name) {
                        pad.set_axis(name, *value, time);
                    }
                }
                Change::Mark => {}
                Change::Audio { level, low, mid, high, intensity } => {
                    self.audio = Some(AudioLevels { level: *level, low: *low, mid: *mid, high: *high, intensity: *intensity })
                }
                Change::NoAudio => self.audio = None,
                Change::AudioHit { strength, band } => {
                    self.events.push(ModeEvent::AudioHit(AudioHit { strength: *strength, band: *band }))
                }
                Change::AudioClip { embedding } => self.events.push(ModeEvent::AudioClip(Arc::from(embedding.as_slice()))),
                Change::Screen { brightness, motion, action } => {
                    self.screen =
                        Some(ScreenLevels { brightness: *brightness as f32, motion: *motion as f32, action: *action as f32 })
                }
                Change::NoScreen => self.screen = None,
                Change::Flash { strength } => self.events.push(ModeEvent::ScreenFlash(*strength)),
                Change::ScreenClip { embedding } => self.events.push(ModeEvent::ScreenClip(Arc::from(embedding.as_slice()))),
                Change::Indicator { name, value } => self.events.push(ModeEvent::Indicator { name: name.clone(), value: *value }),
                Change::ExternalValue { name, value } => self.events.push(ModeEvent::ExternalValue { name: name.clone(), value: value.clone() }),
                Change::ExternalEvent { name, data } => self.events.push(ModeEvent::ExternalEvent { name: name.clone(), data: data.clone() }),
            }
            self.next += 1;
        }
        events
    }
}

fn round(x: f64, scale: f64) -> f64 {
    (x * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recording keeps the zones its indicators were read in only when they did not change during it.
    #[test]
    fn recordings_know_the_zones_indicators_were_read_in() {
        let zones = vec![Zone { indicator: "hp".into(), ..Zone::default() }];
        let moved = vec![Zone { indicator: "hp".into(), rect: [0.5, 0.5, 0.1, 0.1], ..Zone::default() }];
        let mut rec = Recorder::new(10.0, "T".into(), None);
        rec.reading(10.0, Some(&zones));
        assert_eq!(rec.session(11.0, None, None).header.zones, Some(zones.clone()));
        rec.reading(10.5, Some(&moved));
        assert_eq!(rec.session(11.0, None, None).header.zones, None, "changed during the recording");

        let mut recent = Recorder::rolling(0.0, 1.0);
        recent.reading(0.5, Some(&zones));
        recent.reading(0.5, None);
        assert_eq!(recent.session(1.0, Some("T"), None).header.zones, None, "not read");
        recent.reading(1.0, Some(&moved));
        assert_eq!(recent.session(1.5, Some("T"), None).header.zones, None, "changed within the last second");
        assert_eq!(recent.session(2.5, Some("T"), None).header.zones, Some(moved), "changed before it");
    }

    #[test]
    fn recording_plays_back_what_was_recorded() {
        let dir = std::env::temp_dir().join(format!("gameviber-session-{}", std::process::id()));
        let mut pad = PadState::default();
        let mut rec = Recorder::new(10.0, "Simple".into(), Some("Game".into()));
        rec.tick(10.0, RumbleLevels::default(), &[], &pad, Senses::default(), &[]);
        let press = pad.button("A", true, 10.5).unwrap();
        pad.set_axis("LX", 0.5, 10.5);
        rec.tick(10.5, RumbleLevels { strong: 0.8, weak: 0.2 }, &[press], &pad, Senses::default(), &[]);
        rec.tick(10.52, RumbleLevels { strong: 0.8, weak: 0.2 }, &[], &pad, Senses::default(), &[]);
        let release = pad.button("A", false, 11.0).unwrap();
        rec.tick(11.0, RumbleLevels::default(), &[release], &pad, Senses::default(), &[]);
        assert_eq!(rec.changes.len(), 5);
        let path = rec.session(12.0, None, None).save_in(&dir).unwrap();

        let listed = list_in(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].header.game.as_deref(), Some("Game"));
        assert_eq!(listed[0].header.duration, 2.0);

        let mut player = Player::new(Session::open(&path).unwrap(), 100.0);
        let mut pad = PadState::default();
        assert!(player.advance(100.2, &mut pad).is_empty());
        assert_eq!(player.rumble.strong, 0.0);
        let events = player.advance(100.5, &mut pad);
        assert_eq!(events, vec![ButtonEvent { name: "A", pressed: true }]);
        assert_eq!(player.rumble.strong, 0.8);
        assert_eq!(pad.axes()["LX"], 0.5);
        player.advance(101.5, &mut pad);
        assert!(!pad.held().contains("A"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn audio_is_recorded_and_replayed() {
        let pad = PadState::default();
        let mut rec = Recorder::new(0.0, "T".into(), None);
        let loud = AudioLevels { level: 0.8, low: 0.5, mid: 0.2, high: 0.1, intensity: 0.3 };
        let hit = AudioHit { strength: 0.7, band: Band::Low };
        let clip: Embedding = Arc::from(vec![0.6f32, 0.8].as_slice());
        let none = RumbleLevels::default();
        let heard = |audio| Senses { audio, screen: None };
        rec.tick(0.5, none, &[], &pad, heard(Some(loud)), &[ModeEvent::AudioHit(hit), ModeEvent::AudioClip(clip)]);
        // Too small a change to be recorded.
        rec.tick(0.6, none, &[], &pad, heard(Some(AudioLevels { level: 0.81, ..loud })), &[]);
        rec.tick(1.0, none, &[], &pad, Senses::default(), &[]);
        assert_eq!(rec.changes.len(), 4, "{:?}", rec.changes);

        let session = rec.session(1.5, None, None);
        let text = serde_json::to_string(&session.changes).unwrap();
        assert!(text.contains(r#"{"audio_hit":{"strength":0.7,"band":"low"}}"#), "{text}");
        assert!(text.contains(r#""no_audio""#), "{text}");

        let mut player = Player::new(session, 10.0);
        let mut pad = PadState::default();
        player.advance(10.5, &mut pad);
        assert_eq!(player.audio, Some(loud));
        let events = player.take_events();
        assert!(matches!(events[..], [ModeEvent::AudioHit(h), ModeEvent::AudioClip(ref c)] if h == hit && c[..] == [0.6, 0.8]));
        assert!(player.take_events().is_empty());
        player.advance(11.0, &mut pad);
        assert_eq!(player.audio, None);
    }

    #[test]
    fn image_indicators_and_other_programs_are_recorded_and_replayed() {
        let pad = PadState::default();
        let none = RumbleLevels::default();
        let mut rec = Recorder::new(0.0, "T".into(), Some("game.exe".into()));
        let seen = ScreenLevels { brightness: 0.5, motion: 0.25, action: 0.75 };
        let events = [
            ModeEvent::ScreenFlash(0.8),
            ModeEvent::ScreenClip(Arc::from(vec![1.0f32, 0.0].as_slice())),
            ModeEvent::Indicator { name: "hp".into(), value: IndicatorValue::Gauge(0.5) },
            ModeEvent::Indicator { name: "battle_hud".into(), value: IndicatorValue::Visibility(true) },
            ModeEvent::ExternalValue { name: "ammo".into(), value: serde_json::json!(3) },
            ModeEvent::ExternalEvent { name: "kill".into(), data: serde_json::json!({"weapon": "bow"}) },
            ModeEvent::Button(ButtonEvent { name: "A", pressed: true }),
        ];
        rec.tick(0.5, none, &[], &pad, Senses { audio: None, screen: Some(seen) }, &events);
        rec.tick(1.0, none, &[], &pad, Senses::default(), &[]);
        let session = rec.session(1.5, None, None);
        assert_eq!(session.changes.len(), 8, "the button is recorded from the buttons, not the events: {:?}", session.changes);
        let text = serde_json::to_string(&session.changes).unwrap();
        assert!(text.contains(r#"{"indicator":{"name":"hp","value":{"gauge":0.5}}}"#), "{text}");
        assert!(text.contains(r#"{"external_event":{"name":"kill","data":{"weapon":"bow"}}}"#), "{text}");
        // Recordings made before the terms changed.
        let old = r#"[{"zone":{"name":"hp","value":{"bar":0.5}}},{"zone":{"name":"hud","value":{"visible":true}}},{"custom":{"name":"ammo","value":3}},{"external":{"name":"kill","data":null}}]"#;
        let old: Vec<Change> = serde_json::from_str(old).unwrap();
        assert!(matches!(old[..], [Change::Indicator { value: IndicatorValue::Gauge(_), .. }, Change::Indicator { value: IndicatorValue::Visibility(true), .. }, Change::ExternalValue { .. }, Change::ExternalEvent { .. }]));

        let mut player = Player::new(session, 0.0);
        let mut pad = PadState::default();
        player.advance(0.5, &mut pad);
        assert_eq!(player.screen, Some(seen));
        assert_eq!(player.take_events(), events[..6]);
        player.advance(1.0, &mut pad);
        assert_eq!(player.screen, None);
    }

    #[test]
    fn images_are_recorded_saved_and_read_back() {
        let dir = std::env::temp_dir().join(format!("gameviber-frames-{}", std::process::id()));
        let image = |shade: u8| Frame {
            width: 32,
            height: 18,
            source_width: 32,
            source_height: 18,
            count: 1,
            pixels: [shade, shade, shade, 255].repeat(32 * 18),
        };
        let mut rec = Recorder::rolling(0.0, 1.0);
        let pad = PadState::default();
        for step in 0..=20 {
            let time = step as f64 * 0.1;
            rec.tick(time, RumbleLevels::default(), &[], &pad, Senses::default(), &[]);
            if rec.wants_frame(time) {
                rec.frame(time, encode_frame(&image(step as u8 * 10)).unwrap());
            }
        }
        // Every 0.5 s, only those of the last second kept.
        assert_eq!(rec.frames.iter().map(|f| f.t).collect::<Vec<_>>(), [1.0, 1.5, 2.0]);
        let session = rec.session(2.0, Some("T"), None);
        assert_eq!(session.header.frames, 3);
        assert_eq!(session.frames.iter().map(|f| f.t).collect::<Vec<_>>(), [0.0, 0.5, 1.0], "timed from the window's start");

        let path = session.save_in(&dir).unwrap();
        let read = Session::open(&path).unwrap();
        assert_eq!(read.frames.iter().map(|f| f.t).collect::<Vec<_>>(), [0.0, 0.5, 1.0]);
        let last = read.frames[2].data.load().unwrap();
        assert_eq!((last.width, last.height), (32, 18));
        assert!(last.pixels[0].abs_diff(200) <= 2, "JPEG keeps the shade: {}", last.pixels[0]);
        assert_eq!(list_in(&dir)[0].header.frames, 3);
        delete(&path).unwrap();
        assert!(!path.exists() && !frames_dir(&path).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rolling_window_keeps_the_starting_state() {
        let mut pad = PadState::default();
        let mut rec = Recorder::rolling(0.0, 1.0);
        let press = pad.button("LT", true, 0.5).unwrap();
        rec.tick(0.5, RumbleLevels { strong: 0.4, weak: 0.0 }, &[press], &pad, Senses::default(), &[]);
        let press = pad.button("A", true, 0.6).unwrap();
        rec.tick(0.6, RumbleLevels { strong: 0.4, weak: 0.0 }, &[press], &pad, Senses::default(), &[]);
        let release = pad.button("A", false, 0.7).unwrap();
        rec.tick(0.7, RumbleLevels { strong: 0.4, weak: 0.0 }, &[release], &pad, Senses::default(), &[]);
        rec.mark(0.8);
        rec.mark(2.0);
        rec.tick(2.0, RumbleLevels { strong: 0.9, weak: 0.0 }, &[], &pad, Senses::default(), &[]);

        let session = rec.session(2.5, Some("Surge"), Some("Game"));
        assert_eq!(session.header.duration, 1.0);
        assert_eq!(session.header.mode, "Surge");
        assert_eq!(
            session.changes,
            vec![
                (0.0, Change::Button { name: "LT".into(), pressed: true }),
                (0.0, Change::Rumble { strong: 0.4, weak: 0.0 }),
                (0.5, Change::Mark),
                (0.5, Change::Rumble { strong: 0.9, weak: 0.0 }),
            ]
        );
        assert_eq!(session.header.marks, 1, "the mark before the window is dropped");
    }

    /// An indicator unknown from before the recording, never changing, is in it.
    #[test]
    fn a_recording_starts_from_what_is_already_true() {
        let menu = Change::Indicator { name: "menu".into(), value: IndicatorValue::Unknown };
        let mut rec = Recorder::new(10.0, "Surge".into(), None).starting_from([menu.clone()]);
        rec.tick(10.5, RumbleLevels::default(), &[], &PadState::default(), Senses::default(), &[]);
        assert_eq!(rec.session(11.0, None, None).changes, vec![(0.0, menu)]);
    }
}
