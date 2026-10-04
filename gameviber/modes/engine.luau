-- SPDX-License-Identifier: MIT
-- Racing games (Forza, Gran Turismo...). The throttle (RT) drives a pulsing vibration
-- whose rate and strength rise like engine RPM; the brake (LT) slows it down. The game's
-- rumble adds road texture and sharp rises in rumble become impact pulses.
mode {
  api = 1,
  name = "Engine",
  description = "Racing games: the throttle drives an engine-like pulsing vibration, "
             .. "rumble adds road texture and impacts.",
  category = "Racing",
  help = "The throttle (RT) revs an engine-like pulsing vibration that gets faster and stronger; the brake "
      .. "(LT) slows it down. Bumps and crashes from the game add road texture and impacts.",
  main_params = { "idle_level", "max_level", "road" },
  author = "GameViber",
  params = {
    idle_level = number(0.1, 0, 0.5, "Idle intensity", 0.01),
    max_level  = number(0.7, 0, 1, "Full throttle intensity", 0.05),
    idle_freq  = number(1, 0.2, 5, "Idle pulse rate (Hz)", 0.1),
    -- The output is limited to 20 updates/s per toy, so faster rates blur.
    max_freq   = number(4, 0.5, 8, "Full throttle pulse rate (Hz)", 0.1),
    depth      = number(0.5, 0, 1, "Pulse depth", 0.05),
    rev_up     = number(1.5, 0.1, 10, "Rev up speed", 0.1),
    rev_down   = number(0.8, 0.1, 10, "Rev down speed", 0.1),
    road       = number(0.6, 0, 2, "Rumble weight", 0.05),
    impact     = number(0.7, 0, 1, "Impact threshold", 0.05),
    impact_len = number(0.3, 0.05, 2, "Impact pulse length (s)", 0.05),
    engine_off = number(10, 0, 120, "Engine off after inactivity (s), 0 = never", 1),
  },
  feedback = {
    idle     = ask("Engine at idle", { "Too weak", "Good", "Too strong" }, "Good", { param = "idle_level" }),
    throttle = ask("Full throttle", { "Too weak", "Good", "Too strong" }, "Good", { param = "max_level" }),
    road     = ask("Road and impacts", { "Too weak", "Good", "Too strong" }, "Good", { param = "road" }),
  },
}

local rpm, phase, prev_level = 0, 0, 0

function on_start()
  rpm, phase, prev_level = 0, 0, 0
end

function on_rumble(ev)
  if ev.level >= P.impact and prev_level < P.impact then
    pulse(1, P.impact_len)
  end
  prev_level = ev.level
end

function tick(dt, input)
  local throttle = clamp(input.axes.RT - input.axes.LT * 0.5, 0, 1)
  local speed = throttle > rpm and P.rev_up or P.rev_down
  rpm += clamp(throttle - rpm, -speed * dt, speed * dt)

  phase = (phase + dt * lerp(P.idle_freq, P.max_freq, rpm)) % 1
  local level = lerp(P.idle_level, P.max_level, rpm)
  local engine = level * (1 - P.depth * (0.5 - 0.5 * math.sin(2 * math.pi * phase)))
  if P.engine_off > 0 and input.input_idle > P.engine_off then
    engine = 0
  end

  set(math.max(engine, input.rumble.level * P.road))

  plot("rpm", rpm)
end
