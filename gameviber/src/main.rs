//! GameViber : intercepte le rumble envoyé par les jeux à la manette et le
//! relaie vers Intiface Central.

mod intiface;
mod rumble;
mod source;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;

use crate::intiface::Intiface;
use crate::rumble::RumbleState;
use crate::source::ebpf::EbpfSource;
use crate::source::proxy::ProxySource;
use crate::source::{SourceEvent, SourceKind};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SourceChoice {
    /// Manette virtuelle uinput (sans root, sauf --hide)
    Proxy,
    /// Observation passive par sonde eBPF (root requis, le jeu voit la vraie manette)
    Ebpf,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Combine {
    Avg,
    Max,
}

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Méthode d'interception du rumble
    #[arg(long, value_enum, default_value = "proxy")]
    source: SourceChoice,
    /// Manette à utiliser (/dev/input/eventX). Proxy : détection auto ;
    /// ebpf : toutes les manettes si absent
    #[arg(long)]
    device: Option<PathBuf>,
    /// Proxy : masquer la vraie manette aux jeux (root requis)
    #[arg(long)]
    hide: bool,
    /// Proxy : ne pas faire vibrer la vraie manette
    #[arg(long)]
    no_passthrough: bool,
    /// URL du serveur Intiface
    #[arg(long, default_value = "ws://127.0.0.1:12345")]
    url: String,
    /// Ne pas se connecter à Intiface, juste logger le rumble
    #[arg(long)]
    no_intiface: bool,
    #[arg(long, default_value_t = 1.0)]
    multiplier: f64,
    /// Vitesse minimale 0..1
    #[arg(long, default_value_t = 0.0)]
    baseline: f64,
    /// Combinaison des deux moteurs
    #[arg(long, value_enum, default_value = "avg")]
    combine: Combine,
    /// Période d'évaluation / d'envoi (ms)
    #[arg(long, default_value_t = 50)]
    interval_ms: u64,
    /// Logs détaillés (effets, boutons)
    #[arg(short, long)]
    verbose: bool,
}

/// Formule du Game Haptics Router : moyenne (ou max) des moteurs × multiplicateur,
/// plancher `baseline`, bornée à 1.
fn to_speed(strong: u16, weak: u16, args: &Args) -> f64 {
    let level = match args.combine {
        Combine::Avg => (strong as f64 + weak as f64) / 2.0,
        Combine::Max => strong.max(weak) as f64,
    };
    if level == 0.0 && args.baseline == 0.0 {
        return 0.0;
    }
    (level / 65535.0 * args.multiplier).max(args.baseline).min(1.0)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let level = if args.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(format!("{level},tungstenite=info,buttplug=info")))
        .format_target(false)
        .init();

    let (tx, mut rx) = mpsc::unbounded_channel::<SourceEvent>();
    let mut proxy = None;
    let mut _ebpf = None;
    match args.source {
        SourceChoice::Proxy => {
            proxy = Some(ProxySource::start(args.device.as_deref(), !args.no_passthrough, args.hide, tx)?)
        }
        SourceChoice::Ebpf => _ebpf = Some(EbpfSource::start(tx)?),
    }
    let device_filter = match args.source {
        SourceChoice::Ebpf => args.device.as_ref().map(|p| p.to_string_lossy().into_owned()),
        SourceChoice::Proxy => None,
    };
    let intiface = (!args.no_intiface).then(|| Intiface::spawn(args.url.clone()));

    let mut states: HashMap<String, RumbleState> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_millis(args.interval_ms));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut last_speed = None;

    loop {
        tokio::select! {
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
            Some(ev) = rx.recv() => {
                if device_filter.as_ref().is_some_and(|d| *d != ev.device) {
                    continue;
                }
                apply_event(&mut states, ev);
            }
            _ = tick.tick() => {
                let now = Instant::now();
                let (strong, weak) = states
                    .values_mut()
                    .map(|s| s.motors(now))
                    .fold((0, 0), |(s, w), (s2, w2)| (s.max(s2), w.max(w2)));
                let speed = (to_speed(strong, weak, &args) * 100.0).round() / 100.0;
                if last_speed != Some(speed) {
                    log::info!("rumble fort={strong:5} faible={weak:5} -> vitesse {speed:.2}");
                    if let Some(i) = &intiface {
                        i.set_speed(speed);
                    }
                    last_speed = Some(speed);
                }
            }
        }
    }

    log::info!("Arrêt");
    if let Some(i) = intiface {
        i.shutdown().await;
    }
    if let Some(p) = proxy {
        p.shutdown();
    }
    Ok(())
}

fn apply_event(states: &mut HashMap<String, RumbleState>, ev: SourceEvent) {
    let now = Instant::now();
    match ev.kind {
        SourceKind::Button { code, pressed } => {
            log::debug!("{} bouton {:?} {}", ev.device, evdev::KeyCode(code), if pressed { "pressé" } else { "relâché" });
            return;
        }
        SourceKind::Axis { code, value } => {
            log::trace!("{} axe {:?} = {value}", ev.device, evdev::AbsoluteAxisCode(code));
            return;
        }
        _ => {}
    }
    let state = states.entry(ev.device.clone()).or_default();
    match ev.kind {
        SourceKind::Upload { id, effect } => {
            log::debug!("{} upload id={id} {:?} durée={}ms", ev.device, effect.kind, effect.length_ms);
            state.upload(id, effect);
        }
        SourceKind::Erase { id } => state.erase(id),
        SourceKind::Play { id, count } => {
            log::debug!("{} play id={id} count={count}", ev.device);
            state.play(id, count, now);
        }
        SourceKind::Gain(gain) => state.set_gain(gain),
        SourceKind::Button { .. } | SourceKind::Axis { .. } => unreachable!(),
    }
}
