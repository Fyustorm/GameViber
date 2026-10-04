You are going to write a GameViber mode made specifically for the game **{{GAME}}**.

# Context

GameViber is a Linux application that intercepts the rumble (force feedback) a game sends
to a gamepad, together with the player's button presses and stick/trigger positions, and
listens to the game's sound; it turns them into vibrations on adult toys connected
through Intiface Central (Buttplug protocol). The transformation is done by a **mode**:
a small Luau script. The full mode API specification is at the end of this message: it
is the only API you may use.

The goal is a mode that makes {{GAME}} feel great: it must match the game's real
mechanics (combat, parries, dodges, special moves, gauges, exploration, menus...) and the
way the game actually uses rumble.

# What to do

1. **Research the game.** If you can browse the web, look up {{GAME}}'s default
   controller bindings (Xbox layout) and its main mechanics. Otherwise use what you know
   and say which bindings you are unsure about. Find out, as far as you can, when the
   game makes the gamepad rumble (hits dealt, hits taken, parries, explosions, engines,
   cutscenes...), and what its music sounds like in its main phases (battles, exploration,
   towns, dialogue). Note which phases share the same kind of music: the sound cannot
   tell them apart (e.g. a game whose dungeons play epic, rhythmic music both while
   exploring and in fights).
2. **Design the mode.** Pick 2 to 4 mechanics that map well to vibrations, using what the
   mode can observe: rumble levels and vibration start/end, button presses, held buttons,
   stick and trigger axes, idle time, and the game's sound (loudness, hits, and scenes
   recognized from the music, §6.3). Explain the design in a few sentences.
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
- When the game's phases change how it should feel (battles, exploration, cutscenes),
  declare `audio_scenes` (§6.3): **2 or 3 contrasted scenes, described as sound**
  (music style, tempo, instruments, voices), never as game events. Describe what is
  only heard in that scene and the others as lighter background, e.g.
  `battle = "aggressive battle theme with heavy drums, brass stabs and screams"`,
  `explore = "light adventurous orchestral background music"`, `talk = "people talking,
  voice acting dialogue"`. Only declare scenes the music really tells apart, named after
  what they mean in the game (`dungeon` rather than `battle` when dungeons and fights
  share their music), and say in the design which phases the sound cannot separate.
  **Never rely on scenes alone**: they arrive 2 to 15 s late and are often wrong for a
  while. Use them for the mood (overall level, which mechanics are active) and tell the
  rest apart with the rumble and buttons (e.g. a fight inside a dungeon); never time an
  effect on them. The typical use (§6.3): the scene sets the tension, not the algorithm.
  The mechanics stay the same everywhere; in tense phases the mode adds a background,
  faded in and out over a few seconds, and its peaks still come from the rumble, buttons
  and hits. The mode must still work when `input.audio.scene` is nil (no sound captured,
  model not downloaded).
- **Intense phases usually feel best with a background that runs whatever the player
  does**: a slow sine **wave** (2 to 10 s per cycle) for battles and intense action, or
  a **heartbeat** pattern for games built on tension (horror, stealth). Detect the phase
  with an audio scene, recent rumble or combat activity (with tunable delays), and make
  the wave's low point, high point and period parameters. **A wave never goes down to
  0**: its low point is above 0 (e.g. 0.05 to 0.15, and its parameter minimum above 0
  too). GameViber plays any value of 0.01 or more at least at each toy's weakest
  intensity, so the low point is felt as the toy's gentlest vibration.
- `on_audio_hit` fires on any sudden sound, music beats included: filter `ev.strength`
  with a parameter and prefer the rumble for hits when the game rumbles.
- Call `plot("name", value)` for the internal state worth tuning (gauges, counters).
- Declare 2 to 5 `feedback` questions (§4.2) about the mechanics you designed, so the
  player can say what feels wrong with a few clicks, e.g.
  `dash = ask("Dash vibration length", { "Too short", "Good", "Too long" }, "Good", { param = "dash_len" })`.
  Link a question with `param` to the number parameter that fixes it whenever there is
  one: answers before the default must call for a larger value (`invert = true` if not).
- Feed the in-game overlay: it already shows the current audio scene; add
  `hud(label, value, max)` for the 1 or 2 gauges the player cares about while playing, and `hud_event(text)` when the mode detects something worth
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
