//! Unprivileged side: starts the helper through pkexec on first use and
//! exchanges JSON lines with it. Never blocks the caller: the authorization
//! dialog runs in the background and progress is exposed through `state()`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use super::{Reply, Request, WireProbe, SUBCOMMAND};

/// pkexec exit codes when the user dismissed or failed the authorization.
const PKEXEC_DISMISSED: i32 = 126;
const PKEXEC_NOT_AUTHORIZED: i32 = 127;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Phase {
    #[default]
    NotStarted,
    /// pkexec started, waiting for the password dialog.
    Authorizing,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub struct HelperState {
    pub phase: Phase,
    pub ebpf: bool,
    pub hidden: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Default)]
struct Process {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
}

#[derive(Default)]
pub struct Helper {
    process: Mutex<Process>,
    state: Arc<Mutex<HelperState>>,
    probe_sink: Arc<Mutex<Option<UnboundedSender<WireProbe>>>>,
}

impl Helper {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn state(&self) -> HelperState {
        self.state.lock().unwrap().clone()
    }

    /// Where probe events go (None = dropped).
    pub fn set_probe_sink(&self, sink: Option<UnboundedSender<WireProbe>>) {
        *self.probe_sink.lock().unwrap() = sink;
    }

    /// Sends a request, starting the helper (and its authorization) if needed.
    /// Undo requests (unhide, stop) are dropped when the helper is not running:
    /// there is nothing to undo, and no reason to ask for a password.
    pub fn request(&self, request: Request) {
        let mut process = self.process.lock().unwrap();
        let running = process.child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)));
        if !running && matches!(request, Request::Unhide | Request::StopEbpf) {
            return;
        }
        if !running {
            if let Err(e) = self.spawn(&mut process) {
                self.state.lock().unwrap().phase = Phase::Failed(format!("cannot start pkexec: {e}"));
                return;
            }
        }
        let line = serde_json::to_string(&request).expect("request serializes");
        let written = process.stdin.as_mut().map(|stdin| writeln!(stdin, "{line}").and_then(|_| stdin.flush()));
        if !matches!(written, Some(Ok(()))) {
            log::warn!("cannot send {request:?} to the helper");
        }
    }

    fn spawn(&self, process: &mut Process) -> std::io::Result<()> {
        let exe = std::env::current_exe()?;
        log::info!("starting the privileged helper (pkexec {} {SUBCOMMAND})", exe.display());
        let mut child = Command::new("pkexec")
            .arg(&exe)
            .arg(SUBCOMMAND)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            // Own process group: Ctrl+C in our terminal must not reach the helper.
            .process_group(0)
            .spawn()?;
        let stdout = child.stdout.take().expect("piped stdout");
        process.stdin = child.stdin.take();
        process.child = Some(child);
        *self.state.lock().unwrap() = HelperState { phase: Phase::Authorizing, ..Default::default() };

        let (state, sink) = (self.state.clone(), self.probe_sink.clone());
        std::thread::Builder::new().name("helper-client".into()).spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                match serde_json::from_str::<Reply>(&line) {
                    Ok(Reply::Probe(probe)) => {
                        if let Some(sink) = sink.lock().unwrap().as_ref() {
                            let _ = sink.send(probe);
                        }
                    }
                    Ok(reply) => apply_reply(&state, reply),
                    Err(e) => log::warn!("unexpected helper output {line:?}: {e}"),
                }
            }
            let mut state = state.lock().unwrap();
            state.phase = match state.phase {
                Phase::Authorizing => Phase::Failed("authorization refused or cancelled".into()),
                _ => Phase::Failed("helper exited".into()),
            };
            state.ebpf = false;
            state.hidden = None;
        })?;
        Ok(())
    }

    /// Closes the helper's stdin so it restores everything and exits.
    pub fn shutdown(&self) {
        let mut process = self.process.lock().unwrap();
        process.stdin = None;
        if let Some(mut child) = process.child.take() {
            let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if matches!(status.code(), Some(PKEXEC_DISMISSED | PKEXEC_NOT_AUTHORIZED)) {
                            log::debug!("helper was never authorized");
                        }
                        break;
                    }
                    Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                    _ => {
                        log::warn!("the helper did not exit in time");
                        break;
                    }
                }
            }
        }
    }
}

fn apply_reply(state: &Mutex<HelperState>, reply: Reply) {
    let mut state = state.lock().unwrap();
    match reply {
        Reply::Ready => {
            log::info!("privileged helper ready");
            state.phase = Phase::Ready;
        }
        Reply::EbpfStarted => state.ebpf = true,
        Reply::EbpfStopped => state.ebpf = false,
        Reply::Hidden { device } => state.hidden = Some(device),
        Reply::Unhidden => state.hidden = None,
        Reply::Error { message } => {
            log::error!("helper: {message}");
            state.last_error = Some(message);
        }
        Reply::Probe(_) => {}
    }
}
