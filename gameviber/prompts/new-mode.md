You are going to write a GameViber mode made specifically for the game **{{GAME}}**.

# Context

GameViber is a Linux application that intercepts the rumble (force feedback) a game sends
to a gamepad, together with the player's button presses and stick/trigger positions, and
turns them into vibrations on adult toys connected through Intiface Central (Buttplug
protocol). The transformation is done by a **mode**: a small Luau script. The full mode
API specification is at the end of this message: it is the only API you may use.

The goal is a mode that makes {{GAME}} feel great: it must match the game's real
mechanics (combat, parries, dodges, special moves, gauges, exploration, menus...) and the
way the game actually uses rumble.

# What to do

1. **Research the game.** If you can browse the web, look up {{GAME}}'s default
   controller bindings (Xbox layout) and its main mechanics. Otherwise use what you know
   and say which bindings you are unsure about. Find out, as far as you can, when the
   game makes the gamepad rumble (hits dealt, hits taken, parries, explosions, engines,
   cutscenes...).
2. **Design the mode.** Pick 2 to 4 mechanics that map well to vibrations, using what the
   mode can observe: rumble levels and vibration start/end, button presses, held buttons,
   stick and trigger axes, idle time. Explain the design in a few sentences.
3. **Write the complete `.luau` file** in a single code block.
4. After the code, give a short **tuning guide**: which parameters to adjust first if
   something feels off (e.g. parries not detected, too strong in menus).

# Rules for the script

- Use only the API v1 described in the specification below (§13 lists features that do
  **not** exist). Luau sandbox: no `io`, `os`, `require`, files or network.
- Start with a header comment naming the game and summarising the mode.
- In `mode { }`: `api = 1`, `name` (short, may include the game's name), a one-line
  `description`, `category = "{{GAME}}"`, a plain-language `help` for players (2-3
  sentences, no jargon), `author = "AI for GameViber"`, `version = "1.0"`, and 1 to 3
  `main_params`: the settings a player is most likely to tune.
- **Every gamepad button the mode reacts to must be a `button_param`**, whose default is
  the game's default binding, and whose label says what the button does in the game
  (e.g. `button_param("LT", "Parry button")`). Players can remap controls, so never
  compare against a hard-coded button name. Triggers are available as the buttons `LT`
  and `RT` (pressed past half travel) and as the axes `input.axes.LT` / `input.axes.RT`.
- Every timing or threshold heuristic is a parameter with its unit in the label, e.g.
  `"Parry window (s)"`, `"Hit = vibration above"`. **The rumble does not say who took a
  hit**: anything like "a hit taken" or "a successful parry" is a guess and must stay
  tunable.
- Output to each toy is limited to 20 updates per second, so waveforms faster than about
  5 Hz blur. Keep patterns and pulses at least 0.1 s long.
- Safety is not the script's job: the global cap, the panic stop and zeroing on gamepad
  loss are handled by GameViber. Do not reimplement them.
- Quiet moments (menus, pauses, cutscenes without rumble) should fade to 0: use
  `input.idle`, `input.input_idle` or `input.rumble_idle`. Note that `rumble_idle`
  counts from mode activation when no vibration has happened yet.
- Keep the game's own rumble perceptible (usually `math.max(effect, rumble * weight)`)
  so the player still feels the game.
- Call `plot("name", value)` for the internal state worth tuning (gauges, counters).
- Declare 2 to 5 `feedback` questions (§4.2) about the mechanics you designed, so the
  player can say what feels wrong with a few clicks, e.g.
  `dash = ask("Dash vibration length", { "Too short", "Good", "Too long" }, "Good", { param = "dash_len" })`.
  Link a question with `param` to the number parameter that fixes it whenever there is
  one: answers before the default must call for a larger value (`invert = true` if not).
- Feed the in-game overlay: `hud(label, value, max)` for the 1 or 2 gauges the player
  cares about while playing, and `hud_event(text)` when the mode detects something worth
  telling ("Parry!", "Combo x5", "Overheat!"). Keep texts short.
- Initialise per-fight state in `on_start()`; use `persist` only for state that should
  survive editing the script.
- Clamp every gauge to its range; `set()` and `pulse()` already clamp to 0..1.
- English only, readable code, short comments where the intent is not obvious.

# Example

Here is a complete mode written for Prince of Persia: The Lost Crown (default bindings:
parry on LT, Athra Surges with LT held + X or Y). Follow its structure and style, not its
mechanics: {{GAME}} deserves its own design.

```lua
{{EXAMPLE}}
```

# Mode API specification

{{SPEC}}
