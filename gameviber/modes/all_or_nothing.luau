-- Demanding games and souls-likes (Elden Ring, Sekiro, Hades...). A reward gauge slowly
-- fills while you play without taking big hits; a strong vibration (often a hit taken)
-- empties it with a short punishment pulse. The longer you survive, the stronger it gets.
mode {
  api = 1,
  name = "All or Nothing",
  description = "Souls-likes: a gauge fills while you survive and a big hit empties it.",
  category = "Souls-likes",
  help = "A reward gauge slowly fills while you play without taking big hits, and the vibration grows "
      .. "with it. A big hit empties the gauge with a short punishment pulse.",
  main_params = { "rise", "hit", "max_level" },
  author = "GameViber",
  params = {
    rise          = number(0.01, 0.001, 0.1, "Gauge gained per second of play", 0.001),
    active_within = number(2, 0.5, 10, "Playing = input within (s)", 0.5),
    hit           = number(0.6, 0, 1, "Hit = vibration above", 0.05),
    loss          = number(1, 0, 1, "Gauge lost per hit (fraction)", 0.05),
    punish        = number(0.8, 0, 1, "Punishment pulse intensity", 0.05),
    punish_len    = number(0.5, 0.1, 3, "Punishment pulse length (s)", 0.1),
    min_level     = number(0.05, 0, 1, "Vibration at empty gauge", 0.01),
    max_level     = number(1, 0, 1, "Vibration at full gauge", 0.05),
    curve         = choice("quadratic", { "linear", "quadratic" }, "Gauge curve"),
    rumble_weight = number(0.3, 0, 1, "Rumble multiplier", 0.05),
    pause_idle    = number(30, 0, 600, "Pause after inactivity (s), 0 = never", 5),
  },
}

persist.gauge = persist.gauge or 0
local prev_level = 0

function on_start()
  prev_level = 0
end

function on_rumble(ev)
  if ev.level >= P.hit and prev_level < P.hit then
    persist.gauge *= 1 - P.loss
    pulse(P.punish, P.punish_len)
  end
  prev_level = ev.level
end

function tick(dt, input)
  if input.input_idle <= P.active_within then
    persist.gauge = math.min(persist.gauge + P.rise * dt, 1)
  end

  local g = persist.gauge
  if P.curve == "quadratic" then
    g *= g
  end
  local reward = lerp(P.min_level, P.max_level, g)
  if P.pause_idle > 0 and input.idle > P.pause_idle then
    reward = 0
  end

  set(math.max(reward, input.rumble.level * P.rumble_weight))

  plot("gauge", persist.gauge)
end
