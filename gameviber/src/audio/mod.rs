//! Listens to the game's sound (docs/spec-modes.md §6.3): cheap measures
//! every 20 ms (levels, hits) and, when the active mode declares audio scenes
//! and the model is downloaded, a CLAP embedding of the last 10 s every 2 s.
//! Everything runs on its own threads; the engine drains `Audio::poll()`
//! once per tick.

pub mod capture;
pub mod clap;
pub mod features;

use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use clap::Embedding;
pub use features::{AudioHit, AudioLevels, Band};

use crate::config::AudioSource;
use capture::Capture;

pub const SAMPLE_RATE: u32 = 48_000;
/// A new scene embedding every this many seconds.
pub const CLIP_STEP_SECS: f64 = 2.0;
/// How often the playing applications are listed again.
const RETARGET: Duration = Duration::from_secs(3);
/// A stream sending nothing this long is paused: silence is analysed instead.
const STALL: Duration = Duration::from_millis(250);
/// The scene model is unloaded after this long without clips.
const MODEL_IDLE: Duration = Duration::from_secs(30);

/// What the engine receives, in order.
#[derive(Debug, Clone)]
pub enum Output {
    Levels(AudioLevels),
    Hit(AudioHit),
    /// Embedding of the last 10 s of sound.
    Clip(Embedding),
    /// Nothing is captured any more (audio off, or the chosen application is gone).
    Inactive,
}

/// For the GUI.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// What is being listened to.
    pub target: Option<String>,
    /// Applications playing sound.
    pub streams: Vec<String>,
    pub error: Option<String>,
}

struct Control {
    source: AudioSource,
    games: Vec<(u32, String)>,
    /// The active mode declares audio scenes: compute embeddings.
    scenes: bool,
    stop: bool,
    changed: bool,
    status: Status,
}

pub struct Audio {
    control: Arc<Mutex<Control>>,
    outputs: mpsc::Receiver<Output>,
    thread: Option<JoinHandle<()>>,
}

impl Audio {
    pub fn start(source: AudioSource) -> Self {
        let control = Arc::new(Mutex::new(Control {
            source,
            games: Vec::new(),
            scenes: false,
            stop: false,
            changed: true,
            status: Status::default(),
        }));
        let (tx, outputs) = mpsc::channel();
        let thread = {
            let control = control.clone();
            std::thread::Builder::new().name("audio".into()).spawn(move || run(control, tx)).expect("spawn the audio thread")
        };
        Self { control, outputs, thread: Some(thread) }
    }

    pub fn set_source(&self, source: AudioSource) {
        let mut control = self.control.lock().unwrap();
        if control.source != source {
            control.source = source;
            control.changed = true;
        }
    }

    /// Processes showing the in-game overlay: their sound is preferred.
    pub fn set_games(&self, games: Vec<(u32, String)>) {
        let mut control = self.control.lock().unwrap();
        if control.games != games {
            control.games = games;
            control.changed = true;
        }
    }

    pub fn set_scenes(&self, wanted: bool) {
        self.control.lock().unwrap().scenes = wanted;
    }

    pub fn poll(&self) -> Vec<Output> {
        self.outputs.try_iter().collect()
    }

    pub fn status(&self) -> Status {
        self.control.lock().unwrap().status.clone()
    }

    pub fn shutdown(mut self) {
        self.control.lock().unwrap().stop = true;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(control: Arc<Mutex<Control>>, tx: mpsc::Sender<Output>) {
    let (clips_tx, clips_rx) = mpsc::sync_channel::<Vec<f32>>(1);
    {
        let tx = tx.clone();
        std::thread::Builder::new().name("audio-scenes".into()).spawn(move || scenes(clips_rx, tx)).expect("spawn the scene thread");
    }
    let mut capture: Option<Capture> = None;
    let mut analyzer = features::Analyzer::new();
    let mut recent: VecDeque<f32> = VecDeque::with_capacity(clap::CLIP_SAMPLES);
    let mut since_clip = 0usize;
    let mut last_target_check: Option<Instant> = None;
    let mut last_samples = Instant::now();
    let clip_step = (CLIP_STEP_SECS * SAMPLE_RATE as f64) as usize;
    loop {
        let (source, games, wanted, changed) = {
            let mut c = control.lock().unwrap();
            if c.stop {
                break;
            }
            (c.source.clone(), c.games.clone(), c.scenes, std::mem::take(&mut c.changed))
        };
        let ended = capture.as_mut().is_some_and(Capture::ended);
        if changed || ended || last_target_check.is_none_or(|t| t.elapsed() >= RETARGET) {
            last_target_check = Some(Instant::now());
            let streams = if source == AudioSource::Off { Vec::new() } else { capture::list_streams() };
            let target = capture::choose(&source, &streams, &games);
            if ended || target.as_ref() != capture.as_ref().map(|c| &c.target) {
                let was_active = capture.is_some();
                capture = None;
                analyzer = features::Analyzer::new();
                recent.clear();
                let mut error = None;
                if let Some(target) = target {
                    match Capture::start(target.clone()) {
                        Ok(c) => {
                            log::info!("listening to the sound of {}", target.describe());
                            capture = Some(c);
                        }
                        Err(e) => {
                            log::warn!("cannot capture the sound (pw-record): {e}");
                            error = Some(format!("pw-record: {e}"));
                        }
                    }
                }
                if was_active && capture.is_none() {
                    let _ = tx.send(Output::Inactive);
                }
                control.lock().unwrap().status.error = error;
            }
            let mut c = control.lock().unwrap();
            c.status.target = capture.as_ref().map(|c| c.target.describe());
            c.status.streams = streams.into_iter().map(|s| s.app).fold(Vec::new(), |mut list, app| {
                if !list.contains(&app) {
                    list.push(app);
                }
                list
            });
        }

        let Some(active) = capture.as_ref() else {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        let samples = match active.samples.recv_timeout(Duration::from_millis(50)) {
            Ok(samples) => {
                last_samples = Instant::now();
                samples
            }
            // A paused stream sends nothing: that is silence.
            Err(mpsc::RecvTimeoutError::Timeout) if last_samples.elapsed() >= STALL => {
                let silent = (last_samples.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as usize;
                last_samples = Instant::now();
                vec![0.0; silent]
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // pw-record exited: `ended()` restarts it on the next turn.
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
        };
        analyzer.push(&samples, |levels, hit| {
            let _ = tx.send(Output::Levels(levels));
            if let Some(hit) = hit {
                let _ = tx.send(Output::Hit(hit));
            }
        });
        if recent.len() + samples.len() > clap::CLIP_SAMPLES {
            recent.drain(..(recent.len() + samples.len() - clap::CLIP_SAMPLES).min(recent.len()));
        }
        recent.extend(samples.iter().rev().take(clap::CLIP_SAMPLES).rev());
        since_clip += samples.len();
        if since_clip >= clip_step && recent.len() == clap::CLIP_SAMPLES {
            since_clip = 0;
            if wanted && clap::model_ready() {
                // Skipped when the previous clip is still being processed.
                let _ = clips_tx.try_send(recent.iter().copied().collect());
            }
        }
    }
}

/// Embeds clips with the CLAP audio model, loaded on the first clip.
fn scenes(clips: mpsc::Receiver<Vec<f32>>, tx: mpsc::Sender<Output>) {
    let mut encoder: Option<clap::AudioEncoder> = None;
    let mut failed = false;
    loop {
        let clip = match clips.recv_timeout(MODEL_IDLE) {
            Ok(clip) => clip,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if encoder.take().is_some() {
                    log::debug!("audio scene model unloaded");
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if encoder.is_none() && !failed {
            match clap::AudioEncoder::load() {
                Ok(e) => {
                    log::info!("audio scene model loaded");
                    encoder = Some(e);
                }
                Err(e) => {
                    log::error!("cannot load the audio scene model: {e:#}");
                    failed = true;
                }
            }
        }
        let Some(encoder) = encoder.as_mut() else { continue };
        let started = Instant::now();
        match encoder.embed(&clip) {
            Ok(embedding) => {
                log::trace!("scene embedding in {:?}", started.elapsed());
                if tx.send(Output::Clip(embedding)).is_err() {
                    break;
                }
            }
            Err(e) => log::warn!("scene embedding failed: {e:#}"),
        }
    }
}
