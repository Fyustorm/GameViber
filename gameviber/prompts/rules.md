# What feels good

- **Continuous beats intermittent.** A vibration that keeps going and rises and falls with
  the action feels far better than a short buzz, silence, another short buzz. Build a
  base that follows what is going on, and use pulses as accents on top of it, not alone.
- **Leave room for the accents.** Pulses add to what plays: keep waves and backgrounds
  at 0.6 or below by default, or a pulse on top of them is cut at 1 and barely felt.
- **Intense phases get a background that runs whatever the player does**: a slow sine
  wave (2 to 10 s per cycle) in battles and intense action, a heartbeat pattern in games
  built on tension (horror, stealth). Detect the phase with recent rumble, combat
  activity, a phase or an indicator, with tunable delays, and fade it in and out over a few
  seconds.
- **A wave never goes down to 0.** Its low point is a parameter above 0 (e.g. 0.05 to
  0.15, minimum above 0 too). Any value of 0.01 or more plays at least at the toy's
  weakest intensity, which the player sets per toy.
- **Keep the game's rumble perceptible**, usually `math.max(effect, rumble * weight)`.
- **Quiet moments fade to 0**: menus, pauses, cutscenes without rumble (`input.idle`;
  `rumble_idle` counts from mode activation until the first vibration). Pulses too:
  sound impacts also come from menus, dialogue and music, so `on_impact` only plays when
  the mode's own check says the game is in action (an indicator, else recent rumble with
  a tunable delay, or a phase), never on its own.
- Toys get at most 20 updates per second: keep pulses and pattern steps at least 0.1 s
  long, and waves slower than about 5 Hz.

# Rules for the script

- Structure: `mode { ... }` holds the declaration only; callbacks are global functions
  defined after it (`function tick(dt, input)`, `function on_impact(ev)`).
- Only the API in the specification below. Luau sandbox: no `io`, `os`, `require`, files
  or network. English only, readable code, short comments where the intent is not
  obvious.
- **Every gamepad button the mode reacts to is a `button_param`** defaulting to the
  game's binding, labelled with what it does in the game (`button_param("LT", "Parry
  button")`). Never compare against a hard-coded button name.
- **Every heuristic is a parameter** with its unit in the label (`"Parry window (s)"`,
  `"Hit = vibration above"`). The rumble does not say who took a hit: "hit taken" or
  "successful parry" are guesses and must stay tunable.
- Phases (§6.3): when the request lists the game's phases, use those names and do not
  declare `phases`; otherwise declare 2 or 3 contrasted ones, each described as what is
  only heard (`sound`) and what is only seen (`screen`) there. Use them for the mood only
  (they come seconds late), never to time an effect. The mode must work when
  `input.phase` is nil.
  Filter `on_impact` with a strength parameter and the same action check as the
  background. Prefer these high-level inputs; the raw
  ones (§6.4) only when they say something the high-level ones do not.
- Indicators and external inputs (§6.5, in advanced requests) exist only once the player set them up:
  read them defensively (`input.indicators.hp or 1`). They say exactly what phases guess.
- `plot()` the internal state worth tuning; `hud()` for 1 or 2 gauges and `hud_event()`
  for short messages ("Parry!") in the in-game overlay.
- 2 to 5 `feedback` questions (§4.2) about the mechanics, each linked with `param` to the
  number parameter that fixes it when there is one.
- With strokers, a `thrust()` is also the pulse vibrators feel: never add a `pulse()` on
  the same channel for the same moment, they add up.
- Safety is not the script's job: the global cap, the panic stop and zeroing on gamepad
  loss are handled by GameViber.
