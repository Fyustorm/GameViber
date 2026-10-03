-- Forwards the game's rumble, like the Intiface Game Haptics Router.
mode {
  api = 1,
  name = "Simple",
  description = "Forwards the game's rumble, like the Game Haptics Router.",
  category = "Any game",
  help = "The game's rumble goes straight to your toys: when the gamepad would vibrate, your toy does. "
      .. "Use Multiplier to make it stronger and Minimum vibration for a constant background.",
  main_params = { "multiplier", "baseline" },
  author = "GameViber",
  params = {
    combine    = choice("avg", { "avg", "max" }, "Motor combination"),
    multiplier = number(1, 0, 5, "Multiplier", 0.1),
    baseline   = number(0, 0, 1, "Minimum vibration", 0.01),
  },
  feedback = {
    strength = ask("Overall strength", { "Too weak", "Good", "Too strong" }, "Good", { param = "multiplier" }),
    between  = ask("Between the game's vibrations", { "Too empty", "Good", "Never stops" }, "Good", { param = "baseline" }),
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
