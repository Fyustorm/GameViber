//! Linux: captures the game's sound through PipeWire's `pw-record`: every playback
//! stream of one application, or the default output (everything the
//! computer plays). The graph is read with `pw-dump`; an application's
//! streams are linked to the capture with `pw-link`, so that a game opening
//! several streams (Wine often does) is heard whole, and streams it opens
//! later are added.

use std::collections::HashSet;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Context;
use serde_json::Value;

use super::{Stream, Target};
use crate::audio::{features, SAMPLE_RATE};

/// How long a new capture node may take to appear in the graph, with its ports.
const NODE_WAIT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Port {
    id: u32,
    node: u32,
    output: bool,
}

/// The audio part of the PipeWire graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    /// Playback streams, sorted by application name.
    pub streams: Vec<Stream>,
    ports: Vec<Port>,
    /// Node ids by `node.name`.
    names: Vec<(String, u32)>,
}

impl Graph {
    pub fn read() -> Self {
        match Command::new("pw-dump").stdin(Stdio::null()).stderr(Stdio::null()).output() {
            Ok(o) if o.status.success() => Self::parse(&o.stdout),
            Ok(_) | Err(_) => Self::default(),
        }
    }

    fn parse(json: &[u8]) -> Self {
        let Ok(Value::Array(objects)) = serde_json::from_slice::<Value>(json) else { return Self::default() };
        let mut graph = Self::default();
        for o in &objects {
            let (Some(id), Some(info)) = (o.get("id").and_then(Value::as_u64), o.get("info")) else { continue };
            let id = id as u32;
            let Some(props) = info.get("props") else { continue };
            let text = |key: &str| props.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
            let number = |key: &str| match props.get(key) {
                Some(Value::String(s)) => s.parse().ok(),
                Some(Value::Number(n)) => n.as_u64().map(|n| n as u32),
                _ => None,
            };
            match o.get("type").and_then(Value::as_str) {
                Some("PipeWire:Interface:Port") => {
                    let Some(node) = number("node.id") else { continue };
                    if props.get("port.monitor").and_then(Value::as_bool) == Some(true) {
                        continue;
                    }
                    let output = info.get("direction").and_then(Value::as_str) == Some("output");
                    graph.ports.push(Port { id, node, output });
                }
                _ => {
                    graph.names.push((text("node.name"), id));
                    if text("media.class") != "Stream/Output/Audio" {
                        continue;
                    }
                    let app = Some(text("application.name")).filter(|s| !s.is_empty()).unwrap_or_else(|| text("node.name"));
                    graph.streams.push(Stream { node: id, app, binary: text("application.process.binary"), pid: number("application.process.id") });
                }
            }
        }
        graph.streams.sort_by_key(|s| s.app.to_lowercase());
        graph
    }

    fn node(&self, name: &str) -> Option<u32> {
        self.names.iter().find(|(n, _)| n == name).map(|(_, id)| *id)
    }

    fn ports(&self, node: u32, output: bool) -> impl Iterator<Item = u32> + '_ {
        self.ports.iter().filter(move |p| p.node == node && p.output == output).map(|p| p.id)
    }
}

/// A running `pw-record`, whose samples arrive on a channel.
pub struct Capture {
    pub target: Target,
    child: Child,
    /// `node.name` of the capture, unique to it.
    node_name: String,
    /// Output ports already linked to the capture.
    linked: HashSet<u32>,
    pub samples: mpsc::Receiver<Vec<f32>>,
}

impl Capture {
    pub fn start(target: Target) -> anyhow::Result<Self> {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let node_name = format!("gameviber-audio-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed));
        let mut command = Command::new("pw-record");
        command.args(["--raw", "--format", "f32", "--channels", "1", "--latency", "20ms"]);
        command.args(["--rate", &SAMPLE_RATE.to_string()]);
        let mut props = format!(
            "node.name={node_name} node.description=\"GameViber game audio\" node.dont-reconnect=true node.passive=true"
        );
        match &target {
            // Not linked by PipeWire: `sync` links the application's streams.
            Target::App { .. } => {
                command.args(["--target", "0"]);
            }
            Target::Everything => props.push_str(" stream.capture.sink=true"),
        }
        command.args(["-P", &format!("{{ {props} }}"), "-"]);
        let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().context("pw-record")?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let (tx, samples) = mpsc::channel();
        std::thread::Builder::new().name("audio-capture".into()).spawn(move || {
            let mut bytes = vec![0u8; features::HOP * 4];
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
        let mut capture = Self { target, child, node_name, linked: HashSet::new(), samples };
        if let Target::App { streams, .. } = capture.target.clone() {
            let started = Instant::now();
            loop {
                let graph = Graph::read();
                if graph.node(&capture.node_name).is_some_and(|node| graph.ports(node, false).next().is_some()) {
                    capture.sync(&graph, &streams);
                    break;
                }
                anyhow::ensure!(started.elapsed() < NODE_WAIT && !capture.ended(), "pw-record did not start");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        Ok(capture)
    }

    /// Follows the application's streams: links the ones not linked yet.
    pub fn update(&mut self, graph: &Graph, target: &Target) -> bool {
        if !self.target.same_capture(target) {
            return false;
        }
        if let Target::App { streams, .. } = target {
            self.sync(graph, streams);
        }
        self.target = target.clone();
        true
    }

    fn sync(&mut self, graph: &Graph, streams: &[Stream]) {
        let Some(node) = graph.node(&self.node_name) else { return };
        let inputs: Vec<u32> = graph.ports(node, false).collect();
        if inputs.is_empty() {
            return;
        }
        for stream in streams {
            for output in graph.ports(stream.node, true) {
                if self.linked.contains(&output) {
                    continue;
                }
                let ok = inputs.iter().all(|input| {
                    Command::new("pw-link")
                        .args([output.to_string(), input.to_string()])
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .is_ok_and(|s| s.success())
                });
                if ok {
                    log::debug!("sound of {} (node {}) linked", stream.app, stream.node);
                    self.linked.insert(output);
                } else {
                    log::warn!("cannot link the sound of {} (port {output})", stream.app);
                }
            }
        }
        // Ports of streams that went away.
        self.linked.retain(|port| graph.ports.iter().any(|p| p.id == *port));
    }

    /// `pw-record` stopped.
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
    use crate::audio::capture::choose;
    use crate::config::AudioSource;

    const DUMP: &str = r#"[
      { "id": 46, "type": "PipeWire:Interface:Node", "info": { "props": { "media.class": "Audio/Sink", "node.name": "speakers" } } },
      { "id": 80, "type": "PipeWire:Interface:Node", "info": { "props": { "media.class": "Stream/Output/Audio",
          "application.name": "METAPHOR.exe", "application.process.binary": "wine64-preloader",
          "application.process.id": "4242", "node.name": "wine" } } },
      { "id": 84, "type": "PipeWire:Interface:Node", "info": { "props": { "media.class": "Stream/Output/Audio",
          "application.name": "METAPHOR.exe", "application.process.id": "4242" } } },
      { "id": 81, "type": "PipeWire:Interface:Node", "info": { "props": { "media.class": "Stream/Output/Audio",
          "application.name": "Firefox", "application.process.binary": "firefox", "application.process.id": 77 } } },
      { "id": 82, "type": "PipeWire:Interface:Node", "info": { "props": { "media.class": "Stream/Input/Audio",
          "application.name": "OBS", "node.name": "gameviber-audio-1-0" } } },
      { "id": 90, "type": "PipeWire:Interface:Port", "info": { "direction": "output", "props": { "node.id": 80 } } },
      { "id": 91, "type": "PipeWire:Interface:Port", "info": { "direction": "output", "props": { "node.id": 80 } } },
      { "id": 92, "type": "PipeWire:Interface:Port", "info": { "direction": "input", "props": { "node.id": 82 } } },
      { "id": 93, "type": "PipeWire:Interface:Port", "info": { "direction": "output", "props": { "node.id": 46, "port.monitor": true } } },
      { "id": 99, "type": "PipeWire:Interface:Link" }
    ]"#;

    #[test]
    fn the_graph_lists_streams_and_ports() {
        let graph = Graph::parse(DUMP.as_bytes());
        let apps: Vec<_> = graph.streams.iter().map(|s| (s.app.as_str(), s.node, s.pid)).collect();
        assert_eq!(apps, [("Firefox", 81, Some(77)), ("METAPHOR.exe", 80, Some(4242)), ("METAPHOR.exe", 84, Some(4242))]);
        assert_eq!(graph.node("gameviber-audio-1-0"), Some(82));
        assert_eq!(graph.ports(80, true).collect::<Vec<_>>(), [90, 91]);
        assert_eq!(graph.ports(82, false).collect::<Vec<_>>(), [92]);
        assert_eq!(graph.ports(46, true).count(), 0, "monitor ports are left out");
    }

    #[test]
    fn an_application_is_heard_through_all_its_streams() {
        let streams = Graph::parse(DUMP.as_bytes()).streams;
        let nodes = |t: Option<Target>| match t {
            Some(Target::App { streams, .. }) => streams.iter().map(|s| s.node).collect::<Vec<_>>(),
            other => panic!("{other:?}"),
        };
        assert_eq!(nodes(choose(&AudioSource::Auto, &streams, &[(4242, "whatever".into())])), [80, 84]);
        assert_eq!(nodes(choose(&AudioSource::Auto, &streams, &[(1, "C:\\Games\\METAPHOR.exe".into())])), [80, 84]);
        assert_eq!(choose(&AudioSource::Auto, &streams, &[]), Some(Target::Everything));
        assert_eq!(choose(&AudioSource::Everything, &streams, &[(4242, "x".into())]), Some(Target::Everything));
        assert_eq!(nodes(choose(&AudioSource::App("Firefox".into()), &streams, &[])), [81]);
        assert_eq!(choose(&AudioSource::App("Gone".into()), &streams, &[]), None);
        assert_eq!(choose(&AudioSource::Off, &streams, &[]), None);
        let metaphor = choose(&AudioSource::App("METAPHOR.exe".into()), &streams, &[]).unwrap();
        assert_eq!(metaphor.describe(), "METAPHOR.exe (2 streams)");
    }
}
