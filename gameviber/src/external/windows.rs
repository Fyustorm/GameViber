//! Windows: the named pipe other programs write messages to.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::{parse, reject, Message, Status};
use crate::platform::windows::pipe::Server;

/// `\\.\pipe\gameviber-<user>-external`: `echo {"event":"kill"} > \\.\pipe\gameviber-<user>-external`.
const PIPE: &str = "external";

pub fn start_pipe(tx: mpsc::UnboundedSender<Message>, status: Arc<Mutex<Status>>) {
    let mut server = match Server::bind(PIPE, false) {
        Ok(server) => server,
        Err(e) => {
            log::warn!("inputs: no pipe: {e}");
            return;
        }
    };
    status.lock().unwrap().pipe = Some(PathBuf::from(server.path()));
    std::thread::Builder::new()
        .name("inputs-pipe".into())
        .spawn(move || loop {
            // One writer at a time; reading ends when it closes its end.
            let Ok(client) = server.accept() else { return };
            for line in BufReader::new(client).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                match parse(&line) {
                    Ok(messages) => {
                        for m in messages {
                            if tx.send(m).is_err() {
                                return;
                            }
                        }
                    }
                    Err(why) => reject(&status, &line, why),
                }
            }
        })
        .expect("spawn the inputs pipe thread");
}
