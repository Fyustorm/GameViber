-- Horror and survival games (Resident Evil, Silent Hill, Dead Space...). A heartbeat
-- runs all the time; every vibration raises the stress, which speeds it up and makes it
-- stronger, then the heart slowly calms down. Optional random jump scares.
mode {
  api = 1,
  name = "Heartbeat",
  description = "Horror games: a heartbeat that speeds up with every vibration and slowly "
             .. "calms down, with optional jump scares.",
  category = "Horror, survival",
  help = "A heartbeat plays all the time. Each time the game vibrates, your heart races and beats harder, "
      .. "then slowly calms down when things get quiet. Optional random jump scares.",
  main_params = { "rest_bpm", "max_bpm", "jumps" },
  author = "GameViber",
  params = {
    rest_bpm      = number(60, 30, 120, "Resting heart rate (BPM)", 1),
    max_bpm       = number(150, 80, 180, "Max heart rate (BPM)", 1),
    rest_level    = number(0.15, 0, 1, "Resting intensity", 0.01),
    max_level     = number(0.9, 0, 1, "Max intensity", 0.05),
    per_hit       = number(0.15, 0, 1, "Stress per vibration", 0.01),
    calm_delay    = number(3, 0, 30, "Calm down after no rumble for (s)", 0.5),
    calm          = number(0.03, 0, 0.5, "Stress lost per second", 0.005),
    rumble_weight = number(0.5, 0, 2, "Rumble multiplier", 0.05),
    jumps         = number(0, 0, 10, "Jump scares per 10 minutes", 0.5),
    pause_idle    = number(60, 0, 600, "Pause after inactivity (s), 0 = never", 5),
  },
}

-- Beat shape in seconds from the start of a beat: "lub" then a weaker "dub". Fixed
-- lengths keep each bump longer than the output update period at high BPM.
local LUB_END, DUB_START, DUB_END, DUB_LEVEL = 0.1, 0.2, 0.3, 0.6
local JUMP_STRESS, JUMP_LEN = 0.3, 0.4

persist.stress = persist.stress or 0
local beat_t = 0

local function add_stress(n)
  persist.stress = clamp(persist.stress + n, 0, 1)
end

local function beat_shape(t)
  if t < LUB_END then
    return 1
  elseif t >= DUB_START and t < DUB_END then
    return DUB_LEVEL
  end
  return 0
end

function on_start()
  beat_t = 0
end

function on_rumble_start(ev)
  add_stress(P.per_hit * ev.level)
end

function tick(dt, input)
  if input.rumble_idle > P.calm_delay then
    add_stress(-P.calm * dt)
  end
  if P.jumps > 0 and random() < P.jumps / 600 * dt then
    pulse(1, JUMP_LEN)
    add_stress(JUMP_STRESS)
  end

  local s = persist.stress
  local period = 60 / lerp(P.rest_bpm, P.max_bpm, s)
  beat_t += dt
  if beat_t >= period then
    beat_t %= period
  end
  local heart = beat_shape(beat_t) * lerp(P.rest_level, P.max_level, s)
  if P.pause_idle > 0 and input.idle > P.pause_idle then
    heart = 0
  end

  set(math.max(heart, input.rumble.level * P.rumble_weight))

  plot("stress", s)
  plot("bpm", 60 / period)
end
