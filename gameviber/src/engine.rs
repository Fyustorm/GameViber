//! The engine ties everything together on its own thread: event sources,
//! force-feedback state, gamepad state, the active Lua mode (with hot
//! reload), the safety layer, channel -> toy routing and the Intiface
//! output. It publishes a `Shared` snapshot for the GUI and obeys `Command`s.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;

pub use crate::config::SourceChoice;
use crate::config::{self, ModeEntry, Settings};
use crate::helper::client::Helper;
use crate::gamepad::{PadState, BUTTONS};
use crate::intiface::{Intiface, IntifaceStatus, Toy, ToyOutputs};
use crate::mode::rumble_events::RumbleLevels;
use crate::mode::{ModeEvent, ModeInfo, ModeRuntime, ParamValue};
use crate::rumble::RumbleState;
use crate::source::ebpf::EbpfSource;
use crate::source::proxy::{Hide, ProxySource};
use crate::source::{EventSender, SourceEvent, SourceKind};

const TICK: Duration = Duration::from_millis(20);
/// How often user mode files are checked for changes.
const RELOAD_CHECK: Duration = Duration::from_secs(1);
/// Length of the history kept for the GUI graphs.
pub const HISTORY_SECS: f64 = 10.0;

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
}

#[derive(Debug, Clone)]
pub enum Command {
    SelectMode(String),
    /// Re-reads the active mode file (hot reload).
    ReloadMode,
    RefreshModes,
    SetParam(String, ParamValue),
    Resume,
    Panic,
    Rearm,
    SetCap(f64),
    SetRouting { channel: String, toys: Vec<String> },
    SetSource { source: SourceChoice, hide: bool },
    SimRumble { strong: f64, weak: f64 },
    SimButton { name: String, pressed: bool },
    Shutdown,
}

#[derive(Debug, Clone, Default)]
pub struct ModeView {
    pub id: String,
    pub info: Option<ModeInfo>,
    pub values: BTreeMap<String, ParamValue>,
    pub error: Option<String>,
    pub suspended: bool,
}

#[derive(Debug, Clone)]
pub struct Sample {
    pub t: f64,
    pub strong: f64,
    pub weak: f64,
    pub channels: BTreeMap<String, f64>,
}

/// Snapshot read by the GUI.
#[derive(Debug, Default)]
pub struct Shared {
    pub source: String,
    pub intiface: IntifaceStatus,
    pub intiface_enabled: bool,
    pub modes: Vec<ModeEntry>,
    pub mode: ModeView,
    pub panic: bool,
    pub settings: Settings,
    pub history: VecDeque<Sample>,
    pub plots: BTreeMap<String, VecDeque<[f64; 2]>>,
    pub held: Vec<&'static str>,
    pub toy_levels: BTreeMap<String, f64>,
    pub time: f64,
    pub stopped: bool,
}

pub type SharedHandle = Arc<Mutex<Shared>>;

struct ActiveMode {
    entry: ModeEntry,
    runtime: Option<ModeRuntime>,
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

    fn shutdown(self) {
        match self {
            Source::Proxy(p) => p.shutdown(),
            Source::Ebpf(e) => e.shutdown(),
            Source::Failed(_) | Source::None => {}
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
    /// Parameter values changed since the last save (saves are batched: sliders send many changes).
    params_dirty: bool,
    last_channels: BTreeMap<String, f64>,
}

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
    let (source_tx, mut rx) = mpsc::unbounded_channel::<SourceEvent>();
    let intiface = opts.intiface.then(|| Intiface::spawn(settings.url.clone()));
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
        params_dirty: false,
        last_channels: BTreeMap::new(),
    };
    engine.start_source();
    engine.refresh_modes();
    let first_mode = engine.opts.mode.clone().unwrap_or_else(|| engine.settings.active_mode.clone());
    engine.select_mode(&first_mode);

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
        let opts = &self.opts;
        let tx = self.source_tx.clone();
        let root = unsafe { libc::geteuid() } == 0;
        self.source = match self.settings.source {
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
        };
        if let Source::Failed(e) = &self.source {
            log::error!("source unavailable: {e}");
        }
    }

    fn switch_source(&mut self, source: SourceChoice, hide: bool) {
        if source == self.settings.source && hide == self.settings.hide && !matches!(self.source, Source::Failed(_)) {
            return;
        }
        log::info!("switching source to {source:?}{}", if hide && source == SourceChoice::Proxy { " (hidden)" } else { "" });
        std::mem::replace(&mut self.source, Source::None).shutdown();
        self.states.clear();
        self.pad = PadState::default();
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
        match ev.kind {
            SourceKind::Button { code, pressed } => {
                if let Some(b) = self.pad.key(code, pressed, time) {
                    log::debug!("button {} {}", b.name, if b.pressed { "pressed" } else { "released" });
                    self.events.push(ModeEvent::Button(b));
                }
            }
            SourceKind::Axis { code, value } => {
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
        }
    }

    fn on_command(&mut self, command: Command) {
        match command {
            Command::SelectMode(id) => self.select_mode(&id),
            Command::ReloadMode => self.reload_mode(),
            Command::RefreshModes => self.refresh_modes(),
            Command::SetParam(name, value) => self.set_param(&name, &value),
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
            Command::SetSource { source, hide } => self.switch_source(source, hide),
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

    fn refresh_modes(&mut self) {
        let modes = config::list_modes();
        self.shared.lock().unwrap().modes = modes;
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
        let mut active =
            ActiveMode { modified: entry.modified(), entry, runtime: None, error: None, suspended: false };
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

        let (strong, weak) = self
            .states
            .values_mut()
            .map(|s| s.motors(now))
            .fold((0u16, 0u16), |(s, w), (s2, w2)| (s.max(s2), w.max(w2)));
        let levels = RumbleLevels {
            strong: (strong as f64 / 65535.0).max(self.sim.strong),
            weak: (weak as f64 / 65535.0).max(self.sim.weak),
        };
        if self.pad.panic_combo(time) {
            self.trigger_panic("BACK + START");
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
            }
        }
        self.last_toys = toys.toys.clone();

        let mut channels = BTreeMap::new();
        let mut plots = Vec::new();
        let events = std::mem::take(&mut self.events);
        let input_idle = self.pad.input_idle(time);
        if let Some(active) = self.mode.as_mut() {
            if let Some(rt) = active.runtime.as_mut() {
                if !active.suspended && !self.panic {
                    match rt.step(dt, levels, &self.pad, input_idle, &events) {
                        Ok(out) => {
                            channels = out.channels;
                            plots = out.plots;
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
        // Safety layer: panic / suspension output 0 (channels stay empty), global cap.
        let cap = self.settings.global_cap;
        channels.values_mut().for_each(|v| *v = v.min(cap));
        if channels != self.last_channels {
            let text: Vec<String> = channels.iter().map(|(c, v)| format!("{c}={v:.2}")).collect();
            log::debug!("output {} (rumble {:.2}/{:.2})", text.join(" "), levels.strong, levels.weak);
            self.last_channels = channels.clone();
        }

        let mut toy_outputs = ToyOutputs::new();
        let mut toy_levels = BTreeMap::new();
        for toy in &toys.toys {
            let level = channels.iter().filter(|(c, _)| self.routed(c, &toy.name)).map(|(_, v)| *v).fold(0.0, f64::max);
            toy_outputs.insert(toy.index, level);
            toy_levels.insert(toy.name.clone(), level);
        }
        if let Some(i) = &self.intiface {
            i.set_outputs(toy_outputs);
        }

        let mut shared = self.shared.lock().unwrap();
        shared.time = time;
        shared.panic = self.panic;
        shared.source = self.source.status();
        shared.intiface = toys;
        shared.intiface_enabled = self.intiface.is_some();
        shared.settings = self.settings.clone();
        shared.held = self.pad.held().iter().copied().collect();
        shared.toy_levels = toy_levels;
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

    fn publish_mode(&self) {
        let view = match &self.mode {
            Some(active) => ModeView {
                id: active.entry.id.clone(),
                info: active.runtime.as_ref().map(|rt| rt.info().clone()),
                values: active.runtime.as_ref().map(|rt| rt.param_values().clone()).unwrap_or_default(),
                error: active.error.clone(),
                suspended: active.suspended,
            },
            None => ModeView::default(),
        };
        self.shared.lock().unwrap().mode = view;
    }

    async fn shutdown(mut self) {
        log::info!("shutting down");
        self.save_params();
        if let Some(active) = self.mode.as_mut() {
            if let Some(rt) = active.runtime.as_mut() {
                let _ = rt.stop();
            }
        }
        if let Some(i) = self.intiface.take() {
            i.shutdown().await;
        }
        std::mem::replace(&mut self.source, Source::None).shutdown();
        self.helper.shutdown();
    }
}
