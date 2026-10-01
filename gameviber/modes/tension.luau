-- Turn-based games with timed defense (Clair Obscur, Sea of Stars, Persona...).
-- A slow wave builds up during fights and fades away while exploring. A vibration right
-- after a parry or dodge press is probably a successful parry and gets a reward pulse.
-- Long, strong vibrations (big attacks) are amplified.
mode {
  api = 1,
  name = "Tension",
  description = "Turn-based games: a wave builds during fights, parries are rewarded, "
             .. "big attacks are amplified.",
  author = "GameViber",
  params = {
    parry          = button_param("RB", "Parry button"),
    dodge          = button_param("B", "Dodge button"),
    window         = number(0.3, 0.05, 1, "Parry window (s)", 0.05),
    parry_level    = number(1, 0, 1, "Parry reward intensity", 0.05),
    parry_len      = number(0.4, 0.1, 2, "Parry reward length (s)", 0.05),
    combat_timeout = number(20, 5, 120, "Fight ends after no rumble for (s)", 1),
    build          = number(90, 5, 600, "Time to reach the max wave (s)", 5),
    wave_max       = number(0.35, 0, 1, "Max wave intensity", 0.05),
    wave_period    = number(4, 1, 20, "Wave period (s)", 0.5),
    rumble_weight  = number(1, 0, 2, "Rumble multiplier", 0.05),
    big_peak       = number(0.7, 0, 1, "Big attack: level above", 0.05),
    big_len        = number(0.5, 0.1, 3, "Big attack: lasting longer than (s)", 0.1),
    big_boost      = number(1.5, 1, 3, "Big attack multiplier", 0.1),
  },
}

local FADE_SECS = 5

-- Times start far in the past so nothing counts as recent at activation.
local last_rumble, last_defense, vibration_start = -1e9, -1e9, 0
local heat, parries = 0, 0

function on_start()
  last_rumble, last_defense, vibration_start = -1e9, -1e9, 0
  heat, parries = 0, 0
end

function on_button(ev)
  if ev.pressed and (ev.button == P.parry or ev.button == P.dodge) then
    last_defense = ev.t
  end
end

function on_rumble_start(ev)
  vibration_start = ev.t
  if ev.t - last_defense <= P.window then
    pulse(P.parry_level, P.parry_len)
    parries += 1
    last_defense = -1e9
  end
end

function tick(dt, input)
  local r = input.rumble
  if r.active then
    last_rumble = input.time
  end

  local in_combat = input.time - last_rumble < P.combat_timeout
  if in_combat then
    heat = math.min(heat + dt / P.build, 1)
  else
    heat = math.max(heat - dt / FADE_SECS, 0)
  end
  local phase = 2 * math.pi * input.time / P.wave_period
  local wave = heat * P.wave_max * (0.5 - 0.5 * math.cos(phase))

  local hit = r.level * P.rumble_weight
  if r.active and r.level >= P.big_peak and input.time - vibration_start >= P.big_len then
    hit *= P.big_boost
  end

  set(math.max(wave, hit))

  plot("heat", heat)
  plot("parries", parries)
end
