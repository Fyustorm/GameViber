//! Windows: the running GameViber reads a named pipe of the user's
//! (`platform::windows::pipe`); a second start writes its link there, one
//! line, and quits. `gameviber://` links open GameViber through
//! `HKCU\Software\Classes\gameviber`, which the installer writes, and which
//! GameViber writes itself when no GameViber still installed has it.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use super::{Claim, MAX_LINK, SCHEME};
use crate::platform::windows::pipe::{self, Server};
use crate::platform::windows::registry::{self, Root};

const PIPE: &str = "instance";

pub struct Instance {
    server: Option<Server>,
}

/// Becomes the running GameViber, or hands `link` to the one already running.
pub fn claim(link: Option<&str>) -> Claim {
    if forward(PIPE, link) {
        return Claim::Forwarded;
    }
    open_links();
    match Server::bind(PIPE, true) {
        Ok(server) => Claim::First(Instance { server: Some(server) }),
        // Another start made it in between.
        Err(_) if forward(PIPE, link) => Claim::Forwarded,
        Err(e) => {
            log::warn!("links: cannot listen on {}: {e}", pipe::path(PIPE));
            Claim::First(Instance { server: None })
        }
    }
}

/// Writes `link` to the GameViber listening on the pipe `name`, if one does.
fn forward(name: &str, link: Option<&str>) -> bool {
    match pipe::send(name, link.unwrap_or_default().trim()) {
        Ok(()) => {
            log::info!("GameViber already runs: handed it {}", link.unwrap_or("nothing"));
            true
        }
        Err(_) => false,
    }
}

impl Instance {
    /// Calls `on_link` with what each later start hands over ("" for nothing), on a thread of its own.
    pub fn listen(self, on_link: impl Fn(String) + Send + 'static) {
        let Some(mut server) = self.server else { return };
        let spawned = std::thread::Builder::new().name("links".into()).spawn(move || loop {
            let Ok(client) = server.accept() else { return };
            let mut line = String::new();
            if BufReader::new(client.take(MAX_LINK as u64 + 1)).read_line(&mut line).is_ok() {
                on_link(line.trim().to_owned());
            }
        });
        if let Err(e) = spawned {
            log::warn!("links: cannot listen: {e}");
        }
    }
}

fn scheme_key() -> String {
    format!(r"Software\Classes\{SCHEME}")
}

/// The executable the registry opens links with, from its command line.
fn registered_exe(command: &str) -> Option<&str> {
    let command = command.trim();
    match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next(),
        None => command.split_whitespace().next(),
    }
}

/// Makes this GameViber open `gameviber://` links, unless another one still installed does.
fn open_links() {
    let Ok(exe) = std::env::current_exe() else { return };
    let command = registry::read_string(Root::CurrentUser, &format!(r"{}\shell\open\command", scheme_key()), "");
    if command.as_deref().and_then(registered_exe).is_some_and(|registered| Path::new(registered).is_file()) {
        return;
    }
    let exe = exe.display().to_string();
    let written = registry::write_strings(&scheme_key(), &[("", "URL:GameViber"), ("URL Protocol", "")])
        .and_then(|()| registry::write_strings(&format!(r"{}\DefaultIcon", scheme_key()), &[("", &format!("\"{exe}\",0"))]))
        .and_then(|()| registry::write_strings(&format!(r"{}\shell\open\command", scheme_key()), &[("", &format!("\"{exe}\" \"%1\""))]));
    match written {
        Ok(()) => log::info!("GameViber opens {SCHEME}:// links"),
        Err(e) => log::warn!("cannot make GameViber open {SCHEME}:// links: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_start_hands_its_link_over() {
        let name = format!("links-test-{}", std::process::id());
        let server = Server::bind(&name, true).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        Instance { server: Some(server) }.listen(move |link| tx.send(link).unwrap());
        let timeout = std::time::Duration::from_secs(2);

        assert!(forward(&name, Some(" gameviber://mode/abc ")));
        assert_eq!(rx.recv_timeout(timeout).unwrap(), "gameviber://mode/abc");
        assert!(forward(&name, None));
        assert_eq!(rx.recv_timeout(timeout).unwrap(), "");
        assert!(!forward("links-test-nobody", None), "nobody listens");
    }

    #[test]
    fn the_registered_executable_is_read_from_its_command() {
        assert_eq!(registered_exe(r#""C:\Program Files\GameViber\gameviber.exe" "%1""#), Some(r"C:\Program Files\GameViber\gameviber.exe"));
        assert_eq!(registered_exe(r"C:\GameViber\gameviber.exe %1"), Some(r"C:\GameViber\gameviber.exe"));
        assert_eq!(registered_exe(""), None);
    }
}
