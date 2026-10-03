-- Fighting games (Tekken, Street Fighter...). Vibrations close together build a combo:
-- each hit is stronger than the previous one, and a long enough combo ends with a burst
-- sized by its length. Button mashing adds a light background tension.
mode {
  api = 1,
  name = "Combo",
  description = "Fighting games: chained hits grow stronger and end with a burst; "
             .. "button mashing builds a light tension.",
  category = "Fighting",
  help = "Hits that follow each other quickly form a combo: each one feels stronger than the last, and a long "
      .. "combo ends with a burst. Mashing buttons adds a light tension in the background.",
  main_params = { "window", "base", "burst" },
  author = "GameViber",
  rumble_release = 0.05,
  params = {
    window        = number(0.6, 0.1, 2, "Combo window (s)", 0.05),
    base          = number(0.35, 0, 1, "First hit intensity", 0.05),
    max_hits      = number(10, 2, 30, "Hits for full intensity", 1),
    hit_len       = number(0.15, 0.05, 1, "Minimum hit length (s)", 0.01),
    min_burst     = number(3, 2, 20, "Hits needed for a burst", 1),
    burst         = number(1, 0, 1, "Burst intensity", 0.05),
    per_press     = number(0.04, 0, 0.5, "Tension per button press", 0.01),
    tension       = number(0.15, 0, 0.5, "Max tension vibration", 0.01),
    tension_decay = number(0.3, 0, 5, "Tension lost per second", 0.05),
  },
}

local burst = pattern {
  { 0.00, 1.0 }, { 0.08, 0.3 }, { 0.16, 1.0 }, { 0.24, 0.3 }, { 0.32, 1.0 }, { 0.60, 0.0 },
}

local hits, last_hit, tension = 0, 0, 0

-- Intensity of the current combo step, from P.base (first hit) to 1 (max_hits).
local function combo_level()
  if hits == 0 then
    return 0
  end
  return lerp(P.base, 1, math.min((hits - 1) / (P.max_hits - 1), 1))
end

local function end_combo()
  if hits >= P.min_burst then
    local size = math.min(hits / P.max_hits, 1)
    play(burst, { loops = 1 + math.floor(size * 3), scale = P.burst * lerp(0.5, 1, size) })
  end
  hits = 0
end

function on_start()
  hits, last_hit, tension = 0, 0, 0
end

function on_rumble_start(ev)
  hits += 1
  last_hit = ev.t
  pulse(math.max(ev.level, combo_level()), P.hit_len)
end

function on_rumble_end(ev)
  last_hit = ev.t
end

function on_button(ev)
  if ev.pressed then
    tension = math.min(tension + P.per_press, 1)
  end
end

function tick(dt, input)
  if hits > 0 and not input.rumble.active and input.time - last_hit > P.window then
    end_combo()
  end
  tension = math.max(tension - P.tension_decay * dt, 0)

  -- Long vibrations (throws, supers) keep the combo intensity while they last.
  local hit = input.rumble.active and math.max(input.rumble.level, combo_level()) or 0
  set(math.max(hit, tension * P.tension))

  plot("hits", hits)
  plot("tension", tension)
end
