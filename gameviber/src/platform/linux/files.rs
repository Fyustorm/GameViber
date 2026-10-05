//! File dialogs, through the desktop's file chooser portal
//! (`org.freedesktop.portal.FileChooser`): the desktop's own dialog, with no
//! GUI toolkit to link. They block until the player picks or cancels: call
//! them from a thread of their own.

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{bail, Context};
use zbus::blocking::Connection;
use zbus::zvariant::Value;

use super::helper::dialog::portal_parent;
use super::portal;

const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";

/// Asks for a file to open, among the files named `*.<extension>` (`kind`
/// names them in the dialog). None: cancelled.
pub fn open_file(title: &str, kind: &str, extension: &str) -> anyhow::Result<Option<PathBuf>> {
    ask("OpenFile", title, kind, extension, None)
}

/// Asks where to save a file, suggesting `name`. None: cancelled.
pub fn save_file(title: &str, kind: &str, extension: &str, name: &str) -> anyhow::Result<Option<PathBuf>> {
    ask("SaveFile", title, kind, extension, Some(name))
}

fn ask(method: &str, title: &str, kind: &str, extension: &str, name: Option<&str>) -> anyhow::Result<Option<PathBuf>> {
    // Each request has a handle of its own.
    static REQUESTS: AtomicU32 = AtomicU32::new(0);
    let token = format!("gameviber_file_{}", REQUESTS.fetch_add(1, Ordering::Relaxed));
    let conn = Connection::session().context("no session bus")?;
    let filter = (kind, vec![(0u32, format!("*.{extension}"))]);
    let mut options = HashMap::from([
        ("handle_token", Value::from(token.as_str())),
        ("modal", Value::from(true)),
        ("filters", Value::from(vec![filter.clone()])),
        ("current_filter", Value::from(filter)),
    ]);
    if let Some(name) = name {
        options.insert("current_name", Value::from(name));
    }
    let results = portal::request(&conn, FILE_CHOOSER, method, &token, &(portal_parent(), title, options))
        .context("the desktop has no file dialog (xdg-desktop-portal)")?;
    let Some(results) = results else { return Ok(None) };
    let uris: Vec<String> = results.get("uris").and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok()).unwrap_or_default();
    let Some(uri) = uris.first() else { return Ok(None) };
    file_path(uri).map(Some)
}

/// The path of a `file://` URI.
fn file_path(uri: &str) -> anyhow::Result<PathBuf> {
    let Some(path) = uri.strip_prefix("file://") else { bail!("not a local file: {uri}") };
    // "file://host/path": only the local host.
    let path = &path[path.find('/').context("not a file path")?..];
    let mut bytes = Vec::with_capacity(path.len());
    let mut rest = path.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (b, tail) {
            (b'%', [h, l, tail @ ..]) if hex(*h).is_some() && hex(*l).is_some() => {
                bytes.push((hex(*h).unwrap() * 16 + hex(*l).unwrap()) as u8);
                rest = tail;
            }
            _ => {
                bytes.push(b);
                rest = tail;
            }
        }
    }
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_become_paths() {
        assert_eq!(file_path("file:///home/me/My%20Modes/a%C3%A9.gameviber").unwrap(), PathBuf::from("/home/me/My Modes/aé.gameviber"));
        assert_eq!(file_path("file://localhost/tmp/x%2").unwrap(), PathBuf::from("/tmp/x%2"));
        assert!(file_path("https://example.com/x").is_err());
    }
}
