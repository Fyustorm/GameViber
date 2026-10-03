-- Action metroidvanias (Prince of Persia: The Lost Crown, Hollow Knight, Dead Cells...).
-- Platforming stays quiet: only the game's rumble plays. Fights wake up a pulsing glow
-- driven by a surge gauge that fills with every hit and much faster with parries (a
-- vibration right after the parry press). Holding the parry button and pressing a
-- surge button spends the gauge on a big crescendo, like Lost Crown's Athra Surges.
mode {
  api = 1,
  name = "Surge",
  description = "Action metroidvanias: hits and parries fill a surge gauge that glows during "
             .. "fights; spending it triggers a big crescendo.",
  category = "Metroidvania",
  help = "Exploration stays calm. During fights, a pulsing glow grows as you land hits and parries. "
      .. "A successful parry gives a strong pulse, and a special move (hold parry + attack) spends "
      .. "the gauge on a big crescendo.",
  main_params = { "parry_level", "glow_max", "surge_level" },
  author = "GameViber",
  params = {
    parry          = button_param("LT", "Parry button (also the surge modifier)"),
    surge_a        = button_param("X", "Surge button 1"),
    surge_b        = button_param("Y", "Surge button 2"),
    window         = number(0.3, 0.05, 1, "Parry window (s)", 0.05),
    parry_level    = number(1, 0, 1, "Parry pulse intensity", 0.05),
    parry_len      = number(0.35, 0.1, 2, "Parry pulse length (s)", 0.05),
    per_hit        = number(0.04, 0, 0.5, "Gauge per vibration", 0.01),
    per_parry      = number(0.2, 0, 1, "Gauge per parry", 0.05),
    surge_cost     = number(0.5, 0.1, 1, "Gauge spent per surge", 0.05),
    surge_level    = number(1, 0, 1, "Surge intensity", 0.05),
    glow_min       = number(0.05, 0, 1, "Fight glow at empty gauge", 0.01),
    glow_max       = number(0.4, 0, 1, "Fight glow at full gauge", 0.05),
    glow_period    = number(2, 0.5, 10, "Glow pulse period (s)", 0.1),
    combat_timeout = number(8, 2, 60, "Fight ends after no rumble for (s)", 1),
    drain          = number(0.02, 0, 0.5, "Gauge lost per second outside fights", 0.005),
    rumble_weight  = number(1, 0, 2, "Rumble multiplier", 0.05),
  },
  feedback = {
    parry = ask("Parry detection", { "Often missed", "Good", "Also on hits taken" }, "Good", { param = "window" }),
    glow  = ask("Glow during fights", { "Too weak", "Good", "Too strong" }, "Good", { param = "glow_max" }),
    surge = ask("Surge crescendo", { "Too weak", "Good", "Too strong" }, "Good", { param = "surge_level" }),
  },
}

local FADE_SECS = 2

-- Rising crescendo with a final hit, about 1.5 s.
local surge_wave = pattern {
  { 0.0, 0.3 }, { 0.2, 0.5 }, { 0.3, 0.3 }, { 0.5, 0.7 }, { 0.6, 0.4 },
  { 0.8, 0.9 }, { 0.9, 0.6 }, { 1.1, 1.0 }, { 1.5, 0.0 },
}

persist.gauge = persist.gauge or 0
-- Times start far in the past so nothing counts as recent at activation.
local last_rumble, last_parry = -1e9, -1e9
local holding, fight, parries, surges = false, 0, 0, 0

function on_start()
  last_rumble, last_parry = -1e9, -1e9
  holding, fight, parries, surges = false, 0, 0, 0
end

local function add(n)
  persist.gauge = clamp(persist.gauge + n, 0, 1)
end

function on_button(ev)
  if ev.button == P.parry then
    holding = ev.pressed
    if ev.pressed then
      last_parry = ev.t
    end
  elseif ev.pressed and holding and (ev.button == P.surge_a or ev.button == P.surge_b) then
    -- The surge's own rumble must not count as a parry.
    last_parry = -1e9
    if persist.gauge >= P.surge_cost then
      add(-P.surge_cost)
      play(surge_wave, { scale = P.surge_level })
      surges += 1
      hud_event("Surge!")
    end
  end
end

function on_rumble_start(ev)
  last_rumble = ev.t
  if ev.t - last_parry <= P.window then
    pulse(P.parry_level, P.parry_len)
    add(P.per_parry)
    parries += 1
    hud_event("Parry!")
    last_parry = -1e9
  else
    add(P.per_hit)
  end
end

function tick(dt, input)
  local r = input.rumble
  if r.active then
    last_rumble = input.time
  end

  if input.time - last_rumble < P.combat_timeout then
    fight = math.min(fight + dt / FADE_SECS, 1)
  else
    fight = math.max(fight - dt / FADE_SECS, 0)
    add(-P.drain * dt)
  end

  local phase = 2 * math.pi * input.time / P.glow_period
  local glow = lerp(P.glow_min, P.glow_max, persist.gauge) * (0.6 + 0.4 * math.cos(phase)) * fight

  set(math.max(glow, r.level * P.rumble_weight))

  hud("Surge gauge", math.floor(persist.gauge * 100), 100)
  plot("gauge", persist.gauge)
  plot("fight", fight)
  plot("parries", parries)
  plot("surges", surges)
end
