-- Forwards the game's rumble, like the Intiface Game Haptics Router.
mode {
  api = 1,
  name = "Simple",
  description = "Forwards the game's rumble, like the Game Haptics Router.",
  author = "GameViber",
  params = {
    combine    = choice("avg", { "avg", "max" }, "Motor combination"),
    multiplier = number(1, 0, 5, "Multiplier", 0.1),
    baseline   = number(0, 0, 1, "Minimum vibration", 0.01),
  },
}

function tick(dt, input)
  local r = input.rumble
  local level = (P.combine == "max") and r.level or r.avg
  if level == 0 and P.baseline == 0 then
    set(0)
  else
    set(math.max(level * P.multiplier, P.baseline))
  end
end
