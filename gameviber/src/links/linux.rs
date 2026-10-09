//! Linux: the running GameViber listens on a Unix socket in the runtime
//! directory (only the player reaches it); a second start writes its link
//! there, one line, and quits. A socket left by a GameViber that crashed
//! refuses connections: it is replaced.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use super::{Claim, MAX_LINK};

const TIMEOUT: Duration = Duration::from_secs(2);

fn socket_path() -> PathBuf {
    crate::platform::linux::runtime_dir().join("gameviber").join("instance")
}

pub struct Instance {
    listener: Option<UnixListener>,
}

/// Becomes the running GameViber, or hands `link` to the one already running.
pub fn claim(link: Option<&str>) -> Claim {
    let path = socket_path();
    if forward(&path, link) {
        return Claim::Forwarded;
    }
    crate::platform::linux::desktop::open_links();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(&path);
    match UnixListener::bind(&path) {
        Ok(listener) => Claim::First(Instance { listener: Some(listener) }),
        // Another start bound it in between.
        Err(_) if forward(&path, link) => Claim::Forwarded,
        Err(e) => {
            log::warn!("links: cannot listen on {}: {e}", path.display());
            Claim::First(Instance { listener: None })
        }
    }
}

/// Writes `link` to the GameViber listening at `path`, if one does.
fn forward(path: &PathBuf, link: Option<&str>) -> bool {
    let Ok(mut stream) = UnixStream::connect(path) else { return false };
    let _ = stream.set_write_timeout(Some(TIMEOUT));
    match writeln!(stream, "{}", link.unwrap_or_default().trim()) {
        Ok(()) => {
            log::info!("GameViber already runs: handed it {}", link.unwrap_or("nothing"));
            true
        }
        Err(e) => {
            log::warn!("links: GameViber runs but does not answer: {e}");
            false
        }
    }
}

impl Instance {
    /// Calls `on_link` with what each later start hands over ("" for nothing), on a thread of its own.
    pub fn listen(self, on_link: impl Fn(String) + Send + 'static) {
        let Some(listener) = self.listener else { return };
        let spawned = std::thread::Builder::new().name("links".into()).spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(TIMEOUT));
                let mut line = String::new();
                if BufReader::new(stream.take(MAX_LINK as u64 + 1)).read_line(&mut line).is_ok() {
                    on_link(line.trim().to_owned());
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("links: cannot listen: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_start_hands_its_link_over() {
        let dir = std::env::temp_dir().join(format!("gameviber-links-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("instance");
        let listener = UnixListener::bind(&path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        Instance { listener: Some(listener) }.listen(move |link| tx.send(link).unwrap());

        assert!(forward(&path, Some(" gameviber://mode/abc ")));
        assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), "gameviber://mode/abc");
        assert!(forward(&path, None));
        assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), "");

        // Left by a GameViber that crashed: nobody answers.
        let stale = dir.join("stale");
        drop(UnixListener::bind(&stale).unwrap());
        assert!(!forward(&stale, None));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
