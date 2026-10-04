//! Captures the game's sound through PipeWire's `pw-record`: one
//! application's playback stream, or the default output (everything the
//! computer plays). Streams are listed with `pw-dump`.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;

use serde_json::Value;

use super::SAMPLE_RATE;
use crate::config::AudioSource;

/// An application playing sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub serial: u64,
    /// `application.name`, as shown in the GUI.
    pub app: String,
    pub binary: String,
    pub pid: Option<u32>,
}

/// What `pw-record` listens to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Stream(Stream),
    /// The default output's monitor.
    Everything,
}

impl Target {
    pub fn describe(&self) -> String {
        match self {
            Target::Stream(s) => s.app.clone(),
            Target::Everything => "everything the computer plays".to_owned(),
        }
    }
}

/// Applications playing sound right now.
pub fn list_streams() -> Vec<Stream> {
    let output = match Command::new("pw-dump").stdin(Stdio::null()).stderr(Stdio::null()).output() {
        Ok(o) if o.status.success() => o.stdout,
        Ok(_) | Err(_) => return Vec::new(),
    };
    parse_streams(&output)
}

fn parse_streams(json: &[u8]) -> Vec<Stream> {
    let Ok(Value::Array(objects)) = serde_json::from_slice::<Value>(json) else { return Vec::new() };
    let mut streams: Vec<Stream> = objects
        .iter()
        .filter_map(|o| {
            let props = o.get("info")?.get("props")?;
            let text = |key: &str| props.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
            if text("media.class") != "Stream/Output/Audio" {
                return None;
            }
            let pid = props.get("application.process.id").and_then(|v| match v {
                Value::String(s) => s.parse().ok(),
                Value::Number(n) => n.as_u64().map(|n| n as u32),
                _ => None,
            });
            let app = Some(text("application.name")).filter(|s| !s.is_empty()).unwrap_or_else(|| text("node.name"));
            Some(Stream { serial: props.get("object.serial")?.as_u64()?, app, binary: text("application.process.binary"), pid })
        })
        .collect();
    streams.sort_by_key(|s| s.app.to_lowercase());
    streams
}

/// Picks what to listen to. `games` are the processes showing the overlay
/// (pid, executable name).
pub fn choose(source: &AudioSource, streams: &[Stream], games: &[(u32, String)]) -> Option<Target> {
    match source {
        AudioSource::Off => None,
        AudioSource::App(name) => {
            streams.iter().find(|s| s.app == *name || s.binary == *name).cloned().map(Target::Stream)
        }
        AudioSource::Auto => {
            let stem = |exe: &str| exe.rsplit(['/', '\\']).next().unwrap_or(exe).trim_end_matches(".exe").to_lowercase();
            let game = streams.iter().find(|s| {
                games.iter().any(|(pid, exe)| {
                    let exe = stem(exe);
                    s.pid == Some(*pid) || (!exe.is_empty() && (stem(&s.app) == exe || stem(&s.binary) == exe))
                })
            });
            Some(game.cloned().map_or(Target::Everything, Target::Stream))
        }
    }
}

/// A running `pw-record`, whose samples arrive on a channel.
pub struct Capture {
    pub target: Target,
    child: Child,
    pub samples: mpsc::Receiver<Vec<f32>>,
}

impl Capture {
    pub fn start(target: Target) -> anyhow::Result<Self> {
        let mut command = Command::new("pw-record");
        command.args(["--raw", "--format", "f32", "--channels", "1", "--latency", "20ms"]);
        command.args(["--rate", &SAMPLE_RATE.to_string()]);
        let mut props = String::from(
            "node.name=gameviber-audio node.description=\"GameViber game audio\" node.dont-reconnect=true node.passive=true",
        );
        match &target {
            Target::Stream(s) => {
                command.args(["--target", &s.serial.to_string()]);
                props.push_str(" stream.capture.sink=false");
            }
            Target::Everything => props.push_str(" stream.capture.sink=true"),
        }
        command.args(["-P", &format!("{{ {props} }}"), "-"]);
        let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let (tx, samples) = mpsc::channel();
        std::thread::Builder::new().name("audio-capture".into()).spawn(move || {
            let mut bytes = vec![0u8; super::features::HOP * 4];
            let mut filled = 0;
            loop {
                match stdout.read(&mut bytes[filled..]) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => filled += n,
                }
                // Whole samples only; a partial one waits for the next read.
                let whole = filled / 4 * 4;
                let chunk: Vec<f32> =
                    bytes[..whole].chunks_exact(4).map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])).collect();
                bytes.copy_within(whole..filled, 0);
                filled -= whole;
                if tx.send(chunk).is_err() {
                    break;
                }
            }
        })?;
        Ok(Self { target, child, samples })
    }

    /// `pw-record` stopped (the stream went away).
    pub fn ended(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"[
      { "id": 46, "info": { "props": { "media.class": "Audio/Sink", "object.serial": 46, "node.name": "speakers" } } },
      { "id": 80, "info": { "props": { "media.class": "Stream/Output/Audio", "object.serial": 812,
          "application.name": "METAPHOR.exe", "application.process.binary": "wine64-preloader",
          "application.process.id": "4242" } } },
      { "id": 81, "info": { "props": { "media.class": "Stream/Output/Audio", "object.serial": 813,
          "application.name": "Firefox", "application.process.binary": "firefox", "application.process.id": 77 } } },
      { "id": 82, "info": { "props": { "media.class": "Stream/Input/Audio", "object.serial": 900,
          "application.name": "OBS" } } },
      { "id": 83, "type": "PipeWire:Interface:Link" }
    ]"#;

    #[test]
    fn playback_streams_are_listed() {
        let streams = parse_streams(DUMP.as_bytes());
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].app, "Firefox");
        assert_eq!(streams[0].pid, Some(77));
        assert_eq!(streams[1], Stream { serial: 812, app: "METAPHOR.exe".into(), binary: "wine64-preloader".into(), pid: Some(4242) });
    }

    #[test]
    fn auto_prefers_the_game_showing_the_overlay() {
        let streams = parse_streams(DUMP.as_bytes());
        let by_pid = choose(&AudioSource::Auto, &streams, &[(4242, "whatever".into())]);
        assert!(matches!(by_pid, Some(Target::Stream(s)) if s.serial == 812));
        let by_name = choose(&AudioSource::Auto, &streams, &[(1, "C:\\Games\\METAPHOR.exe".into())]);
        assert!(matches!(by_name, Some(Target::Stream(s)) if s.serial == 812));
        assert_eq!(choose(&AudioSource::Auto, &streams, &[]), Some(Target::Everything));
        let app = choose(&AudioSource::App("Firefox".into()), &streams, &[]);
        assert!(matches!(app, Some(Target::Stream(s)) if s.serial == 813));
        assert_eq!(choose(&AudioSource::App("Gone".into()), &streams, &[]), None);
        assert_eq!(choose(&AudioSource::Off, &streams, &[]), None);
    }
}
