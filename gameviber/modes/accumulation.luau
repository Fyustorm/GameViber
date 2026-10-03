-- Every vibration and every press of the bonus button adds points.
-- The game's rumble is scaled by the points, which drain while idle.
mode {
  api = 1,
  name = "Accumulation",
  description = "Vibrations and bonus presses add points; the rumble is scaled by points, "
             .. "which drain while idle.",
  category = "Any game",
  help = "Every vibration and every press of the bonus button adds points. The more points, the stronger "
      .. "the game's rumble feels. Points slowly drain when nothing happens.",
  main_params = { "per_hit", "bonus", "decay" },
  author = "GameViber",
  params = {
    per_hit   = number(5, 0, 50, "Points per vibration"),
    bonus     = button_param("A", "Bonus button"),
    per_press = number(1, 0, 10, "Points per bonus press"),
    decay     = number(2, 0, 20, "Points lost per second"),
    idle      = number(1.5, 0, 10, "Idle delay before draining (s)"),
    idle_src  = choice("any", { "any", "rumble", "input" }, "Idle means no..."),
    max       = number(100, 10, 500, "Max points", 5),
    floor     = number(0.2, 0, 1, "Constant vibration at max points", 0.05),
  },
  feedback = {
    build = ask("How fast it builds up", { "Too slow", "Good", "Too fast" }, "Good", { param = "per_hit" }),
    fade  = ask("How fast it fades when idle", { "Too slow", "Good", "Too fast" }, "Good", { param = "decay" }),
    top   = ask("Vibration at full points", { "Too weak", "Good", "Too strong" }, "Good", { param = "floor" }),
  },
}

persist.points = persist.points or 0

local function add(n)
  persist.points = clamp(persist.points + n, 0, P.max)
end

function on_rumble_start(ev)
  add(P.per_hit)
end

function on_button(ev)
  if ev.pressed and ev.button == P.bonus then
    add(P.per_press)
  end
end

function tick(dt, input)
  local idle = ({ any = input.idle, rumble = input.rumble_idle, input = input.input_idle })[P.idle_src]
  if idle > P.idle then
    add(-P.decay * dt)
  end

  local ratio = persist.points / P.max
  set(math.max(input.rumble.level * ratio, P.floor * ratio))

  plot("points", persist.points)
  plot("ratio", ratio)
end
