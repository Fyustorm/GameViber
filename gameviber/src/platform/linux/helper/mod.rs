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

use gameviber_common::{ProbeRecord, EVIOCRMFF, FF_EFFECT_SIZE};
use serde::{Deserialize, Serialize};

pub const SUBCOMMAND: &str = "helper";
/// The polkit action a package installs for the helper (`packaging/linux/`):
/// it names the request in the password dialog, and only covers this path.
const PACKAGE_ACTION: &str = "io.github.gameviber.GameViber.helper";
const PACKAGE_POLICY: &str = "/usr/share/polkit-1/actions/io.github.gameviber.GameViber.policy";
const PACKAGE_EXE: &str = "/usr/bin/gameviber";
/// The action pkexec checks for any other program.
pub const PKEXEC_ACTION: &str = "org.freedesktop.policykit.exec";

/// The polkit action pkexec will check when starting the helper.
pub fn polkit_action() -> &'static str {
    let packaged = std::path::Path::new(PACKAGE_POLICY).exists()
        && std::env::current_exe().is_ok_and(|exe| exe == std::path::Path::new(PACKAGE_EXE));
    if packaged {
        PACKAGE_ACTION
    } else {
        PKEXEC_ACTION
    }
}

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

/// A probe record, as the helper sends it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireProbe {
    pub erased: bool,
    pub pid: u32,
    pub fd: i32,
    pub effect_id: i16,
    /// The raw `struct ff_effect` (`ProbeRecord::effect`).
    pub effect: Vec<u8>,
}

impl From<&ProbeRecord> for WireProbe {
    fn from(record: &ProbeRecord) -> Self {
        Self {
            erased: record.cmd == EVIOCRMFF,
            pid: record.pid,
            fd: record.fd,
            effect_id: record.effect_id(),
            effect: record.effect.to_vec(),
        }
    }
}

impl WireProbe {
    /// The uploaded effect (None for an erasure, or a malformed record).
    pub fn effect(&self) -> Option<crate::rumble::Effect> {
        let raw: &[u8; FF_EFFECT_SIZE] = self.effect.as_slice().try_into().ok()?;
        (!self.erased).then(|| crate::rumble::Effect::from_kernel(raw))
    }
}

/// Reads the probe's records from its ring buffer entries.
pub fn read_record(item: &[u8]) -> Option<ProbeRecord> {
    // SAFETY: the probe writes whole `ProbeRecord`s; the length is checked.
    (item.len() >= std::mem::size_of::<ProbeRecord>()).then(|| unsafe { std::ptr::read_unaligned(item.as_ptr() as *const ProbeRecord) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_package_policy_covers_the_helper_only() {
        let policy = include_str!("../../../../../packaging/linux/io.github.gameviber.GameViber.policy");
        assert!(policy.contains(&format!("<action id=\"{PACKAGE_ACTION}\">")));
        assert!(policy.contains(&format!("\"org.freedesktop.policykit.exec.path\">{PACKAGE_EXE}<")));
        assert!(policy.contains(&format!("\"org.freedesktop.policykit.exec.argv1\">{SUBCOMMAND}<")));
        assert!(PACKAGE_POLICY.ends_with(&format!("/{}.policy", crate::shortcuts::APP_ID)));
    }

    #[test]
    fn messages_round_trip_as_json_lines() {
        let mut record = ProbeRecord { pid: 42, fd: 7, cmd: gameviber_common::EVIOCSFF, effect: [0; FF_EFFECT_SIZE] };
        record.effect[..2].copy_from_slice(&gameviber_common::FF_RUMBLE.to_ne_bytes());
        record.effect[2..4].copy_from_slice(&3i16.to_ne_bytes());
        record.effect[16..18].copy_from_slice(&0x8000u16.to_ne_bytes());
        let probe = WireProbe::from(&record);
        assert_eq!((probe.effect_id, probe.erased), (3, false));
        let effect = probe.effect().unwrap();
        assert_eq!(effect.kind, crate::rumble::EffectKind::Rumble { strong: 0x8000, weak: 0 });
        let erased = WireProbe::from(&ProbeRecord { cmd: EVIOCRMFF, ..record });
        assert!(erased.erased && erased.effect().is_none());
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
