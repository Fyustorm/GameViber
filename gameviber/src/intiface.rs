//! Output to Intiface Central (official Buttplug client over websocket).
//! The task keeps the connection alive, publishes the toy list and applies
//! the requested per-toy intensity to every actuator able to render it,
//! rate-limited as described in the spec (safety layer).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use buttplug::connector::ButtplugRemoteClientConnector;
use buttplug::device::{ButtplugClientDevice, ClientDeviceCommandValue, ClientDeviceOutputCommand};
use buttplug::{ButtplugClient, ButtplugClientEvent, ButtplugWebsocketClientTransport};
use buttplug_core::message::OutputType;
use futures::StreamExt;
use tokio::sync::watch;

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
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct IntifaceStatus {
    pub connected: bool,
    pub server: String,
    pub toys: Vec<Toy>,
    pub error: Option<String>,
}

/// Toy index -> intensity.
pub type ToyOutputs = BTreeMap<u32, f64>;

pub struct Intiface {
    outputs: watch::Sender<ToyOutputs>,
    status: watch::Receiver<IntifaceStatus>,
    client: watch::Receiver<Option<Arc<ButtplugClient>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Intiface {
    pub fn spawn(url: String) -> Self {
        let (outputs, outputs_rx) = watch::channel(ToyOutputs::new());
        let (status_tx, status) = watch::channel(IntifaceStatus::default());
        let (client_tx, client) = watch::channel(None);
        let task = tokio::spawn(run(url, outputs_rx, status_tx, client_tx));
        Self { outputs, status, client, task }
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

fn toy_list(client: &ButtplugClient) -> Vec<Toy> {
    client.devices().into_iter().map(|(index, d)| Toy { index, name: d.name().to_string() }).collect()
}

async fn run(
    url: String,
    outputs: watch::Receiver<ToyOutputs>,
    status: watch::Sender<IntifaceStatus>,
    client_slot: watch::Sender<Option<Arc<ButtplugClient>>>,
) {
    loop {
        let client = Arc::new(ButtplugClient::new("GameViber"));
        let connector: ButtplugRemoteClientConnector<ButtplugWebsocketClientTransport> =
            ButtplugRemoteClientConnector::new(ButtplugWebsocketClientTransport::new_insecure_connector(&url));
        let mut events = client.event_stream();
        if let Err(e) = client.connect(connector).await {
            log::warn!("Intiface unavailable at {url} ({e}), retrying in {RETRY_DELAY:?}");
            status.send_replace(IntifaceStatus { error: Some(e.to_string()), ..Default::default() });
            tokio::time::sleep(RETRY_DELAY).await;
            continue;
        }
        let server = client.server_name().unwrap_or_default();
        log::info!("connected to Intiface '{server}' ({url})");
        if let Err(e) = client.start_scanning().await {
            log::warn!("StartScanning refused: {e}");
        }
        client_slot.send_replace(Some(client.clone()));
        let publish = |client: &ButtplugClient| {
            status.send_replace(IntifaceStatus {
                connected: true,
                server: server.clone(),
                toys: toy_list(client),
                error: None,
            });
        };
        publish(&client);

        let mut sent: BTreeMap<u32, f64> = BTreeMap::new();
        let mut ticker = tokio::time::interval(SEND_PERIOD);
        loop {
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
                }
                event = events.next() => match event {
                    Some(ButtplugClientEvent::DeviceAdded(device)) => {
                        log::info!("toy added: [{}] {}", device.index(), device.name());
                        publish(&client);
                    }
                    Some(ButtplugClientEvent::DeviceRemoved(device)) => {
                        log::info!("toy removed: {}", device.name());
                        sent.remove(&device.index());
                        publish(&client);
                    }
                    Some(ButtplugClientEvent::Error(e)) => log::warn!("Intiface error: {e}"),
                    Some(ButtplugClientEvent::ServerDisconnect) | None => break,
                    Some(_) => {}
                },
            }
        }
        client_slot.send_replace(None);
        status.send_replace(IntifaceStatus { error: Some("disconnected".into()), ..Default::default() });
        log::warn!("disconnected from Intiface, retrying in {RETRY_DELAY:?}");
        tokio::time::sleep(RETRY_DELAY).await;
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
