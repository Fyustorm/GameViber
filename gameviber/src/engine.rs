//! The engine ties everything together on its own thread: event sources,
//! force-feedback state, gamepad state, the game's sound, the active Lua mode
//! (with hot reload), the safety layer, channel -> toy routing and the Intiface
//! output. It publishes a `Shared` snapshot for the GUI and obeys `Command`s.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use gameviber_common::overlay::{CaptureRequest, Event, Gauge, Hello, OverlayState};

use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;

use crate::audio::{self, Audio, AudioHit, AudioLevels, Embedding};
pub use crate::config::SourceChoice;
use crate::config::{self, AudioSource, ModeEntry, OverlaySettings, Presets, Settings, ToySettings};
use crate::helper::client::Helper;
use crate::gamepad::{self, PadState, BUTTONS};
use crate::inputs::{self, Inputs, InputsView};
use crate::intiface::{Intiface, IntifaceStatus, Toy, ToyOutputs};
use crate::mode::rumble_events::RumbleLevels;
use crate::mode::scenes::Sense;
use crate::mode::{HudGauge, ModeEvent, ModeInfo, ModeRuntime, ParamValue};
use crate::models::{self, Model, ModelState};
use crate::overlay;
use crate::profile::Profile;
use crate::screen::zones::ZoneReader;
use crate::screen::{self, ImageScenes, ScreenLevels, ScreenView};
use crate::rumble::RumbleState;
use crate::session::{self, Player, Recorder, RecordingInfo, Senses};
use crate::source::ebpf::EbpfSource;
use crate::source::proxy::{Hide, ProxySource};
pub use crate::source::SourceHealth;
use crate::source::{EventSender, SourceEvent, SourceKind};

const TICK: Duration = Duration::from_millis(20);
/// How often user mode files are checked for changes.
const RELOAD_CHECK: Duration = Duration::from_secs(1);
/// Length of the history kept for the GUI graphs.
pub const HISTORY_SECS: f64 = 10.0;
/// "Buzz" test of a single toy from the GUI.
pub const TEST_LEVEL: f64 = 0.5;
const TEST_LENGTH: Duration = Duration::from_millis(800);
/// How often a lost gamepad is looked for again (proxy source).
const SOURCE_RETRY: Duration = Duration::from_secs(2);
/// What `Command::SaveRecent` saves: the last seconds of play.
pub const RECENT_SECS: f64 = 120.0;
/// A marked moment is saved this long after the (last) mark, to include what followed.
pub const MARK_SAVE_DELAY: f64 = 15.0;
/// The game's image is compared with the scenes this often.
const IMAGE_SCENE_STEP: f64 = 1.0;

#[derive(Debug, Clone)]
pub struct EngineOptions {
    /// Overrides (and updates) the saved source when set.
    pub source: Option<SourceChoice>,
    pub device: Option<PathBuf>,
    /// Forces hiding on (otherwise the saved setting applies).
    pub hide: bool,
    pub passthrough: bool,
    pub url: Option<String>,
    pub intiface: bool,
    pub mode: Option<String>,
    /// Preset of the startup mode to load.
    pub preset: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Command {
    SelectMode(String),
    /// Re-reads the active mode file (hot reload).
    ReloadMode,
    RefreshModes,
    /// Language of the requests to AI assistants.
    SetLanguage(String),
    /// Deletes a user mode; the default mode takes over if it was active.
    DeleteMode(String),
    SetParam(String, ParamValue),
    /// Applies a named preset of the active mode.
    LoadPreset(String),
    /// Saves the current parameter values under a name (replacing a preset of that name).
    SavePreset(String),
    DeletePreset(String),
    /// Puts every parameter back to its default.
    ResetParams,
    Resume,
    Panic,
    Rearm,
    SetCap(f64),
    SetRouting { channel: String, toys: Vec<String> },
    /// Buttons held together for the panic stop (at least two).
    SetPanicCombo(Vec<String>),
    /// Buttons held together to mark a moment that felt wrong (at least two).
    SetMarkCombo(Vec<String>),
    /// Buttons held together to capture the game's image (at least two).
    SetCaptureCombo(Vec<String>),
    SetToySettings { toy: String, settings: ToySettings },
    SetSource { source: SourceChoice, hide: bool },
    /// Intiface server address; reconnects.
    SetUrl(String),
    /// Short vibration of one toy at a 0..1 intensity (shaped by its settings), to
    /// identify it or feel its settings.
    TestToy(String, f64),
    /// First-launch setup done (or skipped).
    SetOnboarded(bool),
    SetOverlay(OverlaySettings),
    SimRumble { strong: f64, weak: f64 },
    SimButton { name: String, pressed: bool },
    /// Records the game's rumble and the player's inputs until `StopRecording`.
    StartRecording,
    StopRecording,
    /// Saves the last `RECENT_SECS` of play as a recording.
    SaveRecent,
    /// Restarts the active mode and feeds it a recording instead of the gamepad;
    /// `to_toys` false keeps the toys still (graphs only).
    Replay { path: PathBuf, to_toys: bool },
    StopReplay,
    DeleteRecording(PathBuf),
    /// Where the game's sound is captured from.
    SetAudio(AudioSource),
    /// Downloads a scene model.
    DownloadModel(Model),
    /// Whether the GUI shows the game's image (the overlay copies it meanwhile).
    WatchScreen(bool),
    /// Whether modes see the game's image.
    SetScreen(bool),
    /// Replaces the profile of the game being played (zones, inputs...).
    SaveProfile(Profile),
    /// Captures the game's current image into its profile, as an example of a
    /// scene ("" for captures to sort later).
    CaptureScene(String),
    /// The scene the capture combo files images under ("" to sort them later).
    SetCaptureScene(String),
    /// Deletes a capture of the profile (by file name).
    DeleteCapture(String),
    /// Files a capture under another scene.
    MoveCapture { file: String, scene: String },
    /// Port other programs send values and events to; 0 turns the WebSocket off.
    SetInputsPort(u16),
    Shutdown,
}

#[derive(Debug, Clone, Default)]
pub struct ModeView {
    pub id: String,
    pub info: Option<ModeInfo>,
    pub values: BTreeMap<String, ParamValue>,
    pub presets: Presets,
    pub error: Option<String>,
    pub suspended: bool,
}

/// The recording being replayed.
#[derive(Debug, Clone)]
pub struct ReplayView {
    pub info: RecordingInfo,
    /// Seconds into the recording.
    pub position: f64,
    pub to_toys: bool,
}

/// The game's sound, for the GUI.
#[derive(Debug, Clone, Default)]
pub struct AudioView {
    pub status: audio::Status,
    pub levels: Option<AudioLevels>,
    /// Engine time of the last hit, and the hit.
    pub last_hit: Option<(f64, AudioHit)>,
    pub model: ModelState,
}

/// The active mode's scenes (§6.3), for the GUI.
#[derive(Debug, Clone, Default)]
pub struct SceneView {
    /// The current scene...
    pub scene: Option<String>,
    /// ...and the average probability of each, sorted by name.
    pub scenes: Vec<(String, f64)>,
    /// Some sense compares the game with the scenes.
    pub ready: bool,
    /// Which senses the mode's scenes use, and which do compare.
    pub sound: (bool, bool),
    pub screen: (bool, bool),
    pub examples: bool,
}

#[derive(Debug, Clone)]
pub struct Sample {
    pub t: f64,
    pub strong: f64,
    pub weak: f64,
    pub channels: BTreeMap<String, f64>,
}

/// Snapshot read by the GUI.
#[derive(Debug, Clone, Default)]
pub struct Shared {
    /// Technical description of the source.
    pub source: String,
    pub source_health: SourceHealth,
    /// Names of the gamepads the source listens to.
    pub gamepads: Vec<String>,
    /// Engine time of the last rumble from a game (simulator excluded).
    pub last_rumble: Option<f64>,
    /// A real gamepad button or axis was received.
    pub buttons_seen: bool,
    pub intiface: IntifaceStatus,
    pub intiface_enabled: bool,
    pub modes: Vec<ModeEntry>,
    /// Declaration of every mode (by id), or why it does not load.
    pub catalog: BTreeMap<String, Result<ModeInfo, String>>,
    pub mode: ModeView,
    pub panic: bool,
    pub settings: Settings,
    pub history: VecDeque<Sample>,
    pub plots: BTreeMap<String, VecDeque<[f64; 2]>>,
    pub held: Vec<&'static str>,
    /// Sticks (-1..1) and triggers (0..1).
    pub axes: BTreeMap<&'static str, f64>,
    pub toy_levels: BTreeMap<String, f64>,
    /// Games currently showing the in-game overlay.
    pub overlay_clients: Vec<Hello>,
    /// Another process holds the overlay socket: games show its panel, not ours.
    pub overlay_unavailable: bool,
    /// Seconds recorded so far, while recording.
    pub recording: Option<f64>,
    pub replay: Option<ReplayView>,
    /// Saved recordings, newest first.
    pub recordings: Vec<RecordingInfo>,
    pub audio: AudioView,
    pub screen: ScreenView,
    pub scenes: SceneView,
    /// Profile of the game being played (the one showing the overlay).
    pub profile: Option<Profile>,
    pub inputs: InputsView,
    /// The scene the capture combo files images under ("": to sort).
    pub capture_scene: String,
    pub time: f64,
    pub stopped: bool,
}

pub type SharedHandle = Arc<Mutex<Shared>>;

struct ActiveMode {
    entry: ModeEntry,
    runtime: Option<ModeRuntime>,
    presets: Presets,
    error: Option<String>,
    suspended: bool,
    modified: Option<SystemTime>,
}

enum Source {
    Proxy(ProxySource),
    Ebpf(EbpfSource),
    Failed(String),
    None,
}

impl Source {
    fn status(&self) -> String {
        match self {
            Source::Proxy(p) => p.status(),
            Source::Ebpf(e) => e.status(),
            Source::Failed(e) => format!("source error: {e}"),
            Source::None => "no source (simulator only)".into(),
        }
    }

    fn health(&self) -> SourceHealth {
        match self {
            Source::Proxy(p) => p.health(),
            Source::Ebpf(e) => e.health(),
            Source::Failed(e) => SourceHealth::Failed(e.clone()),
            Source::None => SourceHealth::Off,
        }
    }

    fn gamepads(&self) -> Vec<String> {
        match self {
            Source::Proxy(p) if matches!(p.health(), SourceHealth::Failed(_)) => Vec::new(),
            Source::Proxy(p) => vec![p.gamepad().to_owned()],
            Source::Ebpf(e) => e.gamepads(),
            Source::Failed(_) | Source::None => Vec::new(),
        }
    }

    fn shutdown(self) {
        match self {
            Source::Proxy(p) => p.shutdown(),
            Source::Ebpf(e) => e.shutdown(),
            Source::Failed(_) | Source::None => {}
        }
    }

    /// A source that was selected cannot see any gamepad (failed, unplugged).
    fn lost(&self) -> bool {
        match self.health() {
            SourceHealth::Failed(_) => true,
            SourceHealth::Working => self.gamepads().is_empty(),
            SourceHealth::Off | SourceHealth::Waiting(_) => false,
        }
    }
}

struct Engine {
    opts: EngineOptions,
    shared: SharedHandle,
    settings: Settings,
    source: Source,
    source_tx: EventSender,
    helper: Arc<Helper>,
    states: HashMap<String, RumbleState>,
    sim: RumbleLevels,
    pad: PadState,
    events: Vec<ModeEvent>,
    mode: Option<ActiveMode>,
    panic: bool,
    intiface: Option<Intiface>,
    last_toys: Vec<Toy>,
    start: Instant,
    last_tick: Instant,
    last_reload_check: Instant,
    /// The gamepad is lost: outputs are held at 0 (safety layer).
    source_lost: bool,
    last_source_retry: Instant,
    /// Parameter values changed since the last save (saves are batched: sliders send many changes).
    params_dirty: bool,
    last_channels: BTreeMap<String, f64>,
    last_rumble: Option<f64>,
    buttons_seen: bool,
    recorder: Option<Recorder>,
    /// Always on: the last `RECENT_SECS` of play.
    recent: Recorder,
    /// A moment was marked: when to save the last minutes.
    mark_save: Option<f64>,
    /// Recording being replayed, and whether the toys play it.
    player: Option<(Player, bool)>,
    /// Toy being buzzed by `Command::TestToy`, at what intensity, until when.
    test: Option<(String, f64, Instant)>,
    overlay: overlay::Server,
    /// Overlay messages and when they were raised.
    overlay_events: Vec<(String, f64)>,
    /// Mode name and preset shown by the overlay, and since when.
    overlay_title: (String, Option<String>, f64),
    audio: Option<Audio>,
    audio_levels: Option<AudioLevels>,
    last_hit: Option<(f64, AudioHit)>,
    /// Text embeddings of scene descriptions, computed off the engine thread.
    scene_texts: (std_mpsc::Sender<SceneTexts>, std_mpsc::Receiver<SceneTexts>),
    /// Descriptions being encoded, per sense.
    scene_request: HashMap<Sense, Vec<(String, String)>>,
    /// Descriptions that could not be encoded (not tried again).
    scene_failed: HashMap<Sense, Vec<(String, String)>>,
    /// The GUI shows the game's image: the overlay copies it even when modes do not see it.
    screen_watch: bool,
    screen: screen::Analyzer,
    screen_levels: Option<ScreenLevels>,
    screen_view: ScreenView,
    image_scenes: ImageScenes,
    /// When the last image was submitted for its embedding.
    last_image_submit: f64,
    /// Embeddings of the profile's captures being computed: game, then (file, embedding) as they come.
    capture_job: Option<(String, std_mpsc::Receiver<(String, Embedding)>)>,
    capture_scene: String,
    /// Profile of the game being played, and the mean of its examples per scene.
    profile: Option<Profile>,
    example_centroids: Vec<(String, Embedding)>,
    zones: ZoneReader,
    inputs: Inputs,
    ticks: u64,
}

type SceneTexts = (Sense, Vec<(String, String)>, Result<Vec<Embedding>, String>);

/// Runs the engine until `Command::Shutdown` or SIGINT/SIGTERM.
pub fn run(opts: EngineOptions, shared: SharedHandle, commands: mpsc::UnboundedReceiver<Command>) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let result = runtime.block_on(run_async(opts, shared.clone(), commands));
    shared.lock().unwrap().stopped = true;
    result
}

async fn run_async(
    opts: EngineOptions,
    shared: SharedHandle,
    mut commands: mpsc::UnboundedReceiver<Command>,
) -> anyhow::Result<()> {
    let mut settings = Settings::load();
    if let Some(url) = &opts.url {
        settings.url = url.clone();
    }
    if let Some(source) = opts.source {
        settings.source = source;
    }
    if opts.hide {
        settings.hide = true;
    }
    overlay::update_installed(settings.overlay.all_games);
    let (source_tx, mut rx) = mpsc::unbounded_channel::<SourceEvent>();
    let intiface = opts.intiface.then(|| Intiface::spawn(settings.url.clone()));
    let audio = Audio::start(settings.audio.clone());
    let inputs = Inputs::start(settings.inputs_port);
    let now = Instant::now();
    let mut engine = Engine {
        opts,
        shared,
        settings,
        source: Source::None,
        source_tx,
        helper: Helper::new(),
        states: HashMap::new(),
        sim: RumbleLevels::default(),
        pad: PadState::default(),
        events: Vec::new(),
        mode: None,
        panic: false,
        intiface,
        last_toys: Vec::new(),
        start: now,
        last_tick: now,
        last_reload_check: now,
        source_lost: false,
        last_source_retry: now,
        params_dirty: false,
        last_channels: BTreeMap::new(),
        last_rumble: None,
        buttons_seen: false,
        test: None,
        recorder: None,
        recent: Recorder::rolling(0.0, RECENT_SECS),
        mark_save: None,
        player: None,
        overlay: overlay::Server::new(),
        overlay_events: Vec::new(),
        overlay_title: (String::new(), None, 0.0),
        audio: Some(audio),
        audio_levels: None,
        last_hit: None,
        scene_texts: std_mpsc::channel(),
        scene_request: HashMap::new(),
        scene_failed: HashMap::new(),
        screen_watch: false,
        screen: screen::Analyzer::default(),
        screen_levels: None,
        screen_view: ScreenView::default(),
        image_scenes: ImageScenes::start(),
        last_image_submit: f64::NEG_INFINITY,
        capture_job: None,
        capture_scene: String::new(),
        profile: None,
        example_centroids: Vec::new(),
        zones: ZoneReader::default(),
        inputs,
        ticks: 0,
    };
    engine.apply_combos();
    engine.start_source();
    engine.refresh_modes();
    engine.refresh_recordings();
    let first_mode = engine.opts.mode.clone().unwrap_or_else(|| engine.settings.active_mode.clone());
    engine.select_mode(&first_mode);
    if let Some(preset) = engine.opts.preset.clone() {
        engine.load_preset(&preset);
    }

    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    loop {
        tokio::select! {
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
            Some(ev) = rx.recv() => engine.on_source_event(ev),
            command = commands.recv() => match command {
                Some(Command::Shutdown) | None => break,
                Some(command) => engine.on_command(command),
            },
            _ = tick.tick() => engine.tick(),
        }
    }
    engine.shutdown().await;
    Ok(())
}

impl Engine {
    fn time(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn start_source(&mut self) {
        self.source = self.new_source();
        if let Source::Failed(e) = &self.source {
            log::error!("source unavailable: {e}");
        }
    }

    fn new_source(&self) -> Source {
        let opts = &self.opts;
        let tx = self.source_tx.clone();
        let root = unsafe { libc::geteuid() } == 0;
        match self.settings.source {
            SourceChoice::Proxy => {
                let hide = match (self.settings.hide, root) {
                    (false, _) => Hide::No,
                    (true, true) => Hide::Local,
                    (true, false) => Hide::Helper(self.helper.clone()),
                };
                match ProxySource::start(opts.device.as_deref(), opts.passthrough, hide, tx) {
                    Ok(s) => Source::Proxy(s),
                    Err(e) => Source::Failed(format!("{e:#}")),
                }
            }
            SourceChoice::Ebpf => match EbpfSource::start(tx, &self.helper) {
                Ok(s) => Source::Ebpf(s),
                Err(e) => Source::Failed(format!("{e:#}")),
            },
            SourceChoice::None => Source::None,
        }
    }

    /// Proxy source: once the gamepad is lost, looks for it again every
    /// `SOURCE_RETRY` (the eBPF source watches for gamepads by itself).
    fn retry_source(&mut self) {
        if self.settings.source != SourceChoice::Proxy || self.last_source_retry.elapsed() < SOURCE_RETRY {
            return;
        }
        let SourceHealth::Failed(previous) = self.source.health() else { return };
        self.last_source_retry = Instant::now();
        std::mem::replace(&mut self.source, Source::None).shutdown();
        self.source = self.new_source();
        match &self.source {
            Source::Failed(e) if *e == previous => log::debug!("gamepad still unavailable: {e}"),
            Source::Failed(e) => log::warn!("gamepad unavailable, retrying every {SOURCE_RETRY:?}: {e}"),
            _ => log::info!("gamepad found again, capture restarted"),
        }
    }

    /// Applies the saved combos, falling back to the defaults when invalid or
    /// identical to another.
    fn apply_combos(&mut self) {
        let parse = |names: &mut Vec<String>, default: [&'static str; 2], what: &str| {
            gamepad::parse_combo(names).unwrap_or_else(|| {
                log::warn!("invalid {what} combo {names:?}, using the default");
                *names = default.map(str::to_owned).to_vec();
                default.into_iter().collect()
            })
        };
        let panic = parse(&mut self.settings.panic_combo, gamepad::DEFAULT_PANIC_COMBO, "panic");
        let mut mark = parse(&mut self.settings.mark_combo, gamepad::DEFAULT_MARK_COMBO, "mark");
        if mark == panic {
            log::warn!("the mark combo is the panic combo, using the default");
            self.settings.mark_combo = gamepad::DEFAULT_MARK_COMBO.map(str::to_owned).to_vec();
            mark = gamepad::DEFAULT_MARK_COMBO.into_iter().collect();
        }
        let mut capture = parse(&mut self.settings.capture_combo, gamepad::DEFAULT_CAPTURE_COMBO, "capture");
        if capture == panic || capture == mark {
            log::warn!("the capture combo is another combo, using the default");
            self.settings.capture_combo = gamepad::DEFAULT_CAPTURE_COMBO.map(str::to_owned).to_vec();
            capture = gamepad::DEFAULT_CAPTURE_COMBO.into_iter().collect();
        }
        self.pad.set_panic_combo(panic);
        self.pad.set_mark_combo(mark);
        self.pad.set_capture_combo(capture);
    }

    /// The player marked this moment as feeling wrong: noted in the recordings,
    /// and the last minutes are saved a little later.
    fn mark_moment(&mut self, time: f64) {
        log::info!("moment marked");
        self.recent.mark(time);
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.mark(time);
        }
        self.overlay_events.push(("Moment marked".to_owned(), time));
        self.mark_save = Some(time + MARK_SAVE_DELAY);
    }

    fn switch_source(&mut self, source: SourceChoice, hide: bool) {
        if source == self.settings.source && hide == self.settings.hide && !matches!(self.source, Source::Failed(_)) {
            return;
        }
        log::info!("switching source to {source:?}{}", if hide && source == SourceChoice::Proxy { " (hidden)" } else { "" });
        std::mem::replace(&mut self.source, Source::None).shutdown();
        self.states.clear();
        self.events.extend(self.pad.release_all().into_iter().map(ModeEvent::Button));
        self.buttons_seen = false;
        self.settings.source = source;
        self.settings.hide = hide;
        self.settings.save();
        self.start_source();
    }

    fn on_source_event(&mut self, ev: SourceEvent) {
        // ebpf: --device restricts which gamepad is observed.
        let filter = self.opts.device.as_ref().filter(|_| self.settings.source == SourceChoice::Ebpf);
        if filter.is_some_and(|d| *d.to_string_lossy() != ev.device) {
            return;
        }
        let time = self.time();
        let now = Instant::now();
        // A replay drives the buttons and axes.
        if self.player.is_some() && matches!(ev.kind, SourceKind::Button { .. } | SourceKind::Axis { .. }) {
            self.buttons_seen = true;
            return;
        }
        match ev.kind {
            SourceKind::Button { code, pressed } => {
                self.buttons_seen = true;
                if let Some(b) = self.pad.key(code, pressed, time) {
                    log::debug!("button {} {}", b.name, if b.pressed { "pressed" } else { "released" });
                    self.events.push(ModeEvent::Button(b));
                }
            }
            SourceKind::Axis { code, value } => {
                self.buttons_seen = true;
                let buttons = self.pad.axis(code, value, time);
                self.events.extend(buttons.into_iter().map(ModeEvent::Button));
            }
            SourceKind::Upload { id, effect } => {
                log::debug!("{} upload id={id} {:?} length={}ms", ev.device, effect.kind, effect.length_ms);
                self.states.entry(ev.device).or_default().upload(id, effect);
            }
            SourceKind::Erase { id } => self.states.entry(ev.device).or_default().erase(id),
            SourceKind::Play { id, count } => {
                log::debug!("{} play id={id} count={count}", ev.device);
                self.states.entry(ev.device).or_default().play(id, count, now);
            }
            SourceKind::Gain(gain) => self.states.entry(ev.device).or_default().set_gain(gain),
            SourceKind::Removed => {
                log::info!("{} unplugged", ev.device);
                self.states.remove(&ev.device);
                self.events.extend(self.pad.release_all().into_iter().map(ModeEvent::Button));
            }
        }
    }

    fn on_command(&mut self, command: Command) {
        match command {
            Command::SelectMode(id) => self.select_mode(&id),
            Command::ReloadMode => self.reload_mode(),
            Command::RefreshModes => self.refresh_modes(),
            Command::SetLanguage(language) => {
                let language = language.trim();
                self.settings.language = if language.is_empty() { config::DEFAULT_LANGUAGE.into() } else { language.into() };
                self.settings.save();
            }
            Command::DeleteMode(id) => {
                if self.mode.as_ref().is_some_and(|m| m.entry.id == id) {
                    self.select_mode(config::DEFAULT_MODE);
                }
                match ModeEntry::from_id(&id).delete() {
                    Ok(()) => log::info!("mode {id} deleted"),
                    Err(e) => log::error!("cannot delete {id}: {e}"),
                }
                self.refresh_modes();
            }
            Command::SetParam(name, value) => self.set_param(&name, &value),
            Command::LoadPreset(name) => self.load_preset(&name),
            Command::SavePreset(name) => self.save_preset(&name),
            Command::DeletePreset(name) => self.delete_preset(&name),
            Command::ResetParams => self.apply_params(&BTreeMap::new(), None),
            Command::Resume => self.resume(),
            Command::Panic => self.trigger_panic("GUI"),
            Command::Rearm => {
                if self.panic {
                    log::info!("panic stop re-armed");
                    self.panic = false;
                    self.resume();
                }
            }
            Command::SetCap(cap) => {
                self.settings.global_cap = cap.clamp(0.0, 1.0);
                self.settings.save();
            }
            Command::SetRouting { channel, toys } => {
                self.settings.routing.insert(channel, toys);
                self.settings.save();
            }
            Command::SetToySettings { toy, settings } => {
                if settings == ToySettings::default() {
                    self.settings.toys.remove(&toy);
                } else {
                    self.settings.toys.insert(toy, settings);
                }
                self.settings.save();
            }
            Command::SetPanicCombo(combo) => {
                let taken = [&self.settings.mark_combo, &self.settings.capture_combo].map(|c| gamepad::parse_combo(c));
                if gamepad::parse_combo(&combo).is_some_and(|c| !taken.contains(&Some(c))) {
                    log::info!("panic combo set to {}", gamepad::combo_text(&combo));
                    self.settings.panic_combo = combo;
                    self.apply_combos();
                    self.settings.save();
                }
            }
            Command::SetCaptureCombo(combo) => {
                let taken = [&self.settings.panic_combo, &self.settings.mark_combo].map(|c| gamepad::parse_combo(c));
                if gamepad::parse_combo(&combo).is_some_and(|c| !taken.contains(&Some(c))) {
                    log::info!("capture combo set to {}", gamepad::combo_text(&combo));
                    self.settings.capture_combo = combo;
                    self.apply_combos();
                    self.settings.save();
                }
            }
            Command::SetCaptureScene(scene) => self.capture_scene = scene,
            Command::SetMarkCombo(combo) => {
                let taken = [&self.settings.panic_combo, &self.settings.capture_combo].map(|c| gamepad::parse_combo(c));
                if gamepad::parse_combo(&combo).is_some_and(|c| !taken.contains(&Some(c))) {
                    log::info!("mark combo set to {}", gamepad::combo_text(&combo));
                    self.settings.mark_combo = combo;
                    self.apply_combos();
                    self.settings.save();
                }
            }
            Command::SetSource { source, hide } => self.switch_source(source, hide),
            Command::SetUrl(url) => self.set_url(url),
            Command::TestToy(name, level) => self.test = Some((name, level.clamp(0.0, 1.0), Instant::now() + TEST_LENGTH)),
            Command::SetOverlay(overlay) => {
                self.settings.overlay = overlay;
                self.settings.save();
            }
            Command::SetOnboarded(done) => {
                self.settings.onboarded = done;
                self.settings.save();
            }
            Command::SimRumble { strong, weak } => {
                self.sim = RumbleLevels { strong: strong.clamp(0.0, 1.0), weak: weak.clamp(0.0, 1.0) }
            }
            Command::SimButton { name, pressed } => {
                let time = self.time();
                if let Some(name) = BUTTONS.iter().find(|b| **b == name) {
                    if let Some(b) = self.pad.button(name, pressed, time) {
                        self.events.push(ModeEvent::Button(b));
                    }
                }
            }
            Command::StartRecording => self.start_recording(),
            Command::StopRecording => self.stop_recording(),
            Command::SaveRecent => self.save_recent(),
            Command::Replay { path, to_toys } => self.start_replay(&path, to_toys),
            Command::StopReplay => self.stop_replay(),
            Command::DeleteRecording(path) => {
                match std::fs::remove_file(&path) {
                    Ok(()) => log::info!("recording {} deleted", path.display()),
                    Err(e) => log::error!("cannot delete {}: {e}", path.display()),
                }
                self.refresh_recordings();
            }
            Command::SetAudio(source) => {
                log::info!("game audio: {source:?}");
                if let Some(audio) = &self.audio {
                    audio.set_source(source.clone());
                }
                self.settings.audio = source;
                self.settings.save();
            }
            Command::DownloadModel(model) => model.start_download(),
            Command::WatchScreen(watch) => self.screen_watch = watch,
            Command::SetScreen(on) => {
                log::info!("modes see the game's image: {on}");
                self.settings.screen = on;
                self.settings.save();
            }
            Command::SaveProfile(profile) => {
                if self.profile.as_ref().is_some_and(|p| p.game == profile.game) {
                    profile.save();
                    self.set_profile(Some(profile));
                }
            }
            Command::CaptureScene(scene) => self.capture(scene, self.time()),
            Command::DeleteCapture(file) => self.edit_profile(|p| p.remove_capture(&file)),
            Command::MoveCapture { file, scene } => self.edit_profile(|p| {
                if let Some(capture) = p.captures.iter_mut().find(|c| c.file == file) {
                    capture.scene = scene;
                }
            }),
            Command::SetInputsPort(port) => {
                self.settings.inputs_port = port;
                self.settings.save();
                self.inputs.stop();
                self.inputs = Inputs::start(port);
            }
            Command::Shutdown => {}
        }
        self.publish_mode();
    }

    fn trigger_panic(&mut self, from: &str) {
        if !self.panic {
            log::warn!("PANIC STOP ({from}): all toys stopped, mode suspended until re-armed");
            self.panic = true;
        }
    }

    fn refresh_recordings(&mut self) {
        self.shared.lock().unwrap().recordings = session::list();
    }

    fn start_recording(&mut self) {
        if self.recorder.is_some() || self.player.is_some() {
            return;
        }
        log::info!("recording started");
        self.recorder = Some(Recorder::new(self.time(), self.mode_name(), self.game()));
    }

    fn mode_name(&self) -> String {
        self.mode.as_ref().and_then(|m| m.runtime.as_ref()).map(|rt| rt.info().name.clone()).unwrap_or_default()
    }

    /// The game showing the in-game overlay, if any.
    fn game(&self) -> Option<String> {
        self.overlay.clients().first().map(|c| c.exe.clone())
    }

    fn save_recent(&mut self) {
        let session = self.recent.session(self.time(), Some(&self.mode_name()), self.game().as_deref());
        match session.save() {
            Ok(path) => log::info!("last {:.0} s saved to {}", session.header.duration, path.display()),
            Err(e) => log::error!("cannot save the last minutes: {e}"),
        }
        self.refresh_recordings();
    }

    fn stop_recording(&mut self) {
        let Some(recorder) = self.recorder.take() else { return };
        match recorder.session(self.time(), None, None).save() {
            Ok(path) => log::info!("recording saved to {}", path.display()),
            Err(e) => log::error!("cannot save the recording: {e}"),
        }
        self.refresh_recordings();
    }

    fn start_replay(&mut self, path: &std::path::Path, to_toys: bool) {
        if self.recorder.is_some() {
            return;
        }
        let player = match Player::open(path, self.time()) {
            Ok(p) => p,
            Err(e) => {
                log::error!("cannot replay {}: {e:#}", path.display());
                return;
            }
        };
        log::info!("replaying {}{}", path.display(), if to_toys { "" } else { " (toys still)" });
        // A fresh mode, so that replaying the same session gives the same result.
        if let Some(id) = self.mode.as_ref().map(|m| m.entry.id.clone()) {
            self.select_mode(&id);
        }
        self.pad.release_all();
        self.player = Some((player, to_toys));
    }

    fn stop_replay(&mut self) {
        if self.player.take().is_some() {
            log::info!("replay stopped");
            self.events.extend(self.pad.release_all().into_iter().map(ModeEvent::Button));
        }
    }

    fn refresh_modes(&mut self) {
        let modes = config::list_modes();
        let catalog = modes
            .iter()
            .map(|entry| {
                let info = entry.source().map_err(|e| e.to_string()).and_then(|src| ModeRuntime::probe(&entry.chunk_name(), &src));
                (entry.id.clone(), info)
            })
            .collect();
        let mut shared = self.shared.lock().unwrap();
        shared.modes = modes;
        shared.catalog = catalog;
    }

    fn set_url(&mut self, url: String) {
        let url = url.trim().to_owned();
        if url.is_empty() || url == self.settings.url {
            return;
        }
        log::info!("Intiface server address set to {url}");
        self.settings.url = url.clone();
        self.settings.save();
        if let Some(old) = self.intiface.take() {
            tokio::spawn(old.shutdown());
            self.intiface = Some(Intiface::spawn(url));
        }
    }

    fn load(entry: &ModeEntry, persist: Option<&crate::mode::PersistValue>) -> Result<ModeRuntime, String> {
        let source = entry.source().map_err(|e| format!("cannot read {}: {e}", entry.id))?;
        ModeRuntime::load(&entry.chunk_name(), &source, &entry.load_params(), persist)
    }

    fn select_mode(&mut self, id: &str) {
        self.save_params();
        if let Some(mut old) = self.mode.take() {
            if let Some(rt) = old.runtime.as_mut() {
                if let Err(e) = rt.stop() {
                    log::warn!("{}: on_stop failed: {e}", old.entry.key);
                }
            }
        }
        let entry = ModeEntry::from_id(id);
        let mut active = ActiveMode {
            modified: entry.modified(),
            presets: entry.load_presets(),
            entry,
            runtime: None,
            error: None,
            suspended: false,
        };
        match Self::load(&active.entry, None) {
            Ok(mut rt) => {
                log::info!("mode '{}' loaded ({})", rt.info().name, active.entry.id);
                if let Err(e) = rt.start() {
                    active.error = Some(e);
                    active.suspended = true;
                }
                active.runtime = Some(rt);
            }
            Err(e) => {
                log::error!("mode {} failed to load: {e}", active.entry.id);
                active.error = Some(e);
            }
        }
        self.settings.active_mode = id.to_owned();
        self.settings.save();
        self.mode = Some(active);
        self.shared.lock().unwrap().plots.clear();
        self.publish_mode();
    }

    /// Hot reload: keeps the previous version running if the new one fails to load.
    fn reload_mode(&mut self) {
        let Some(active) = self.mode.as_mut() else { return };
        active.modified = active.entry.modified();
        let persist = active.runtime.as_ref().and_then(|rt| rt.persist_snapshot());
        match Self::load(&active.entry, persist.as_ref()) {
            Ok(mut rt) => {
                if let Some(old) = active.runtime.as_mut() {
                    if let Err(e) = old.stop() {
                        log::warn!("{}: on_stop failed: {e}", active.entry.key);
                    }
                }
                active.error = None;
                active.suspended = false;
                if let Err(e) = rt.start() {
                    active.error = Some(e);
                    active.suspended = true;
                }
                log::info!("mode '{}' reloaded", rt.info().name);
                active.runtime = Some(rt);
                self.refresh_modes();
            }
            Err(e) => {
                log::error!("reload of {} failed, previous version kept: {e}", active.entry.id);
                active.error = Some(format!("reload failed (previous version still running): {e}"));
            }
        }
        self.shared.lock().unwrap().plots.clear();
        self.publish_mode();
    }

    fn set_param(&mut self, name: &str, value: &ParamValue) {
        let Some(active) = self.mode.as_mut() else { return };
        let Some(rt) = active.runtime.as_mut() else { return };
        if let Err(e) = rt.set_param(name, value) {
            log::warn!("parameter {name}: {e}");
            active.error = Some(e);
        }
        self.params_dirty = true;
    }

    fn load_preset(&mut self, name: &str) {
        let Some(active) = self.mode.as_ref() else { return };
        match active.presets.presets.get(name) {
            Some(values) => self.apply_params(&values.clone(), Some(name)),
            None => log::warn!("mode {} has no preset '{name}'", active.entry.key),
        }
    }

    /// Sets every parameter from `values` (defaults for missing ones) and records `preset`
    /// as the active preset.
    fn apply_params(&mut self, values: &BTreeMap<String, ParamValue>, preset: Option<&str>) {
        let Some(active) = self.mode.as_mut() else { return };
        let Some(rt) = active.runtime.as_mut() else { return };
        if let Err(e) = rt.apply_params(values) {
            log::warn!("applying parameters: {e}");
            active.error = Some(e);
        }
        match preset {
            Some(name) => log::info!("preset '{name}' loaded"),
            None => log::info!("parameters reset to defaults"),
        }
        active.presets.active = preset.map(str::to_owned);
        active.entry.save_presets(&active.presets);
        self.params_dirty = true;
    }

    fn save_preset(&mut self, name: &str) {
        let name = name.trim();
        let Some(ActiveMode { entry, runtime: Some(rt), presets, .. }) = self.mode.as_mut() else { return };
        if name.is_empty() {
            return;
        }
        presets.presets.insert(name.to_owned(), rt.param_values().clone());
        presets.active = Some(name.to_owned());
        entry.save_presets(presets);
        log::info!("preset '{name}' saved");
    }

    fn delete_preset(&mut self, name: &str) {
        let Some(active) = self.mode.as_mut() else { return };
        if active.presets.presets.remove(name).is_none() {
            return;
        }
        if active.presets.active.as_deref() == Some(name) {
            active.presets.active = None;
        }
        active.entry.save_presets(&active.presets);
        log::info!("preset '{name}' deleted");
    }

    fn save_params(&mut self) {
        if !std::mem::take(&mut self.params_dirty) {
            return;
        }
        if let Some(ActiveMode { entry, runtime: Some(rt), .. }) = &self.mode {
            entry.save_params(rt.param_values());
        }
    }

    fn resume(&mut self) {
        let Some(active) = self.mode.as_mut() else { return };
        let Some(rt) = active.runtime.as_mut() else { return };
        active.error = None;
        active.suspended = false;
        if let Err(e) = rt.start() {
            active.error = Some(e);
            active.suspended = true;
        }
    }

    fn check_reload(&mut self) {
        if self.last_reload_check.elapsed() < RELOAD_CHECK {
            return;
        }
        self.last_reload_check = Instant::now();
        self.save_params();
        let changed = self.mode.as_ref().is_some_and(|m| !m.entry.builtin && m.entry.modified() != m.modified);
        if changed {
            self.reload_mode();
        }
    }

    fn routed(&self, channel: &str, toy: &str) -> bool {
        match self.settings.routing.get(channel) {
            Some(toys) => toys.iter().any(|t| t == toy),
            None => channel == "main",
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_tick).as_secs_f64();
        self.last_tick = now;
        let time = self.time();
        self.check_reload();
        self.retry_source();
        if self.player.as_ref().is_some_and(|(p, _)| p.finished(time)) {
            self.stop_replay();
        }
        if let Some((player, _)) = self.player.as_mut() {
            let buttons = player.advance(time, &mut self.pad);
            self.events.extend(buttons.into_iter().map(ModeEvent::Button));
        }
        let audio_levels = self.poll_audio(time);
        self.poll_screen(time);
        for message in self.inputs.poll(time) {
            // While replaying, the recording is what other programs sent.
            if self.player.is_some() {
                continue;
            }
            self.events.push(match message {
                inputs::Message::Set(name, value) => ModeEvent::Custom { name, value },
                inputs::Message::Event(name, data) => ModeEvent::External { name, data },
            });
        }
        // While replaying, the recording is the source.
        let lost = self.player.is_none() && self.source.lost();
        if lost != self.source_lost {
            self.source_lost = lost;
            if lost {
                log::warn!("gamepad lost: toys held at 0 until it is back");
                // Effects still playing would never be stopped by the game.
                self.states.clear();
                self.events.extend(self.pad.release_all().into_iter().map(ModeEvent::Button));
            } else {
                log::info!("gamepad back: toys follow the mode again");
            }
        }

        let (strong, weak) = self
            .states
            .values_mut()
            .map(|s| s.motors(now))
            .fold((0u16, 0u16), |(s, w), (s2, w2)| (s.max(s2), w.max(w2)));
        if strong > 0 || weak > 0 {
            self.last_rumble = Some(time);
        }
        let game = match &self.player {
            Some((player, _)) => player.rumble,
            None => RumbleLevels { strong: strong as f64 / 65535.0, weak: weak as f64 / 65535.0 },
        };
        let levels = RumbleLevels { strong: game.strong.max(self.sim.strong), weak: game.weak.max(self.sim.weak) };
        let buttons: Vec<_> =
            self.events.iter().filter_map(|e| if let ModeEvent::Button(b) = e { Some(b.clone()) } else { None }).collect();
        let senses = Senses { audio: audio_levels, screen: self.screen_levels };
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.tick(time, levels, &buttons, &self.pad, senses, &self.events);
            if recorder.elapsed(time) >= session::MAX_SECS {
                log::warn!("recording stopped after {:.0} min", session::MAX_SECS / 60.0);
                self.stop_recording();
            }
        }
        if self.player.is_none() {
            self.recent.tick(time, levels, &buttons, &self.pad, senses, &self.events);
        }
        if self.pad.panic_combo(time) {
            self.trigger_panic(&gamepad::combo_text(&self.settings.panic_combo));
        }
        if self.pad.take_capture(time) && self.player.is_none() {
            self.capture(self.capture_scene.clone(), time);
        }
        if self.pad.take_mark(time) && self.player.is_none() {
            self.mark_moment(time);
        }
        if self.mark_save.is_some_and(|at| time >= at) {
            self.mark_save = None;
            self.save_recent();
        }

        let toys = self.intiface.as_ref().map(|i| i.status()).unwrap_or_default();
        for toy in &toys.toys {
            if !self.last_toys.iter().any(|t| t.name == toy.name) {
                self.events.push(ModeEvent::Device { connected: true, name: toy.name.clone() });
            }
        }
        for toy in &self.last_toys {
            if !toys.toys.iter().any(|t| t.name == toy.name) {
                self.events.push(ModeEvent::Device { connected: false, name: toy.name.clone() });
                self.overlay_events.push((format!("Toy lost: {}", toy.name), time));
            }
        }
        self.last_toys = toys.toys.clone();

        let mut channels = BTreeMap::new();
        let mut plots = Vec::new();
        let mut hud = Vec::new();
        let events = std::mem::take(&mut self.events);
        let input_idle = self.pad.input_idle(time);
        self.update_scenes();
        if let Some(active) = self.mode.as_mut() {
            if let Some(rt) = active.runtime.as_mut() {
                rt.set_audio(audio_levels);
                rt.set_screen(self.screen_levels);
                if !active.suspended && !self.panic {
                    match rt.step(dt, levels, &self.pad, input_idle, &events) {
                        Ok(out) => {
                            channels = out.channels;
                            plots = out.plots;
                            hud = out.hud;
                            self.overlay_events.extend(out.hud_events.into_iter().map(|e| (e, time)));
                        }
                        Err(e) => {
                            log::error!("mode '{}' suspended: {e}", rt.info().name);
                            active.error = Some(e);
                            active.suspended = true;
                            self.publish_mode();
                        }
                    }
                }
            }
        }
        // Safety layer: panic / suspension output 0 (channels stay empty), lost gamepad 0,
        // per-toy shaping, then the global cap. The mode keeps running so that its state
        // follows the game.
        let cap = self.settings.global_cap;
        if self.source_lost {
            channels.values_mut().for_each(|v| *v = 0.0);
        }
        if channels != self.last_channels {
            let text: Vec<String> = channels.iter().map(|(c, v)| format!("{c}={v:.2}")).collect();
            log::debug!("output {} (rumble {:.2}/{:.2})", text.join(" "), levels.strong, levels.weak);
            self.last_channels = channels.clone();
        }

        if self.test.as_ref().is_some_and(|(_, _, until)| now >= *until) {
            self.test = None;
        }
        let mut toy_outputs = ToyOutputs::new();
        let mut toy_levels = BTreeMap::new();
        let still = self.player.as_ref().is_some_and(|(_, to_toys)| !to_toys);
        for toy in &toys.toys {
            let mut level = channels.iter().filter(|(c, _)| !still && self.routed(c, &toy.name)).map(|(_, v)| *v).fold(0.0, f64::max);
            if let Some((_, test, _)) = self.test.as_ref().filter(|(name, ..)| !self.panic && *name == toy.name) {
                level = level.max(*test);
            }
            let shape = self.settings.toys.get(&toy.name).copied().unwrap_or_default();
            let level = shape.shape(level).min(cap);
            toy_outputs.insert(toy.index, level);
            toy_levels.insert(toy.name.clone(), level);
        }
        if let Some(i) = &self.intiface {
            i.set_outputs(toy_outputs);
        }
        // The graphs show what the mode asks for, within the cap.
        channels.values_mut().for_each(|v| *v = v.min(cap));
        let output = toy_levels.values().copied().fold(0.0, f64::max);
        self.ticks += 1;
        // 25 updates per second are plenty for the overlay.
        if self.ticks % 2 == 0 {
            let state = self.overlay_state(time, output, hud, &toys);
            self.overlay.update(&state);
        }

        let mut shared = self.shared.lock().unwrap();
        shared.time = time;
        shared.panic = self.panic;
        shared.source = self.source.status();
        shared.source_health = self.source.health();
        shared.gamepads = self.source.gamepads();
        shared.last_rumble = self.last_rumble;
        shared.buttons_seen = self.buttons_seen;
        shared.intiface = toys;
        shared.intiface_enabled = self.intiface.is_some();
        shared.settings = self.settings.clone();
        shared.held = self.pad.held().iter().copied().collect();
        shared.axes = self.pad.axes().clone();
        shared.toy_levels = toy_levels;
        shared.recording = self.recorder.as_ref().map(|r| r.elapsed(time));
        shared.replay = self.player.as_ref().map(|(p, to_toys)| ReplayView {
            info: p.info.clone(),
            position: p.position(time),
            to_toys: *to_toys,
        });
        shared.overlay_clients = self.overlay.clients();
        shared.audio = self.audio_view(audio_levels);
        shared.screen = self.screen_view.clone();
        shared.scenes = self.scene_view();
        shared.profile = self.profile.clone();
        shared.inputs = self.inputs.view();
        shared.capture_scene = self.capture_scene.clone();
        shared.overlay_unavailable = self.overlay.unavailable();
        shared.history.push_back(Sample { t: time, strong: levels.strong, weak: levels.weak, channels });
        while shared.history.front().is_some_and(|s| time - s.t > HISTORY_SECS) {
            shared.history.pop_front();
        }
        for (name, value) in plots {
            shared.plots.entry(name).or_default().push_back([time, value]);
        }
        for series in shared.plots.values_mut() {
            while series.front().is_some_and(|p| time - p[0] > HISTORY_SECS) {
                series.pop_front();
            }
        }
    }

    /// What the in-game overlay shows.
    fn overlay_state(&mut self, time: f64, output: f64, hud: Vec<HudGauge>, toys: &IntifaceStatus) -> OverlayState {
        const EVENT_KEEP_SECS: f64 = 3.0;
        let s = &self.settings.overlay;
        let (name, preset) = match &self.mode {
            Some(active) => (
                active.runtime.as_ref().map(|rt| rt.info().name.clone()).unwrap_or_else(|| active.entry.key.clone()),
                active.presets.active.clone(),
            ),
            None => (String::new(), None),
        };
        if (&name, &preset) != (&self.overlay_title.0, &self.overlay_title.1) {
            self.overlay_title = (name.clone(), preset.clone(), time);
        }
        self.overlay_events.retain(|(_, t)| time - t < EVENT_KEEP_SECS);

        let mut alerts = Vec::new();
        if self.mode.as_ref().is_some_and(|m| m.suspended || m.runtime.is_none()) {
            alerts.push("Mode error: see GameViber".to_owned());
        }
        if self.source_lost {
            alerts.push(match self.settings.source {
                SourceChoice::Ebpf if matches!(self.source.health(), SourceHealth::Failed(_)) => "Gamepad capture stopped",
                _ => "No gamepad: toys stopped",
            }.to_owned());
        }
        if self.intiface.is_some() && !toys.connected {
            alerts.push("Intiface Central not connected".to_owned());
        } else if self.intiface.is_some() && toys.toys.is_empty() {
            alerts.push("No toy connected".to_owned());
        }

        OverlayState {
            visible: s.visible,
            corner: s.corner,
            scale: s.scale,
            opacity: s.opacity,
            output: output as f32,
            cap: self.settings.global_cap as f32,
            panic: self.panic,
            mode: name,
            preset,
            mode_age: (time - self.overlay_title.2) as f32,
            scene: self.mode.as_ref().and_then(|m| m.runtime.as_ref()).and_then(|rt| rt.scene_state().0),
            gauges: hud.into_iter().map(|g| Gauge { label: g.label, value: g.value as f32, max: g.max as f32 }).collect(),
            events: self.overlay_events.iter().map(|(text, t)| Event { text: text.clone(), age: (time - t) as f32 }).collect(),
            alerts,
            capture: (self.settings.screen || self.screen_watch).then(CaptureRequest::default),
            ..OverlayState::default()
        }
    }

    /// Reads the newest copy of the game's image: measures, flashes, zones,
    /// and an embedding now and then when scenes or examples need one.
    fn poll_screen(&mut self, time: f64) {
        const STALE_SECS: f64 = 2.0;
        let game = self.screen_view.game.clone().or_else(|| self.game());
        if self.profile.as_ref().map(|p| &p.game) != game.as_ref() {
            self.set_profile(game.map(|g| Profile::load(&g)));
        }
        let wanted = self.settings.screen || self.screen_watch;
        let replaying = self.player.is_some();
        if let Some((hello, frame)) = self.overlay.frame().filter(|_| wanted) {
            let frame = Arc::new(frame);
            let (levels, flash) = self.screen.push(time, &frame);
            if self.settings.screen && !replaying {
                self.screen_levels = Some(levels);
                if let Some(strength) = flash {
                    self.events.push(ModeEvent::ScreenFlash(strength));
                }
                if let Some(profile) = &self.profile {
                    for (name, value) in self.zones.update(&profile.zones, &frame) {
                        self.events.push(ModeEvent::Zone { name, value });
                    }
                }
            } else if let Some(profile) = &self.profile {
                // Zones are still measured for the editor.
                self.zones.update(&profile.zones, &frame);
            }
            let rt = self.mode.as_ref().and_then(|m| m.runtime.as_ref());
            let scenes_use_image = self.settings.screen && rt.is_some_and(|rt| rt.scene_sense(Sense::Screen) || rt.scene_sense(Sense::Examples));
            if scenes_use_image && Model::Image.ready() && time - self.last_image_submit >= IMAGE_SCENE_STEP {
                self.last_image_submit = time;
                self.image_scenes.submit(frame.clone());
            }
            self.screen_view.levels = Some(levels);
            self.screen_view.game = Some(hello.exe);
            self.screen_view.frame = Some(frame);
            self.screen_view.rate = self.screen.rate();
        } else if !wanted || self.screen.last().is_some_and(|t| time - t > STALE_SECS) {
            // Nobody wants it, or the game stopped sending (closed, or paused rendering).
            if self.screen.last().is_some() || self.screen_levels.is_some() {
                self.screen.reset();
                self.screen_levels = None;
                self.zones.clear();
                self.screen_view = ScreenView::default();
            }
        }
        if let Some((player, _)) = &self.player {
            // The recording is the image (its events come with the sound's).
            self.screen_levels = player.screen;
        }
        for image in self.image_scenes.poll() {
            if self.screen_levels.is_some() && !replaying {
                self.events.push(ModeEvent::ScreenClip(image));
            }
        }
        self.embed_captures();
        self.screen_view.model = Model::Image.state();
        // One entry per zone, whatever the number of places it is drawn in.
        let mut names: Vec<&String> = self.profile.iter().flat_map(|p| &p.zones).map(|z| &z.name).collect();
        names.dedup();
        names.sort();
        names.dedup();
        self.screen_view.zones = names
            .into_iter()
            .map(|n| (n.clone(), self.zones.measures.get(n).copied().flatten(), self.zones.values().get(n).copied()))
            .collect();
    }

    /// Captures the game's current image into its profile under `scene` ("": to
    /// sort), and says so in the in-game overlay.
    fn capture(&mut self, scene: String, time: f64) {
        let (Some(profile), Some(frame)) = (&mut self.profile, &self.screen_view.frame) else {
            log::warn!("no image of the game to capture");
            self.overlay_events.push(("No image to capture".to_owned(), time));
            return;
        };
        match profile.add_capture(&scene, frame) {
            Ok(()) => {
                profile.save();
                let count = profile.captures.iter().filter(|c| c.scene == scene).count();
                let what = if scene.is_empty() { "to sort".to_owned() } else { scene.clone() };
                log::info!("capture ({what}) added to the profile of {}", profile.game);
                self.overlay_events.push((format!("📸 Captured: {what} ({count})"), time));
                let profile = profile.clone();
                self.set_profile(Some(profile));
            }
            Err(e) => log::error!("cannot save the capture: {e}"),
        }
    }

    fn edit_profile(&mut self, edit: impl FnOnce(&mut Profile)) {
        if let Some(mut profile) = self.profile.clone() {
            edit(&mut profile);
            profile.save();
            self.set_profile(Some(profile));
        }
    }

    /// Computes the embeddings of the profile's captures that have none, in a
    /// thread of its own, once the image model is downloaded.
    fn embed_captures(&mut self) {
        if let Some((game, rx)) = &self.capture_job {
            let mut results = Vec::new();
            let done = loop {
                match rx.try_recv() {
                    Ok(result) => results.push(result),
                    Err(std_mpsc::TryRecvError::Empty) => break false,
                    Err(std_mpsc::TryRecvError::Disconnected) => break true,
                }
            };
            if self.profile.as_ref().is_some_and(|p| p.game == *game) && !results.is_empty() {
                self.edit_profile(|p| {
                    for (file, embedding) in results {
                        if let Some(capture) = p.captures.iter_mut().find(|c| c.file == file) {
                            capture.embedding = embedding.to_vec();
                        }
                    }
                });
            }
            if done {
                self.capture_job = None;
            }
            return;
        }
        let Some(profile) = &self.profile else { return };
        let missing: Vec<String> = profile.captures.iter().filter(|c| c.embedding.is_empty()).map(|c| c.file.clone()).collect();
        if missing.is_empty() || !Model::Image.ready() {
            return;
        }
        let game = profile.game.clone();
        let (tx, rx) = std_mpsc::channel();
        self.capture_job = Some((game.clone(), rx));
        std::thread::spawn(move || {
            let mut encoder = match screen::clip::ImageEncoder::load() {
                Ok(e) => e,
                Err(e) => return log::error!("cannot load the image scene model: {e:#}"),
            };
            for file in missing {
                let embedded = crate::profile::load_capture(&game, &file).map(|frame| encoder.embed(&frame));
                match embedded {
                    Some(Ok(embedding)) => {
                        if tx.send((file, embedding)).is_err() {
                            return;
                        }
                    }
                    Some(Err(e)) => log::warn!("cannot analyse the capture {file}: {e:#}"),
                    None => log::warn!("cannot read the capture {file}"),
                }
            }
        });
    }

    /// The game being played changed, or its profile was edited.
    fn set_profile(&mut self, profile: Option<Profile>) {
        if self.profile.as_ref().map(|p| &p.game) != profile.as_ref().map(|p| &p.game) {
            if let Some(p) = &profile {
                log::info!("game profile: {} ({} zones)", p.game, p.zones.len());
            }
            self.zones.clear();
        }
        self.example_centroids = profile.as_ref().map(Profile::example_centroids).unwrap_or_default();
        self.profile = profile;
        // The active mode compares with the new examples.
        if let Some(rt) = self.mode.as_mut().and_then(|m| m.runtime.as_mut()) {
            rt.set_scene_references(Sense::Examples, self.example_centroids.clone());
        }
    }

    /// Drains the audio service. While replaying, the recording is the sound.
    fn poll_audio(&mut self, time: f64) -> Option<AudioLevels> {
        let games = self.overlay.clients().into_iter().map(|c| (c.pid, c.exe)).collect();
        let outputs = match &self.audio {
            Some(audio) => {
                audio.set_games(games);
                audio.poll()
            }
            None => Vec::new(),
        };
        if let Some((player, _)) = self.player.as_mut() {
            // The recording's sound, image and values from other programs.
            for event in player.take_events() {
                if let ModeEvent::AudioHit(hit) = &event {
                    self.last_hit = Some((time, *hit));
                }
                self.events.push(event);
            }
            return player.audio;
        }
        for output in outputs {
            match output {
                audio::Output::Levels(levels) => self.audio_levels = Some(levels),
                audio::Output::Hit(hit) => {
                    self.last_hit = Some((time, hit));
                    self.events.push(ModeEvent::AudioHit(hit));
                }
                audio::Output::Clip(clip) => self.events.push(ModeEvent::AudioClip(clip)),
                audio::Output::Inactive => self.audio_levels = None,
            }
        }
        self.audio_levels
    }

    /// Gets the active mode's scene descriptions encoded for each sense whose
    /// model is downloaded (off the engine thread: a text model takes a moment
    /// to load), hands it the profile's examples, and asks the audio service
    /// for embeddings only while the mode can use them.
    fn update_scenes(&mut self) {
        while let Ok((sense, descriptions, result)) = self.scene_texts.1.try_recv() {
            self.scene_request.remove(&sense);
            match result {
                Ok(texts) => {
                    let rt = self.mode.as_mut().and_then(|m| m.runtime.as_mut());
                    if let Some(rt) = rt.filter(|rt| rt.scene_descriptions(sense) == descriptions) {
                        log::info!("{sense:?} scenes of '{}' ready", rt.info().name);
                        rt.set_scene_references(sense, descriptions.into_iter().map(|(name, _)| name).zip(texts).collect());
                    }
                }
                Err(e) => {
                    log::error!("cannot prepare the {sense:?} scenes: {e}");
                    self.scene_failed.insert(sense, descriptions);
                }
            }
        }
        let Some(rt) = self.mode.as_mut().and_then(|m| m.runtime.as_mut()) else {
            if let Some(audio) = &self.audio {
                audio.set_scenes(false);
            }
            return;
        };
        if !rt.scene_sense(Sense::Examples) && !self.example_centroids.is_empty() && rt.info().uses_screen_scenes() {
            rt.set_scene_references(Sense::Examples, self.example_centroids.clone());
        }
        if let Some(audio) = &self.audio {
            audio.set_scenes(rt.info().uses_sound_scenes() && Model::Sound.ready());
        }
        for (sense, model) in [(Sense::Sound, Model::Sound), (Sense::Screen, Model::Image)] {
            let descriptions = rt.scene_descriptions(sense);
            if descriptions.is_empty() || !model.ready() || rt.scene_sense(sense) {
                continue;
            }
            if self.scene_request.contains_key(&sense) || self.scene_failed.get(&sense) == Some(&descriptions) {
                continue;
            }
            self.scene_request.insert(sense, descriptions.clone());
            let tx = self.scene_texts.0.clone();
            std::thread::spawn(move || {
                let texts: Vec<String> = descriptions.iter().map(|(_, d)| d.clone()).collect();
                let result = models::text_embeddings(model, &texts).map_err(|e| format!("{e:#}"));
                let _ = tx.send((sense, descriptions, result));
            });
        }
    }

    fn audio_view(&self, levels: Option<AudioLevels>) -> AudioView {
        AudioView {
            status: self.audio.as_ref().map(Audio::status).unwrap_or_default(),
            levels,
            last_hit: self.last_hit,
            model: Model::Sound.state(),
        }
    }

    fn scene_view(&self) -> SceneView {
        let Some(rt) = self.mode.as_ref().and_then(|m| m.runtime.as_ref()).filter(|rt| !rt.info().scenes.is_empty()) else {
            return SceneView::default();
        };
        let (scene, scenes) = rt.scene_state();
        SceneView {
            scene,
            scenes,
            ready: rt.scenes_ready(),
            sound: (rt.info().uses_sound_scenes(), rt.scene_sense(Sense::Sound)),
            screen: (rt.info().uses_screen_scenes(), rt.scene_sense(Sense::Screen)),
            examples: rt.scene_sense(Sense::Examples),
        }
    }

    fn publish_mode(&self) {
        let view = match &self.mode {
            Some(active) => ModeView {
                id: active.entry.id.clone(),
                info: active.runtime.as_ref().map(|rt| rt.info().clone()),
                values: active.runtime.as_ref().map(|rt| rt.param_values().clone()).unwrap_or_default(),
                presets: active.presets.clone(),
                error: active.error.clone(),
                suspended: active.suspended,
            },
            None => ModeView::default(),
        };
        self.shared.lock().unwrap().mode = view;
    }

    async fn shutdown(mut self) {
        log::info!("shutting down");
        self.stop_recording();
        if self.mark_save.take().is_some() {
            self.save_recent();
        }
        self.save_params();
        if let Some(active) = self.mode.as_mut() {
            if let Some(rt) = active.runtime.as_mut() {
                let _ = rt.stop();
            }
        }
        self.inputs.stop();
        if let Some(i) = self.intiface.take() {
            i.shutdown().await;
        }
        std::mem::replace(&mut self.source, Source::None).shutdown();
        if let Some(audio) = self.audio.take() {
            audio.shutdown();
        }
        self.helper.shutdown();
    }
}
