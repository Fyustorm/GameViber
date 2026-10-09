use super::*;
use crate::audio::AudioHit;
use crate::mode::phases::{Ignored, PhaseDecl, Sense};
use crate::mode::IndicatorValue;
use crate::screen::ScreenLevels;
use crate::package::{IndicatorKind, Inputs, PhaseDef, Zone};

const SIMPLE: &str = include_str!("../../modes/simple.luau");
const ACCUMULATION: &str = include_str!("../../modes/accumulation.luau");
const COMBO: &str = include_str!("../../modes/combo.luau");
const OVERHEAT: &str = include_str!("../../modes/overheat.luau");
const TENSION: &str = include_str!("../../modes/tension.luau");
const ENGINE: &str = include_str!("../../modes/engine.luau");
const HEARTBEAT: &str = include_str!("../../modes/heartbeat.luau");
const ALL_OR_NOTHING: &str = include_str!("../../modes/all_or_nothing.luau");
const AMBIENT: &str = include_str!("../../modes/ambient.luau");
const SURGE: &str = include_str!("../../modes/surge.luau");
const DT: f64 = 0.02;

fn load(source: &str) -> ModeRuntime {
    let mut rt = ModeRuntime::load("test", source, &BTreeMap::new(), None).expect("load");
    rt.start().expect("start");
    rt
}

fn load_err(source: &str) -> String {
    match ModeRuntime::load("test", source, &BTreeMap::new(), None) {
        Ok(_) => panic!("expected a load error"),
        Err(e) => e,
    }
}

fn rumble(strong: f64, weak: f64) -> RumbleLevels {
    RumbleLevels { strong, weak }
}

/// Ticks with no player input for a long time.
fn step(rt: &mut ModeRuntime, r: RumbleLevels) -> TickOutput {
    rt.step(DT, r, &PadState::default(), 1e9, &[]).expect("step")
}

fn main_out(rt: &mut ModeRuntime, r: RumbleLevels) -> f64 {
    step(rt, r).channels["main"]
}

fn plot_value(out: &TickOutput, name: &str) -> f64 {
    out.plots.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no plot '{name}'")).1
}

/// Steps `secs` seconds with the given rumble, pad and input idle time.
fn run(rt: &mut ModeRuntime, secs: f64, r: RumbleLevels, pad: &PadState, input_idle: f64) -> TickOutput {
    let mut out = TickOutput::default();
    for _ in 0..(secs / DT).round() as usize {
        out = rt.step(DT, r, pad, input_idle, &[]).expect("step");
    }
    out
}

fn press(rt: &mut ModeRuntime, name: &'static str) -> TickOutput {
    let ev = ModeEvent::Button(ButtonEvent { name, pressed: true });
    rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 0.0, &[ev]).expect("step")
}

fn wrap(body: &str) -> String {
    format!("mode {{ api = 1, name = 'T', channels = {{ 'main', 'aux' }} }}\n{body}")
}

#[test]
fn simple_mode_matches_ghr_formula() {
    let mut rt = load(SIMPLE);
    assert_eq!(rt.info().name, "Simple");
    let names: Vec<_> = rt.info().params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["combine", "multiplier", "baseline"], "params keep declaration order");
    assert!((main_out(&mut rt, rumble(0.5, 0.25)) - 0.375).abs() < 1e-9);
    rt.set_param("combine", &ParamValue::Text("max".into())).unwrap();
    rt.set_param("multiplier", &ParamValue::Number(2.0)).unwrap();
    assert_eq!(main_out(&mut rt, rumble(0.5, 0.25)), 1.0);
    assert_eq!(main_out(&mut rt, rumble(0.0, 0.0)), 0.0);
}

#[test]
fn saved_params_are_applied_and_validated() {
    let saved = BTreeMap::from([
        ("multiplier".to_owned(), ParamValue::Number(99.0)),
        ("combine".to_owned(), ParamValue::Text("bogus".into())),
    ]);
    let rt = ModeRuntime::load("test", SIMPLE, &saved, None).unwrap();
    assert_eq!(rt.param_values()["multiplier"], ParamValue::Number(5.0), "clamped to max");
    assert_eq!(rt.param_values()["combine"], ParamValue::Text("avg".into()), "invalid -> default");
}

#[test]
fn accumulation_gains_points_and_drains_when_idle() {
    let mut rt = load(ACCUMULATION);
    // One vibration: +5 points, output = rumble level * 5%.
    let out = step(&mut rt, rumble(1.0, 0.0));
    assert!((out.channels["main"] - 0.05).abs() < 1e-9);
    assert!(out.plots.iter().any(|(n, v)| n == "points" && *v == 5.0));
    // Bonus button: +1 point.
    let press = ModeEvent::Button(ButtonEvent { name: "A", pressed: true });
    let out = rt.step(DT, rumble(1.0, 0.0), &PadState::default(), 0.0, &[press]).unwrap();
    assert!(out.plots.iter().any(|(n, v)| n == "points" && *v == 6.0));
    // Long idle: points drain back to 0.
    for _ in 0..500 {
        step(&mut rt, rumble(0.0, 0.0));
    }
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert!(out.plots.iter().any(|(n, v)| n == "points" && *v == 0.0));
}

#[test]
fn persist_survives_reload() {
    let rt = load(ACCUMULATION);
    let mut rt = rt;
    step(&mut rt, rumble(1.0, 0.0));
    let persist = rt.persist_snapshot();
    let mut reloaded = ModeRuntime::load("test", ACCUMULATION, &BTreeMap::new(), persist.as_ref()).unwrap();
    reloaded.start().unwrap();
    let out = step(&mut reloaded, rumble(1.0, 0.0));
    assert!(out.plots.iter().any(|(n, v)| n == "points" && *v == 5.0), "{:?}", out.plots);
}

#[test]
fn rumble_callbacks_receive_start_and_end() {
    let src = wrap(
        "starts, ends, peak = 0, 0, 0
         function on_rumble_start(ev) starts += 1 end
         function on_rumble_end(ev) ends += 1; peak = ev.peak end
         function tick(dt, input) plot('s', starts); plot('e', ends); plot('p', peak) end",
    );
    let mut rt = load(&src);
    step(&mut rt, rumble(0.6, 0.0));
    step(&mut rt, rumble(0.9, 0.0));
    for _ in 0..10 {
        step(&mut rt, rumble(0.0, 0.0));
    }
    let out = step(&mut rt, rumble(0.0, 0.0));
    let get = |name: &str| out.plots.iter().find(|(n, _)| n == name).unwrap().1;
    assert_eq!((get("s"), get("e"), get("p")), (1.0, 1.0, 0.9));
}

#[test]
fn input_table_exposes_buttons_axes_and_idle() {
    let src = wrap(
        "function tick(dt, input)
           plot('a', input.buttons.A and 1 or 0)
           plot('lx', input.axes.LX)
           plot('time', input.time)
         end",
    );
    let mut rt = load(&src);
    let mut pad = PadState::default();
    pad.button("A", true, 0.0);
    pad.axis(crate::gamepad::codes::ABS_X, 0.5, 0.0);
    let out = rt.step(DT, rumble(0.0, 0.0), &pad, 0.0, &[]).unwrap();
    let get = |name: &str| out.plots.iter().find(|(n, _)| n == name).unwrap().1;
    assert_eq!((get("a"), get("lx")), (1.0, 0.5));
    assert!((get("time") - DT).abs() < 1e-9);
}

#[test]
fn pulses_patterns_and_channels() {
    let src = wrap(
        "local h
         function on_start()
           set(0.1, 'aux')
           pulse(0.8, 0.05)
           h = play(pattern { { 0, 0.5 }, { 1, 0.5 } }, { channel = 'aux', loops = 0 })
         end
         function tick(dt, input)
           if input.time > 0.1 then h:stop() end
         end",
    );
    let mut rt = load(&src);
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert_eq!((out.channels["main"], out.channels["aux"]), (0.8, 0.5));
    for _ in 0..5 {
        step(&mut rt, rumble(0.0, 0.0));
    }
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert_eq!((out.channels["main"], out.channels["aux"]), (0.0, 0.1));
}

#[test]
fn strokes_and_thrusts_for_strokers() {
    let mut rt = load(&wrap(
        "local n = 0\n\
         function tick(dt, input)\n\
           n = n + 1\n\
           if n == 1 then stroke(0.6, 0.3); stroke(0.9, nil, 'aux') end\n\
           if n == 2 then thrust(0.8, 0.5, 'aux') end\n\
           if n == 3 then stroke(nil) end\n\
         end",
    ));
    let out = step(&mut rt, rumble(0.0, 0.0));
    // Felt as their speed by every toy; the length defaults to the whole range.
    assert_eq!((out.channels["main"], out.channels["aux"]), (0.6, 0.9));
    assert_eq!(out.strokes["main"], outputs::StrokeIntent { speed: 0.6, length: 0.3 });
    assert_eq!(out.strokes["aux"].length, 1.0);
    assert!(out.thrusts.is_empty());
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert_eq!(out.thrusts, vec![("aux".to_owned(), outputs::ThrustIntent { length: 0.8, seconds: 0.5 })]);
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert!(out.thrusts.is_empty());
    assert!(!out.strokes.contains_key("main") && out.channels["main"] == 0.0);
    assert!(rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &[]).is_ok());
    let mut bad = load(&wrap("function tick() stroke(0.5, 1, 'nope') end"));
    assert!(bad.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &[]).unwrap_err().contains("unknown channel"));
}

#[test]
fn motions_from_funscripts_and_scripts() {
    let track = crate::funscript::Track::new(vec![(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)]).unwrap();
    let funscripts = crate::funscript::Funscripts::from([("hit".to_owned(), std::sync::Arc::new(track))]);
    let source = wrap(
        "local hit = funscript('hit')\n\
         local wave = motion { { 0, 0.5 }, { 2, 1 } }\n\
         local n = 0\n\
         function tick(dt, input)\n\
           n = n + 1\n\
           if n == 1 then plot('d', hit.duration + wave.duration); play(hit, { speed = 2, depth = 0.5 }) end\n\
           if n == 2 then h = play(wave, { channel = 'aux', loops = 0 }) end\n\
           if n == 3 then h:stop() end\n\
         end",
    );
    let mut rt = ModeRuntime::load_with("test", &source, &BTreeMap::new(), None, funscripts).expect("load");
    rt.start().unwrap();
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert_eq!(plot_value(&out, "d"), 3.0);
    let main = &out.motions["main"];
    assert_eq!((main.rate, main.depth, main.center), (2.0, 0.5, 0.5));
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert!(out.motions.contains_key("aux") && out.motions.contains_key("main"));
    // Felt by other toys as how fast it moves: 2 lengths a second, twice as fast, half as deep.
    assert!(out.channels["main"] > 0.5);
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert!(!out.motions.contains_key("aux"));
    // A funscript the package lacks is an error at load, as a motion that is not one.
    let missing = ModeRuntime::load("test", &wrap("local m = funscript('nope')"), &BTreeMap::new(), None).err().unwrap();
    assert!(missing.contains("no funscript 'nope'"), "{missing}");
    assert!(load_err(&wrap("local m = motion { { 0, 0 } }")).contains("at least two points"));
}

#[test]
fn timers_after_every_and_cancel() {
    let src = wrap(
        "count, once = 0, 0
         local h
         function on_start()
           h = every(0.04, function() count += 1 end)
           after(0.05, function() once += 1 end)
         end
         function tick(dt, input)
           if count >= 3 then h:cancel() end
           plot('count', count); plot('once', once)
         end",
    );
    let mut rt = load(&src);
    let mut last = TickOutput::default();
    for _ in 0..50 {
        last = step(&mut rt, rumble(0.0, 0.0));
    }
    let get = |name: &str| last.plots.iter().find(|(n, _)| n == name).unwrap().1;
    assert_eq!((get("count"), get("once")), (3.0, 1.0));
}

#[test]
fn params_are_read_only() {
    let src = "mode { api = 1, name = 'T', params = { x = number(1, 0, 2, 'X') } }
               function tick() P.x = 2 end";
    let mut rt = load(src);
    let err = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 0.0, &[]).unwrap_err();
    assert!(err.contains("read-only"), "{err}");
}

#[test]
fn param_changed_callback_is_called() {
    let src = "mode { api = 1, name = 'T', params = { x = number(1, 0, 2, 'X') } }
               seen = 0
               function on_param_changed(name, value) seen = value end
               function tick() plot('seen', seen) end";
    let mut rt = load(src);
    rt.set_param("x", &ParamValue::Number(1.5)).unwrap();
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert_eq!(out.plots[0].1, 1.5);
    assert!(rt.set_param("nope", &ParamValue::Number(1.0)).is_err());
}

#[test]
fn load_errors_are_reported() {
    assert!(load_err("function tick() end").contains("mode { ... } was never called"));
    assert!(load_err("mode { api = 1, name = 'T' }").contains("tick"));
    assert!(load_err("mode { api = 2, name = 'T' } function tick() end").contains("api"));
    let syntax = load_err("mode { api = 1, name = 'T' ");
    assert!(syntax.contains("test:"), "syntax errors carry location: {syntax}");
    assert!(load_err("mode { api = 1, name = 'T', params = { x = 3 } } function tick() end").contains("param 'x'"));
    assert!(load_err("mode { api = 1, name = 'T', params = { b = button_param('Z', '') } } function tick() end")
        .contains("unknown button"));
}

#[test]
fn runtime_errors_carry_location() {
    let mut rt = load(&wrap("function tick() set(0.5, 'nope') end"));
    let err = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 0.0, &[]).unwrap_err();
    assert!(err.contains("unknown channel"), "{err}");
}

#[test]
fn infinite_loop_hits_time_budget() {
    let mut rt = load(&wrap("function tick() while true do end end"));
    let err = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 0.0, &[]).unwrap_err();
    assert!(err.contains("time budget"), "{err}");
    assert!(load_err("while true do end").contains("time budget"));
}

#[test]
fn a_tick_has_a_budget_for_all_its_calls() {
    let mut rt = load(&wrap("function tick() for i = 1, 1000 do end end"));
    assert!(rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 0.0, &[]).is_ok());
    // Earlier calls of the tick took it all.
    rt.tick_deadline.set(Some(Instant::now()));
    let err = rt.run_tick(DT, rumble(0.0, 0.0), &PadState::default(), 0.0, &[]).unwrap_err();
    assert!(err.contains("time budget"), "{err}");
}

#[test]
fn what_a_mode_keeps_outside_lua_is_bounded() {
    let err = |body: &str| {
        let mut rt = load(&wrap(&format!("function tick() {body} end")));
        rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 0.0, &[]).unwrap_err()
    };
    assert!(err("for i = 1, 65 do every(1, function() end) end").contains("at most 64 timers"));
    assert!(err("for i = 1, 33 do plot('p' .. i, i) end").contains("at most 32 series"));
    assert!(err("for i = 1, 65 do pulse(1, 10) end").contains("at most 64 at once"));
    assert!(err("local p = pattern { { 0, 1 } } for i = 1, 17 do play(p, { loops = 0 }) end").contains("at most 16 patterns"));
    let points: Vec<String> = (0..257).map(|i| format!("{{ {i}, 1 }}")).collect();
    assert!(err(&format!("pattern {{ {} }}", points.join(", "))).contains("at most 256 points"));

    // Plotting the same series again is fine; overlay messages past a few a tick are dropped.
    let mut rt = load(&wrap("function tick() for i = 1, 100 do plot('same', i) hud_event('x' .. i) end end"));
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert_eq!(out.hud_events, ["x1", "x2", "x3", "x4"]);
}

#[test]
fn sandbox_blocks_escapes() {
    let src = wrap(
        "function tick()
           plot('io', io == nil and 1 or 0)
           plot('os', os == nil and 1 or 0)
           plot('loadstring', loadstring == nil and 1 or 0)
           plot('require', require == nil and 1 or 0)
           plot('debug', debug == nil and 1 or 0)
         end",
    );
    let mut rt = load(&src);
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert!(out.plots.iter().all(|(_, v)| *v == 1.0), "{:?}", out.plots);
    assert!(load_err(&wrap("math.floor = nil function tick() end")).contains("readonly"));
}

#[test]
fn memory_limit_is_enforced() {
    let mut rt = load(&wrap(
        "local t = {}
         function tick() for i = 1, 100000 do t[#t + 1] = string.rep('x', 1000) end end",
    ));
    let mut failed = false;
    for _ in 0..40 {
        if let Err(e) = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 0.0, &[]) {
            assert!(e.contains("memory") || e.contains("time budget"), "{e}");
            failed = true;
            break;
        }
    }
    assert!(failed);
}

#[test]
fn stop_resets_outputs() {
    let mut rt = load(&wrap("function tick() end function on_start() set(0.7) end"));
    assert_eq!(main_out(&mut rt, rumble(0.0, 0.0)), 0.7);
    rt.stop().unwrap();
    assert_eq!(main_out(&mut rt, rumble(0.0, 0.0)), 0.0);
}

#[test]
fn builtin_modes_load_and_run() {
    let mut pad = PadState::default();
    pad.axis(crate::gamepad::codes::ABS_RZ, 1.0, 0.0);
    pad.button("RB", true, 0.0);
    for src in [COMBO, OVERHEAT, TENSION, ENGINE, HEARTBEAT, ALL_OR_NOTHING, AMBIENT, SURGE] {
        let mut rt = load(src);
        for i in 0..1000 {
            let level = if i % 50 < 10 { 0.9 } else { 0.0 };
            let out = rt.step(DT, rumble(level, 0.2), &pad, (i % 300) as f64, &[]).expect("step");
            let v = out.channels["main"];
            assert!((0.0..=1.0).contains(&v), "{}: output {v}", rt.info().name);
        }
    }
}

#[test]
fn builtin_modes_describe_themselves_for_players() {
    for src in [SIMPLE, ACCUMULATION, COMBO, OVERHEAT, TENSION, ENGINE, HEARTBEAT, ALL_OR_NOTHING, AMBIENT, SURGE] {
        let info = ModeRuntime::probe("test", src).expect("probe");
        assert!(!info.category.is_empty() && !info.help.is_empty(), "{}: category and help", info.name);
        assert!((2..=5).contains(&info.feedback.len()), "{}: feedback questions", info.name);
        assert!(info.feedback.iter().all(|q| q.param.is_some()), "{}: quick fixes", info.name);
        assert!((1..=3).contains(&info.main_params.len()), "{}: main_params", info.name);
    }
}

#[test]
fn feedback_questions_are_declared_with_ask() {
    let src = "mode { api = 1, name = 'T',\n\
               params = { dash = number(0.3, 0.1, 1, 'Dash length (s)', 0.05), on = bool(true, 'On') },\n\
               feedback = {\n\
                 dash = ask('Dash vibration length', { 'Too short', 'Good', 'Too long' }, 'Good', { param = 'dash' }),\n\
                 menus = ask('Vibrates in menus'),\n\
                 parry = ask('Parries', { 'Missed', 'Good', 'On hits' }, nil, { param = 'dash', invert = true }),\n\
               } }\nfunction tick() end";
    let info = ModeRuntime::probe("test", src).unwrap();
    let ids: Vec<_> = info.feedback.iter().map(|q| q.id.as_str()).collect();
    assert_eq!(ids, ["dash", "menus", "parry"], "declaration order");
    let dash = &info.feedback[0];
    assert_eq!((dash.default, dash.param.as_deref()), (1, Some("dash")));
    assert!(info.feedback[1].options.is_empty(), "checkbox");
    assert_eq!(info.feedback[2].default, 1, "middle answer by default");

    let param = &info.params[0];
    assert_eq!(dash.quick_fix(0, param, 0.3), Some(0.4), "too short: longer, snapped to the step");
    assert_eq!(dash.quick_fix(2, param, 0.3), Some(0.2));
    assert_eq!(dash.quick_fix(1, param, 0.3), None, "default answer");
    assert_eq!(dash.quick_fix(2, param, 0.1), None, "already at the minimum");
    assert_eq!(info.feedback[2].quick_fix(0, param, 0.3), Some(0.2), "inverted");

    for (decl, error) in [
        ("x = 3", "ask()"),
        ("x = ask('X', { 'only' })", "at least 2"),
        ("x = ask('X', { 'a', 'b' }, 'c')", "not one of the answers"),
        ("x = ask('X', { 'a', 'b' }, 'a', { param = 'on' })", "number() parameter"),
        ("x = ask('X', { 'a', 'b' }, 'a', { param = 'nope' })", "number() parameter"),
        ("x = ask('X', nil, nil, { param = 'dash' })", "checkbox"),
    ] {
        let src = format!(
            "mode {{ api = 1, name = 'T', params = {{ dash = number(1, 0, 2, 'D'), on = bool(true, 'On') }}, \
             feedback = {{ {decl} }} }}\nfunction tick() end"
        );
        let err = load_err(&src);
        assert!(err.contains(error), "{decl}: {err}");
    }
}

#[test]
fn main_params_must_name_declared_parameters() {
    let ok = "mode { api = 1, name = 'T', main_params = { 'gain' }, params = { gain = number(1, 0, 2, 'Gain') } }\n\
              function tick() end";
    assert_eq!(ModeRuntime::probe("test", ok).unwrap().main_params, ["gain"]);
    let err = load_err("mode { api = 1, name = 'T', main_params = { 'nope' } }\nfunction tick() end");
    assert!(err.contains("no parameter named 'nope'"), "{err}");
}

#[test]
fn combo_hits_grow_and_reset_after_window() {
    let mut rt = load(COMBO);
    let hit = |rt: &mut ModeRuntime| {
        let out = step(rt, rumble(0.2, 0.0));
        run(rt, 0.2, rumble(0.0, 0.0), &PadState::default(), 1e9);
        out
    };
    let first = hit(&mut rt).channels["main"];
    let second = hit(&mut rt).channels["main"];
    assert!(second > first, "{first} -> {second}");
    let out = run(&mut rt, 1.0, rumble(0.0, 0.0), &PadState::default(), 1e9);
    assert_eq!(plot_value(&out, "hits"), 0.0);
}

#[test]
fn overheat_heats_while_firing_then_resets() {
    let mut rt = load(OVERHEAT);
    let mut pad = PadState::default();
    pad.button("RT", true, 0.0);
    let out = run(&mut rt, 2.0, rumble(0.0, 0.0), &pad, 0.0);
    assert!((plot_value(&out, "heat") - 0.3).abs() < 0.01);
    run(&mut rt, 6.0, rumble(0.0, 0.0), &pad, 0.0);
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert!(out.channels["main"] > 0.5, "overheating");
    let out = run(&mut rt, 3.5, rumble(0.0, 0.0), &PadState::default(), 1e9);
    assert_eq!(plot_value(&out, "heat"), 0.0);
}

#[test]
fn tension_rewards_vibration_right_after_parry() {
    let mut rt = load(TENSION);
    press(&mut rt, "RB");
    let out = step(&mut rt, rumble(0.1, 0.0));
    assert_eq!(out.channels["main"], 1.0);
    assert_eq!(plot_value(&out, "parries"), 1.0);
    // Same vibration without a parry press: plain rumble.
    run(&mut rt, 1.0, rumble(0.0, 0.0), &PadState::default(), 1e9);
    let out = step(&mut rt, rumble(0.1, 0.0));
    assert!(out.channels["main"] < 0.2);
}

#[test]
fn engine_revs_with_throttle() {
    let mut rt = load(ENGINE);
    let idle = run(&mut rt, 1.0, rumble(0.0, 0.0), &PadState::default(), 0.0);
    assert_eq!(plot_value(&idle, "rpm"), 0.0);
    let mut pad = PadState::default();
    pad.axis(crate::gamepad::codes::ABS_RZ, 1.0, 0.0);
    let out = run(&mut rt, 1.0, rumble(0.0, 0.0), &pad, 0.0);
    assert_eq!(plot_value(&out, "rpm"), 1.0);
    let off = run(&mut rt, 1.0, rumble(0.0, 0.0), &PadState::default(), 1e9);
    assert_eq!(off.channels["main"], 0.0, "engine off when idle");
}

#[test]
fn heartbeat_speeds_up_with_stress_and_calms_down() {
    let mut rt = load(HEARTBEAT);
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert!((plot_value(&out, "bpm") - 60.0).abs() < 1e-6);
    for _ in 0..5 {
        step(&mut rt, rumble(1.0, 0.0));
        run(&mut rt, 0.2, rumble(0.0, 0.0), &PadState::default(), 0.0);
    }
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert!((plot_value(&out, "stress") - 0.75).abs() < 1e-6);
    let out = run(&mut rt, 60.0, rumble(0.0, 0.0), &PadState::default(), 0.0);
    assert_eq!(plot_value(&out, "stress"), 0.0);
}

#[test]
fn all_or_nothing_fills_while_playing_and_empties_on_hit() {
    let mut rt = load(ALL_OR_NOTHING);
    let out = run(&mut rt, 10.0, rumble(0.0, 0.0), &PadState::default(), 0.0);
    assert!((plot_value(&out, "gauge") - 0.1).abs() < 1e-6);
    // `input.idle` also counts rumble idle time, which starts at activation.
    let out = run(&mut rt, 25.0, rumble(0.0, 0.0), &PadState::default(), 1e9);
    assert!((plot_value(&out, "gauge") - 0.1).abs() < 1e-6, "frozen while idle");
    assert_eq!(out.channels["main"], 0.0, "paused while idle");
    let out = step(&mut rt, rumble(0.9, 0.0));
    assert_eq!(plot_value(&out, "gauge"), 0.0);
    assert_eq!(out.channels["main"], 0.8, "punishment pulse");
}

#[test]
fn ambient_wave_fades_out_when_idle() {
    let mut rt = load(AMBIENT);
    let out = run(&mut rt, 4.0, rumble(0.0, 0.0), &PadState::default(), 0.0);
    assert!((out.channels["main"] - 0.25).abs() < 1e-3);
    let out = run(&mut rt, 40.0, rumble(0.0, 0.0), &PadState::default(), 1e9);
    assert_eq!(out.channels["main"], 0.0);
}

#[test]
fn surge_fills_with_parries_and_spends_on_a_surge() {
    let mut rt = load(SURGE);
    for i in 1..=3 {
        press(&mut rt, "LT");
        let out = step(&mut rt, rumble(0.1, 0.0));
        assert_eq!(out.channels["main"], 1.0, "parry pulse");
        assert_eq!(plot_value(&out, "parries"), i as f64);
        run(&mut rt, 0.5, rumble(0.0, 0.0), &PadState::default(), 0.0);
    }
    // A vibration without a parry press only adds a little.
    let out = step(&mut rt, rumble(0.1, 0.0));
    assert!((plot_value(&out, "gauge") - 0.64).abs() < 1e-6);
    run(&mut rt, 0.5, rumble(0.0, 0.0), &PadState::default(), 0.0);
    // Hold parry + surge button: the gauge pays for a crescendo.
    press(&mut rt, "LT");
    let out = press(&mut rt, "X");
    assert_eq!(plot_value(&out, "surges"), 1.0);
    assert!((plot_value(&out, "gauge") - 0.14).abs() < 1e-6);
    let out = run(&mut rt, 1.0, rumble(0.0, 0.0), &PadState::default(), 0.0);
    assert!(out.channels["main"] > 0.8, "crescendo");
    // Not enough gauge left for a second surge.
    let out = press(&mut rt, "Y");
    assert_eq!(plot_value(&out, "surges"), 1.0);
    // Out of combat the glow fades and the gauge drains.
    let out = run(&mut rt, 20.0, rumble(0.0, 0.0), &PadState::default(), 1e9);
    assert_eq!(plot_value(&out, "fight"), 0.0);
    assert_eq!(out.channels["main"], 0.0);
    assert!(plot_value(&out, "gauge") < 0.14);
}

/// A request for a mode made for `game`.
fn new_mode<'a>(game: &'a str, language: &'a str, depth: prompt::Depth, described: Option<&'a str>, phases: &'a [String]) -> prompt::NewMode<'a> {
    prompt::NewMode { game, language, style: prompt::Style::Direct, depth, described, phases, instructions: "" }
}

#[test]
fn ai_prompt_names_the_game_and_embeds_the_api() {
    let text = prompt::new_mode_prompt(&prompt::Templates::builtin(), &new_mode("  Hades II ", "Français", prompt::Depth::Quick, None, &[]));
    assert!(text.contains("**Hades II**"));
    assert!(text.contains("category = \"Hades II\""));
    assert!(text.contains("answer in Français"), "language");
    assert!(text.contains("Continuous beats intermittent"), "rules");
    assert!(text.contains("## 3. Mode file") && text.contains("### 4.2 Feedback questions"), "spec");
    assert!(text.contains("## 10. Execution and sandbox"), "the section after a left-out one");
    for left_out in ["## 1. ", "## 2. ", "### 4.1 ", "## 11. ", "## 12. ", "## 13. ", "## 14. "] {
        assert!(!text.contains(left_out), "{left_out}");
    }
    assert!(!text.contains("{{") && !text.contains("<!--"), "every placeholder and marker replaced");
    // A quick request: high-level inputs only.
    assert!(text.contains("### 6.3 ") && text.contains("on_phase"), "phases are high level");
    for advanced in ["### 6.4 ", "### 6.5 ", "### 7.1 "] {
        assert!(!text.contains(advanced), "{advanced}");
    }
    assert!(text.contains("Nothing more: use the rumble"), "{text}");

    // An advanced request: the raw inputs and the inputs set up for the mode.
    let profile = "Indicators of the screen (`input.indicators`, `on_indicator`):\n- `battle_hud`: true while shown, false otherwise\n";
    let advanced = prompt::new_mode_prompt(&prompt::Templates::builtin(), &new_mode("Hades II", "English", prompt::Depth::Advanced, Some(profile), &[]));
    assert!(advanced.contains("### 6.4 ") && advanced.contains("### 6.5 ") && advanced.contains("### 7.1 "));
    assert!(advanced.contains("- `battle_hud`: true while shown"), "the profile");
    let blank = prompt::new_mode_prompt(&prompt::Templates::builtin(), &new_mode("Hades II", "English", prompt::Depth::Advanced, None, &[]));
    let phases = ["battle".to_owned(), "hub".to_owned()];
    let quick_phases = prompt::new_mode_prompt(&prompt::Templates::builtin(), &new_mode("Hades II", "English", prompt::Depth::Quick, None, &phases));
    assert!(quick_phases.contains("already recognizes this game's phases: `battle`, `hub`"), "a quick request names the game's phases");
    assert!(blank.contains("tell the player which indicators to draw"), "no profile yet");
    assert!(text.trim_end().ends_with("The player may write some below."), "room for the player's own instructions");
}

#[test]
fn request_styles_ask_for_questions_an_analysis_or_the_script() {
    let templates = prompt::Templates::builtin();
    let phases = ["battle".to_owned(), "story".to_owned()];
    let request = |style, instructions| {
        let r = prompt::NewMode { style, instructions, ..new_mode("Hades II", "English", prompt::Depth::Quick, None, &phases) };
        prompt::new_mode_prompt(&templates, &r)
    };
    let direct = request(prompt::Style::Direct, "");
    let chat = request(prompt::Style::Conversation, "  No vibration in menus.  ");
    let analysis = request(prompt::Style::Analysis, "");
    let after = request(prompt::Style::AfterAnalysis, "");
    for text in [&direct, &chat, &analysis, &after] {
        assert!(!text.contains("{{") && !text.contains("<!--"), "every placeholder and marker replaced");
        assert!(text.contains("# The player's own instructions"));
    }
    assert!(direct.contains("Design the mode") && !direct.contains("Ask the player"));
    assert!(chat.contains("Ask the player before writing anything") && chat.contains("contrasted designs"));
    assert!(chat.trim_end().ends_with("follow them.\n\nNo vibration in menus."), "{chat}");
    // The analysis proposes the setup, with the indicators' API; no script yet.
    assert!(analysis.contains("Do not write the mode yet") && analysis.contains("\"otherwise\": true"));
    assert!(analysis.contains("### 6.5 ") && analysis.contains("Continuous beats intermittent"));
    // The script after it, in the same conversation: short.
    assert!(after.contains("The player set up in GameViber what you proposed") && after.contains("`battle`, `story`"));
    assert!(!after.contains("## 3. Mode file") && !after.contains("Continuous beats intermittent") && !after.contains("turns what a game does"));
    assert!(after.contains("category = \"Hades II\""));
    assert!(after.len() * 5 < direct.len());
}

#[test]
fn an_analysis_setup_is_read_and_set_up() {
    let answer = "Here is the plan.\n\n```json\n{\n  \"phases\": [\n    { \"name\": \"battle\", \"sound\": \"drums\", \"indicators\": [\"hp\", \"battle_menu\"] },\n    \
                  { \"name\": \"exploration\", \"indicators\": [\"hp\"] },\n    { \"name\": \"story\", \"otherwise\": true },\n    { \"name\": \"bad name\" }\n  ],\n  \
                  \"indicators\": [\n    { \"name\": \"hp\", \"kind\": \"gauge\", \"where\": \"top left\" },\n    \
                  { \"name\": \"battle_menu\", \"kind\": \"visibility\", \"where\": \"bottom right\" }\n  ]\n}\n```\n\nCapture each phase.";
    let setup = prompt::extract_setup(answer).unwrap();
    assert_eq!((setup.phases.len(), setup.indicators[0].kind, setup.indicators[1].place.as_str()), (4, IndicatorKind::Gauge, "bottom right"));
    assert!(prompt::extract_setup("```lua\nmode { api = 1 }\n```").is_none());

    // A phase already set up keeps what the player chose.
    let mut inputs = Inputs::default();
    inputs.phases.push(PhaseDef { name: "battle".into(), indicators: vec!["menu".into()], ..PhaseDef::default() });
    inputs.zones.push(Zone { indicator: "hp".into(), kind: IndicatorKind::Gauge, ..Zone::default() });
    let applied = inputs.apply_setup(&setup);
    assert_eq!((applied.phases_added, applied.phases_completed, applied.indicators_to_draw), (2, 1, 1));
    let names: Vec<&str> = inputs.phases.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["battle", "exploration", "story"], "the invalid name left out");
    assert_eq!((inputs.phases[0].sound.as_deref(), &inputs.phases[0].indicators[..]), (Some("drums"), &["menu".to_owned()][..]));
    assert!(inputs.phases[2].otherwise && inputs.phases[1].indicators == ["hp"]);
    assert_eq!(inputs.to_draw().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["battle_menu"], "hp is drawn already");
}

#[test]
fn left_out_spec_sections_exist() {
    let spec = include_str!("../../../docs/spec-modes.md");
    for heading in [
        "## 1. Goal",
        "## 2. Architecture",
        "### 4.1 Presets",
        "### 6.4 Raw sound and image",
        "### 6.5 The mode's inputs",
        "### 7.1 Advanced inputs",
        "## 11. Errors",
        "## 12. Safety",
        "## 13. Planned",
        "## 14. Examples",
    ] {
        assert!(spec.lines().any(|l| l.starts_with(heading)), "{heading}");
    }
}

#[test]
fn templates_keep_their_placeholders() {
    for template in prompt::Template::ALL {
        for placeholder in template.placeholders() {
            assert!(template.builtin().contains(placeholder), "{placeholder} in {}", template.title());
        }
    }
}

#[test]
fn feel_prompt_holds_the_problem_settings_and_session() {
    let source = include_str!("../../modes/surge.luau");
    let rt = ModeRuntime::load("surge.luau", source, &BTreeMap::new(), None).unwrap();
    let mut values = rt.param_values().clone();
    values.insert("window".into(), ParamValue::Number(0.45));
    let problems = ["Parries are not detected".to_owned(), "too strong while exploring".to_owned()];
    let mut report = prompt::FeelReport {
        name: "Surge",
        game: "Hollow Knight",
        problems: &problems,
        history: &["2026-10-03: quick fix".to_owned()],
        params: &rt.info().params,
        values: &values,
        source,
        session: Some("### Vibrations sent by the game (0)\n"),
        full: true,
        language: "English",
        inputs: None,
    };
    let templates = prompt::Templates::builtin();
    let text = prompt::feel_prompt(&templates, &report);
    assert!(text.contains("to the player in **Hollow Knight**"));
    assert!(text.contains("- Parries are not detected\n- too strong while exploring"));
    assert!(text.contains("# Earlier attempts\n\n- 2026-10-03: quick fix"));
    assert!(text.contains("| Parry window (s) | `window` | 0.45 | 0.3 |"), "{text}");
    assert!(text.contains("| Parry button (also the surge modifier) | `parry` | \"LT\" | \"LT\" |"));
    assert!(text.contains("name = \"Surge\""), "source");
    assert!(text.contains("### Vibrations sent by the game (0)"), "session");
    assert!(text.contains("Continuous beats intermittent"), "rules");
    assert!(text.contains("## 5. Callbacks") && !text.contains("## 14. Examples"), "spec");
    assert!(!text.contains("### 6.5 "), "a mode using high-level inputs only gets the quick specification");
    assert!(!text.contains("{{") && !text.contains("<!--"), "every placeholder and marker replaced");

    // For the conversation that wrote the mode: no context, rules or API.
    report.full = false;
    let short = prompt::feel_prompt(&templates, &report);
    assert!(short.contains("- Parries are not detected") && short.contains("| Parry window (s) | `window` | 0.45 | 0.3 |"));
    assert!(!short.contains("name = \"Surge\""), "the conversation has the code");
    assert!(!short.contains("Continuous beats intermittent") && !short.contains("## 5. Callbacks"));
    assert!(!short.contains("turns what a game does") && !short.contains("\n\n\n"));
    assert!(short.len() * 3 < text.len());
    assert!(prompt::has_script("```lua\nmode { api = 1 }\n```"));
    assert!(!prompt::has_script("Set the parry window to 0.5 s."));
}

#[test]
fn ai_answer_script_is_extracted() {
    let answer = "Here is the design.\n\n```lua\nlocal x = 1\n```\n\n```luau\nmode { api = 1, name = 'G' }\n\
                  function tick() end\n```\n\nTune `x` first.";
    let script = prompt::extract_script(answer);
    assert_eq!(script, "mode { api = 1, name = 'G' }\nfunction tick() end\n");
    assert_eq!(ModeRuntime::probe("test", &script).unwrap().name, "G");
    assert_eq!(prompt::extract_script("  mode { }  "), "mode { }\n", "bare code");
    assert_eq!(prompt::extract_script("```lua\nmode { }\nfunction"), "mode { }\nfunction\n", "cut short");
    assert_eq!(prompt::file_stem(" Prince of Persia: The Lost Crown "), "prince-of-persia-the-lost-crown");
}

#[test]
fn hud_gauges_persist_and_events_last_one_tick() {
    let src = wrap(
        "function on_start() hud('Gauge', 3, 10) end
         function on_button(ev)
           if ev.button == 'A' then hud('Gauge', nil) end
           if ev.button == 'B' then hud_event('Parry!') end
           if ev.button == 'X' then for i = 1, 5 do hud('g' .. i, i) end end
         end
         function tick() end",
    );
    let mut rt = load(&src);
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert_eq!(out.hud, [HudGauge { label: "Gauge".into(), value: 3.0, max: 10.0 }]);
    assert!(step(&mut rt, rumble(0.0, 0.0)).hud.len() == 1, "kept without calling hud again");
    let out = press(&mut rt, "B");
    assert_eq!(out.hud_events, ["Parry!"]);
    assert!(step(&mut rt, rumble(0.0, 0.0)).hud_events.is_empty());
    assert!(press(&mut rt, "A").hud.is_empty(), "nil removes");
    let err = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 0.0, &[ModeEvent::Button(ButtonEvent { name: "X", pressed: true })]);
    assert!(err.unwrap_err().contains("at most 4 gauges"));
}

#[test]
fn apply_params_sets_all_values_and_defaults_the_rest() {
    let src = "mode { api = 1, name = 'T', params = {
                 x = number(1, 0, 2, 'X'), y = number(0.5, 0, 1, 'Y'), c = choice('a', { 'a', 'b' }, 'C') } }
               changes = 0
               function on_param_changed() changes += 1 end
               function tick() plot('changes', changes) end";
    let mut rt = load(src);
    rt.set_param("y", &ParamValue::Number(0.9)).unwrap();
    let preset = BTreeMap::from([
        ("x".to_owned(), ParamValue::Number(5.0)),
        ("c".to_owned(), ParamValue::Text("bogus".into())),
        ("stale".to_owned(), ParamValue::Bool(true)),
    ]);
    rt.apply_params(&preset).unwrap();
    let values = rt.param_values();
    assert_eq!(values["x"], ParamValue::Number(2.0), "clamped");
    assert_eq!(values["y"], ParamValue::Number(0.5), "missing -> default");
    assert_eq!(values["c"], ParamValue::Text("a".into()), "invalid -> default");
    // y set by hand, then x and y changed by the preset; c unchanged: no callback.
    assert_eq!(plot_value(&step(&mut rt, rumble(0.0, 0.0)), "changes"), 3.0);
}

fn audio_levels(level: f64) -> AudioLevels {
    AudioLevels { level, low: level / 2.0, mid: 0.0, high: 0.0, intensity: 0.25 }
}

#[test]
fn input_audio_and_hits_follow_the_game_sound() {
    let src = wrap(
        "hits, impacts = 0, 0
         function on_audio_hit(ev) hits += 1; plot('strength', ev.strength); log(ev.band) end
         function on_impact(ev) impacts += 1; plot('source', ev.source == 'sound' and 1 or 2) end
         function tick(dt, input)
           plot('active', input.audio.active and 1 or 0)
           plot('level', input.audio.level)
           plot('low', input.audio.low)
           plot('audio_intensity', input.audio.intensity)
           plot('intensity', input.intensity)
           plot('hits', hits)
           plot('impacts', impacts)
           plot('phase', input.phase == nil and 0 or 1)
         end",
    );
    let mut rt = load(&src);
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert_eq!((plot_value(&out, "active"), plot_value(&out, "level"), plot_value(&out, "intensity")), (0.0, 0.0, 0.0));

    rt.set_audio(Some(audio_levels(0.8)));
    let hit = |strength| ModeEvent::AudioHit(AudioHit { strength, band: crate::audio::Band::Low });
    let out = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &[hit(0.6), hit(0.1)]).unwrap();
    assert_eq!(plot_value(&out, "active"), 1.0);
    assert_eq!(plot_value(&out, "level"), 0.8);
    assert_eq!(plot_value(&out, "low"), 0.4);
    assert_eq!(plot_value(&out, "audio_intensity"), 0.25);
    assert_eq!(plot_value(&out, "intensity"), 0.25, "the sound alone");
    assert_eq!(plot_value(&out, "hits"), 2.0);
    assert_eq!(plot_value(&out, "impacts"), 1.0, "weak hits are no impacts");
    assert_eq!(plot_value(&out, "source"), 1.0);
    assert_eq!(plot_value(&out, "phase"), 0.0, "no phases declared");

    // The image adds to the intensity, and its flashes are impacts.
    rt.set_screen(Some(ScreenLevels { brightness: 0.5, motion: 0.2, action: 0.75 }));
    let out = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &[ModeEvent::ScreenFlash(0.7)]).unwrap();
    assert_eq!(plot_value(&out, "intensity"), 0.5, "the mean of both senses");
    assert_eq!((plot_value(&out, "impacts"), plot_value(&out, "source")), (2.0, 2.0));

    // Hits are dropped while the sound is not captured.
    rt.set_audio(None);
    let out = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &[hit(0.9)]).unwrap();
    assert_eq!((plot_value(&out, "active"), plot_value(&out, "hits")), (0.0, 2.0));
}

#[test]
fn a_phase_keeps_the_events_it_ignores_from_the_mode() {
    let src = wrap(
        "hits, sound, screen = 0, 0, 0
         function on_audio_hit(ev) hits += 1 end
         function on_impact(ev) if ev.source == 'sound' then sound += 1 else screen += 1 end end
         function tick(dt, input)
           plot('hits', hits); plot('sound', sound); plot('screen', screen)
           plot('menu', input.phase == 'menu' and 1 or 0)
         end",
    );
    let mut rt = load(&src);
    rt.set_audio(Some(audio_levels(0.5)));
    rt.set_screen(Some(ScreenLevels::default()));
    let phase = |name: &str, indicator: Option<&str>, ignore| PhaseDecl {
        name: name.into(),
        sound: None,
        screen: None,
        indicators: indicator.into_iter().map(Into::into).collect(),
        otherwise: false,
        hold: 0.0,
        ignore,
    };
    rt.set_game_phases(&[
        phase("menu", Some("menu_button"), Ignored { sound_hits: true, flashes: false }),
        phase("play", None, Ignored::default()),
    ]);
    let events = [ModeEvent::AudioHit(AudioHit { strength: 0.8, band: crate::audio::Band::Mid }), ModeEvent::ScreenFlash(0.7)];
    let counts = |out: &TickOutput| ["hits", "sound", "screen", "menu"].map(|k| plot_value(out, k));

    // Outside the menu, the mode gets everything.
    let out = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &events).unwrap();
    assert_eq!(counts(&out), [1.0, 1.0, 1.0, 0.0]);

    // The menu's indicator shows: once it is the phase, the sound's hits stop, its flashes do not.
    let shown = ModeEvent::Indicator { name: "menu_button".into(), value: IndicatorValue::Visibility(true) };
    rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &[shown]).unwrap();
    for _ in 0..30 {
        step(&mut rt, rumble(0.0, 0.0));
    }
    let out = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &events).unwrap();
    assert_eq!(counts(&out), [1.0, 1.0, 2.0, 1.0]);

    // The menu gone, hits come again.
    let hidden = ModeEvent::Indicator { name: "menu_button".into(), value: IndicatorValue::Visibility(false) };
    rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &[hidden]).unwrap();
    let out = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &events).unwrap();
    assert_eq!(counts(&out), [2.0, 2.0, 3.0, 0.0]);
}

#[test]
fn input_screen_follows_the_game_image() {
    let src = wrap(
        "function tick(dt, input)
           plot('active', input.screen.active and 1 or 0)
           plot('brightness', input.screen.brightness)
           plot('motion', input.screen.motion)
           plot('action', input.screen.action)
         end",
    );
    let mut rt = load(&src);
    assert_eq!(plot_value(&step(&mut rt, rumble(0.0, 0.0)), "active"), 0.0);
    rt.set_screen(Some(ScreenLevels { brightness: 0.25, motion: 0.5, action: 0.125 }));
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert_eq!(
        ["active", "brightness", "motion", "action"].map(|k| plot_value(&out, k)),
        [1.0, 0.25, 0.5, 0.125]
    );
}

#[test]
fn phases_are_declared_and_reported() {
    let src = "mode { api = 1, name = 'T', phase_window = 4,
                 phases = {
                   calm = { sound = 'calm ambient music', screen = 'a quiet village' },
                   battle = { sound = 'intense battle music' },
                 } }
               last, previous, changes = 'none', 'none', 0
               function on_phase(ev)
                 changes += 1
                 last = ev.phase or 'none'
                 previous = ev.previous or 'none'
                 plot('confidence', ev.confidence)
               end
               function tick(dt, input)
                 plot('changes', changes)
                 plot('battle', input.phases.battle)
                 plot('is_battle', input.phase == 'battle' and 1 or 0)
               end";
    let mut rt = load(src);
    let info = rt.info();
    assert_eq!(info.phases[0].name, "battle", "sorted by name");
    assert_eq!(info.phases[0].screen, None);
    assert_eq!(info.phase_window, 4.0);
    assert!(rt.uses_sound_phases() && rt.uses_screen_phases(false));
    assert_eq!(
        rt.phase_descriptions(Sense::Sound),
        [("battle".to_owned(), "intense battle music".to_owned()), ("calm".to_owned(), "calm ambient music".to_owned())]
    );
    assert_eq!(rt.phase_descriptions(Sense::Screen), [("calm".to_owned(), "a quiet village".to_owned())]);
    // Text embeddings of battle and calm: two axes.
    let axis = |i: usize| -> crate::audio::Embedding { (0..2).map(|k| if k == i { 1.0 } else { 0.0 }).collect() };
    rt.set_audio(Some(audio_levels(0.5)));
    let clip = ModeEvent::AudioClip(axis(0));
    let out = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &[clip.clone()]).unwrap();
    assert_eq!(plot_value(&out, "changes"), 0.0, "nothing before the descriptions are encoded");
    assert!(!rt.phases_ready());

    rt.set_phase_references(Sense::Sound, vec![("battle".into(), axis(0)), ("calm".into(), axis(1))]);
    let out = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &[clip]).unwrap();
    assert_eq!(plot_value(&out, "changes"), 1.0);
    assert_eq!(plot_value(&out, "is_battle"), 1.0);
    assert!(plot_value(&out, "confidence") > 0.99);
    assert!(plot_value(&out, "battle") > 0.99);
    assert_eq!(rt.phase_state().0.as_deref(), Some("battle"));

    // The sound stops: no sense is left, the phase is forgotten, with an event.
    rt.set_audio(None);
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert_eq!(plot_value(&out, "changes"), 2.0);
    assert_eq!(plot_value(&out, "is_battle"), 0.0);
}

#[test]
fn phase_declarations_are_checked() {
    let one = load_err("mode { api = 1, name = 'T', phases = { a = { sound = 'x' } } } function tick() end");
    assert!(one.contains("mode.phases must declare 2 to 8 phases"), "{one}");
    let empty = load_err("mode { api = 1, name = 'T', phases = { a = { sound = 'x' }, b = { screen = ' ' } } } function tick() end");
    assert!(empty.contains("needs a sound or a screen description"), "{empty}");
    let flat = load_err("mode { api = 1, name = 'T', phases = { a = 'x', b = 'y' } } function tick() end");
    assert!(flat.contains("must be a table"), "{flat}");
    let window = load_err("mode { api = 1, name = 'T', phase_window = 0.5 } function tick() end");
    assert!(window.contains("mode.phase_window must be between 2 and 60"), "{window}");
}

#[test]
fn names_of_the_first_api_version_still_work() {
    let src = "mode { api = 1, name = 'T', scene_window = 4,
                 scenes = { calm = { sound = 'calm music' }, battle = { sound = 'battle music' } } }
               zone_events = 0
               function on_scene(ev) end
               function on_zone(ev) if ev.zone == 'hp' then zone_events += 1 end end
               function tick(dt, input)
                 plot('hp', input.zones.hp or -1)
                 plot('ammo', input.custom.ammo or -1)
                 plot('zone_events', zone_events)
                 plot('no_scene', input.scene == nil and input.scene_confidence == 0 and 1 or 0)
               end";
    let mut rt = load(src);
    assert_eq!((rt.info().phases.len(), rt.info().phase_window), (2, 4.0));
    rt.set_screen(Some(ScreenLevels::default()));
    let events = [
        ModeEvent::Indicator { name: "hp".into(), value: IndicatorValue::Gauge(0.5) },
        ModeEvent::ExternalValue { name: "ammo".into(), value: serde_json::json!(3) },
    ];
    let out = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &events).unwrap();
    assert_eq!(["hp", "ammo", "zone_events", "no_scene"].map(|k| plot_value(&out, k)), [0.5, 3.0, 1.0, 1.0]);
    assert!(load_err("mode { api = 1, name = 'T', scene_window = 0.5 } function tick() end").contains("mode.scene_window"));
}

#[test]
fn indicators_and_external_inputs_reach_the_mode() {
    let src = wrap(
        "zone_events, events = 0, 0
         function on_indicator(ev)
           zone_events += 1
           if ev.indicator == 'hp' then plot('previous_hp', ev.previous or -1) end
         end
         function on_event(ev)
           events += 1
           plot('damage', ev.data.amount)
           log(ev.name)
         end
         function tick(dt, input)
           plot('hud', input.indicators.battle_hud and 1 or 0)
           plot('hp', input.indicators.hp or -1)
           plot('zone_events', zone_events)
           plot('events', events)
           plot('ammo', input.external.ammo or -1)
           plot('stance', input.external.stance == 'low' and 1 or 0)
         end",
    );
    let mut rt = load(&src);
    rt.set_screen(Some(ScreenLevels::default()));
    let indicator = |name: &str, value| ModeEvent::Indicator { name: name.into(), value };
    let external = |name: &str, value| ModeEvent::ExternalValue { name: name.into(), value };
    let events = [
        indicator("battle_hud", IndicatorValue::Visibility(true)),
        indicator("hp", IndicatorValue::Gauge(0.5)),
        external("ammo", serde_json::json!(12)),
        external("stance", serde_json::json!("low")),
        ModeEvent::ExternalEvent { name: "hit".into(), data: serde_json::json!({ "amount": 30 }) },
    ];
    let out = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &events).unwrap();
    assert_eq!(
        ["hud", "hp", "previous_hp", "zone_events", "events", "damage", "ammo", "stance"].map(|k| plot_value(&out, k)),
        [1.0, 0.5, -1.0, 2.0, 1.0, 30.0, 12.0, 1.0]
    );
    // An unchanged indicator is not reported again; a JSON null removes a value.
    let events = [indicator("hp", IndicatorValue::Gauge(0.5)), indicator("hp", IndicatorValue::Gauge(0.25)), external("ammo", serde_json::Value::Null)];
    let out = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &events).unwrap();
    assert_eq!(["hp", "previous_hp", "zone_events", "ammo"].map(|k| plot_value(&out, k)), [0.25, 0.5, 3.0, -1.0]);
    // A bar not on screen (a menu): nil, not 0.
    let out = rt.step(DT, rumble(0.0, 0.0), &PadState::default(), 1e9, &[indicator("hp", IndicatorValue::Unknown)]).unwrap();
    assert_eq!(["hp", "previous_hp", "zone_events"].map(|k| plot_value(&out, k)), [-1.0, 0.25, 4.0]);
    // The image goes: so do the indicators read on it.
    rt.set_screen(None);
    let out = step(&mut rt, rumble(0.0, 0.0));
    assert_eq!((plot_value(&out, "hud"), plot_value(&out, "hp")), (0.0, -1.0));
}

#[test]
fn callbacks_may_be_fields_of_the_mode_table_and_read_input() {
    // How AI assistants sometimes write modes: callbacks in `mode { }`, `input` read outside `tick`.
    let src = "mode { api = 1, name = 'T',
          on_button = function(ev) persist.pressed_at = input.time end,
          tick = function(dt, input) set(0.5); plot('pressed_at', persist.pressed_at or -1) end,
        }";
    let mut rt = load(src);
    assert_eq!(main_out(&mut rt, rumble(0.0, 0.0)), 0.5, "tick from the mode table");
    let out = press(&mut rt, "A");
    assert!((plot_value(&out, "pressed_at") - 2.0 * DT).abs() < 1e-9, "input.time is the current tick's");
    assert!(load_err("mode { api = 1, name = 'T', tick = 3 }").contains("'tick' must be a function"));
}
