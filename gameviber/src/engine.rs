//! The engine ties everything together on its own thread: event sources,
//! force-feedback state, gamepad state, the game's sound, the active Lua mode
//! (with hot reload), the safety layer, channel -> toy routing and the Intiface
//! output. It publishes a `Shared` snapshot for the GUI and obeys `Command`s.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use gameviber_common::overlay::{CaptureRequest, Event, Gauge, Hello, OverlayState};

use tokio::sync::mpsc;

use crate::audio::{self, Audio, AudioHit, AudioLevels, Embedding};
pub use crate::config::SourceChoice;
use crate::config::{self, AudioSource, ModeEntry, OverlaySettings, Presets, Settings, ToySettings};
use crate::gamepad::{self, PadState, BUTTONS};
use crate::external::{self, ExternalInputs, ExternalView};
use crate::intiface::{self, Intiface, IntifaceStatus, Toy, ToyOutputs};
use crate::mode::rumble_events::RumbleLevels;
use crate::mode::phases::Sense;
use crate::mode::{HudGauge, ModeEvent, ModeInfo, ModeRuntime, ParamValue};
use crate::models::{self, Model, ModelState};
use crate::overlay;
use crate::platform;
use crate::game::Game;
use crate::package;
use crate::screen::indicators::IndicatorReader;
use crate::shortcuts::{Action, Shortcuts};
use crate::screen::{self, Frame, ImagePhases, ScreenLevels, ScreenView};
use crate::rumble::RumbleState;
use crate::session::{self, Change, Recorder, RecordingInfo, Senses};
pub use crate::source::SourceHealth;
use crate::source::{ActiveSource, EventSender, PadInfo, SourceEvent, SourceKind, SourceOptions, Sources};
/// Play with a mode in one go that counts as a session of it (community stats).
const SESSION_SECS: f64 = 120.0;

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
/// The game's image is compared with the phases this often.
pub const IMAGE_PHASE_STEP: f64 = 1.0;

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
    /// Play time with the modes installed from the community and votes are sent to it.
    SetShareStats(bool),
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
    /// The player's mapping of a gamepad's buttons (`gamepad::mapping`); the source restarts with it.
    SaveMapping(gamepad::mapping::Mapping),
    /// Forgets the player's mapping of a gamepad (by SDL GUID); the source restarts.
    ForgetMapping(String),
    /// Intiface server address; reconnects.
    SetUrl(String),
    /// Disconnects from Intiface, reconnects, starts or stops its scan for toys.
    Intiface(intiface::Control),
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
    /// Images of the game recorded sessions keep per second.
    SetRecordingImages(f64),
    /// Saves the last `RECENT_SECS` of play as a recording.
    SaveRecent,
    /// The toys play what a mode sent during a recording (the session player's
    /// replay, `outputs`: its channels every step), from `from` seconds at
    /// `speed`, instead of what the active mode sends.
    PlaySession { path: PathBuf, outputs: SessionOutputs, from: f64, speed: f64 },
    StopSession,
    DeleteRecording(PathBuf),
    /// Where the game's sound is captured from.
    SetAudio(AudioSource),
    /// Downloads a phase model.
    DownloadModel(Model),
    /// Whether the GUI shows the game's image (the overlay copies it meanwhile).
    WatchScreen(bool),
    /// Whether modes see the game's image.
    SetScreen(bool),
    /// Adds a game, linked to an executable or not, and makes it the active game.
    CreateGame { name: String, executable: Option<String> },
    /// Replaces a game (its name, executables, modes, sound).
    SaveGame(Game),
    DeleteGame(String),
    /// The game being played, by id; None: no game.
    SelectGame(Option<String>),
    /// The executable now runs as this game (and no other).
    LinkExecutable { game: String, executable: String },
    /// A mode and its game were imported (`sharing`): the modes and games are read again.
    Imported { game: String },
    /// Adds a mode to a game's modes, or takes it out.
    AddGameMode { game: String, mode: String },
    RemoveGameMode { game: String, mode: String },
    /// Captures the game's current image into the active game, as an example
    /// of a phase ("" for captures to sort later).
    CapturePhase(String),
    /// The phase the capture combo files images under ("" to sort them later).
    SetCapturePhase(String),
    /// Replaces the inputs a mode reads (phases, indicators, values from other
    /// programs), in its package (`Inputs::dir`); its captures change through their own commands.
    SaveInputs(package::Inputs),
    /// Adds images (from files) to the captures of a mode (by package), under a phase ("" to sort them later).
    AddCaptures { dir: PathBuf, phase: String, frames: Vec<Frame> },
    /// Deletes a capture of a mode (by file name).
    DeleteCapture { dir: PathBuf, file: String },
    /// Files a capture under another phase.
    MoveCapture { dir: PathBuf, file: String, phase: String },
    /// Keyboard shortcuts for the combos' actions (through the desktop), on or off.
    SetKeyboardShortcuts(bool),
    /// Opens the desktop's settings of the keyboard shortcuts.
    ConfigureShortcuts,
    /// New versions are looked for automatically, or not.
    SetCheckUpdates(bool),
    /// Port other programs send values and events to; 0 turns the WebSocket off.
    SetExternalPort(u16),
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

/// What a mode sent during a recording: its channels, by seconds into it.
pub type SessionOutputs = Arc<Vec<(f64, BTreeMap<String, f64>)>>;

/// The recording the toys play.
#[derive(Debug, Clone)]
pub struct ReplayView {
    pub path: PathBuf,
    /// Seconds into the recording.
    pub position: f64,
}

/// The toys play a recording (`Command::PlaySession`).
struct Replay {
    path: PathBuf,
    outputs: SessionOutputs,
    from: f64,
    speed: f64,
    /// Engine time it started at.
    start: f64,
}

impl Replay {
    fn position(&self, time: f64) -> f64 {
        self.from + (time - self.start) * self.speed
    }

    fn finished(&self, time: f64) -> bool {
        self.outputs.last().is_none_or(|(t, _)| self.position(time) > *t)
    }

    /// The channels at `time`.
    fn channels(&self, time: f64) -> BTreeMap<String, f64> {
        let position = self.position(time);
        let i = self.outputs.partition_point(|(t, _)| *t <= position);
        i.checked_sub(1).map(|i| self.outputs[i].1.clone()).unwrap_or_default()
    }
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

/// The active mode's phases (§6.3), for the GUI.
#[derive(Debug, Clone, Default)]
pub struct PhaseView {
    /// The current phase...
    pub phase: Option<String>,
    /// ...and the average probability of each, sorted by name.
    pub phases: Vec<(String, f64)>,
    /// Which senses the mode's phases use, and which do compare.
    pub sound: (bool, bool),
    pub screen: (bool, bool),
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
    /// The gamepad whose buttons can be set up (proxy).
    pub pad: Option<PadInfo>,
    /// What players should know about their gamepads (one that cannot vibrate...).
    pub source_hint: Option<String>,
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
    pub phases: PhaseView,
    /// Every game, and the one being played.
    pub games: Vec<Game>,
    pub game: Option<Game>,
    /// The inputs the active mode reads (capture embeddings left out); None
    /// for a mode without a package (built-in).
    pub mode_inputs: Option<package::Inputs>,
    /// The executable showing the overlay, and whether no game runs as it.
    pub running_executable: Option<String>,
    /// Its Steam app id, when Steam started it.
    pub running_app: Option<u32>,
    pub unlinked_executable: Option<String>,
    pub external: ExternalView,
    pub shortcuts: crate::shortcuts::Status,
    /// The phase the capture combo files images under ("": to sort).
    pub capture_phase: String,
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
    Running(Box<dyn ActiveSource>),
    Failed(String),
    None,
}

impl Source {
    fn status(&self) -> String {
        match self {
            Source::Running(s) => s.status(),
            Source::Failed(e) => format!("source error: {e}"),
            Source::None => "no source (simulator only)".into(),
        }
    }

    fn health(&self) -> SourceHealth {
        match self {
            Source::Running(s) => s.health(),
            Source::Failed(e) => SourceHealth::Failed(e.clone()),
            Source::None => SourceHealth::Off,
        }
    }

    fn gamepads(&self) -> Vec<String> {
        match self {
            Source::Running(s) => s.gamepads(),
            Source::Failed(_) | Source::None => Vec::new(),
        }
    }

    fn pad(&self) -> Option<PadInfo> {
        match self {
            Source::Running(s) => s.pad(),
            Source::Failed(_) | Source::None => None,
        }
    }

    fn hint(&self) -> Option<String> {
        match self {
            Source::Running(s) => s.hint(),
            Source::Failed(_) | Source::None => None,
        }
    }

    fn shutdown(self) {
        if let Source::Running(s) = self {
            s.shutdown();
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
    sources: Sources,
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
    /// Recording the toys play.
    replay: Option<Replay>,
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
    /// Text embeddings of phase descriptions, computed off the engine thread.
    phase_texts: (std_mpsc::Sender<PhaseTexts>, std_mpsc::Receiver<PhaseTexts>),
    /// Descriptions being encoded, per sense.
    phase_request: HashMap<Sense, Vec<(String, String)>>,
    /// Descriptions that could not be encoded (not tried again).
    phase_failed: HashMap<Sense, Vec<(String, String)>>,
    /// The GUI shows the game's image: the overlay copies it even when modes do not see it.
    screen_watch: bool,
    screen: screen::Analyzer,
    screen_levels: Option<ScreenLevels>,
    screen_view: ScreenView,
    image_phases: ImagePhases,
    /// When the last image was submitted for its embedding.
    last_image_submit: f64,
    /// Embeddings of the active mode's captures being computed: its package, then (file, embedding) as they come.
    capture_job: Option<(std::path::PathBuf, std_mpsc::Receiver<(String, Embedding)>)>,
    capture_phase: String,
    /// Every game, the one being played, the inputs of the active mode and the mean of its captures per phase.
    games: Vec<Game>,
    game: Option<Game>,
    mode_inputs: package::Inputs,
    /// The executable showing the overlay, as last seen.
    running: Option<String>,
    /// Its Steam app id, when Steam started it.
    running_app: Option<u32>,
    example_centroids: Vec<(String, Embedding)>,
    indicators: IndicatorReader,
    external: ExternalInputs,
    shortcuts: Option<Shortcuts>,
    ticks: u64,
    /// Seconds a game ran with the active mode, not yet counted (`community::record_play`)...
    played: f64,
    /// ...how long it has run with it in one go, and whether that made a session (2 minutes)...
    session: (String, f64, bool),
    /// ...and what is sent to the community when the player shares their stats.
    plays: crate::community::PlayReport,
}

type PhaseTexts = (Sense, Vec<(String, String)>, Result<Vec<Embedding>, String>);

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
    if settings.installation_id.is_empty() {
        settings.installation_id = crate::community::new_installation_id();
        settings.save();
    }
    let shortcuts = settings.keyboard_shortcuts.then(Shortcuts::start);
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
    let external = ExternalInputs::start(settings.external_port);
    let now = Instant::now();
    let mut engine = Engine {
        opts,
        shared,
        settings,
        source: Source::None,
        source_tx,
        sources: Sources::new(),
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
        replay: None,
        overlay: overlay::Server::new(),
        overlay_events: Vec::new(),
        overlay_title: (String::new(), None, 0.0),
        audio: Some(audio),
        audio_levels: None,
        last_hit: None,
        phase_texts: std_mpsc::channel(),
        phase_request: HashMap::new(),
        phase_failed: HashMap::new(),
        screen_watch: false,
        screen: screen::Analyzer::default(),
        screen_levels: None,
        screen_view: ScreenView::default(),
        image_phases: ImagePhases::start(),
        last_image_submit: f64::NEG_INFINITY,
        capture_job: None,
        capture_phase: String::new(),
        games: Game::list(),
        game: None,
        mode_inputs: package::Inputs::default(),
        running: None,
        running_app: None,
        example_centroids: Vec::new(),
        indicators: IndicatorReader::default(),
        external,
        shortcuts,
        ticks: 0,
        played: 0.0,
        session: (String::new(), 0.0, false),
        plays: Default::default(),
    };
    engine.apply_combos();
    engine.start_source();
    engine.forget_missing_modes();
    engine.refresh_modes();
    engine.refresh_recordings();
    let first_mode = engine.opts.mode.clone().unwrap_or_else(|| engine.settings.active_mode.clone());
    engine.select_mode(&first_mode);
    let last_game = engine.settings.active_game.clone();
    engine.select_game(last_game);
    if let Some(preset) = engine.opts.preset.clone() {
        engine.load_preset(&preset);
    }

    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut stop = platform::StopSignals::new()?;
    loop {
        tokio::select! {
            _ = stop.recv() => break,
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
        if self.settings.source == SourceChoice::None {
            return Source::None;
        }
        let opts = SourceOptions { device: self.opts.device.clone(), passthrough: self.opts.passthrough, hide: self.settings.hide };
        match self.sources.start(self.settings.source, &opts, self.source_tx.clone()) {
            Ok(s) => Source::Running(s),
            Err(e) => Source::Failed(format!("{e:#}")),
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
        gamepad::reserve_extras([&panic, &mark, &capture]);
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
        self.settings.source = source;
        self.settings.hide = hide;
        self.settings.save();
        self.restart_source();
    }

    /// Starts the source over (a gamepad's mapping changed, another source).
    fn restart_source(&mut self) {
        std::mem::replace(&mut self.source, Source::None).shutdown();
        self.states.clear();
        self.events.extend(self.pad.release_all().into_iter().map(ModeEvent::Button));
        self.buttons_seen = false;
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
            Command::SetShareStats(on) => {
                log::info!("community stats shared: {on}");
                self.settings.share_stats = Some(on);
                self.settings.save();
            }
            Command::DeleteMode(id) => {
                if self.mode.as_ref().is_some_and(|m| m.entry.id == id) {
                    // A variant gives way to its mode.
                    let entry = ModeEntry::from_id(&id);
                    let next = if entry.variant.is_some() { entry.main_id() } else { config::DEFAULT_MODE.to_owned() };
                    self.select_mode(&next);
                }
                match ModeEntry::from_id(&id).delete() {
                    Ok(()) => log::info!("mode {id} deleted"),
                    Err(e) => log::error!("cannot delete {id}: {e}"),
                }
                self.forget_missing_modes();
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
            Command::SetCapturePhase(phase) => self.capture_phase = phase,
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
            Command::SaveMapping(mapping) => {
                match gamepad::mapping::save(&mapping) {
                    Ok(()) => log::info!("buttons of {} set up", mapping.name),
                    Err(e) => log::error!("{e:#}"),
                }
                self.restart_source();
            }
            Command::ForgetMapping(guid) => {
                if let Err(e) = gamepad::mapping::forget(&guid) {
                    log::error!("{e:#}");
                }
                self.restart_source();
            }
            Command::SetUrl(url) => self.set_url(url),
            Command::Intiface(control) => {
                if let Some(i) = &self.intiface {
                    i.control(control);
                }
            }
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
            Command::SetRecordingImages(per_second) => {
                self.settings.recording_images = per_second;
                self.settings.save();
            }
            Command::PlaySession { path, outputs, from, speed } => {
                if self.recorder.is_some() {
                    return log::warn!("the toys play no recording while one is made");
                }
                if self.replay.as_ref().is_none_or(|r| r.path != path) {
                    log::info!("the toys play {}", path.display());
                }
                self.replay = Some(Replay { path, outputs, from, speed, start: self.time() });
            }
            Command::StopSession => self.stop_replay(),
            Command::DeleteRecording(path) => {
                match session::delete(&path) {
                    Ok(()) => log::info!("recording {} deleted", path.display()),
                    Err(e) => log::error!("cannot delete {}: {e}", path.display()),
                }
                self.refresh_recordings();
            }
            Command::SetAudio(source) => {
                log::info!("game audio by default: {source:?}");
                // The game being played may listen to a sound of its own.
                if let (Some(audio), None) = (&self.audio, self.game.as_ref().and_then(|g| g.audio.as_ref())) {
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
                self.note_reading();
            }
            Command::CreateGame { name, executable } => {
                let mut game = Game::new(&name);
                let catalog = self.shared.lock().unwrap().catalog.clone();
                for (id, info) in &catalog {
                    // The modes already made for it (AI modes are categorized by their game), not their variants.
                    let variant = ModeEntry::from_id(id).variant.is_some();
                    if !variant && info.as_ref().is_ok_and(|i| i.category.eq_ignore_ascii_case(game.name.trim())) {
                        game.modes.push(id.clone());
                    }
                }
                if let Some(exe) = executable {
                    self.unlink(&exe);
                    if self.running.as_ref() == Some(&exe) {
                        game.steam_app_id = self.running_app;
                    }
                    game.executables.push(exe);
                }
                game.save();
                log::info!("game added: {}", game.name);
                self.games = Game::list();
                self.select_game(Some(game.id));
            }
            Command::SaveGame(game) => {
                game.save();
                self.games = Game::list();
                if self.game.as_ref().is_some_and(|g| g.id == game.id) {
                    self.set_game(Some(game));
                }
            }
            Command::DeleteGame(id) => {
                if let Some(game) = self.games.iter().find(|g| g.id == id) {
                    log::info!("game deleted: {}", game.name);
                    game.delete();
                }
                self.games = Game::list();
                if self.game.as_ref().is_some_and(|g| g.id == id) {
                    self.select_game(None);
                }
            }
            Command::SelectGame(id) => self.select_game(id),
            Command::LinkExecutable { game, executable } => {
                self.unlink(&executable);
                let app = self.running_app.filter(|_| self.running.as_ref() == Some(&executable));
                self.edit_game_by_id(&game, |g| {
                    g.executables.push(executable.clone());
                    g.steam_app_id = app.or(g.steam_app_id);
                });
                self.select_game(Some(game));
            }
            Command::Imported { game } => {
                self.refresh_modes();
                self.games = Game::list();
                if self.game.as_ref().is_some_and(|g| g.id == game) {
                    self.set_game(Game::load(&game));
                }
            }
            Command::AddGameMode { game, mode } => self.edit_game_by_id(&game, |g| {
                if !g.modes.contains(&mode) {
                    g.modes.push(mode);
                }
            }),
            Command::RemoveGameMode { game, mode } => self.edit_game_by_id(&game, |g| g.modes.retain(|m| *m != mode)),
            Command::CapturePhase(phase) => self.capture(phase, self.time()),
            Command::SaveInputs(mut inputs) => {
                // The GUI's copy has no capture embeddings: captures change through their own commands.
                inputs.captures = package::Inputs::load(&inputs.dir).captures;
                inputs.save();
                if inputs.dir == self.mode_inputs.dir {
                    self.set_mode_inputs(inputs);
                }
            }
            Command::AddCaptures { dir, phase, frames } => self.edit_inputs(&dir, |i| {
                for frame in &frames {
                    if let Err(e) = i.add_capture(&phase, frame) {
                        log::error!("cannot save the capture: {e}");
                    }
                }
            }),
            Command::DeleteCapture { dir, file } => self.edit_inputs(&dir, |i| i.remove_capture(&file)),
            Command::MoveCapture { dir, file, phase } => self.edit_inputs(&dir, |p| {
                if let Some(capture) = p.captures.iter_mut().find(|c| c.file == file) {
                    capture.phase = phase;
                }
            }),
            Command::SetKeyboardShortcuts(on) => {
                self.settings.keyboard_shortcuts = on;
                self.settings.save();
                self.shortcuts = on.then(Shortcuts::start);
            }
            Command::SetCheckUpdates(on) => {
                self.settings.check_updates = on;
                self.settings.save();
            }
            Command::ConfigureShortcuts => {
                if let Some(shortcuts) = &self.shortcuts {
                    shortcuts.configure();
                }
            }
            Command::SetExternalPort(port) => {
                self.settings.external_port = port;
                self.settings.save();
                self.external.stop();
                self.external = ExternalInputs::start(port);
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
        if self.recorder.is_some() || self.replay.is_some() {
            return;
        }
        log::info!("recording started");
        let images = self.settings.recording_images;
        // Indicators and values that do not change during the recording are in it all the same.
        let state = (self.indicators.values().iter().map(|(name, value)| Change::Indicator { name: name.clone(), value: *value }))
            .chain(self.external.values().iter().map(|(name, value)| Change::ExternalValue { name: name.clone(), value: value.clone() }))
            .chain(self.pad.held().iter().map(|name| Change::Button { name: (*name).to_owned(), pressed: true }));
        self.recorder = Some(Recorder::new(self.time(), self.mode_name(), self.game()).starting_from(state).images(images).spooled());
        self.note_reading();
    }

    /// Tells the recordings which zones the indicators are read in (modes see them only with the image).
    fn note_reading(&mut self) {
        let time = self.time();
        let zones = self.settings.screen.then_some(self.mode_inputs.zones.as_slice());
        self.recent.reading(time, zones);
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.reading(time, zones);
        }
    }

    fn mode_name(&self) -> String {
        self.mode.as_ref().and_then(|m| m.runtime.as_ref()).map(|rt| rt.info().name.clone()).unwrap_or_default()
    }

    /// The game being played, by name, or else the executable showing the in-game overlay.
    fn game(&self) -> Option<String> {
        self.game.as_ref().map(|g| g.name.clone()).or_else(|| self.overlay.clients().first().map(|c| c.exe.clone()))
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

    fn stop_replay(&mut self) {
        if self.replay.take().is_some() {
            log::info!("the toys stopped playing the recording");
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
        self.set_mode_inputs(package::Inputs::of(&entry));
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
        // Play time with the active mode, counted a minute at a time; a session is 2 minutes in one go.
        let active = self.mode.as_ref().map(|m| m.entry.id.clone()).unwrap_or_default();
        if self.running.is_none() || self.session.0 != active {
            self.session = (active.clone(), 0.0, false);
        }
        if self.running.is_some() {
            self.played += dt;
            self.session.1 += dt;
            let session = self.session.1 >= SESSION_SECS && !self.session.2;
            if session {
                self.session.2 = true;
            }
            if self.played >= 60.0 || session {
                crate::community::record_play(&active, self.played);
                if self.settings.share_stats == Some(true) {
                    self.plays.add(&active, self.played, session);
                    let _ = self.plays.send(crate::community::URL, &self.settings.installation_id, false);
                }
                self.played = 0.0;
            }
        }
        if self.replay.as_ref().is_some_and(|r| r.finished(time)) {
            self.stop_replay();
        }
        let audio_levels = self.poll_audio(time);
        self.poll_screen(time);
        for message in self.external.poll(time) {
            self.events.push(match message {
                external::Message::Set(name, value) => ModeEvent::ExternalValue { name, value },
                external::Message::Event(name, data) => ModeEvent::ExternalEvent { name, data },
            });
        }
        // The toys playing a recording need no gamepad.
        let lost = self.replay.is_none() && self.source.lost();
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
        let game = RumbleLevels { strong: strong as f64 / 65535.0, weak: weak as f64 / 65535.0 };
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
        self.recent.tick(time, levels, &buttons, &self.pad, senses, &self.events);
        for action in self.shortcuts.as_ref().map(Shortcuts::poll).unwrap_or_default() {
            match action {
                Action::Panic => self.trigger_panic("keyboard"),
                Action::Capture => self.capture(self.capture_phase.clone(), time),
                Action::Mark => self.mark_moment(time),
            }
        }
        if self.pad.panic_combo(time) {
            self.trigger_panic(&gamepad::combo_text(&self.settings.panic_combo));
        }
        if self.pad.take_capture(time) {
            self.capture(self.capture_phase.clone(), time);
        }
        if self.pad.take_mark(time) {
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
        self.update_phases();
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
        // A recording the toys play takes the mode's place (the panic stop still holds them).
        if let Some(replay) = self.replay.as_ref().filter(|_| !self.panic) {
            channels = replay.channels(time);
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
        for toy in &toys.toys {
            let mut level = channels.iter().filter(|(c, _)| self.routed(c, &toy.name)).map(|(_, v)| *v).fold(0.0, f64::max);
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
        shared.pad = self.source.pad();
        shared.source_hint = self.source.hint();
        shared.last_rumble = self.last_rumble;
        shared.buttons_seen = self.buttons_seen;
        shared.intiface = toys;
        shared.intiface_enabled = self.intiface.is_some();
        shared.settings = self.settings.clone();
        shared.held = self.pad.held().iter().copied().collect();
        shared.axes = self.pad.axes().clone();
        shared.toy_levels = toy_levels;
        shared.recording = self.recorder.as_ref().map(|r| r.elapsed(time));
        shared.replay = self.replay.as_ref().map(|r| ReplayView { path: r.path.clone(), position: r.position(time) });
        shared.overlay_clients = self.overlay.clients();
        shared.audio = self.audio_view(audio_levels);
        shared.screen = self.screen_view.clone();
        shared.phases = self.phase_view();
        shared.games = self.games.clone();
        shared.game = self.game.clone();
        shared.mode_inputs = self.mode_inputs.has_package().then(|| self.mode_inputs.for_gui());
        shared.running_executable = self.running.clone();
        shared.running_app = self.running_app;
        shared.unlinked_executable = self.running.clone().filter(|exe| !self.games.iter().any(|g| g.is_running(exe, self.running_app)));
        shared.external = self.external.view();
        shared.shortcuts = self.shortcuts.as_ref().map(Shortcuts::status).unwrap_or_default();
        shared.capture_phase = self.capture_phase.clone();
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
        if self.intiface.is_some() && toys.paused {
            alerts.push("Intiface Central disconnected".to_owned());
        } else if self.intiface.is_some() && !toys.connected {
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
            phase: self.mode.as_ref().and_then(|m| m.runtime.as_ref()).and_then(|rt| rt.phase_state().0),
            gauges: hud.into_iter().map(|g| Gauge { label: g.label, value: g.value as f32, max: g.max as f32 }).collect(),
            events: self.overlay_events.iter().map(|(text, t)| Event { text: text.clone(), age: (time - t) as f32 }).collect(),
            alerts,
            capture: self.wants_screen().then(CaptureRequest::default),
            ..OverlayState::default()
        }
    }

    /// Reads the newest copy of the game's image: measures, flashes, indicators,
    /// and an embedding now and then when phases or examples need one.
    fn poll_screen(&mut self, time: f64) {
        const STALE_SECS: f64 = 2.0;
        // A game linked to the executable now showing the overlay becomes the active game.
        let running = self.screen_view.game.clone().or_else(|| self.overlay.clients().first().map(|c| c.exe.clone()));
        if running != self.running {
            self.running = running.clone();
            let pid = self.overlay.clients().into_iter().find(|c| Some(&c.exe) == running.as_ref()).map(|c| c.pid);
            self.running_app = pid.and_then(platform::steam_app_id);
            let app = self.running_app;
            if let Some(game) = running.and_then(|exe| self.games.iter().find(|g| g.is_running(&exe, app))) {
                let (id, known) = (game.id.clone(), game.steam_app_id.is_some());
                if let Some(app) = app.filter(|_| !known) {
                    log::info!("{} is the Steam app {app}", game.name);
                    self.edit_game_by_id(&id, |g| g.steam_app_id = Some(app));
                }
                if self.game.as_ref().map(|g| &g.id) != Some(&id) {
                    self.select_game(Some(id));
                }
            }
        }
        let wanted = self.wants_screen();
        if let Some((hello, frame)) = self.overlay.frame().filter(|_| wanted) {
            let frame = Arc::new(frame);
            let (levels, flash) = self.screen.push(time, &frame);
            self.record_frame(time, &frame);
            if self.settings.screen {
                self.screen_levels = Some(levels);
                if let Some(strength) = flash {
                    self.events.push(ModeEvent::ScreenFlash(strength));
                }
                for (name, value) in self.indicators.update(&self.mode_inputs.zones, &frame) {
                    self.events.push(ModeEvent::Indicator { name, value });
                }
            } else {
                // Indicators are still measured for the editor.
                self.indicators.update(&self.mode_inputs.zones, &frame);
            }
            let rt = self.mode.as_ref().and_then(|m| m.runtime.as_ref());
            let phases_use_image = self.settings.screen && rt.is_some_and(|rt| rt.phase_sense(Sense::Screen) || rt.phase_sense(Sense::Examples));
            if phases_use_image && Model::Image.ready() && time - self.last_image_submit >= IMAGE_PHASE_STEP {
                self.last_image_submit = time;
                self.image_phases.submit(frame.clone());
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
                self.indicators.clear();
                self.screen_view = ScreenView::default();
            }
        }
        for image in self.image_phases.poll() {
            if self.screen_levels.is_some() {
                self.events.push(ModeEvent::ScreenClip(image));
            }
        }
        self.embed_captures();
        self.screen_view.model = Model::Image.state();
        // One entry per indicator, whatever the number of zones it is read in.
        let mut names: Vec<&String> = self.mode_inputs.zones.iter().map(|z| &z.indicator).collect();
        names.dedup();
        names.sort();
        names.dedup();
        self.screen_view.indicators = names
            .into_iter()
            .map(|n| (n.clone(), self.indicators.measures.get(n).copied().flatten(), self.indicators.values().get(n).copied()))
            .collect();
    }

    /// The overlay copies the game's image: modes see it, the GUI shows it, or a session is recorded.
    fn wants_screen(&self) -> bool {
        self.settings.screen || self.screen_watch || self.recorder.is_some()
    }

    /// Gives the recordings the image of the game they are due, encoded once.
    fn record_frame(&mut self, time: f64, frame: &Frame) {
        let recorder = self.recorder.as_ref().is_some_and(|r| r.wants_frame(time));
        let recent = self.recent.wants_frame(time);
        if !recorder && !recent {
            return;
        }
        let jpeg = match session::encode_frame(frame) {
            Ok(jpeg) => jpeg,
            Err(e) => return log::debug!("cannot encode the game's image: {e:#}"),
        };
        if let Some(r) = self.recorder.as_mut().filter(|_| recorder) {
            r.frame(time, jpeg.clone());
        }
        if recent {
            self.recent.frame(time, jpeg);
        }
    }

    /// Captures the game's current image into the active mode's inputs under
    /// `phase` ("": to sort), and says so in the in-game overlay.
    fn capture(&mut self, phase: String, time: f64) {
        let Some(frame) = &self.screen_view.frame else {
            log::warn!("no image of the game to capture");
            self.overlay_events.push(("No image to capture".to_owned(), time));
            return;
        };
        if !self.mode_inputs.has_package() {
            log::warn!("the active mode is built-in: it keeps no captures");
            self.overlay_events.push(("Built-in modes keep no captures".to_owned(), time));
            return;
        }
        let mut inputs = self.mode_inputs.clone();
        match inputs.add_capture(&phase, frame) {
            Ok(()) => {
                inputs.save();
                let count = inputs.captures.iter().filter(|c| c.phase == phase).count();
                let what = if phase.is_empty() { "to sort".to_owned() } else { phase.clone() };
                log::info!("capture ({what}) added to {}", self.mode_name());
                self.overlay_events.push((format!("📸 Captured: {what} ({count})"), time));
                self.set_mode_inputs(inputs);
            }
            Err(e) => log::error!("cannot save the capture: {e}"),
        }
    }

    /// Edits the inputs of a mode (by package), and saves them.
    fn edit_inputs(&mut self, dir: &Path, edit: impl FnOnce(&mut package::Inputs)) {
        if dir.as_os_str().is_empty() {
            return log::warn!("built-in modes read no inputs set up by the player");
        }
        let mut inputs = if dir == self.mode_inputs.dir { self.mode_inputs.clone() } else { package::Inputs::load(dir) };
        edit(&mut inputs);
        inputs.save();
        if inputs.dir == self.mode_inputs.dir {
            self.set_mode_inputs(inputs);
        }
    }

    /// The active mode's inputs changed, or another mode became active.
    fn set_mode_inputs(&mut self, inputs: package::Inputs) {
        if inputs.dir != self.mode_inputs.dir {
            self.indicators.clear();
            self.capture_job = None;
        }
        self.example_centroids = inputs.example_centroids();
        self.mode_inputs = inputs;
        self.note_reading();
        // The active mode compares with the new examples.
        if let Some(rt) = self.mode.as_mut().and_then(|m| m.runtime.as_mut()) {
            rt.set_phase_references(Sense::Examples, self.example_centroids.clone());
        }
    }

    /// Games no longer list modes whose files are gone (deleted modes, or
    /// deleted before deleting took them out of their game).
    fn forget_missing_modes(&mut self) {
        let mut changed = false;
        for game in &mut self.games {
            let before = game.modes.len();
            game.modes.retain(|id| ModeEntry::from_id(id).source().is_ok());
            if game.modes.len() != before {
                log::info!("{}: {} missing mode(s) taken out", game.name, before - game.modes.len());
                game.save();
                changed = true;
            }
        }
        if changed {
            self.games = Game::list();
            if let Some(id) = self.game.as_ref().map(|g| g.id.clone()) {
                self.set_game(Game::load(&id));
            }
        }
    }

    fn edit_game_by_id(&mut self, id: &str, edit: impl FnOnce(&mut Game)) {
        let game = if self.game.as_ref().is_some_and(|g| g.id == id) { self.game.clone() } else { Game::load(id) };
        let Some(mut game) = game else { return };
        edit(&mut game);
        game.save();
        self.games = Game::list();
        if self.game.as_ref().is_some_and(|g| g.id == id) {
            self.set_game(Some(game));
        }
    }

    /// No game runs as `exe` any more.
    fn unlink(&mut self, exe: &str) {
        for game in self.games.iter_mut().filter(|g| g.runs_as(exe)) {
            game.executables.retain(|e| !e.eq_ignore_ascii_case(exe));
            game.save();
        }
        if let Some(game) = &mut self.game {
            game.executables.retain(|e| !e.eq_ignore_ascii_case(exe));
        }
    }

    /// Makes a game the one being played (remembered for the next start).
    fn select_game(&mut self, id: Option<String>) {
        let game = id.as_deref().and_then(Game::load);
        if let Some(game) = &game {
            log::info!("playing {}", game.name);
        }
        self.settings.active_game = game.as_ref().map(|g| g.id.clone());
        self.settings.save();
        self.set_game(game);
    }

    /// Computes the embeddings of the active mode's captures that have none, in a
    /// thread of its own, once the image model is downloaded.
    fn embed_captures(&mut self) {
        if let Some((dir, rx)) = &self.capture_job {
            let mut results = Vec::new();
            let done = loop {
                match rx.try_recv() {
                    Ok(result) => results.push(result),
                    Err(std_mpsc::TryRecvError::Empty) => break false,
                    Err(std_mpsc::TryRecvError::Disconnected) => break true,
                }
            };
            if self.mode_inputs.dir == *dir && !results.is_empty() {
                let mut inputs = self.mode_inputs.clone();
                for (file, embedding) in results {
                    if let Some(capture) = inputs.captures.iter_mut().find(|c| c.file == file) {
                        capture.embedding = embedding.to_vec();
                    }
                }
                inputs.save();
                self.set_mode_inputs(inputs);
            }
            if done {
                self.capture_job = None;
            }
            return;
        }
        let inputs = &self.mode_inputs;
        let missing: Vec<String> = inputs.captures.iter().filter(|c| c.embedding.is_empty()).map(|c| c.file.clone()).collect();
        if missing.is_empty() || !Model::Image.ready() {
            return;
        }
        let dir = inputs.dir.clone();
        let (tx, rx) = std_mpsc::channel();
        self.capture_job = Some((dir.clone(), rx));
        std::thread::spawn(move || {
            let mut encoder = match screen::clip::ImageEncoder::load() {
                Ok(e) => e,
                Err(e) => return log::error!("cannot load the image phase model: {e:#}"),
            };
            for file in missing {
                let embedded = package::load_capture(&dir, &file).map(|frame| encoder.embed(&frame));
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

    /// The game being played changed, or was edited.
    fn set_game(&mut self, game: Option<Game>) {
        let changed = self.game.as_ref().map(|g| &g.id) != game.as_ref().map(|g| &g.id);
        // Its sound, or the default.
        let source = game.as_ref().and_then(|g| g.audio.clone()).unwrap_or_else(|| self.settings.audio.clone());
        if changed || self.game.as_ref().and_then(|g| g.audio.clone()) != game.as_ref().and_then(|g| g.audio.clone()) {
            if let Some(audio) = &self.audio {
                audio.set_source(source);
            }
        }
        self.game = game;
    }

    /// Drains the audio service.
    fn poll_audio(&mut self, time: f64) -> Option<AudioLevels> {
        let games = self.overlay.clients().into_iter().map(|c| (c.pid, c.exe)).collect();
        let outputs = match &self.audio {
            Some(audio) => {
                audio.set_games(games);
                audio.poll()
            }
            None => Vec::new(),
        };
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

    /// Gets the active mode's phase descriptions encoded for each sense whose
    /// model is downloaded (off the engine thread: a text model takes a moment
    /// to load), hands it its examples, and asks the audio service
    /// for embeddings only while the mode can use them.
    fn update_phases(&mut self) {
        while let Ok((sense, descriptions, result)) = self.phase_texts.1.try_recv() {
            self.phase_request.remove(&sense);
            match result {
                Ok(texts) => {
                    let rt = self.mode.as_mut().and_then(|m| m.runtime.as_mut());
                    if let Some(rt) = rt.filter(|rt| rt.phase_descriptions(sense) == descriptions) {
                        log::info!("{sense:?} phases of '{}' ready", rt.info().name);
                        rt.set_phase_references(sense, descriptions.into_iter().map(|(name, _)| name).zip(texts).collect());
                    }
                }
                Err(e) => {
                    log::error!("cannot prepare the {sense:?} phases: {e}");
                    self.phase_failed.insert(sense, descriptions);
                }
            }
        }
        let Some(rt) = self.mode.as_mut().and_then(|m| m.runtime.as_mut()) else {
            if let Some(audio) = &self.audio {
                audio.set_phases(false);
            }
            return;
        };
        // The phases the player set up replace the mode's own.
        rt.set_game_phases(&self.mode_inputs.phase_decls());
        if !rt.phase_sense(Sense::Examples) && !self.example_centroids.is_empty() {
            rt.set_phase_references(Sense::Examples, self.example_centroids.clone());
        }
        if let Some(audio) = &self.audio {
            audio.set_phases(rt.uses_sound_phases() && Model::Sound.ready());
        }
        for (sense, model) in [(Sense::Sound, Model::Sound), (Sense::Screen, Model::Image)] {
            let descriptions = rt.phase_descriptions(sense);
            if descriptions.is_empty() || !model.ready() || rt.phase_sense(sense) {
                continue;
            }
            if self.phase_request.contains_key(&sense) || self.phase_failed.get(&sense) == Some(&descriptions) {
                continue;
            }
            self.phase_request.insert(sense, descriptions.clone());
            let tx = self.phase_texts.0.clone();
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

    fn phase_view(&self) -> PhaseView {
        let Some(rt) = self.mode.as_ref().and_then(|m| m.runtime.as_ref()).filter(|rt| !rt.phase_decls().is_empty()) else {
            return PhaseView::default();
        };
        let (phase, phases) = rt.phase_state();
        PhaseView {
            phase,
            phases,
            sound: (rt.uses_sound_phases(), rt.phase_sense(Sense::Sound)),
            screen: (rt.uses_screen_phases(!self.example_centroids.is_empty()), rt.phase_sense(Sense::Screen) || rt.phase_sense(Sense::Examples)),
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
        // The play time gathered since the last report.
        if self.settings.share_stats == Some(true) {
            if let Some(active) = self.mode.as_ref().map(|m| m.entry.id.clone()) {
                self.plays.add(&active, self.played, false);
            }
            if let Some(sending) = self.plays.send(crate::community::URL, &self.settings.installation_id, true) {
                let _ = tokio::task::spawn_blocking(move || sending.join()).await;
            }
        }
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
        self.external.stop();
        if let Some(i) = self.intiface.take() {
            i.shutdown().await;
        }
        std::mem::replace(&mut self.source, Source::None).shutdown();
        if let Some(audio) = self.audio.take() {
            audio.shutdown();
        }
        self.sources.shutdown();
    }
}
