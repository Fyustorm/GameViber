//! Recorded play sessions: the game's rumble and the player's buttons and
//! axes, as the mode saw them, so that a mode can be tuned on a real session
//! without playing it again.
//!
//! A recording is a JSON Lines file in `~/.config/gameviber/recordings/`: a
//! header line, then one line per change (`[time, change]`).

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config;
use crate::gamepad::{ButtonEvent, PadState, AXES, BUTTONS};
use crate::mode::rumble_events::RumbleLevels;

const FORMAT_VERSION: u32 = 1;
const EXTENSION: &str = "jsonl";
/// Recording stops by itself after this long.
pub const MAX_SECS: f64 = 60.0 * 60.0;
/// Changes smaller than this are not recorded (sensor noise).
const AXIS_STEP: f64 = 0.01;
const RUMBLE_STEP: f64 = 0.001;

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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    Rumble { strong: f64, weak: f64 },
    Button { name: String, pressed: bool },
    Axis { name: String, value: f64 },
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

/// Accumulates the changes of a session, then writes them to a file.
pub struct Recorder {
    started: String,
    game: Option<String>,
    mode: String,
    start: f64,
    rumble: RumbleLevels,
    axes: BTreeMap<&'static str, f64>,
    changes: Vec<(f64, Change)>,
}

impl Recorder {
    pub fn new(time: f64, mode: String, game: Option<String>) -> Self {
        Self {
            started: local_time(),
            game,
            mode,
            start: time,
            rumble: RumbleLevels::default(),
            axes: BTreeMap::new(),
            changes: Vec::new(),
        }
    }

    pub fn elapsed(&self, time: f64) -> f64 {
        time - self.start
    }

    /// Records what changed since the previous tick.
    pub fn tick(&mut self, time: f64, rumble: RumbleLevels, buttons: &[ButtonEvent], pad: &PadState) {
        let t = round(self.elapsed(time), 1000.0);
        if (rumble.strong - self.rumble.strong).abs() >= RUMBLE_STEP || (rumble.weak - self.rumble.weak).abs() >= RUMBLE_STEP {
            self.rumble = rumble;
            let (strong, weak) = (round(rumble.strong, 1000.0), round(rumble.weak, 1000.0));
            self.changes.push((t, Change::Rumble { strong, weak }));
        }
        for b in buttons {
            self.changes.push((t, Change::Button { name: b.name.to_owned(), pressed: b.pressed }));
        }
        for (&name, &value) in pad.axes() {
            let last = self.axes.get(name).copied().unwrap_or(0.0);
            if (value - last).abs() >= AXIS_STEP || (value == 0.0 && last != 0.0) {
                self.axes.insert(name, value);
                self.changes.push((t, Change::Axis { name: name.to_owned(), value: round(value, 100.0) }));
            }
        }
    }

    /// Writes the recording; returns its path.
    pub fn save(self, time: f64) -> io::Result<PathBuf> {
        self.save_in(&recordings_dir(), time)
    }

    fn save_in(self, dir: &Path, time: f64) -> io::Result<PathBuf> {
        let header = Header {
            version: FORMAT_VERSION,
            started: self.started.clone(),
            game: self.game.clone(),
            mode: self.mode.clone(),
            duration: round(self.elapsed(time), 1000.0),
        };
        let label = header.game.as_deref().unwrap_or(&header.mode);
        let stem: String = format!("{} {label}", self.started.replace(':', "-"))
            .chars()
            .map(|c| if c.is_alphanumeric() || " -_.".contains(c) { c } else { '_' })
            .collect();
        let mut path = dir.join(format!("{stem}.{EXTENSION}"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{stem} ({n}).{EXTENSION}"));
            n += 1;
        }
        let mut text = serde_json::to_string(&header).map_err(io::Error::other)?;
        text.push('\n');
        for change in &self.changes {
            text.push_str(&serde_json::to_string(change).map_err(io::Error::other)?);
            text.push('\n');
        }
        config::write_file(&path, &text)?;
        Ok(path)
    }
}

/// Plays a recording back in the mode's time.
pub struct Player {
    pub info: RecordingInfo,
    changes: Vec<(f64, Change)>,
    next: usize,
    start: f64,
    pub rumble: RumbleLevels,
}

impl Player {
    pub fn open(path: &Path, time: f64) -> anyhow::Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut lines = text.lines();
        let header: Header = serde_json::from_str(lines.next().unwrap_or_default())?;
        anyhow::ensure!(header.version <= FORMAT_VERSION, "recording made by a newer GameViber");
        let changes = lines
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<Vec<(f64, Change)>, _>>()?;
        Ok(Self {
            info: RecordingInfo { path: path.to_owned(), header },
            changes,
            next: 0,
            start: time,
            rumble: RumbleLevels::default(),
        })
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
fn local_time() -> String {
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
        rec.tick(10.0, RumbleLevels::default(), &[], &pad);
        let press = pad.button("A", true, 10.5).unwrap();
        pad.set_axis("LX", 0.5, 10.5);
        rec.tick(10.5, RumbleLevels { strong: 0.8, weak: 0.2 }, &[press], &pad);
        rec.tick(10.52, RumbleLevels { strong: 0.8, weak: 0.2 }, &[], &pad);
        let release = pad.button("A", false, 11.0).unwrap();
        rec.tick(11.0, RumbleLevels::default(), &[release], &pad);
        assert_eq!(rec.changes.len(), 5);
        let path = rec.save_in(&dir, 12.0).unwrap();

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
}
