//! Recorded play sessions: the game's rumble, the player's buttons and axes,
//! what was heard of the game's sound and seen of its image (levels, hits,
//! flashes, zones, scene embeddings — never the sound or the images), and the
//! values other programs sent, as the mode saw them, so that a mode can be
//! tuned on a real session without playing it again.
//!
//! A recording is a JSON Lines file in `~/.config/gameviber/recordings/`: a
//! header line, then one line per change (`[time, change]`).

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::audio::{AudioHit, AudioLevels, Band, Embedding};
use crate::config;
use crate::gamepad::{ButtonEvent, PadState, AXES, BUTTONS};
use crate::mode::rumble_events::RumbleLevels;
use crate::mode::{ModeEvent, ZoneValue};
use crate::screen::ScreenLevels;

/// 2: audio changes. 3: image, zones, values from other programs.
const FORMAT_VERSION: u32 = 3;
const EXTENSION: &str = "jsonl";
/// Recording stops by itself after this long.
pub const MAX_SECS: f64 = 60.0 * 60.0;
/// Changes smaller than this are not recorded (sensor noise).
const AXIS_STEP: f64 = 0.01;
const RUMBLE_STEP: f64 = 0.001;
const AUDIO_STEP: f64 = 0.02;

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
    /// Embedding of the last 10 s of sound, for the mode's scenes.
    AudioClip { embedding: Vec<f32> },
    /// Measures of the game's image.
    Screen { brightness: f64, motion: f64, action: f64 },
    /// The image stopped being copied.
    NoScreen,
    Flash { strength: f64 },
    /// Embedding of the game's image, for the mode's scenes.
    ScreenClip { embedding: Vec<f32> },
    Zone { name: String, value: ZoneValue },
    /// A value another program sent (`input.custom`).
    Custom { name: String, value: serde_json::Value },
    /// An event another program sent (`on_event`).
    External { name: String, data: serde_json::Value },
}

/// The game's sound and image during one tick; their events (hits, clips,
/// flashes, zones) and the values other programs sent come with the tick's events.
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
}

pub fn recordings_dir() -> PathBuf {
    config::config_dir().join("recordings")
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
            Some(RecordingInfo { path, header })
        })
        .collect();
    found.sort_by(|a, b| b.path.cmp(&a.path));
    found
}

/// A session in memory: its header and its changes, in seconds from its start.
#[derive(Debug, Clone)]
pub struct Session {
    pub header: Header,
    pub changes: Vec<(f64, Change)>,
}

impl Session {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut lines = text.lines();
        let header: Header = serde_json::from_str(lines.next().unwrap_or_default())?;
        anyhow::ensure!(header.version <= FORMAT_VERSION, "recording made by a newer GameViber");
        let changes = lines.filter(|l| !l.trim().is_empty()).map(serde_json::from_str).collect::<Result<_, _>>()?;
        Ok(Self { header, changes })
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
        while path.exists() {
            path = dir.join(format!("{stem} ({n}).{EXTENSION}"));
            n += 1;
        }
        let mut text = serde_json::to_string(&self.header).map_err(io::Error::other)?;
        text.push('\n');
        for change in &self.changes {
            text.push_str(&serde_json::to_string(change).map_err(io::Error::other)?);
            text.push('\n');
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
        }
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
                    Change::Zone { name, .. } => format!("zone {name}"),
                    Change::Custom { name, .. } => format!("custom {name}"),
                    Change::Mark
                    | Change::AudioHit { .. }
                    | Change::AudioClip { .. }
                    | Change::Flash { .. }
                    | Change::ScreenClip { .. }
                    | Change::External { .. } => continue,
                };
                self.base.insert(key, change);
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
                ModeEvent::Zone { name, value } => Change::Zone { name: name.clone(), value: *value },
                ModeEvent::Custom { name, value } => Change::Custom { name: name.clone(), value: value.clone() },
                ModeEvent::External { name, data } => Change::External { name: name.clone(), data: data.clone() },
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
            Change::Audio { .. } | Change::Screen { .. } | Change::Zone { .. } => true,
            Change::Custom { value, .. } => !value.is_null(),
            Change::Mark
            | Change::NoAudio
            | Change::NoScreen
            | Change::AudioHit { .. }
            | Change::AudioClip { .. }
            | Change::Flash { .. }
            | Change::ScreenClip { .. }
            | Change::External { .. } => false,
        });
        let changes: Vec<(f64, Change)> = base
            .cloned()
            .map(|c| (0.0, c))
            .chain(self.changes.iter().map(|(t, c)| (round(t - from, 1000.0).max(0.0), c.clone())))
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
            },
            changes,
        }
    }
}

/// Plays a session back in real time.
pub struct Player {
    pub info: RecordingInfo,
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
    pub fn open(path: &Path, time: f64) -> anyhow::Result<Self> {
        let session = Session::open(path)?;
        Ok(Self::new(session, path, time))
    }

    pub fn new(session: Session, path: &Path, time: f64) -> Self {
        Self {
            info: RecordingInfo { path: path.to_owned(), header: session.header },
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

    pub fn position(&self, time: f64) -> f64 {
        (time - self.start).min(self.info.header.duration)
    }

    pub fn finished(&self, time: f64) -> bool {
        self.position(time) >= self.info.header.duration && self.next >= self.changes.len()
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
                Change::Zone { name, value } => self.events.push(ModeEvent::Zone { name: name.clone(), value: *value }),
                Change::Custom { name, value } => self.events.push(ModeEvent::Custom { name: name.clone(), value: value.clone() }),
                Change::External { name, data } => self.events.push(ModeEvent::External { name: name.clone(), data: data.clone() }),
            }
            self.next += 1;
        }
        events
    }
}

fn round(x: f64, scale: f64) -> f64 {
    (x * scale).round() / scale
}

/// "2026-10-03 21:14:05", local time.
pub fn local_time() -> String {
    // SAFETY: localtime_r only writes the tm struct it is given.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return format!("{now}");
        }
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let mut player = Player::open(&path, 100.0).unwrap();
        let mut pad = PadState::default();
        assert!(player.advance(100.2, &mut pad).is_empty());
        assert_eq!(player.rumble.strong, 0.0);
        let events = player.advance(100.5, &mut pad);
        assert_eq!(events, vec![ButtonEvent { name: "A", pressed: true }]);
        assert_eq!(player.rumble.strong, 0.8);
        assert_eq!(pad.axes()["LX"], 0.5);
        assert!(!player.finished(101.5));
        player.advance(101.5, &mut pad);
        assert!(!pad.held().contains("A"));
        assert!(player.finished(102.0));
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

        let mut player = Player::new(session, Path::new(""), 10.0);
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
    fn image_zones_and_other_programs_are_recorded_and_replayed() {
        let pad = PadState::default();
        let none = RumbleLevels::default();
        let mut rec = Recorder::new(0.0, "T".into(), Some("game.exe".into()));
        let seen = ScreenLevels { brightness: 0.5, motion: 0.25, action: 0.75 };
        let events = [
            ModeEvent::ScreenFlash(0.8),
            ModeEvent::ScreenClip(Arc::from(vec![1.0f32, 0.0].as_slice())),
            ModeEvent::Zone { name: "hp".into(), value: ZoneValue::Bar(0.5) },
            ModeEvent::Zone { name: "battle_hud".into(), value: ZoneValue::Visible(true) },
            ModeEvent::Custom { name: "ammo".into(), value: serde_json::json!(3) },
            ModeEvent::External { name: "kill".into(), data: serde_json::json!({"weapon": "bow"}) },
            ModeEvent::Button(ButtonEvent { name: "A", pressed: true }),
        ];
        rec.tick(0.5, none, &[], &pad, Senses { audio: None, screen: Some(seen) }, &events);
        rec.tick(1.0, none, &[], &pad, Senses::default(), &[]);
        let session = rec.session(1.5, None, None);
        assert_eq!(session.changes.len(), 8, "the button is recorded from the buttons, not the events: {:?}", session.changes);
        let text = serde_json::to_string(&session.changes).unwrap();
        assert!(text.contains(r#"{"zone":{"name":"hp","value":{"bar":0.5}}}"#), "{text}");
        assert!(text.contains(r#"{"external":{"name":"kill","data":{"weapon":"bow"}}}"#), "{text}");

        let mut player = Player::new(session, Path::new(""), 0.0);
        let mut pad = PadState::default();
        player.advance(0.5, &mut pad);
        assert_eq!(player.screen, Some(seen));
        assert_eq!(player.take_events(), events[..6]);
        player.advance(1.0, &mut pad);
        assert_eq!(player.screen, None);
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
}
