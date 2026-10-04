//! Makes a gamepad invisible to games (needs root): chmod 0600 and removal
//! of the uaccess ACLs on its eventX / jsX nodes, restored afterwards.

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context;

pub struct DeviceHider {
    device: String,
    saved: Vec<(String, u32, String)>, // (node, mode, getfacl output)
}

/// Accepts only `/dev/input/eventN` character devices: the privileged helper
/// receives this path from an unprivileged process.
pub fn validate_event_device(path: &str) -> anyhow::Result<PathBuf> {
    let canonical = std::fs::canonicalize(path).with_context(|| format!("{path}: no such device"))?;
    let name = canonical.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let is_event_name = name.strip_prefix("event").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    anyhow::ensure!(
        canonical.parent() == Some(Path::new("/dev/input")) && is_event_name,
        "{path} is not a /dev/input/eventN device"
    );
    let file_type = std::fs::metadata(&canonical)?.file_type();
    anyhow::ensure!(file_type.is_char_device(), "{path} is not a character device");
    Ok(canonical)
}

impl DeviceHider {
    pub fn hide(event_path: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(unsafe { libc::geteuid() } == 0, "hiding the gamepad needs root");
        let event_path = validate_event_device(event_path)?;
        let event = event_path.file_name().context("invalid gamepad path")?;
        let sysdir = Path::new("/sys/class/input").join(event).join("device");
        let mut me = Self { device: event_path.to_string_lossy().into_owned(), saved: Vec::new() };
        match me.hide_nodes(&sysdir) {
            Ok(()) => Ok(me),
            Err(e) => {
                me.restore(); // undo the nodes already changed
                Err(e)
            }
        }
    }

    fn hide_nodes(&mut self, sysdir: &Path) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(sysdir)? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("event") || name.starts_with("js")) {
                continue;
            }
            let node = format!("/dev/input/{name}");
            let mode = std::fs::metadata(&node)?.permissions().mode() & 0o7777;
            let acl = Command::new("getfacl").args(["-p", &node]).output()?;
            anyhow::ensure!(acl.status.success(), "getfacl {node} failed");
            self.saved.push((node.clone(), mode, String::from_utf8_lossy(&acl.stdout).into_owned()));
            anyhow::ensure!(Command::new("setfacl").args(["-b", &node]).status()?.success(), "setfacl -b {node} failed");
            std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o600))?;
            log::info!("real gamepad hidden: {node}");
        }
        Ok(())
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    pub fn restore(self) {
        use std::io::Write;
        for (node, mode, acl) in self.saved {
            if let Err(e) = std::fs::set_permissions(&node, std::fs::Permissions::from_mode(mode)) {
                log::warn!("cannot restore permissions of {node}: {e}");
            }
            match Command::new("setfacl").args(["-P", "--restore=-"]).stdin(Stdio::piped()).spawn() {
                Ok(mut child) => {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(acl.as_bytes());
                    }
                    let _ = child.wait();
                }
                Err(e) => log::warn!("cannot restore ACLs of {node}: {e}"),
            }
            log::info!("real gamepad visible again: {node}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_outside_dev_input_events() {
        for path in ["/etc/passwd", "/dev/input/../../etc/passwd", "/dev/null", "/dev/input/mice", "/nope"] {
            assert!(validate_event_device(path).is_err(), "{path} should be rejected");
        }
    }

    #[test]
    fn accepts_existing_event_devices() {
        if let Some(Ok(entry)) = std::fs::read_dir("/dev/input").ok().and_then(|mut d| {
            d.find(|e| e.as_ref().is_ok_and(|e| e.file_name().to_string_lossy().starts_with("event")))
        }) {
            assert!(validate_event_device(&entry.path().to_string_lossy()).is_ok());
        }
    }
}
