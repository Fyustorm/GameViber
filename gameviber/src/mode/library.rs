//! Global functions exposed to mode scripts (see docs/spec-modes.md).

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Function, Lua, Table, Value, Variadic};

use super::{display, Ctx, HudGauge, ParamDef, ParamKind, ParamValue, Question, Timer};
use crate::gamepad::BUTTONS;

const PARAM_TAG: &str = "__param";
const ORDER_TAG: &str = "__order";
const PATTERN_TAG: &str = "__pattern";
const QUESTION_TAG: &str = "__question";
const DEFAULT_CHANNEL: &str = "main";
const MAX_HUD_GAUGES: usize = 4;
const MAX_HUD_EVENT_CHARS: usize = 40;
/// What a mode may keep outside its Lua memory (docs/spec-modes.md §10): its
/// callbacks run from code shared by everyone, so they are bounded too.
const MAX_HUD_EVENTS: usize = 4;
const MAX_HUD_LABEL_CHARS: usize = 24;
const MAX_TIMERS: usize = 64;
const MAX_PLOTS: usize = 32;
const MAX_PLOT_NAME_CHARS: usize = 40;
const MAX_PATTERN_POINTS: usize = 256;
/// Log lines per second of the mode's time, and characters per line.
const MAX_LOG_LINES: u32 = 20;
const MAX_LOG_CHARS: usize = 500;

/// Base-library functions that could escape the sandbox or load code.
const REMOVED_GLOBALS: [&str; 5] = ["loadstring", "getfenv", "setfenv", "require", "dofile"];

fn runtime_err(msg: impl Into<String>) -> mlua::Error {
    mlua::Error::runtime(msg.into())
}

pub(super) fn register(lua: &Lua, ctx: &Rc<RefCell<Ctx>>) -> mlua::Result<()> {
    let g = lua.globals();
    for name in REMOVED_GLOBALS {
        g.raw_set(name, Value::Nil)?;
    }

    // --- declaration
    {
        let ctx = ctx.clone();
        g.set(
            "mode",
            lua.create_function(move |_, decl: Table| {
                let mut ctx = ctx.borrow_mut();
                if ctx.declared.is_some() || ctx.outputs.is_some() {
                    return Err(runtime_err("mode { ... } must be called exactly once, at the top level"));
                }
                ctx.declared = Some(decl);
                Ok(())
            })?,
        )?;
    }
    let param_ctor = |kind: &'static str| {
        let ctx = ctx.clone();
        lua.create_function(move |lua, args: Variadic<Value>| {
            let t = lua.create_table()?;
            t.raw_set(PARAM_TAG, kind)?;
            let mut ctx = ctx.borrow_mut();
            ctx.param_seq += 1;
            t.raw_set(ORDER_TAG, ctx.param_seq)?;
            let fields: &[&str] = match kind {
                "number" => &["default", "min", "max", "label", "step"],
                "choice" => &["default", "options", "label"],
                _ => &["default", "label"],
            };
            for (field, value) in fields.iter().zip(args.iter()) {
                t.raw_set(*field, value.clone())?;
            }
            Ok(t)
        })
    };
    g.set("number", param_ctor("number")?)?;
    g.set("bool", param_ctor("bool")?)?;
    g.set("choice", param_ctor("choice")?)?;
    g.set("button_param", param_ctor("button")?)?;
    {
        let ctx = ctx.clone();
        g.set(
            "ask",
            lua.create_function(move |lua, (label, options, default, opts): (String, Option<Table>, Option<String>, Option<Table>)| {
                let t = lua.create_table()?;
                t.raw_set(QUESTION_TAG, true)?;
                let mut ctx = ctx.borrow_mut();
                ctx.param_seq += 1;
                t.raw_set(ORDER_TAG, ctx.param_seq)?;
                t.raw_set("label", label)?;
                t.raw_set("options", options)?;
                t.raw_set("default", default)?;
                t.raw_set("opts", opts)?;
                Ok(t)
            })?,
        )?;
    }

    // --- outputs
    {
        let ctx = ctx.clone();
        g.set(
            "set",
            lua.create_function(move |_, (level, channel): (f64, Option<String>)| {
                let channel = channel.unwrap_or_else(|| DEFAULT_CHANNEL.into());
                ctx.borrow_mut().outputs()?.set(&channel, level).map_err(runtime_err)
            })?,
        )?;
    }
    {
        let ctx = ctx.clone();
        g.set(
            "pulse",
            lua.create_function(move |_, (level, seconds, channel): (f64, f64, Option<String>)| {
                let channel = channel.unwrap_or_else(|| DEFAULT_CHANNEL.into());
                let mut ctx = ctx.borrow_mut();
                let now = ctx.time;
                ctx.outputs()?.pulse(&channel, level, seconds, now).map_err(runtime_err)
            })?,
        )?;
    }
    g.set(
        "pattern",
        lua.create_function(|lua, points: Table| {
            let mut copy = Vec::new();
            for point in points.sequence_values::<Table>() {
                let point = point?;
                let (t, v): (f64, f64) = (point.get(1)?, point.get(2)?);
                if t < 0.0 {
                    return Err(runtime_err("pattern times must be >= 0"));
                }
                copy.push(lua.create_sequence_from([t, v])?);
                if copy.len() > MAX_PATTERN_POINTS {
                    return Err(runtime_err(format!("pattern: at most {MAX_PATTERN_POINTS} points")));
                }
            }
            if copy.is_empty() {
                return Err(runtime_err("pattern needs at least one { time, intensity } point"));
            }
            let pattern = lua.create_table()?;
            pattern.raw_set(PATTERN_TAG, true)?;
            pattern.raw_set("points", lua.create_sequence_from(copy)?)?;
            Ok(pattern)
        })?,
    )?;
    {
        let ctx = ctx.clone();
        g.set(
            "play",
            lua.create_function(move |lua, (pattern, opts): (Table, Option<Table>)| {
                if !pattern.raw_get::<bool>(PATTERN_TAG).unwrap_or(false) {
                    return Err(runtime_err("play() expects a value built with pattern { ... }"));
                }
                let mut points = Vec::new();
                for point in pattern.raw_get::<Table>("points")?.sequence_values::<Table>() {
                    let point = point?;
                    points.push((point.get::<f64>(1)?, point.get::<f64>(2)?));
                }
                let (channel, loops, scale) = match &opts {
                    Some(o) => (
                        o.get::<Option<String>>("channel")?.unwrap_or_else(|| DEFAULT_CHANNEL.into()),
                        o.get::<Option<u32>>("loops")?.unwrap_or(1),
                        o.get::<Option<f64>>("scale")?.unwrap_or(1.0),
                    ),
                    None => (DEFAULT_CHANNEL.into(), 1, 1.0),
                };
                let id = {
                    let mut c = ctx.borrow_mut();
                    let now = c.time;
                    c.outputs()?.play(&channel, points, loops, scale, now).map_err(runtime_err)?
                };
                let handle = lua.create_table()?;
                let ctx = ctx.clone();
                handle.raw_set(
                    "stop",
                    lua.create_function(move |_, _: Variadic<Value>| {
                        ctx.borrow_mut().outputs()?.stop_pattern(id);
                        Ok(())
                    })?,
                )?;
                Ok(handle)
            })?,
        )?;
    }
    {
        let ctx = ctx.clone();
        g.set(
            "stop_all",
            lua.create_function(move |_, ()| {
                ctx.borrow_mut().outputs()?.stop_all();
                Ok(())
            })?,
        )?;
    }

    // --- timers
    for (name, repeating) in [("after", false), ("every", true)] {
        let ctx = ctx.clone();
        g.set(
            name,
            lua.create_function(move |lua, (seconds, func): (f64, Function)| {
                let id = {
                    let mut c = ctx.borrow_mut();
                    if c.timers.len() >= MAX_TIMERS {
                        return Err(runtime_err(format!("{name}: at most {MAX_TIMERS} timers at once")));
                    }
                    let id = c.next_timer;
                    c.next_timer += 1;
                    let seconds = seconds.max(0.0);
                    let due = c.time + seconds;
                    c.timers.push(Timer { id, due, period: repeating.then_some(seconds), func });
                    id
                };
                let handle = lua.create_table()?;
                let ctx = ctx.clone();
                handle.raw_set(
                    "cancel",
                    lua.create_function(move |_, _: Variadic<Value>| {
                        let mut c = ctx.borrow_mut();
                        c.timers.retain(|t| t.id != id);
                        c.cancelled.insert(id);
                        Ok(())
                    })?,
                )?;
                Ok(handle)
            })?,
        )?;
    }

    // --- debug helpers
    {
        let ctx = ctx.clone();
        g.set(
            "plot",
            lua.create_function(move |_, (name, value): (String, f64)| {
                let name: String = name.chars().take(MAX_PLOT_NAME_CHARS).collect();
                let mut c = ctx.borrow_mut();
                if !c.plot_names.contains(&name) {
                    if c.plot_names.len() >= MAX_PLOTS {
                        return Err(runtime_err(format!("plot: at most {MAX_PLOTS} series")));
                    }
                    c.plot_names.insert(name.clone());
                }
                c.plots.push((name, value));
                Ok(())
            })?,
        )?;
    }

    // --- in-game overlay
    {
        let ctx = ctx.clone();
        g.set(
            "hud",
            lua.create_function(move |_, (label, value, max): (String, Option<f64>, Option<f64>)| {
                let label: String = label.chars().take(MAX_HUD_LABEL_CHARS).collect();
                let mut c = ctx.borrow_mut();
                let Some(value) = value else {
                    c.hud.retain(|h| h.label != label);
                    return Ok(());
                };
                let max = max.unwrap_or(1.0);
                if !(max > 0.0 && max.is_finite() && value.is_finite()) {
                    return Err(runtime_err("hud: value must be a number and max a positive number"));
                }
                let full = c.hud.len() >= MAX_HUD_GAUGES;
                match c.hud.iter_mut().find(|h| h.label == label) {
                    Some(h) => {
                        h.value = value;
                        h.max = max;
                    }
                    None if full => return Err(runtime_err(format!("hud: at most {MAX_HUD_GAUGES} gauges"))),
                    None => c.hud.push(HudGauge { label, value, max }),
                }
                Ok(())
            })?,
        )?;
    }
    {
        let ctx = ctx.clone();
        g.set(
            "hud_event",
            lua.create_function(move |_, text: String| {
                let text: String = text.chars().take(MAX_HUD_EVENT_CHARS).collect();
                let mut c = ctx.borrow_mut();
                // More in one tick could not be read anyway.
                if c.hud_events.len() < MAX_HUD_EVENTS {
                    c.hud_events.push(text);
                }
                Ok(())
            })?,
        )?;
    }
    for name in ["log", "print"] {
        let ctx = ctx.clone();
        g.set(
            name,
            lua.create_function(move |_, values: Variadic<Value>| {
                let mut c = ctx.borrow_mut();
                let second = c.time.floor();
                if c.log_window.0 != second {
                    c.log_window = (second, 0);
                }
                c.log_window.1 += 1;
                match c.log_window.1 {
                    n if n <= MAX_LOG_LINES => {
                        let line: String = display(&values).chars().take(MAX_LOG_CHARS).collect();
                        log::info!(target: "mode", "[{}] {line}", c.mode_name);
                    }
                    n if n == MAX_LOG_LINES + 1 => {
                        log::info!(target: "mode", "[{}] more than {MAX_LOG_LINES} lines this second: the rest is left out", c.mode_name);
                    }
                    _ => {}
                }
                Ok(())
            })?,
        )?;
    }

    // --- maths
    g.set("clamp", lua.create_function(|_, (x, a, b): (f64, f64, f64)| Ok(x.max(a).min(b)))?)?;
    g.set("lerp", lua.create_function(|_, (a, b, t): (f64, f64, f64)| Ok(a + (b - a) * t))?)?;
    g.set(
        "map",
        lua.create_function(|_, (x, a1, b1, a2, b2): (f64, f64, f64, f64, f64)| {
            Ok(if b1 == a1 { a2 } else { a2 + (x - a1) * (b2 - a2) / (b1 - a1) })
        })?,
    )?;
    {
        let ctx = ctx.clone();
        g.set(
            "random",
            lua.create_function(move |_, (a, b): (Option<f64>, Option<f64>)| {
                let r = ctx.borrow_mut().random();
                Ok(match (a, b) {
                    (Some(a), Some(b)) => a + (b - a) * r,
                    (Some(b), None) => b * r,
                    _ => r,
                })
            })?,
        )?;
    }
    Ok(())
}

/// Parses a `feedback` entry built by ask().
pub(super) fn parse_question(id: &str, def: &Table) -> Result<(u64, Question), String> {
    let err = |e: mlua::Error| format!("feedback '{id}': {e}");
    if !def.raw_get::<Option<bool>>(QUESTION_TAG).map_err(err)?.unwrap_or(false) {
        return Err(format!("feedback '{id}' must be built with ask()"));
    }
    let order: u64 = def.raw_get(ORDER_TAG).map_err(err)?;
    let label: String = def.raw_get("label").map_err(err)?;
    let options: Vec<String> = match def.raw_get::<Option<Table>>("options").map_err(err)? {
        Some(t) => t.sequence_values::<String>().collect::<mlua::Result<_>>().map_err(err)?,
        None => Vec::new(),
    };
    if options.len() == 1 {
        return Err(format!("feedback '{id}': give at least 2 answers, or none for a checkbox"));
    }
    let default = match def.raw_get::<Option<String>>("default").map_err(err)? {
        Some(d) => options
            .iter()
            .position(|o| *o == d)
            .ok_or_else(|| format!("feedback '{id}': default '{d}' is not one of the answers"))?,
        None => options.len().saturating_sub(1) / 2,
    };
    let (param, invert) = match def.raw_get::<Option<Table>>("opts").map_err(err)? {
        Some(o) => (
            o.get::<Option<String>>("param").map_err(err)?,
            o.get::<Option<bool>>("invert").map_err(err)?.unwrap_or(false),
        ),
        None => (None, false),
    };
    if param.is_some() && options.is_empty() {
        return Err(format!("feedback '{id}': a checkbox cannot adjust a parameter"));
    }
    Ok((order, Question { id: id.to_owned(), label, options, default, param, invert }))
}

/// Parses a `params` entry built by number / bool / choice / button_param.
pub(super) fn parse_param(name: &str, def: &Table) -> Result<(u64, ParamDef), String> {
    let err = |e: mlua::Error| format!("param '{name}': {e}");
    let kind: String = def.raw_get::<Option<String>>(PARAM_TAG).map_err(err)?.ok_or_else(|| {
        format!("param '{name}' must be built with number(), bool(), choice() or button_param()")
    })?;
    let order: u64 = def.raw_get(ORDER_TAG).map_err(err)?;
    let label = def.raw_get::<Option<String>>("label").map_err(err)?.unwrap_or_else(|| name.to_owned());
    let (kind, default) = match kind.as_str() {
        "number" => {
            let default: f64 = def.raw_get("default").map_err(err)?;
            let min: f64 = def.raw_get("min").map_err(err)?;
            let max: f64 = def.raw_get("max").map_err(err)?;
            if min > max {
                return Err(format!("param '{name}': min > max"));
            }
            let step = def.raw_get::<Option<f64>>("step").map_err(err)?;
            (ParamKind::Number { min, max, step }, ParamValue::Number(default.clamp(min, max)))
        }
        "bool" => (ParamKind::Bool, ParamValue::Bool(def.raw_get("default").map_err(err)?)),
        "choice" => {
            let options: Vec<String> = def
                .raw_get::<Table>("options")
                .map_err(err)?
                .sequence_values::<String>()
                .collect::<mlua::Result<_>>()
                .map_err(err)?;
            let default: String = def.raw_get("default").map_err(err)?;
            if !options.contains(&default) {
                return Err(format!("param '{name}': default '{default}' is not one of the options"));
            }
            (ParamKind::Choice(options), ParamValue::Text(default))
        }
        "button" => {
            let default: String = def.raw_get("default").map_err(err)?;
            if !BUTTONS.contains(&default.as_str()) {
                return Err(format!("param '{name}': unknown button '{default}'"));
            }
            (ParamKind::Button, ParamValue::Text(default))
        }
        other => return Err(format!("param '{name}': unknown type '{other}'")),
    };
    Ok((order, ParamDef { name: name.to_owned(), label, kind, default }))
}
