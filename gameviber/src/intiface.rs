//! Output to Intiface Central (official Buttplug client over websocket).
//! The task keeps the connection alive, publishes the toy list and applies
//! the requested per-toy intensity to every actuator able to render it,
//! rate-limited as described in the spec (safety layer). The player can
//! disconnect it, reconnect it at once and start or stop the scan for toys
//! (`Control`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use buttplug::connector::ButtplugRemoteClientConnector;
use buttplug::device::{ButtplugClientDevice, ClientDeviceCommandValue, ClientDeviceOutputCommand};
use buttplug::{ButtplugClient, ButtplugClientEvent, ButtplugWebsocketClientTransport};
use buttplug_core::message::OutputType;
use futures::StreamExt;
use tokio::sync::{mpsc, watch};

const RETRY_DELAY: Duration = Duration::from_secs(5);
/// At most 20 commands per second and per toy.
const SEND_PERIOD: Duration = Duration::from_millis(50);
/// Smaller changes are not sent (except going to 0).
const MIN_CHANGE: f64 = 0.01;

/// Actuator types driven with the intensity (0..1).
const OUTPUTS: [OutputType; 3] = [OutputType::Vibrate, OutputType::Rotate, OutputType::Oscillate];

#[derive(Debug, Clone, PartialEq)]
pub struct Toy {
    pub index: u32,
    /// Unique among the connected toys (see `toy_list`).
    pub name: String,
    /// Another toy has the same name: this one got a number, which depends on the
    /// connection order.
    pub numbered: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct IntifaceStatus {
    pub connected: bool,
    /// Disconnected by the player: no new attempt until `Control::Connect`.
    pub paused: bool,
    /// Intiface Central is looking for new toys (started on connection).
    pub scanning: bool,
    pub server: String,
    pub toys: Vec<Toy>,
    pub error: Option<String>,
}

/// Toy index -> intensity.
pub type ToyOutputs = BTreeMap<u32, f64>;

/// What the player asks of the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// Connects now: after a disconnection by the player, instead of waiting for
    /// the next attempt, or again when connected.
    Connect,
    /// Stops the toys and disconnects, until `Connect`.
    Disconnect,
    StartScanning,
    StopScanning,
}

pub struct Intiface {
    outputs: watch::Sender<ToyOutputs>,
    status: watch::Receiver<IntifaceStatus>,
    client: watch::Receiver<Option<Arc<ButtplugClient>>>,
    control: mpsc::UnboundedSender<Control>,
    task: tokio::task::JoinHandle<()>,
}

impl Intiface {
    pub fn spawn(url: String) -> Self {
        let (outputs, outputs_rx) = watch::channel(ToyOutputs::new());
        let (status_tx, status) = watch::channel(IntifaceStatus::default());
        let (client_tx, client) = watch::channel(None);
        let (control, control_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(url, outputs_rx, status_tx, client_tx, control_rx));
        Self { outputs, status, client, control, task }
    }

    pub fn control(&self, control: Control) {
        let _ = self.control.send(control);
    }

    pub fn set_outputs(&self, outputs: ToyOutputs) {
        self.outputs.send_if_modified(|current| {
            let changed = *current != outputs;
            *current = outputs;
            changed
        });
    }

    pub fn status(&self) -> IntifaceStatus {
        self.status.borrow().clone()
    }

    /// Stops every toy, then the task.
    pub async fn shutdown(self) {
        self.task.abort();
        let client = self.client.borrow().clone();
        if let Some(client) = client {
            let _ = tokio::time::timeout(Duration::from_secs(2), client.stop_all_devices()).await;
            let _ = client.disconnect().await;
        }
    }
}

/// Toys by index. Names identify toys in the routing, so they are unique: the
/// name given in Intiface Central if any, then " #2", " #3"... for identical toys.
fn toy_list(client: &ButtplugClient) -> Vec<Toy> {
    let mut devices: Vec<(u32, String)> = client
        .devices()
        .into_iter()
        .map(|(index, d)| (index, d.display_name().clone().unwrap_or_else(|| d.name().to_string())))
        .collect();
    devices.sort();
    unique_names(devices)
}

fn unique_names(devices: Vec<(u32, String)>) -> Vec<Toy> {
    let mut toys: Vec<Toy> = Vec::with_capacity(devices.len());
    for (index, name) in devices {
        let mut unique = name.clone();
        let mut n = 1;
        while toys.iter().any(|t| t.name == unique) {
            n += 1;
            unique = format!("{name} #{n}");
        }
        toys.push(Toy { index, name: unique, numbered: n > 1 });
    }
    toys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_toys_get_distinct_names() {
        let toys = unique_names(vec![(0, "Lush 3".into()), (1, "Nora".into()), (2, "Lush 3".into()), (3, "Lush 3".into())]);
        let names: Vec<_> = toys.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Lush 3", "Nora", "Lush 3 #2", "Lush 3 #3"]);
        assert_eq!(toys[2].index, 2);
        assert!(!toys[0].numbered && toys[2].numbered);
    }
}

/// Why a connection ended.
enum End {
    Lost,
    Reconnect,
    Paused,
}

async fn run(
    url: String,
    outputs: watch::Receiver<ToyOutputs>,
    status: watch::Sender<IntifaceStatus>,
    client_slot: watch::Sender<Option<Arc<ButtplugClient>>>,
    mut control: mpsc::UnboundedReceiver<Control>,
) {
    let mut paused = false;
    loop {
        if paused {
            status.send_replace(IntifaceStatus { paused: true, ..Default::default() });
            log::info!("disconnected from Intiface by the player");
            loop {
                match control.recv().await {
                    Some(Control::Connect) => break,
                    Some(_) => {}
                    None => return,
                }
            }
            status.send_modify(|s| s.paused = false);
        }
        let client = Arc::new(ButtplugClient::new("GameViber"));
        let connector: ButtplugRemoteClientConnector<ButtplugWebsocketClientTransport> =
            ButtplugRemoteClientConnector::new(ButtplugWebsocketClientTransport::new_insecure_connector(&url));
        let mut events = client.event_stream();
        let connected = tokio::select! {
            result = client.connect(connector) => result,
            // An address that does not answer can take long to fail.
            () = wait_for(&mut control, Control::Disconnect) => {
                paused = true;
                continue;
            }
        };
        if let Err(e) = connected {
            log::warn!("Intiface unavailable at {url} ({e}), retrying in {RETRY_DELAY:?}");
            status.send_replace(IntifaceStatus { error: Some(e.to_string()), ..Default::default() });
            match wait_retry(&mut control).await {
                Some(stop) => paused = stop,
                None => return,
            }
            continue;
        }
        let server = client.server_name().unwrap_or_default();
        log::info!("connected to Intiface '{server}' ({url})");
        let mut current = IntifaceStatus { connected: true, server, ..Default::default() };
        match client.start_scanning().await {
            Ok(()) => current.scanning = true,
            Err(e) => log::warn!("StartScanning refused: {e}"),
        }
        client_slot.send_replace(Some(client.clone()));
        current.toys = toy_list(&client);
        status.send_replace(current.clone());

        let mut sent: BTreeMap<u32, f64> = BTreeMap::new();
        let mut ticker = tokio::time::interval(SEND_PERIOD);
        let end = loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let wanted = outputs.borrow().clone();
                    for (index, device) in client.devices() {
                        let value = wanted.get(&index).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                        let changed = match sent.get(&index).copied() {
                            None => true,
                            Some(last) => (value - last).abs() >= MIN_CHANGE || (value == 0.0 && last != 0.0),
                        };
                        if changed {
                            apply_device(&device, value).await;
                            sent.insert(index, value);
                        }
                    }
                    continue;
                }
                event = events.next() => match event {
                    Some(ButtplugClientEvent::DeviceAdded(device)) => {
                        log::info!("toy added: [{}] {}", device.index(), device.name());
                        current.toys = toy_list(&client);
                    }
                    Some(ButtplugClientEvent::DeviceRemoved(device)) => {
                        log::info!("toy removed: {}", device.name());
                        sent.remove(&device.index());
                        current.toys = toy_list(&client);
                    }
                    Some(ButtplugClientEvent::ScanningFinished) => {
                        log::info!("Intiface stopped scanning");
                        current.scanning = false;
                    }
                    Some(ButtplugClientEvent::Error(e)) => {
                        log::warn!("Intiface error: {e}");
                        continue;
                    }
                    Some(ButtplugClientEvent::ServerDisconnect) | None => break End::Lost,
                    Some(_) => continue,
                },
                command = control.recv() => match command {
                    Some(Control::Disconnect) => break End::Paused,
                    Some(Control::Connect) => break End::Reconnect,
                    Some(scan @ (Control::StartScanning | Control::StopScanning)) => {
                        let start = scan == Control::StartScanning;
                        let result = if start { client.start_scanning().await } else { client.stop_scanning().await };
                        match result {
                            Ok(()) => {
                                log::info!("Intiface {} scanning", if start { "started" } else { "stopped" });
                                current.scanning = start;
                                current.error = None;
                            }
                            Err(e) => {
                                log::warn!("{scan:?} refused: {e}");
                                current.error = Some(e.to_string());
                            }
                        }
                    }
                    None => break End::Paused,
                },
            }
            status.send_replace(current.clone());
        };
        client_slot.send_replace(None);
        match end {
            End::Lost => {
                status.send_replace(IntifaceStatus { error: Some("disconnected".into()), ..Default::default() });
                log::warn!("disconnected from Intiface, retrying in {RETRY_DELAY:?}");
                match wait_retry(&mut control).await {
                    Some(stop) => paused = stop,
                    None => return,
                }
            }
            End::Reconnect | End::Paused => {
                let _ = tokio::time::timeout(Duration::from_secs(2), client.stop_all_devices()).await;
                let _ = client.disconnect().await;
                paused = matches!(end, End::Paused);
                if !paused {
                    log::info!("reconnecting to Intiface");
                    status.send_replace(IntifaceStatus::default());
                }
            }
        }
    }
}

/// Waits `RETRY_DELAY` before the next attempt, less if the player asks to
/// connect now. Returns whether the player asked to stop trying instead, None
/// once the `Intiface` is gone.
async fn wait_retry(control: &mut mpsc::UnboundedReceiver<Control>) -> Option<bool> {
    let delay = tokio::time::sleep(RETRY_DELAY);
    tokio::pin!(delay);
    loop {
        tokio::select! {
            () = &mut delay => return Some(false),
            command = control.recv() => match command {
                Some(Control::Connect) => return Some(false),
                Some(Control::Disconnect) => return Some(true),
                Some(_) => {}
                None => return None,
            },
        }
    }
}

/// Until the player asks for `wanted` (never once the `Intiface` is gone).
async fn wait_for(control: &mut mpsc::UnboundedReceiver<Control>, wanted: Control) {
    loop {
        match control.recv().await {
            Some(command) if command == wanted => return,
            Some(_) => {}
            None => std::future::pending().await,
        }
    }
}

async fn apply_device(device: &ButtplugClientDevice, value: f64) {
    let value = ClientDeviceCommandValue::Percent(value);
    for output in OUTPUTS {
        if !device.output_available(output) {
            continue;
        }
        let Ok(cmd) = ClientDeviceOutputCommand::from_command_value(output, &value) else { continue };
        if let Err(e) = device.run_output(&cmd).await {
            log::warn!("{output:?} command refused by {}: {e}", device.name());
        }
    }
}
