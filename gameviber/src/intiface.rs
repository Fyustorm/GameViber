//! Sortie vers Intiface Central (client Buttplug officiel, websocket).
//! La tâche maintient la connexion (reconnexion automatique) et applique la
//! dernière vitesse demandée à tous les jouets capables de la rendre.

use std::sync::Arc;
use std::time::Duration;

use buttplug::connector::ButtplugRemoteClientConnector;
use buttplug::device::{ButtplugClientDevice, ClientDeviceCommandValue, ClientDeviceOutputCommand};
use buttplug::{ButtplugClient, ButtplugClientEvent, ButtplugWebsocketClientTransport};
use buttplug_core::message::OutputType;
use futures::StreamExt;
use tokio::sync::watch;

const RETRY_DELAY: Duration = Duration::from_secs(5);

/// Types d'actionneurs pilotés avec l'intensité (0..1).
const OUTPUTS: [OutputType; 3] = [OutputType::Vibrate, OutputType::Rotate, OutputType::Oscillate];

pub struct Intiface {
    speed: watch::Sender<f64>,
    task: tokio::task::JoinHandle<()>,
    client: watch::Receiver<Option<Arc<ButtplugClient>>>,
}

impl Intiface {
    pub fn spawn(url: String) -> Self {
        let (speed, speed_rx) = watch::channel(0.0);
        let (client_tx, client) = watch::channel(None);
        let task = tokio::spawn(run(url, speed_rx, client_tx));
        Self { speed, task, client }
    }

    pub fn set_speed(&self, speed: f64) {
        self.speed.send_replace(speed);
    }

    /// Arrête tous les jouets puis la tâche.
    pub async fn shutdown(self) {
        self.task.abort();
        let client = self.client.borrow().clone();
        if let Some(client) = client {
            let _ = tokio::time::timeout(Duration::from_secs(2), client.stop_all_devices()).await;
            let _ = client.disconnect().await;
        }
    }
}

async fn run(url: String, mut speed: watch::Receiver<f64>, client_slot: watch::Sender<Option<Arc<ButtplugClient>>>) {
    loop {
        let client = Arc::new(ButtplugClient::new("GameViber"));
        let connector: ButtplugRemoteClientConnector<ButtplugWebsocketClientTransport> =
            ButtplugRemoteClientConnector::new(ButtplugWebsocketClientTransport::new_insecure_connector(&url));
        let mut events = client.event_stream();
        if let Err(e) = client.connect(connector).await {
            log::warn!("Intiface indisponible sur {url} ({e}), nouvelle tentative dans {RETRY_DELAY:?}");
            tokio::time::sleep(RETRY_DELAY).await;
            continue;
        }
        log::info!("Connecté à Intiface '{}' ({url})", client.server_name().unwrap_or_default());
        for (index, device) in client.devices() {
            log::info!("Appareil : [{index}] {}", device.name());
        }
        if let Err(e) = client.start_scanning().await {
            log::warn!("StartScanning refusé : {e}");
        }
        client_slot.send_replace(Some(client.clone()));
        let current = *speed.borrow();
        apply(&client, current).await;

        loop {
            tokio::select! {
                changed = speed.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let value = *speed.borrow_and_update();
                    apply(&client, value).await;
                }
                event = events.next() => match event {
                    Some(ButtplugClientEvent::DeviceAdded(device)) => {
                        log::info!("Appareil ajouté : [{}] {}", device.index(), device.name());
                        let current = *speed.borrow();
                        apply_device(&device, current).await;
                    }
                    Some(ButtplugClientEvent::DeviceRemoved(device)) => {
                        log::info!("Appareil retiré : {}", device.name());
                    }
                    Some(ButtplugClientEvent::Error(e)) => log::warn!("Erreur Intiface : {e}"),
                    Some(ButtplugClientEvent::ServerDisconnect) | None => break,
                    Some(_) => {}
                },
            }
        }
        client_slot.send_replace(None);
        log::warn!("Déconnecté d'Intiface, nouvelle tentative dans {RETRY_DELAY:?}");
        tokio::time::sleep(RETRY_DELAY).await;
    }
}

async fn apply(client: &ButtplugClient, speed: f64) {
    let devices = client.devices();
    futures::future::join_all(devices.values().map(|d| apply_device(d, speed))).await;
}

async fn apply_device(device: &ButtplugClientDevice, speed: f64) {
    let value = ClientDeviceCommandValue::Percent(speed.clamp(0.0, 1.0));
    for output in OUTPUTS {
        if !device.output_available(output) {
            continue;
        }
        let Ok(cmd) = ClientDeviceOutputCommand::from_command_value(output, &value) else { continue };
        if let Err(e) = device.run_output(&cmd).await {
            log::warn!("commande {output:?} refusée par {} : {e}", device.name());
        }
    }
}
