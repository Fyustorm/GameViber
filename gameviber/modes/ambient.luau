-- Exploration, platformers and cosy games (Mario, Hollow Knight, Zelda...). A slow, soft
-- wave plays in the background with the game's rumble on top. The wave fades out when
-- nothing happens (menus, pause) and comes back as soon as you play.
mode {
  api = 1,
  name = "Ambient",
  description = "Exploration and cosy games: a slow background wave under the game's "
             .. "rumble, fading out when idle.",
  category = "Exploration, cosy",
  help = "A soft, slow wave plays in the background with the game's rumble on top. The wave fades out "
      .. "in menus or when you pause, and comes back as soon as you play.",
  main_params = { "wave_max", "period", "rumble_weight" },
  author = "GameViber",
  params = {
    wave_min      = number(0.05, 0, 1, "Wave low", 0.01),
    wave_max      = number(0.25, 0, 1, "Wave high", 0.01),
    period        = number(8, 1, 60, "Wave period (s)", 0.5),
    rumble_weight = number(1, 0, 3, "Rumble multiplier", 0.05),
    fade_after    = number(30, 0, 600, "Fade out after inactivity (s), 0 = never", 5),
    fade_len      = number(10, 0.5, 60, "Fade duration (s)", 0.5),
  },
}

function tick(dt, input)
  local phase = 2 * math.pi * input.time / P.period
  local wave = lerp(P.wave_min, P.wave_max, 0.5 - 0.5 * math.cos(phase))
  if P.fade_after > 0 then
    wave *= clamp(1 - (input.idle - P.fade_after) / P.fade_len, 0, 1)
  end

  set(math.max(wave, input.rumble.level * P.rumble_weight))

  plot("wave", wave)
end
