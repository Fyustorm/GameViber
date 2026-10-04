//! Privileged helper: the only part of GameViber that runs as root.
//!
//! The GUI / engine stay unprivileged and start `gameviber helper` through
//! `pkexec` on demand (one authorization per session). The helper does two
//! things only: load the eBPF probe and forward its raw events, and hide /
//! restore the real gamepad's device nodes. They talk with JSON lines over
//! the helper's stdin (requests) and stdout (replies). When stdin closes
//! (engine exited or crashed), the helper restores everything and exits.

pub mod client;
pub mod dialog;
pub mod server;

use gameviber_common::{FfEffect, ProbeEvent, FF_UNION_WORDS, PROBE_EVENT_KIND_ERASED};
use serde::{Deserialize, Serialize};

pub const SUBCOMMAND: &str = "helper";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    StartEbpf,
    StopEbpf,
    Hide { device: String },
    Unhide,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Ready,
    EbpfStarted,
    EbpfStopped,
    Probe(WireProbe),
    Hidden { device: String },
    Unhidden,
    Error { message: String },
}

/// Serializable copy of a probe event.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireProbe {
    pub erased: bool,
    pub tgid: u32,
    pub fd: i32,
    pub effect_id: i16,
    pub kind: u16,
    pub direction: u16,
    pub replay_length: u16,
    pub replay_delay: u16,
    pub u: [u16; FF_UNION_WORDS],
}

impl From<&ProbeEvent> for WireProbe {
    fn from(ev: &ProbeEvent) -> Self {
        Self {
            erased: ev.kind == PROBE_EVENT_KIND_ERASED,
            tgid: ev.tgid,
            fd: ev.fd,
            effect_id: ev.effect_id,
            kind: ev.effect.kind,
            direction: ev.effect.direction,
            replay_length: ev.effect.replay_length,
            replay_delay: ev.effect.replay_delay,
            u: ev.effect.u,
        }
    }
}

impl WireProbe {
    pub fn effect(&self) -> FfEffect {
        FfEffect {
            kind: self.kind,
            id: self.effect_id,
            direction: self.direction,
            replay_length: self.replay_length,
            replay_delay: self.replay_delay,
            u: self.u,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_as_json_lines() {
        let probe = WireProbe {
            erased: false,
            tgid: 42,
            fd: 7,
            effect_id: 3,
            kind: 0x50,
            direction: 0,
            replay_length: 300,
            replay_delay: 0,
            u: [1; FF_UNION_WORDS],
        };
        for reply in [Reply::Ready, Reply::Probe(probe), Reply::Error { message: "x".into() }] {
            let line = serde_json::to_string(&reply).unwrap();
            assert!(!line.contains('\n'));
            assert_eq!(serde_json::from_str::<Reply>(&line).unwrap(), reply);
        }
        let request = Request::Hide { device: "/dev/input/event3".into() };
        let line = serde_json::to_string(&request).unwrap();
        assert_eq!(line, r#"{"type":"hide","device":"/dev/input/event3"}"#);
        assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), request);
    }
}
