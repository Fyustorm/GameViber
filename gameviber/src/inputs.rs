//! Values and events other programs send (docs/spec-modes.md §6.5): a game's
//! existing mod, a script reading a game's API, a stream deck... They reach
//! GameViber as JSON over a local WebSocket (`ws://127.0.0.1:<port>`) or, one
//! message per line, through a named pipe in the runtime directory:
//!
//! ```json
//! {"set": {"hp": 0.4, "ammo": 12}}
//! {"event": "kill", "data": {"weapon": "bow"}}
//! ```
//!
//! Browsers are turned away (they send an `Origin` header): a web page must
//! not drive the toys.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::profile::valid_name;

/// At most this many values are kept.
const MAX_VALUES: usize = 64;
/// Longest message taken.
const MAX_MESSAGE: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `input.custom.<name>`; `Null` removes it.
    Set(String, Value),
    /// `on_event`.
    Event(String, Value),
}

/// Parses one message: an object with `set` and/or `event` (and `data`), or an array of them.
pub fn parse(text: &str) -> Result<Vec<Message>, String> {
    if text.len() > MAX_MESSAGE {
        return Err("message too long".into());
    }
    let value: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let mut out = Vec::new();
    let items = match value {
        Value::Array(items) => items,
        other => vec![other],
    };
    for item in items {
        let Value::Object(mut object) = item else { return Err("expected an object".into()) };
        let set = object.remove("set");
        let event = object.remove("event");
        if set.is_none() && event.is_none() {
            return Err("expected \"set\" or \"event\"".into());
        }
        if let Some(set) = set {
            let Value::Object(values) = set else { return Err("\"set\" must be an object of values".into()) };
            for (name, value) in values {
                if !valid_name(&name) {
                    return Err(format!("invalid name '{name}' (letters, digits and _, starting with a letter)"));
                }
                out.push(Message::Set(name, value));
            }
        }
        if let Some(event) = event {
            let Value::String(name) = event else { return Err("\"event\" must be a name".into()) };
            if !valid_name(&name) {
                return Err(format!("invalid event name '{name}'"));
            }
            out.push(Message::Event(name, object.remove("data").unwrap_or(Value::Null)));
        }
    }
    Ok(out)
}

/// What the GUI shows of the inputs.
#[derive(Debug, Clone, Default)]
pub struct InputsView {
    /// WebSocket address, when listening.
    pub address: Option<String>,
    pub pipe: Option<PathBuf>,
    pub error: Option<String>,
    /// Programs connected over the WebSocket.
    pub clients: usize,
    /// Values received, as JSON text.
    pub values: BTreeMap<String, String>,
    /// Latest events: engine time, name.
    pub events: Vec<(f64, String)>,
    /// The last message that could not be read, and why.
    pub rejected: Option<String>,
}

#[derive(Default)]
struct Status {
    address: Option<String>,
    pipe: Option<PathBuf>,
    error: Option<String>,
    clients: usize,
    rejected: Option<String>,
}

/// The WebSocket server and the named pipe, both feeding one channel.
pub struct Inputs {
    rx: mpsc::UnboundedReceiver<Message>,
    status: Arc<Mutex<Status>>,
    server: Option<tokio::task::JoinHandle<()>>,
    values: BTreeMap<String, Value>,
    events: Vec<(f64, String)>,
}

impl Inputs {
    /// Starts listening on `port` (0: WebSocket off) and on the pipe. Needs a Tokio runtime.
    pub fn start(port: u16) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let server = (port != 0).then(|| tokio::spawn(serve(port, tx.clone(), status.clone())));
        start_pipe(tx, status.clone());
        Self { rx, status, server, values: BTreeMap::new(), events: Vec::new() }
    }

    /// Messages received since the last call; `time` dates the events for the GUI.
    pub fn poll(&mut self, time: f64) -> Vec<Message> {
        let mut out = Vec::new();
        while let Ok(message) = self.rx.try_recv() {
            match &message {
                Message::Set(name, Value::Null) => {
                    self.values.remove(name);
                }
                Message::Set(name, value) => {
                    if self.values.len() >= MAX_VALUES && !self.values.contains_key(name) {
                        self.status.lock().unwrap().rejected = Some(format!("{name}: more than {MAX_VALUES} values"));
                        continue;
                    }
                    self.values.insert(name.clone(), value.clone());
                }
                Message::Event(name, _) => {
                    self.events.push((time, name.clone()));
                    if self.events.len() > 8 {
                        self.events.remove(0);
                    }
                }
            }
            out.push(message);
        }
        out
    }

    pub fn view(&self) -> InputsView {
        let status = self.status.lock().unwrap();
        InputsView {
            address: status.address.clone(),
            pipe: status.pipe.clone(),
            error: status.error.clone(),
            clients: status.clients,
            values: self.values.iter().map(|(k, v)| (k.clone(), v.to_string())).collect(),
            events: self.events.clone(),
            rejected: status.rejected.clone(),
        }
    }

    pub fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
        }
    }
}

fn reject(status: &Mutex<Status>, text: &str, why: String) {
    let preview: String = text.chars().take(60).collect();
    log::debug!("input message rejected: {why}: {preview}");
    status.lock().unwrap().rejected = Some(format!("{why}: {preview}"));
}

async fn serve(port: u16, tx: mpsc::UnboundedSender<Message>, status: Arc<Mutex<Status>>) {
    let address = format!("127.0.0.1:{port}");
    let listener = match tokio::net::TcpListener::bind(&address).await {
        Ok(l) => l,
        Err(e) => {
            log::warn!("inputs: cannot listen on {address}: {e}");
            status.lock().unwrap().error = Some(format!("cannot listen on {address}: {e}"));
            return;
        }
    };
    log::info!("inputs: listening on ws://{address}");
    status.lock().unwrap().address = Some(format!("ws://{address}"));
    while let Ok((stream, _)) = listener.accept().await {
        let (tx, status) = (tx.clone(), status.clone());
        tokio::spawn(async move {
            // The handshake callback's signature is tungstenite's.
            #[allow(clippy::result_large_err)]
            let no_browsers = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
                if request.headers().contains_key("origin") {
                    let mut refused = ErrorResponse::new(Some("web pages may not send inputs".into()));
                    *refused.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN;
                    return Err(refused);
                }
                Ok(response)
            };
            let Ok(mut socket) = tokio_tungstenite::accept_hdr_async(stream, no_browsers).await else { return };
            status.lock().unwrap().clients += 1;
            while let Some(Ok(message)) = socket.next().await {
                let WsMessage::Text(text) = message else { continue };
                match parse(&text) {
                    Ok(messages) => messages.into_iter().for_each(|m| drop(tx.send(m))),
                    Err(why) => reject(&status, &text, why),
                }
            }
            status.lock().unwrap().clients -= 1;
        });
    }
}

/// `$XDG_RUNTIME_DIR/gameviber/inputs`: `echo '{"event":"kill"}' > $XDG_RUNTIME_DIR/gameviber/inputs`.
fn pipe_path() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
        // SAFETY: getuid cannot fail.
        PathBuf::from(format!("/tmp/gameviber-{}", unsafe { libc::getuid() }))
    });
    runtime.join("gameviber").join("inputs")
}

fn start_pipe(tx: mpsc::UnboundedSender<Message>, status: Arc<Mutex<Status>>) {
    let path = pipe_path();
    let made = path.parent().map(std::fs::create_dir_all).transpose().map(|_| ()).and_then(|()| {
        use std::os::unix::fs::FileTypeExt;
        if std::fs::metadata(&path).is_ok_and(|m| m.file_type().is_fifo()) {
            return Ok(());
        }
        let _ = std::fs::remove_file(&path);
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(std::io::Error::other)?;
        // SAFETY: mkfifo with a valid path; only the user may write to it.
        match unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) } {
            0 => Ok(()),
            _ => Err(std::io::Error::last_os_error()),
        }
    });
    if let Err(e) = made {
        log::warn!("inputs: no pipe at {}: {e}", path.display());
        return;
    }
    status.lock().unwrap().pipe = Some(path.clone());
    std::thread::Builder::new()
        .name("inputs-pipe".into())
        .spawn(move || loop {
            // Opening blocks until a writer comes; reading ends when the last writer leaves.
            let Ok(file) = std::fs::File::open(&path) else { return };
            for line in BufReader::new(file).lines().map_while(Result::ok) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_are_parsed_and_checked() {
        assert_eq!(
            parse(r#"{"set": {"hp": 0.4, "stance": "low"}}"#).unwrap(),
            vec![Message::Set("hp".into(), json!(0.4)), Message::Set("stance".into(), json!("low"))]
        );
        assert_eq!(
            parse(r#"[{"event": "kill", "data": {"weapon": "bow"}}, {"event": "jump"}]"#).unwrap(),
            vec![Message::Event("kill".into(), json!({"weapon": "bow"})), Message::Event("jump".into(), Value::Null)]
        );
        assert!(parse("hello").unwrap_err().contains("not JSON"));
        assert!(parse(r#"{"hp": 1}"#).unwrap_err().contains("\"set\" or \"event\""));
        assert!(parse(r#"{"set": {"health bar": 1}}"#).unwrap_err().contains("invalid name"));
        assert!(parse(r#"{"event": 3}"#).unwrap_err().contains("must be a name"));
    }

    #[tokio::test]
    async fn websocket_messages_arrive_and_browsers_are_refused() {
        use futures::SinkExt;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let port = 20000 + (std::process::id() % 20000) as u16;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let status = Arc::new(Mutex::new(Status::default()));
        tokio::spawn(serve(port, tx, status.clone()));
        let url = format!("ws://127.0.0.1:{port}");
        let mut socket = None;
        for _ in 0..50 {
            if let Ok((s, _)) = tokio_tungstenite::connect_async(&url).await {
                socket = Some(s);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let mut socket = socket.expect("server up");
        socket.send(WsMessage::Text(r#"{"set": {"hp": 1}}"#.into())).await.unwrap();
        assert_eq!(rx.recv().await, Some(Message::Set("hp".into(), json!(1))));
        socket.send(WsMessage::Text("nonsense".into())).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(status.lock().unwrap().rejected.as_deref().is_some_and(|r| r.contains("not JSON")));

        let mut request = url.into_client_request().unwrap();
        request.headers_mut().insert("Origin", "https://example.com".parse().unwrap());
        assert!(tokio_tungstenite::connect_async(request).await.is_err(), "a web page is refused");
    }
}
