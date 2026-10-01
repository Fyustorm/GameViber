use super::*;

const SIMPLE: &str = include_str!("../../modes/simple.luau");
const ACCUMULATION: &str = include_str!("../../modes/accumulation.luau");
const COMBO: &str = include_str!("../../modes/combo.luau");
const OVERHEAT: &str = include_str!("../../modes/overheat.luau");
const TENSION: &str = include_str!("../../modes/tension.luau");
const ENGINE: &str = include_str!("../../modes/engine.luau");
const HEARTBEAT: &str = include_str!("../../modes/heartbeat.luau");
const ALL_OR_NOTHING: &str = include_str!("../../modes/all_or_nothing.luau");
const AMBIENT: &str = include_str!("../../modes/ambient.luau");
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
    pad.axis(evdev::AbsoluteAxisCode::ABS_X.0, 0.5, 0.0);
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
    pad.axis(evdev::AbsoluteAxisCode::ABS_RZ.0, 1.0, 0.0);
    pad.button("RB", true, 0.0);
    for src in [COMBO, OVERHEAT, TENSION, ENGINE, HEARTBEAT, ALL_OR_NOTHING, AMBIENT] {
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
    pad.axis(evdev::AbsoluteAxisCode::ABS_RZ.0, 1.0, 0.0);
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
