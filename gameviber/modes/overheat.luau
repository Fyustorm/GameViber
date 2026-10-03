-- Action and shooter games (Ratchet & Clank, Doom...). The near-continuous weapon rumble
-- is compressed into a light texture and only sharp peaks stand out. Firing heats a
-- gauge that drives a background vibration; a full gauge triggers an overheat.
mode {
  api = 1,
  name = "Overheat",
  description = "Action/shooter games: continuous rumble is compressed, peaks stand out, "
             .. "firing heats a gauge until it overheats.",
  category = "Action, shooters",
  help = "Constant gunfire is softened into a light texture so that big impacts stand out. Firing heats up "
      .. "a gauge that adds a growing vibration; when it is full, you overheat.",
  main_params = { "texture", "trigger", "heat_level" },
  author = "GameViber",
  params = {
    texture      = number(0.3, 0, 1, "Continuous rumble weight", 0.05),
    peak         = number(0.6, 0, 1, "Peak threshold", 0.05),
    peak_len     = number(0.25, 0.05, 2, "Peak pulse length (s)", 0.05),
    heat_src     = choice("both", { "both", "trigger", "rumble" }, "Heat while..."),
    trigger      = button_param("RT", "Fire button"),
    heat_rate    = number(0.15, 0.01, 1, "Heat per second while firing", 0.01),
    cool_rate    = number(0.25, 0, 2, "Cooling per second", 0.01),
    heat_level   = number(0.5, 0, 1, "Vibration at full heat", 0.05),
    overheat_len = number(3, 0.5, 10, "Overheat duration (s)", 0.5),
  },
  feedback = {
    texture = ask("Continuous rumble", { "Too weak", "Good", "Too strong" }, "Good", { param = "texture" }),
    peaks   = ask("Big hits standing out", { "Too few", "Good", "Too many" }, "Good", { param = "peak", invert = true }),
    heat    = ask("Overheating", { "Never happens", "Good", "Too often" }, "Good", { param = "heat_rate" }),
  },
}

local overheat_wave = pattern { { 0.0, 1.0 }, { 0.15, 0.6 }, { 0.3, 1.0 } }

persist.heat = persist.heat or 0
local overheating, overheat_play, prev_level = false, nil, 0

local function start_overheat()
  overheating = true
  overheat_play = play(overheat_wave, { loops = 0 })
  after(P.overheat_len, function()
    overheat_play:stop()
    overheating = false
    persist.heat = 0
  end)
end

function on_start()
  overheating, prev_level = false, 0
end

function on_rumble(ev)
  if ev.level >= P.peak and prev_level < P.peak then
    pulse(ev.level, P.peak_len)
  end
  prev_level = ev.level
end

function tick(dt, input)
  local trigger = input.buttons[P.trigger]
  local firing = ({
    both = trigger or input.rumble.active,
    trigger = trigger,
    rumble = input.rumble.active,
  })[P.heat_src]

  if not overheating then
    if firing then
      persist.heat = math.min(persist.heat + P.heat_rate * dt, 1)
    else
      persist.heat = math.max(persist.heat - P.cool_rate * dt, 0)
    end
    if persist.heat >= 1 then
      start_overheat()
    end
  end

  set(math.max(input.rumble.level * P.texture, persist.heat * P.heat_level))

  plot("heat", persist.heat)
end
