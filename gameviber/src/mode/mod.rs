//! Lua (Luau) mode runtime, implementing the API described in
//! docs/spec-modes.md: declaration, parameters, callbacks, outputs, timers,
//! debug helpers, sandboxing and hot-reload state.

pub mod library;
pub mod outputs;
pub mod prompt;
pub mod report;
pub mod rumble_events;
pub mod scenes;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::{Function, Lua, LuaOptions, StdLib, Table, Value, Variadic, VmState};
use serde::{Deserialize, Serialize};

use crate::audio::{AudioHit, AudioLevels, Embedding};
use crate::gamepad::{ButtonEvent, PadState, AXES, BUTTONS};
use crate::screen::ScreenLevels;
use scenes::{SceneChange, SceneDecl, SceneTracker, Sense};
use outputs::Outputs;
use rumble_events::{RumbleEvent, RumbleLevels, RumbleTracker, DEFAULT_RELEASE, DEFAULT_THRESHOLD};

pub const API_VERSION: u32 = 1;
/// Wall-clock budget for one callback invocation.
const CALLBACK_BUDGET: Duration = Duration::from_millis(10);
/// Budget for running the file's top level (declaration, helpers).
const LOAD_BUDGET: Duration = Duration::from_millis(200);
const MEMORY_LIMIT: usize = 16 * 1024 * 1024;
const PERSIST_MAX_DEPTH: usize = 16;
const MAX_SCENES: usize = 8;
/// Sound hits at least this strong are impacts (`on_impact`).
const IMPACT_MIN_HIT: f64 = 0.3;
/// Nesting kept from the values external programs send (`input.custom`).
const CUSTOM_MAX_DEPTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamKind {
    Number { min: f64, max: f64, step: Option<f64> },
    Bool,
    Choice(Vec<String>),
    Button,
}

#[derive(Debug, Clone)]
pub struct ParamDef {
    pub name: String,
    pub label: String,
    pub kind: ParamKind,
    pub default: ParamValue,
}

impl ParamDef {
    /// Checks / coerces a value for this parameter.
    pub fn accept(&self, value: &ParamValue) -> Option<ParamValue> {
        match (&self.kind, value) {
            (ParamKind::Number { min, max, .. }, ParamValue::Number(n)) => Some(ParamValue::Number(n.clamp(*min, *max))),
            (ParamKind::Bool, ParamValue::Bool(b)) => Some(ParamValue::Bool(*b)),
            (ParamKind::Choice(options), ParamValue::Text(t)) if options.contains(t) => Some(value.clone()),
            (ParamKind::Button, ParamValue::Text(t)) if BUTTONS.contains(&t.as_str()) => Some(value.clone()),
            _ => None,
        }
    }
}

/// A question the GUI asks a player who finds the mode wrong (`ask()`).
#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub id: String,
    pub label: String,
    /// Answers to pick from; empty for a plain checkbox.
    pub options: Vec<String>,
    /// Index of the answer meaning "fine" (unused for a checkbox).
    pub default: usize,
    /// Number parameter a quick fix adjusts: answers before `default` ask for a
    /// larger value, after it a smaller one (the reverse with `invert`).
    pub param: Option<String>,
    pub invert: bool,
}

impl Question {
    /// New value of the linked parameter for `answer`, or None when the answer is
    /// the default or the value cannot move. Each answer away from the default
    /// moves it by a quarter of its value, at least 5% of its range.
    pub fn quick_fix(&self, answer: usize, param: &ParamDef, current: f64) -> Option<f64> {
        let ParamKind::Number { min, max, step } = param.kind else { return None };
        let mut distance = self.default as f64 - answer as f64;
        if self.invert {
            distance = -distance;
        }
        if distance == 0.0 {
            return None;
        }
        let delta = (current.abs() * 0.25).max((max - min) * 0.05);
        let mut value = current + distance * delta;
        if let Some(step) = step.filter(|s| *s > 0.0) {
            value = min + ((value - min) / step).round() * step;
        }
        let value = (value.clamp(min, max) * 1e6).round() / 1e6;
        (value != current).then_some(value)
    }
}

#[derive(Debug, Clone)]
pub struct ModeInfo {
    pub name: String,
    pub description: String,
    /// Games the mode suits, shown under its name ("Horror, survival").
    pub category: String,
    /// Plain-language explanation of what the player feels.
    pub help: String,
    /// Parameters worth showing first; the others sit behind "All settings".
    pub main_params: Vec<String>,
    pub author: String,
    pub version: String,
    pub channels: Vec<String>,
    pub params: Vec<ParamDef>,
    /// Questions for the "Doesn't feel right?" dialog, in declaration order.
    pub feedback: Vec<Question>,
    pub rumble_threshold: f64,
    pub rumble_release: f64,
    /// Scenes recognized from the game's sound and image (§6.3), sorted by name.
    pub scenes: Vec<SceneDecl>,
    /// Seconds over which scene probabilities are averaged.
    pub scene_window: f64,
}

/// A zone of the game's screen, set up in its Signals (§6.5).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneValue {
    /// An element shown or not.
    Visible(bool),
    /// How full a bar is, 0..1.
    Bar(f64),
    /// Not found on screen (a bar hidden in menus): nil for modes.
    Unknown,
}

/// Plain-data copy of the `persist` table, carried across hot reloads.
#[derive(Debug, Clone, PartialEq)]
pub enum PersistValue {
    Bool(bool),
    Number(f64),
    Text(String),
    Table(Vec<(PersistValue, PersistValue)>),
}

/// Events queued by the engine between two ticks.
#[derive(Debug, Clone, PartialEq)]
pub enum ModeEvent {
    Button(ButtonEvent),
    Device { connected: bool, name: String },
    AudioHit(AudioHit),
    /// Embedding of the last 10 s of the game's sound.
    AudioClip(Embedding),
    /// Embedding of the game's image.
    ScreenClip(Embedding),
    /// A sudden flash of the game's image, 0..1.
    ScreenFlash(f64),
    /// A zone of the screen changed.
    Zone { name: String, value: ZoneValue },
    /// A value sent by another program (`input.custom`); `Null` removes it.
    Custom { name: String, value: serde_json::Value },
    /// An event sent by another program (`on_event`).
    External { name: String, data: serde_json::Value },
}

/// What a tick produced besides channel values.
#[derive(Debug, Default)]
pub struct TickOutput {
    pub channels: BTreeMap<String, f64>,
    pub plots: Vec<(String, f64)>,
    /// In-game overlay gauges set with `hud()`, in the order they were first set.
    pub hud: Vec<HudGauge>,
    /// Overlay messages raised with `hud_event()` during this tick.
    pub hud_events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HudGauge {
    pub label: String,
    pub value: f64,
    pub max: f64,
}

struct Timer {
    id: u64,
    due: f64,
    period: Option<f64>,
    func: Function,
}

/// State shared between the runtime and the Lua API closures.
struct Ctx {
    mode_name: String,
    time: f64,
    outputs: Option<Outputs>,
    plots: Vec<(String, f64)>,
    hud: Vec<HudGauge>,
    hud_events: Vec<String>,
    timers: Vec<Timer>,
    /// Timers cancelled during the current tick, possibly already collected as due.
    cancelled: HashSet<u64>,
    next_timer: u64,
    declared: Option<Table>,
    param_seq: u64,
    rng: u64,
}

impl Ctx {
    fn outputs(&mut self) -> mlua::Result<&mut Outputs> {
        self.outputs.as_mut().ok_or_else(|| mlua::Error::runtime("outputs are not available while loading"))
    }

    fn random(&mut self) -> f64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

struct Callbacks {
    tick: Function,
    on_start: Option<Function>,
    on_stop: Option<Function>,
    on_rumble: Option<Function>,
    on_rumble_start: Option<Function>,
    on_rumble_end: Option<Function>,
    on_button: Option<Function>,
    on_param_changed: Option<Function>,
    on_device: Option<Function>,
    on_audio_hit: Option<Function>,
    on_scene: Option<Function>,
    on_impact: Option<Function>,
    on_zone: Option<Function>,
    on_event: Option<Function>,
}

struct InputTables {
    root: Table,
    rumble: Table,
    buttons: Table,
    axes: Table,
    audio: Table,
    screen: Table,
    scenes: Table,
    zones: Table,
    custom: Table,
}

pub struct ModeRuntime {
    lua: Lua,
    ctx: Rc<RefCell<Ctx>>,
    deadline: Rc<Cell<Option<Instant>>>,
    info: ModeInfo,
    callbacks: Callbacks,
    param_values: BTreeMap<String, ParamValue>,
    values: Table,
    persist: Table,
    input: InputTables,
    tracker: RumbleTracker,
    /// The game's sound right now; None when it is not captured.
    audio: Option<AudioLevels>,
    /// The game's image right now; None when it is not copied.
    screen: Option<ScreenLevels>,
    /// The scenes recognized: the game's when it has some, else the mode's own.
    scene_decls: Vec<SceneDecl>,
    scenes: SceneTracker,
    /// Scene change to report on the next tick (the sound or image stopped).
    pending_scene: Option<SceneChange>,
    /// Zone values as last reported.
    zones: BTreeMap<String, ZoneValue>,
}

type LoadResult<T> = Result<T, String>;

fn lua_err(e: mlua::Error) -> String {
    e.to_string()
}

impl ModeRuntime {
    /// Reads a mode's declaration (name, description, parameters...) without running it.
    pub fn probe(chunk_name: &str, source: &str) -> LoadResult<ModeInfo> {
        Self::load(chunk_name, source, &BTreeMap::new(), None).map(|rt| rt.info)
    }

    /// Loads and validates a mode. `saved` are the persisted parameter values,
    /// `persist` the state carried over from the previous version on hot reload.
    pub fn load(
        chunk_name: &str,
        source: &str,
        saved: &BTreeMap<String, ParamValue>,
        persist: Option<&PersistValue>,
    ) -> LoadResult<Self> {
        let libs = StdLib::MATH | StdLib::STRING | StdLib::TABLE | StdLib::BIT | StdLib::UTF8;
        let lua = Lua::new_with(libs, LuaOptions::new()).map_err(lua_err)?;
        lua.set_memory_limit(MEMORY_LIMIT).map_err(lua_err)?;

        let deadline: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        {
            let deadline = deadline.clone();
            lua.set_interrupt(move |_| match deadline.get() {
                Some(d) if Instant::now() > d => Err(mlua::Error::runtime("time budget exceeded (infinite loop?)")),
                _ => Ok(VmState::Continue),
            });
        }

        let ctx = Rc::new(RefCell::new(Ctx {
            mode_name: chunk_name.to_owned(),
            time: 0.0,
            outputs: None,
            plots: Vec::new(),
            hud: Vec::new(),
            hud_events: Vec::new(),
            timers: Vec::new(),
            cancelled: HashSet::new(),
            next_timer: 1,
            declared: None,
            param_seq: 0,
            rng: seed(),
        }));
        library::register(&lua, &ctx).map_err(lua_err)?;
        lua.sandbox(true).map_err(lua_err)?;

        // Writable globals must be created after sandboxing (which freezes existing tables).
        let values = lua.create_table().map_err(lua_err)?;
        let params_proxy = read_only_proxy(&lua, &values).map_err(lua_err)?;
        let persist_table = match persist {
            Some(p) => match persist_to_lua(&lua, p).map_err(lua_err)? {
                Value::Table(t) => t,
                _ => lua.create_table().map_err(lua_err)?,
            },
            None => lua.create_table().map_err(lua_err)?,
        };
        let globals = lua.globals();
        globals.set("P", params_proxy).map_err(lua_err)?;
        globals.set("persist", &persist_table).map_err(lua_err)?;

        deadline.set(Some(Instant::now() + LOAD_BUDGET));
        let exec = lua.load(source).set_name(format!("={chunk_name}")).exec();
        deadline.set(None);
        exec.map_err(lua_err)?;

        let declared = ctx.borrow_mut().declared.take().ok_or("mode { ... } was never called")?;
        let info = parse_info(&declared)?;

        let mut param_values = BTreeMap::new();
        for def in &info.params {
            let value = saved.get(&def.name).and_then(|v| def.accept(v)).unwrap_or_else(|| def.default.clone());
            values.raw_set(def.name.as_str(), param_to_lua(&lua, &value).map_err(lua_err)?).map_err(lua_err)?;
            param_values.insert(def.name.clone(), value);
        }

        // Callbacks are global functions, or fields of the `mode { }` table.
        let get = |name: &str| -> LoadResult<Option<Function>> {
            let value = match globals.get::<Value>(name).map_err(lua_err)? {
                Value::Nil => declared.get::<Value>(name).map_err(lua_err)?,
                value => value,
            };
            match value {
                Value::Function(f) => Ok(Some(f)),
                Value::Nil => Ok(None),
                _ => Err(format!("'{name}' must be a function")),
            }
        };
        let callbacks = Callbacks {
            tick: get("tick")?.ok_or("the mode must define tick(dt, input)")?,
            on_start: get("on_start")?,
            on_stop: get("on_stop")?,
            on_rumble: get("on_rumble")?,
            on_rumble_start: get("on_rumble_start")?,
            on_rumble_end: get("on_rumble_end")?,
            on_button: get("on_button")?,
            on_param_changed: get("on_param_changed")?,
            on_device: get("on_device")?,
            on_audio_hit: get("on_audio_hit")?,
            on_scene: get("on_scene")?,
            on_impact: get("on_impact")?,
            on_zone: get("on_zone")?,
            on_event: get("on_event")?,
        };


        let input = InputTables {
            root: lua.create_table().map_err(lua_err)?,
            rumble: lua.create_table().map_err(lua_err)?,
            buttons: lua.create_table().map_err(lua_err)?,
            axes: lua.create_table().map_err(lua_err)?,
            audio: lua.create_table().map_err(lua_err)?,
            screen: lua.create_table().map_err(lua_err)?,
            scenes: lua.create_table().map_err(lua_err)?,
            zones: lua.create_table().map_err(lua_err)?,
            custom: lua.create_table().map_err(lua_err)?,
        };
        for (name, table) in [
            ("rumble", &input.rumble),
            ("buttons", &input.buttons),
            ("axes", &input.axes),
            ("audio", &input.audio),
            ("screen", &input.screen),
            ("scenes", &input.scenes),
            ("zones", &input.zones),
            ("custom", &input.custom),
        ] {
            input.root.raw_set(name, table).map_err(lua_err)?;
        }
        // Event callbacks read it too (the state of the previous tick, at the current time).
        input.root.raw_set("time", 0.0).map_err(lua_err)?;
        globals.set("input", &input.root).map_err(lua_err)?;

        ctx.borrow_mut().outputs = Some(Outputs::new(info.channels.clone()));
        let tracker = RumbleTracker::new(info.rumble_threshold, info.rumble_release, 0.0);
        let scenes = SceneTracker::new(&info.scenes, info.scene_window);
        let scene_decls = info.scenes.clone();
        Ok(Self {
            lua,
            ctx,
            deadline,
            info,
            callbacks,
            param_values,
            values,
            persist: persist_table,
            input,
            tracker,
            audio: None,
            screen: None,
            scene_decls,
            scenes,
            pending_scene: None,
            zones: BTreeMap::new(),
        })
    }

    pub fn info(&self) -> &ModeInfo {
        &self.info
    }

    pub fn param_values(&self) -> &BTreeMap<String, ParamValue> {
        &self.param_values
    }

    /// The scenes of the game being played, which replace the mode's own
    /// (§6.3); none: the mode's own scenes.
    pub fn set_game_scenes(&mut self, game: &[SceneDecl]) {
        let mut wanted = if game.is_empty() { self.info.scenes.clone() } else { game.to_vec() };
        wanted.sort_by(|a, b| a.name.cmp(&b.name));
        wanted.truncate(MAX_SCENES);
        if wanted == self.scene_decls {
            return;
        }
        if let Some(change) = self.scenes.reset() {
            self.pending_scene = Some(change);
        }
        self.scenes = SceneTracker::new(&wanted, self.info.scene_window);
        // The zones on screen now, for the scenes tied to one.
        let time = self.ctx.borrow().time;
        for (name, value) in &self.zones {
            if let Some(change) = self.scenes.zone(time, name, matches!(value, ZoneValue::Visible(true) | ZoneValue::Bar(_))) {
                self.pending_scene = Some(change);
            }
        }
        let _ = self.input.scenes.clear();
        self.scene_decls = wanted;
    }

    /// The scenes recognized (the game's or the mode's own).
    pub fn scene_decls(&self) -> &[SceneDecl] {
        &self.scene_decls
    }

    pub fn uses_sound_scenes(&self) -> bool {
        self.scene_decls.iter().any(|s| s.sound.is_some())
    }

    /// Scenes compared with the image: described for it, or with captures (`examples`).
    pub fn uses_screen_scenes(&self, examples: bool) -> bool {
        examples || self.scene_decls.iter().any(|s| s.screen.is_some())
    }

    /// Scenes and their description for `sense` (Sound or Screen), to compute their embeddings.
    pub fn scene_descriptions(&self, sense: Sense) -> Vec<(String, String)> {
        self.scene_decls
            .iter()
            .filter_map(|s| {
                let description = match sense {
                    Sense::Sound => s.sound.as_ref(),
                    Sense::Screen => s.screen.as_ref(),
                    Sense::Examples => None,
                };
                description.map(|d| (s.name.clone(), d.clone()))
            })
            .collect()
    }

    /// What `sense` compares with: a vector per scene (descriptions' embeddings,
    /// or the mean of the profile's example images).
    pub fn set_scene_references(&mut self, sense: Sense, references: Vec<(String, Embedding)>) {
        self.scenes.set_references(sense, references);
    }

    #[cfg(test)]
    pub fn scenes_ready(&self) -> bool {
        self.scenes.ready()
    }

    pub fn scene_sense(&self, sense: Sense) -> bool {
        self.scenes.has(sense)
    }

    /// The current scene and the average probability of each, sorted by name.
    pub fn scene_state(&self) -> (Option<String>, Vec<(String, f64)>) {
        (self.scenes.current().map(str::to_owned), self.scenes.averages().map(|(n, p)| (n.to_owned(), p)).collect())
    }

    /// The game's sound for the next ticks; None when it stops being captured.
    pub fn set_audio(&mut self, levels: Option<AudioLevels>) {
        if levels.is_none() && self.audio.is_some() {
            self.forget_sense(&[Sense::Sound]);
        }
        self.audio = levels;
    }

    /// The game's image for the next ticks; None when it stops being copied.
    pub fn set_screen(&mut self, levels: Option<ScreenLevels>) {
        if levels.is_none() && self.screen.is_some() {
            self.forget_sense(&[Sense::Screen, Sense::Examples]);
            // The zones are read on the image: they are gone with it.
            self.zones.clear();
            let _ = self.input.zones.clear();
        }
        self.screen = levels;
    }

    fn forget_sense(&mut self, senses: &[Sense]) {
        for sense in senses {
            if let Some(change) = self.scenes.forget(*sense) {
                self.pending_scene = Some(change);
            }
        }
    }

    fn call(&self, f: &Function, args: impl mlua::IntoLuaMulti) -> Result<(), String> {
        self.deadline.set(Some(Instant::now() + CALLBACK_BUDGET));
        let result = f.call::<()>(args);
        self.deadline.set(None);
        result.map_err(lua_err)
    }

    fn call_opt(&self, f: &Option<Function>, args: impl mlua::IntoLuaMulti) -> Result<(), String> {
        match f {
            Some(f) => self.call(f, args),
            None => Ok(()),
        }
    }

    /// Resets outputs, timers and the random seed, then calls `on_start`.
    pub fn start(&mut self) -> Result<(), String> {
        {
            let mut ctx = self.ctx.borrow_mut();
            ctx.timers.clear();
            ctx.hud.clear();
            ctx.rng = seed();
            if let Some(o) = ctx.outputs.as_mut() {
                o.stop_all();
            }
        }
        let time = self.ctx.borrow().time;
        self.tracker = RumbleTracker::new(self.info.rumble_threshold, self.info.rumble_release, time);
        self.scenes.reset();
        self.pending_scene = None;
        self.zones.clear();
        let _ = self.input.zones.clear();
        let _ = self.input.custom.clear();
        self.call_opt(&self.callbacks.on_start, ())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        let result = self.call_opt(&self.callbacks.on_stop, ());
        if let Some(o) = self.ctx.borrow_mut().outputs.as_mut() {
            o.stop_all();
        }
        result
    }

    pub fn set_param(&mut self, name: &str, value: &ParamValue) -> Result<(), String> {
        let def = self.info.params.iter().find(|d| d.name == name).ok_or_else(|| format!("unknown parameter '{name}'"))?;
        let value = def.accept(value).ok_or_else(|| format!("invalid value {value:?} for '{name}'"))?;
        let lua_value = param_to_lua(&self.lua, &value).map_err(lua_err)?;
        self.values.raw_set(name, lua_value.clone()).map_err(lua_err)?;
        self.param_values.insert(name.to_owned(), value);
        self.call_opt(&self.callbacks.on_param_changed, (name, lua_value))
    }

    /// Sets every parameter at once (a preset): missing or invalid values fall back to
    /// the default. `on_param_changed` is only called for values that change.
    pub fn apply_params(&mut self, values: &BTreeMap<String, ParamValue>) -> Result<(), String> {
        let wanted: Vec<_> = self
            .info
            .params
            .iter()
            .map(|def| (def.name.clone(), values.get(&def.name).and_then(|v| def.accept(v)).unwrap_or_else(|| def.default.clone())))
            .collect();
        for (name, value) in wanted {
            if self.param_values.get(&name) != Some(&value) {
                self.set_param(&name, &value)?;
            }
        }
        Ok(())
    }

    /// Current `persist` table, for carrying it over a hot reload.
    pub fn persist_snapshot(&self) -> Option<PersistValue> {
        lua_to_persist(&Value::Table(self.persist.clone()), 0)
    }

    /// One engine tick: timers, queued events, derived rumble events, `tick`.
    /// `input_idle` is computed by the caller, whose clock the pad state uses.
    pub fn step(
        &mut self,
        dt: f64,
        rumble: RumbleLevels,
        pad: &PadState,
        input_idle: f64,
        events: &[ModeEvent],
    ) -> Result<TickOutput, String> {
        let time = {
            let mut ctx = self.ctx.borrow_mut();
            ctx.time += dt;
            ctx.time
        };
        self.input.root.raw_set("time", time).map_err(lua_err)?;
        self.run_timers(time)?;

        if let Some(change) = self.pending_scene.take() {
            self.scene_changed(change, time)?;
        }
        if let Some(change) = self.scenes.tick(time) {
            self.scene_changed(change, time)?;
        }
        for event in events {
            match event {
                ModeEvent::Button(event) => {
                    let t = self.lua.create_table().map_err(lua_err)?;
                    t.raw_set("button", event.name).map_err(lua_err)?;
                    t.raw_set("pressed", event.pressed).map_err(lua_err)?;
                    t.raw_set("t", time).map_err(lua_err)?;
                    self.call_opt(&self.callbacks.on_button, t)?;
                }
                ModeEvent::Device { connected, name } => {
                    let t = self.lua.create_table().map_err(lua_err)?;
                    t.raw_set("connected", *connected).map_err(lua_err)?;
                    t.raw_set("name", name.as_str()).map_err(lua_err)?;
                    t.raw_set("t", time).map_err(lua_err)?;
                    self.call_opt(&self.callbacks.on_device, t)?;
                }
                ModeEvent::AudioHit(hit) => {
                    if self.audio.is_none() {
                        continue;
                    }
                    if self.callbacks.on_audio_hit.is_some() {
                        let t = self.lua.create_table().map_err(lua_err)?;
                        t.raw_set("strength", hit.strength).map_err(lua_err)?;
                        t.raw_set("band", hit.band.name()).map_err(lua_err)?;
                        t.raw_set("t", time).map_err(lua_err)?;
                        self.call_opt(&self.callbacks.on_audio_hit, t)?;
                    }
                    if hit.strength >= IMPACT_MIN_HIT {
                        self.impact(hit.strength, "sound", time)?;
                    }
                }
                ModeEvent::AudioClip(sound) => {
                    if let Some(change) = self.scenes.update(time, Sense::Sound, sound) {
                        self.scene_changed(change, time)?;
                    }
                }
                ModeEvent::ScreenClip(image) => {
                    if self.screen.is_none() {
                        continue;
                    }
                    for sense in [Sense::Screen, Sense::Examples] {
                        if let Some(change) = self.scenes.update(time, sense, image) {
                            self.scene_changed(change, time)?;
                        }
                    }
                }
                ModeEvent::ScreenFlash(strength) => {
                    if self.screen.is_some() {
                        self.impact(*strength, "screen", time)?;
                    }
                }
                ModeEvent::Zone { name, value } => {
                    let previous = self.zones.insert(name.clone(), *value);
                    if previous == Some(*value) {
                        continue;
                    }
                    let lua_value = |v: ZoneValue| match v {
                        ZoneValue::Visible(b) => Value::Boolean(b),
                        ZoneValue::Bar(x) => Value::Number(x),
                        ZoneValue::Unknown => Value::Nil,
                    };
                    self.input.zones.raw_set(name.as_str(), lua_value(*value)).map_err(lua_err)?;
                    // A zone can be a sure sign of a scene of the game.
                    let shown = matches!(value, ZoneValue::Visible(true) | ZoneValue::Bar(_));
                    if let Some(change) = self.scenes.zone(time, name, shown) {
                        self.scene_changed(change, time)?;
                    }
                    if self.callbacks.on_zone.is_some() {
                        let t = self.lua.create_table().map_err(lua_err)?;
                        t.raw_set("zone", name.as_str()).map_err(lua_err)?;
                        t.raw_set("value", lua_value(*value)).map_err(lua_err)?;
                        t.raw_set("previous", previous.map(lua_value)).map_err(lua_err)?;
                        t.raw_set("t", time).map_err(lua_err)?;
                        self.call_opt(&self.callbacks.on_zone, t)?;
                    }
                }
                ModeEvent::Custom { name, value } => {
                    let value = json_to_lua(&self.lua, value, 0).map_err(lua_err)?;
                    self.input.custom.raw_set(name.as_str(), value).map_err(lua_err)?;
                }
                ModeEvent::External { name, data } => {
                    if self.callbacks.on_event.is_none() {
                        continue;
                    }
                    let t = self.lua.create_table().map_err(lua_err)?;
                    t.raw_set("name", name.as_str()).map_err(lua_err)?;
                    t.raw_set("data", json_to_lua(&self.lua, data, 0).map_err(lua_err)?).map_err(lua_err)?;
                    t.raw_set("t", time).map_err(lua_err)?;
                    self.call_opt(&self.callbacks.on_event, t)?;
                }
            }
        }

        for event in self.tracker.update(rumble, time) {
            let t = self.lua.create_table().map_err(lua_err)?;
            t.raw_set("t", time).map_err(lua_err)?;
            let (callback, levels) = match event {
                RumbleEvent::Changed(l) => (&self.callbacks.on_rumble, l),
                RumbleEvent::Start(l) => (&self.callbacks.on_rumble_start, l),
                RumbleEvent::End { peak, duration } => {
                    t.raw_set("peak", peak).map_err(lua_err)?;
                    t.raw_set("duration", duration).map_err(lua_err)?;
                    (&self.callbacks.on_rumble_end, RumbleLevels::default())
                }
            };
            if callback.is_none() {
                continue;
            }
            t.raw_set("strong", levels.strong).map_err(lua_err)?;
            t.raw_set("weak", levels.weak).map_err(lua_err)?;
            t.raw_set("level", levels.level()).map_err(lua_err)?;
            self.call_opt(callback, t)?;
        }

        self.fill_input(time, rumble, pad, input_idle).map_err(lua_err)?;
        self.call(&self.callbacks.tick, (dt, self.input.root.clone()))?;

        let mut ctx = self.ctx.borrow_mut();
        let plots = std::mem::take(&mut ctx.plots);
        let hud_events = std::mem::take(&mut ctx.hud_events);
        let hud = ctx.hud.clone();
        let channels = ctx.outputs().map_err(lua_err)?.evaluate(time);
        Ok(TickOutput { channels, plots, hud, hud_events })
    }

    fn impact(&self, strength: f64, source: &str, time: f64) -> Result<(), String> {
        if self.callbacks.on_impact.is_none() {
            return Ok(());
        }
        let t = self.lua.create_table().map_err(lua_err)?;
        t.raw_set("strength", strength).map_err(lua_err)?;
        t.raw_set("source", source).map_err(lua_err)?;
        t.raw_set("t", time).map_err(lua_err)?;
        self.call_opt(&self.callbacks.on_impact, t)
    }

    fn scene_changed(&self, change: SceneChange, time: f64) -> Result<(), String> {
        log::debug!("scene: {:?} -> {:?} ({:.2})", change.previous, change.scene, change.confidence);
        if self.callbacks.on_scene.is_none() {
            return Ok(());
        }
        let t = self.lua.create_table().map_err(lua_err)?;
        t.raw_set("scene", change.scene).map_err(lua_err)?;
        t.raw_set("previous", change.previous).map_err(lua_err)?;
        t.raw_set("confidence", change.confidence).map_err(lua_err)?;
        t.raw_set("t", time).map_err(lua_err)?;
        self.call_opt(&self.callbacks.on_scene, t)
    }

    fn run_timers(&mut self, time: f64) -> Result<(), String> {
        let due: Vec<(u64, Function)> = {
            let mut ctx = self.ctx.borrow_mut();
            let mut due = Vec::new();
            ctx.timers.retain_mut(|t| {
                if t.due > time {
                    return true;
                }
                due.push((t.id, t.func.clone()));
                match t.period {
                    Some(p) => {
                        t.due += p.max(0.001);
                        true
                    }
                    None => false,
                }
            });
            due
        };
        for (id, func) in due {
            // An earlier timer of this batch may have cancelled this one.
            if self.ctx.borrow().cancelled.contains(&id) {
                continue;
            }
            self.call(&func, ())?;
        }
        self.ctx.borrow_mut().cancelled.clear();
        Ok(())
    }

    fn fill_input(&self, time: f64, rumble: RumbleLevels, pad: &PadState, input_idle: f64) -> mlua::Result<()> {
        let rumble_idle = self.tracker.idle(time);
        let root = &self.input.root;
        root.raw_set("time", time)?;
        root.raw_set("rumble_idle", rumble_idle)?;
        root.raw_set("input_idle", input_idle)?;
        root.raw_set("idle", rumble_idle.min(input_idle))?;
        let r = &self.input.rumble;
        r.raw_set("strong", rumble.strong)?;
        r.raw_set("weak", rumble.weak)?;
        r.raw_set("level", rumble.level())?;
        r.raw_set("avg", rumble.avg())?;
        r.raw_set("active", self.tracker.active())?;
        for name in BUTTONS {
            self.input.buttons.raw_set(name, pad.held().contains(name))?;
        }
        for name in AXES {
            self.input.axes.raw_set(name, pad.axes().get(name).copied().unwrap_or(0.0))?;
        }
        let a = &self.input.audio;
        let levels = self.audio.unwrap_or_default();
        a.raw_set("active", self.audio.is_some())?;
        a.raw_set("level", levels.level)?;
        a.raw_set("low", levels.low)?;
        a.raw_set("mid", levels.mid)?;
        a.raw_set("high", levels.high)?;
        a.raw_set("intensity", levels.intensity)?;
        let v = &self.input.screen;
        let screen = self.screen.unwrap_or_default();
        v.raw_set("active", self.screen.is_some())?;
        v.raw_set("brightness", screen.brightness)?;
        v.raw_set("motion", screen.motion)?;
        v.raw_set("action", screen.action)?;
        // The senses there are, averaged.
        let busy: Vec<f64> = [self.audio.map(|l| l.intensity), self.screen.map(|l| l.action as f64)].into_iter().flatten().collect();
        root.raw_set("intensity", if busy.is_empty() { 0.0 } else { busy.iter().sum::<f64>() / busy.len() as f64 })?;
        root.raw_set("scene", self.scenes.current())?;
        root.raw_set("scene_confidence", self.scenes.confidence())?;
        for (name, p) in self.scenes.averages() {
            self.input.scenes.raw_set(name, p)?;
        }
        Ok(())
    }
}

/// A value sent by another program, as Lua sees it (nil for JSON null).
fn json_to_lua(lua: &Lua, value: &serde_json::Value, depth: usize) -> mlua::Result<Value> {
    use serde_json::Value as Json;
    Ok(match value {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        Json::String(s) => Value::String(lua.create_string(s)?),
        _ if depth >= CUSTOM_MAX_DEPTH => Value::Nil,
        Json::Array(items) => {
            let t = lua.create_table()?;
            for (i, item) in items.iter().enumerate() {
                t.raw_set(i + 1, json_to_lua(lua, item, depth + 1)?)?;
            }
            Value::Table(t)
        }
        Json::Object(map) => {
            let t = lua.create_table()?;
            for (k, v) in map {
                t.raw_set(k.as_str(), json_to_lua(lua, v, depth + 1)?)?;
            }
            Value::Table(t)
        }
    })
}

fn seed() -> u64 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    nanos | 1
}

fn read_only_proxy(lua: &Lua, values: &Table) -> mlua::Result<Table> {
    let proxy = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.raw_set("__index", values)?;
    meta.raw_set(
        "__newindex",
        lua.create_function(|_, (_, key): (Value, Value)| -> mlua::Result<()> {
            Err(mlua::Error::runtime(format!("P is read-only (tried to set {key:?})")))
        })?,
    )?;
    proxy.set_metatable(Some(meta))?;
    Ok(proxy)
}

fn param_to_lua(lua: &Lua, value: &ParamValue) -> mlua::Result<Value> {
    Ok(match value {
        ParamValue::Bool(b) => Value::Boolean(*b),
        ParamValue::Number(n) => Value::Number(*n),
        ParamValue::Text(s) => Value::String(lua.create_string(s)?),
    })
}

fn parse_info(declared: &Table) -> LoadResult<ModeInfo> {
    let get_str = |key: &str| declared.get::<Option<String>>(key).map_err(|e| format!("mode.{key}: {e}"));
    let api = declared.get::<Option<u32>>("api").map_err(|e| format!("mode.api: {e}"))?;
    if api != Some(API_VERSION) {
        return Err(format!("mode.api must be {API_VERSION}"));
    }
    let name = get_str("name")?.ok_or("mode.name is required")?;
    let channels = match declared.get::<Option<Table>>("channels").map_err(|e| format!("mode.channels: {e}"))? {
        Some(t) => t.sequence_values::<String>().collect::<mlua::Result<Vec<_>>>().map_err(|e| format!("mode.channels: {e}"))?,
        None => vec!["main".to_owned()],
    };
    if channels.is_empty() || channels.iter().any(|c| c == outputs::ALL_CHANNELS) {
        return Err("mode.channels must list at least one channel and cannot contain '*'".into());
    }

    let mut params = Vec::new();
    if let Some(table) = declared.get::<Option<Table>>("params").map_err(|e| format!("mode.params: {e}"))? {
        for pair in table.pairs::<String, Value>() {
            let (key, def) = pair.map_err(|e| format!("mode.params: {e}"))?;
            let Value::Table(def) = def else {
                return Err(format!("param '{key}' must be built with number(), bool(), choice() or button_param()"));
            };
            params.push(library::parse_param(&key, &def)?);
        }
    }
    params.sort_by_key(|(order, _)| *order);
    let main_params = match declared.get::<Option<Table>>("main_params").map_err(|e| format!("mode.main_params: {e}"))? {
        Some(t) => t.sequence_values::<String>().collect::<mlua::Result<Vec<_>>>().map_err(|e| format!("mode.main_params: {e}"))?,
        None => Vec::new(),
    };
    if let Some(unknown) = main_params.iter().find(|name| !params.iter().any(|(_, def)| def.name == **name)) {
        return Err(format!("mode.main_params: no parameter named '{unknown}'"));
    }
    let mut feedback = Vec::new();
    if let Some(table) = declared.get::<Option<Table>>("feedback").map_err(|e| format!("mode.feedback: {e}"))? {
        for pair in table.pairs::<String, Value>() {
            let (key, def) = pair.map_err(|e| format!("mode.feedback: {e}"))?;
            let Value::Table(def) = def else {
                return Err(format!("feedback '{key}' must be built with ask()"));
            };
            let (order, question) = library::parse_question(&key, &def)?;
            if let Some(param) = &question.param {
                let def = params.iter().map(|(_, d)| d).find(|d| d.name == *param);
                if !matches!(def.map(|d| &d.kind), Some(ParamKind::Number { .. })) {
                    return Err(format!("feedback '{key}': param '{param}' must be a number() parameter"));
                }
            }
            feedback.push((order, question));
        }
    }
    feedback.sort_by_key(|(order, _)| *order);
    let number = |key: &str, default: f64| -> LoadResult<f64> {
        Ok(declared.get::<Option<f64>>(key).map_err(|e| format!("mode.{key}: {e}"))?.unwrap_or(default))
    };
    let mut scenes = Vec::new();
    if let Some(table) = declared.get::<Option<Table>>("scenes").map_err(|e| format!("mode.scenes: {e}"))? {
        for pair in table.pairs::<String, Value>() {
            let (name, def) = pair.map_err(|e| format!("mode.scenes: {e}"))?;
            let Value::Table(def) = def else {
                return Err(format!("mode.scenes: scene '{name}' must be a table: {{ sound = \"...\", screen = \"...\" }}"));
            };
            let text = |key: &str| -> LoadResult<Option<String>> {
                let value = def.get::<Option<String>>(key).map_err(|e| format!("mode.scenes.{name}.{key}: {e}"))?;
                Ok(value.filter(|v| !v.trim().is_empty()))
            };
            let (sound, screen) = (text("sound")?, text("screen")?);
            if sound.is_none() && screen.is_none() {
                return Err(format!("mode.scenes: scene '{name}' needs a sound or a screen description"));
            }
            scenes.push(SceneDecl { name, sound, screen, zone: None, hold: 0.0 });
        }
        scenes.sort_by(|a, b| a.name.cmp(&b.name));
        if !(2..=MAX_SCENES).contains(&scenes.len()) {
            return Err(format!("mode.scenes must declare 2 to {MAX_SCENES} scenes"));
        }
    }
    let scene_window = number("scene_window", scenes::DEFAULT_WINDOW)?;
    if !(2.0..=60.0).contains(&scene_window) {
        return Err("mode.scene_window must be between 2 and 60 seconds".into());
    }

    Ok(ModeInfo {
        name,
        description: get_str("description")?.unwrap_or_default(),
        category: get_str("category")?.unwrap_or_default(),
        help: get_str("help")?.unwrap_or_default(),
        main_params,
        author: get_str("author")?.unwrap_or_default(),
        version: get_str("version")?.unwrap_or_default(),
        channels,
        params: params.into_iter().map(|(_, def)| def).collect(),
        feedback: feedback.into_iter().map(|(_, q)| q).collect(),
        rumble_threshold: number("rumble_threshold", DEFAULT_THRESHOLD)?,
        rumble_release: number("rumble_release", DEFAULT_RELEASE)?,
        scenes,
        scene_window,
    })
}

fn lua_to_persist(value: &Value, depth: usize) -> Option<PersistValue> {
    match value {
        Value::Boolean(b) => Some(PersistValue::Bool(*b)),
        Value::Integer(i) => Some(PersistValue::Number(*i as f64)),
        Value::Number(n) => Some(PersistValue::Number(*n)),
        Value::String(s) => Some(PersistValue::Text(s.to_string_lossy())),
        Value::Table(t) if depth < PERSIST_MAX_DEPTH => {
            let entries = t
                .pairs::<Value, Value>()
                .filter_map(Result::ok)
                .filter_map(|(k, v)| Some((lua_to_persist(&k, depth + 1)?, lua_to_persist(&v, depth + 1)?)))
                .collect();
            Some(PersistValue::Table(entries))
        }
        _ => None,
    }
}

fn persist_to_lua(lua: &Lua, value: &PersistValue) -> mlua::Result<Value> {
    Ok(match value {
        PersistValue::Bool(b) => Value::Boolean(*b),
        PersistValue::Number(n) => Value::Number(*n),
        PersistValue::Text(s) => Value::String(lua.create_string(s)?),
        PersistValue::Table(entries) => {
            let t = lua.create_table()?;
            for (k, v) in entries {
                t.raw_set(persist_to_lua(lua, k)?, persist_to_lua(lua, v)?)?;
            }
            Value::Table(t)
        }
    })
}

/// Formats Lua values for `log` / `print`.
fn display(values: &Variadic<Value>) -> String {
    values
        .iter()
        .map(|v| match v {
            Value::String(s) => s.to_string_lossy(),
            Value::Nil => "nil".into(),
            Value::Boolean(b) => b.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Number(n) => format!("{n}"),
            other => format!("<{}>", other.type_name()),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
