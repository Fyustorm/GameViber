# GameViber — Mode specification (API v1)

Status: implemented (v1, with the game's sound and image) · API version: `1`

## 1. Goal and scope

A **mode** is a Lua script that turns what happens in the game (rumble sent to the
gamepad, player inputs, the game's sound and image, values other programs send, time)
into commands for the toys connected to Intiface.
Modes are created and edited on the fly from the built-in editor, without recompiling.

**Included in v1**

- A single active mode at a time.
- A single intercepted gamepad.
- Scalar Buttplug outputs: `Vibrate`, `Rotate`, `Oscillate`.
- Hot reload, parameters adjustable from the GUI, debug graphs, simulator.
- What GameViber makes of the game's sound and image: phases, impacts, intensity (§6.3);
  their raw measures (§6.4); indicators read on the screen, example images and values
  other programs send (external inputs), set up by the player for a mode (§6.5).

**Not in v1** (see §13): mode chaining, automatic per-game mode, linear outputs
(strokers), multiple gamepads, block editor.

## 2. Architecture

```
Gamepad proxy ──► event queue ──► Lua runtime (active mode) ──► Safety layer ──► Buttplug output
 (uinput, FF,     (timestamped)     fixed 50 Hz tick               (not scriptable)   (rate limited)
  buttons, axes)
```

- The **proxy** produces raw events: FF effects (upload, play, stop), buttons, axes.
- The **audio analysis** listens to the game's sound through PipeWire and produces levels,
  hits and phase embeddings (§6.3), on its own threads.
- The **in-game overlay** copies small images of the game (§6.4); GameViber measures
  them, reads the mode's indicators and embeds them for phases.
- **Other programs** send values and events over a local WebSocket or a pipe (§6.5).
- The **runtime** normalizes these events, dispatches them to the mode's callbacks, then
  calls `tick`. It runs in its own thread.
- The **safety layer** applies the global cap and the panic button, and handles connection
  losses. A script cannot bypass it.
- The **output** forwards logical channels to the real actuators, drops duplicates and
  limits the rate sent to Intiface.

## 3. Mode file

- One file = one mode, `.luau` extension, UTF-8 encoding.
- Locations:
  - user modes: `~/.config/gameviber/modes/`
  - built-in modes: embedded in the binary, read-only and duplicable from the editor.
- The file must call the `mode { ... }` function **exactly once**, at the top level:

```lua
mode {
  api         = 1,                          -- required
  name        = "Accumulation",             -- required, shown in the GUI
  description = "Vibrations and bonus presses add points...",
  category    = "Any game",                 -- games it suits, shown on the mode tile
  help        = "Every vibration adds points...", -- plain-language explanation for players
  main_params = { "per_hit", "bonus" },     -- parameters shown first (see §4)
  author      = "me",
  version     = "1.0",
  channels    = { "main" },                 -- output channels, default { "main" }
  params      = { ... },                    -- see §4
  feedback    = { ... },                    -- questions for the player, see §4.2
  phases      = { ... },                    -- phases recognized from the sound and image, see §6.3
}
```

## 4. Parameters

Every parameter declared in `params` is shown automatically in the GUI and read in the
script through the read-only table `P`.

| Constructor | GUI control | Value in `P` |
|---|---|---|
| `number(default, min, max, label [, step])` | slider | number |
| `bool(default, label)` | checkbox | boolean |
| `choice(default, { "a", "b", ... }, label)` | drop-down list | string |
| `button_param(default, label)` | gamepad button selector | button name (§6.2) |

```lua
params = {
  per_hit = number(5, 0, 50, "Points per vibration"),
  source  = choice("any", { "any", "rumble", "input" }, "Idle means no..."),
  bonus   = button_param("A", "Bonus button"),
}
```

- `main_params` lists the few parameters a player is most likely to tune. The GUI shows
  them next to the mode and keeps the others behind "All settings". Every name must be
  a declared parameter. Without `main_params`, the GUI shows every parameter.
- Values are saved per mode in `~/.config/gameviber/params/<key>.toml`
  (key = built-in mode name, or file name without extension).
- On reload, a value is kept if the parameter keeps the same name and type. Otherwise it
  is reset to its default.
- Any change from the GUI calls `on_param_changed(name, value)`, if that callback is
  defined.

### 4.1 Presets

A **preset** is a named set of parameter values for one mode, typically one per game
(e.g. "Tekken 8" for the Combo mode). Presets are managed from the GUI (load, save,
save as, delete, reset to defaults) and are invisible to the script.

- They are stored in `~/.config/gameviber/presets/<key>.toml`, with the name of the
  preset last loaded or saved (`active`):

```toml
active = "Tekken 8"

[presets."Tekken 8"]
window = 0.4
max_hits = 8

[presets."Street Fighter 6"]
window = 0.5
```

- Loading a preset sets **every** parameter: a value missing from the preset, or no
  longer valid (wrong type, unknown option), falls back to the default; numbers are
  clamped to their range. Values of parameters the mode no longer declares are ignored.
- `on_param_changed` is called only for the parameters whose value actually changes.
- The GUI shows "(modified)" when the current values differ from the active preset;
  selecting the preset again reverts the changes.
- `--preset <name>` loads a preset of the startup mode (useful with `--headless`).

### 4.2 Feedback questions

When a mode does not feel right, the player opens **Doesn't feel right?** in the GUI.
`feedback` declares the questions it asks, so that the player says what is wrong with a
few clicks instead of describing it. Without `feedback`, the GUI asks generic questions
("Too strong overall", "Misses moments that should be felt"...); with it, the mode's
questions come first and the generic ones sit behind "Other problems".

```lua
feedback = {
  dash  = ask("Dash vibration length", { "Too short", "Good", "Too long" }, "Good", { param = "dash_len" }),
  parry = ask("Parry detection", { "Often missed", "Good", "Also on hits taken" }, "Good", { param = "window" }),
  hit   = ask("Hits", { "Often missed", "Good", "Small bumps too" }, "Good", { param = "hit", invert = true }),
  menus = ask("Vibrates in menus or cutscenes"),
},
```

`ask(label [, answers [, default [, opts]]])`:

- `answers`: 2 or more answers, shown as buttons. Without answers, the question is a
  checkbox (ticked = a problem).
- `default`: the answer meaning "fine", selected at first. Defaults to the middle answer.
- `opts.param`: a `number()` parameter that fixes the problem. When the player picks
  another answer, the GUI offers a **quick fix** that moves the parameter: answers
  before the default call for a larger value, answers after it for a smaller one (the
  reverse with `opts.invert = true`). Each answer away from the default moves it by a
  quarter of its value, at least 5% of its range, snapped to its step.
- Questions are shown in declaration order; their keys only identify them.

Answers away from the default go into the request sent to the AI assistant, together
with the setting each one is linked to, the earlier rounds of fixes for this mode, and a
recorded session replayed into the mode. Scripts never see the answers.

## 5. Callbacks

All callbacks are optional, except `tick`. They are global functions defined after
`mode { }` (`function tick(dt, input) ... end`); functions of the same names in the
`mode { }` table are accepted too.

| Callback | Called when |
|---|---|
| `on_start()` | the mode is activated, or reloaded |
| `on_stop()` | the mode is deactivated, reloaded, or suspended after an error |
| `tick(dt, input)` | on every tick (50 Hz), after events are dispatched |
| `on_rumble(ev)` | on every rumble level change |
| `on_rumble_start(ev)` | a vibration starts (§6.1) |
| `on_rumble_end(ev)` | a vibration ends (§6.1) |
| `on_button(ev)` | a button is pressed or released |
| `on_param_changed(name, value)` | a parameter was changed in the GUI |
| `on_device(ev)` | a toy is connected or disconnected (`ev.connected`, `ev.name`) |
| `on_phase(ev)` | the phase recognized in the game's sound and image changes (§6.3) |
| `on_impact(ev)` | an impact is heard or seen (§6.3) |
| `on_audio_hit(ev)` | any hit is heard in the game's sound (§6.4) |
| `on_indicator(ev)` | an indicator of the screen changes (§6.5) |
| `on_event(ev)` | another program sends an event (§6.5) |

**Execution order on every tick:**

1. Events accumulated since the previous tick are dispatched to the callbacks, in
   chronological order.
2. `tick(dt, input)` is called.
3. Outputs are frozen, then handed to the safety layer.

## 6. Events

Every event has an `ev.t` field: the mode time in seconds at the tick where the event is
dispatched (the same as `input.time`).

### 6.1 Rumble

The game's rumble is normalized to 0..1:

- `strong`: heavy motor, left;
- `weak`: light motor, right;
- `level` = `max(strong, weak)`.

It is computed from evdev force-feedback semantics (effects, durations, gain), as in the
prototype.

| Field | `on_rumble` | `on_rumble_start` | `on_rumble_end` |
|---|---|---|---|
| `strong`, `weak`, `level` | ✓ | ✓ | values at 0 |
| `peak`: max level of the vibration | | | ✓ |
| `duration`: duration in s | | | ✓ |

**Splitting into vibrations**

- A vibration starts when `level` goes above `rumble_threshold` (default 0.05).
- It ends when `level` stays below that threshold for `rumble_release` (default 80 ms).
  This delay avoids splitting a burst of very short effects into several vibrations.
- Both settings are global in the GUI. A mode can override them in `mode { }`:
  `rumble_threshold = 0.1`, `rumble_release = 0.15`.

### 6.2 Buttons

`on_button(ev)` receives:

- `ev.button`: normalized name, Xbox layout;
- `ev.pressed`: `true` on press, `false` on release.

Available buttons:

```
A B X Y  LB RB  BACK START GUIDE  LS RS  DPAD_UP DPAD_DOWN DPAD_LEFT DPAD_RIGHT
P1 P2 P3 P4  SHARE
```

`P1` to `P4` are the back paddles and `SHARE` the share button, on the gamepads whose
driver reports them (many gamepads in Xbox 360 mode only copy the paddles onto other
buttons). The player may keep them for GameViber's combos: the game then never gets them (with
the proxy source), modes still do.

The d-pad is converted to buttons even when the driver exposes it as axes (hat). Triggers
stay axes (§7), but `LT` and `RT` also generate a button event when they cross 0.5 up or
down.

Axis movements do not generate callbacks, to avoid a flood of events. Their current state
is read from `input.axes` on every tick.

### 6.3 Phases, impacts and intensity

GameViber listens to the game's sound through PipeWire (by default the game showing the
in-game overlay, or else everything the computer plays; the player can pick another
source per game, or for all in Setup) and, through the in-game overlay, looks at its image. Both are
analysed on the player's computer and never saved: recorded sessions keep only what is
measured, so that a replay feeds the mode the same events. What GameViber makes of them
comes in three high-level inputs; prefer them to the raw measures of §6.4.

**Phases** are the parts of the game that should not feel the same (a battle,
exploring, a dialogue). **The player sets them up for a mode**, in its Inputs (§6.5):
their names, how they sound, and captures of how they look. A request to an AI assistant
lists their names, and the mode only reads them (`input.phase`, `on_phase`) without
declaring anything.

A mode that suits any game (a built-in one), or played with a game that has no phases
yet, can describe its own in words, as they sound and as they look; the phases set up
for it, when it has some, replace them:

```lua
phases = {
  battle  = { sound  = "aggressive battle theme with heavy drums and brass stabs",
              screen = "a turn-based battle menu with command icons and enemy monsters" },
  explore = { sound  = "light adventurous orchestral background music",
              screen = "a character running through a forest or a dungeon" },
  story   = { sound  = "people talking, voice acting dialogue over soft music",
              screen = "a dialogue box with a character portrait and subtitles" },
},
phase_window = 10,   -- seconds the probabilities are averaged over (2 to 60), default 10
```

- 2 to 8 phases, sorted by name. Each has a `sound` description, a `screen` description,
  or both. A sound model (CLAP) compares the last 10 s of sound with the `sound`
  descriptions every 2 s; an image model (CLIP) compares the image with the `screen`
  descriptions every second, and with the game's **captures** (§6.5) of phases of the
  same name. Each sense only speaks about
  the phases it has a description or examples for; their probabilities are multiplied
  and averaged over `phase_window`.
- A phase is entered when its average reaches 0.5 and, when another phase is current,
  beats it by 0.1. `on_phase(ev)` is then called with `ev.phase`, `ev.previous` (nil at
  first) and `ev.confidence` (its average). When neither the sound nor the image is left,
  the phase is forgotten: `ev.phase` is nil. `input.phase`, `input.phase_confidence` and
  `input.phases.<name>` (the average probability of each) follow it.
- In its Inputs, a phase can be tied to **indicators** (§6.5) shown only in it (the
  battle menu): while they are all shown (for half a second) the phase is certain,
  entered at once with a confidence of 1, and while indicators are read it is only
  entered through them. An indicator can be part of the signs of several phases: of the
  signs shown, the one of the most indicators wins (battle: a gauge and its menu,
  exploration: the gauge alone). One phase can be the phase of **none of the others**
  (story): while indicators are read, it is entered when no phase's sign has been shown
  for half a second and no phase without a sign is likely. Each phase set up is also
  **kept** a few seconds (3 by default) after its last sign (its indicators gone, or
  another phase more likely), so that a sign that comes and goes does not flip it. When
  the indicators are gone and no other phase is likely, `ev.phase` is nil.
  Modes still get phases late and must not time effects on them.
- A phase set up can also **ignore** the sound's hits (`on_audio_hit`, and `on_impact`
  from the sound) or the image's flashes (`on_impact` from the screen): while it is the
  current phase, the mode does not get them (a menu's clicks and music heard as hits).
- **Describe what is only heard or only seen in a phase**: music style, tempo,
  instruments, voices for `sound`; the interface, framing and colors for `screen`. Two or
  three contrasted phases work much better than many close ones. Make them contrast:
  describe the strong phase by what only it has ("aggressive battle theme with heavy
  drums, brass stabs and screams") and the others as lighter background.
- **Each sense only knows what it shows.** Phases sharing their music cannot be told
  apart by the sound: in Metaphor: ReFantazio, dungeons play epic, rhythmic music both
  while exploring and in fights. The image tells them apart when its descriptions name
  what differs (the battle menu), and much better with example images: give such phases
  a `screen` description, or merge them into one phase and let the rumble and buttons
  tell them apart. Measured on that game: story versus action is reliable from the sound;
  battle versus exploration needs the image (about 85 % right with ten examples per
  phase, less from descriptions alone).
- **Expect mistakes.** A phase appears seconds late (2 to 15 s from the sound) and can
  stay wrong for a while (a cutscene keeping the battle music). Use phases for the mood
  (overall level, which mechanics are active), not to time effects; combine them with
  the rumble and the buttons, which say when the action really happens.
- **Typical use: the phase sets the tension, not the algorithm.** The mode keeps the
  same mechanics everywhere; in tense phases it adds a background that runs whatever
  the player does, and lets its peaks come from the rumble, the buttons and impacts, as
  usual. The background is usually:
  - a **slow wave** (a sine, 2 to 10 s per cycle) in battles and intense action;
  - a **heartbeat** (a `pattern`, §8) in games built on tension (horror, stealth).

  **A wave never goes down to 0**: its low point is a parameter above 0 (e.g. 0.05 to
  0.15, with a minimum above 0 too), not silence. GameViber plays any value of 0.01 or
  more at least at each toy's weakest intensity (set by the player per toy), so the low point is felt as the
  toy's gentlest vibration and the wave never stops while the phase lasts. Fade the
  background in and out over a few seconds, so a late or wrong phase is barely felt:

```lua
phases = {
  dungeon = { sound = "epic rhythmic orchestral music with heavy drums" },
  story   = { sound = "people talking, voice acting dialogue over soft music",
              screen = "a dialogue box with a character portrait and subtitles" },
},
params = {
  wave_low    = number(0.08, 0.02, 0.4, "Dungeon wave low point"),
  wave_high   = number(0.3, 0.05, 1, "Dungeon wave high point"),
  wave_period = number(5, 2, 10, "Dungeon wave period (s)"),
},

local tension = 0
function tick(dt, input)
  local target = input.phase == "dungeon" and 1 or 0
  tension += (target - tension) * math.min(1, dt / 3)  -- ~3 s fade
  local swing = 0.5 - 0.5 * math.cos(2 * math.pi * input.time / P.wave_period)
  local wave = (P.wave_low + (P.wave_high - P.wave_low) * swing) * tension
  set(math.max(wave, input.rumble.level))              -- peaks still follow the game
end
```

  The same background suits phases told apart otherwise, e.g. "the game rumbled in the
  last 10 s" for a fight (with the delay as a parameter).

- Phases need the models, downloaded once (the sound model, about 200 MB, and the image
  model, about 150 MB, from the Creator's Phases tab or Setup › Sound), and cost a little processor
  time while the mode is active. Without them, or without sound and image,
  `input.phase` stays nil: **a mode must work without phases**.

**Impacts**: `on_impact(ev)` is called on a strong hit in the sound (an impact, a shot,
an explosion, but also a door or a beat; about 20 ms late) or a sudden flash of the
image (an explosion, a critical hit; about 0.1 s late).

- `ev.strength`: 0..1. `ev.source`: `"sound"` or `"screen"`.
- Impacts do not say what happened nor to whom: filter them with a parameter
  (`if ev.strength > P.min_impact then ... end`) and combine them with the rumble.

**Intensity**: `input.intensity` (0..1) is how busy the game is over the last seconds:
loudness and hits of the sound, motion of the image, averaged over the senses there are.
It rises in action and falls in menus and dialogue; it is slow, not a trigger.

### 6.4 Raw sound and image

The measures behind §6.3, for what the high-level inputs do not say.

**Sound levels** (`input.audio`, §7.1) are measured every 20 ms. They are relative to
the game's recent loudest moments (a 40 dB range under a slowly decaying peak), so they
do not depend on the volume setting; silence reads 0.

- `level`: overall loudness; `low` (below 250 Hz: explosions, engines, bass), `mid`
  (250 Hz to 4 kHz: voices, most effects), `high` (4 to 16 kHz: clashes, shots, cymbals).
- `intensity`: loudness and density of hits over the last ~6 s.

**Hits**: `on_audio_hit(ev)` is called on every sudden attack in the sound, weak ones
included (`on_impact` only gets the strong ones), with `ev.strength` (0..1, compared with
the strongest recent hits of the same band) and `ev.band` (`"low"`, `"mid"` or `"high"`).

**The image** (`input.screen`, §7.1): the in-game overlay copies a small image of the
game ten times per second (only in games started with it, see the Overlay page; the
player can turn this off on a game's captures page).

- `brightness`: average brightness, 0..1.
- `motion`: how much the image changed since the previous copy, 0..1 (camera moves,
  effects); `action`: motion over the last ~6 s.

### 6.5 The mode's inputs

Besides what GameViber reads by itself (the rumble, the buttons, the sound, the image),
the player teaches it about the game in a mode's **Inputs**: phases (§6.3), captures,
indicators and external inputs. They are part of the mode (its package) and travel with
it when it is shared; built-in modes have none. Everything here exists only once the
player set it up: **read it defensively** (`input.indicators.hp or 1`).

**Captures**: images of the game the player captured per phase ("battle", "dungeon"...),
in game with a gamepad combo or from the Creator. They are the examples phases are
recognized with, and indicators are drawn on them.

**Indicators**: parts of the game's interface whose state is read, in one or more
**zones** (rectangles of the screen the player drew on a capture, checked on all of
them; an indicator shown in several places has a zone for each).

- A **visibility** indicator (the battle interface, a warning) reads `true` while
  the screen there looks like when it was drawn, `false` otherwise.
- A **gauge** (health, stamina) reads how full its bar is, 0..1, measured with the colors of
  its full and empty parts (or with how it looks full and empty along its length:
  gradients, segments, hearts), each color an equal share for a bar filled again over itself
  in another color once full (green up to 0.5, then yellow over it up to 1), or **nil
  while it is not on screen** (a menu, a cutscene; or while a visibility indicator the player
  set it to be read under is not as required: a menu's button shown): not knowing is not 0 health. Keep the last known value when the mode needs one. A bar that moves (Metaphor shifts a character's health bar
  with its stance) is found in a wider zone as the longest run of its two colors.
- `input.indicators.<name>` holds the value; `on_indicator(ev)` is called when it
  changes, with `ev.indicator`, `ev.value` (nil for a gauge gone from the screen) and
  `ev.previous` (nil at first). Indicators are read ten times per second; a gauge change
  below 0.02 is not reported.
- Indicators say exactly what phases guess (the battle interface is on screen or not)
  and come within 0.1 s: prefer them when the game has them.


**External inputs**: values and events from other programs. A game's existing mod, a script reading a game's API or
anything else can send JSON to `ws://127.0.0.1:12350` (the port is set in Setup;
web pages are refused) or, one message per line, to the pipe
`$XDG_RUNTIME_DIR/gameviber/external`:

```json
{"set": {"hp": 0.4, "stance": "low"}}
{"event": "kill", "data": {"weapon": "bow"}}
```

- `set` keeps values in `input.external.<name>` (numbers, booleans, strings, or tables);
  `null` removes one. Names are letters, digits and `_`, starting with a letter.
- `event` calls `on_event(ev)` with `ev.name` and `ev.data` (nil without `data`).
- The player declares in the Creator (Other programs) what the program sends ("hp: health, 0 to
  100"), so that an AI assistant writing a mode for the game knows it.

## 7. The `input` table (current state, read-only)

`input` is also a global: event callbacks read it as the previous tick left it, except
`input.time`, already the current tick's.

```lua
input.time               -- s since the mode was activated
input.rumble.strong      -- 0..1
input.rumble.weak        -- 0..1
input.rumble.level       -- max(strong, weak)
input.rumble.avg         -- (strong + weak) / 2
input.rumble.active      -- true during a vibration (§6.1)
input.buttons.A          -- true while held (same for every button in §6.2)
input.axes.LX, LY, RX, RY  -- -1..1 (0.1 dead zone applied)
input.axes.LT, RT        -- 0..1
input.rumble_idle        -- s since the end of the last vibration (0 while active)
input.input_idle         -- s since the last player input (button, or axis outside the dead zone)
input.idle               -- min(rumble_idle, input_idle)
input.phase              -- name of the current phase (§6.3), or nil
input.phase_confidence   -- 0..1, average probability of the current phase
input.phases.battle      -- 0..1, average probability of each declared phase
input.intensity          -- 0..1, how busy the game's sound and image are lately (§6.3)
```

The idle times do not take the sound or the image into account.

### 7.1 Advanced inputs

```lua
input.audio.active       -- true while the game's sound is captured (§6.4)
input.audio.level        -- 0..1, and input.audio.low, .mid, .high per band
input.audio.intensity    -- 0..1, loudness and hits over the last ~6 s
input.screen.active      -- true while the game's image is copied (§6.4)
input.screen.brightness  -- 0..1
input.screen.motion      -- 0..1, change since the previous copy
input.screen.action      -- 0..1, motion over the last ~6 s
input.indicators.battle_hud  -- true/false, or 0..1 for a gauge; nil if unknown (§6.5)
input.external.hp            -- what another program set; nil until it does (§6.5)
```

When the sound is not captured, every `input.audio` number is 0; likewise for
`input.screen` without the image, and the indicators are gone with it.

Modes written before the terms changed keep working: `scenes`, `scene_window`,
`on_scene` (`ev.scene`), `input.scene`, `input.scene_confidence`, `input.scenes`,
`input.zones`, `on_zone` (`ev.zone`) and `input.custom` are the same as `phases`,
`phase_window`, `on_phase` (`ev.phase`), `input.phase`, `input.phase_confidence`,
`input.phases`, `input.indicators`, `on_indicator` (`ev.indicator`) and
`input.external`. New modes use the new names.

## 8. Outputs

### 8.1 Channels

The script does not know the toys. It writes to **logical channels**, declared in
`channels`. The GUI maps each channel to one or more toys, for example `main` to the
Lush and `aux` to the Nora. A toy mapped to several channels plays the strongest of
them; a toy mapped to no channel stays at 0.

Toys are identified by the name given in Intiface Central, or else by their device
name; identical toys are numbered (`Lush 3`, `Lush 3 #2`) in the order they connect.

### 8.2 Functions

```lua
set(x [, channel])            -- base level of the channel, 0..1, held until the next set
pulse(x, seconds [, channel]) -- temporary overlay of intensity x for `seconds`
play(pattern [, opts])        -- plays a pattern (§8.3), returns a handle with :stop()
stop_all()                    -- resets the base level to 0 and cancels pulses and patterns
```

- `channel` defaults to `"main"`, or `"*"` for all channels.
- Values outside 0..1 are silently clamped.
- **Final channel value** = `max(base level, active pulses, active patterns)`.
- v1: a channel drives all the actuators (vibration, rotation, oscillation) of the toys
  mapped to it. For the script, it is always an intensity between 0 and 1. A stroker
  (a toy moving to positions) plays it as strokes, faster and longer as it rises
  (§12): a mode does not need to know it.
- Strokers follow the intensity a few hundred milliseconds late, at the next change of
  direction (a strong rise turns them sooner), and cannot render waveforms faster than
  their strokes: give them the envelope, not the pulses.
- Without routing settings, `main` drives all toys and the other channels none.

### 8.3 Patterns

```lua
local heartbeat = pattern {
  { 0.00, 0.8 }, { 0.10, 0.0 }, { 0.20, 0.6 }, { 0.30, 0.0 }, { 0.80, 0.0 },
}  -- list of { time in s, intensity }, linearly interpolated

play(heartbeat, { channel = "main", loops = 3, scale = 0.5 })  -- loops = 0: loop forever
```

### 8.4 In-game overlay

When the player uses the in-game overlay (a small panel drawn over the game), it shows
the mode's name, the toy output and anything the mode adds with:

```lua
hud(label, value [, max])   -- gauge shown as a bar with "value / max"; max defaults to 1
hud(label, nil)             -- removes the gauge
hud_event(text)             -- short message ("Parry!", "Combo x5") that fades out after ~2 s
```

- Gauges are kept until changed or removed, so calling `hud()` on every tick or only
  on changes both work. They are cleared when the mode (re)starts. At most 4 gauges;
  they are shown in the order they were first set.
- `hud_event` texts are cut to 40 characters; the overlay shows the last 3.
- Without the overlay, both functions do nothing visible: they are safe to call always.
- Use them for what the player wants to see while playing (the main gauge, detected
  parries or combos); keep `plot()` for tuning curves.

## 9. Utilities

| Function | Purpose |
|---|---|
| `plot(name, value)` | draws a curve in the editor's debug panel |
| `log(...)` | writes to the editor console (`print` is redirected here) |
| `after(seconds, fn)` | calls `fn` once after the delay, returns a handle with `:cancel()` |
| `every(seconds, fn)` | calls `fn` periodically, returns a handle with `:cancel()` |
| `clamp(x, a, b)`, `lerp(a, b, t)`, `map(x, a1, b1, a2, b2)` | common maths |
| `random([a, b])` | float in [0, 1[, [0, a[ or [a, b[; seed reset on every `on_start` |
| `persist` | table kept across hot reloads (not across application launches) |

Timers (`after`, `every`) are evaluated at the start of every tick, before events. Their
resolution is therefore 20 ms.

## 10. Execution and sandbox

- Engine: **Luau** via `mlua`, in sandbox mode.
- Available libraries: `math`, `string`, `table`, `bit32`, `utf8`.
- Unavailable libraries: `io`, `os`, `debug`, `coroutine`, `require`, `loadstring`,
  `getfenv` / `setfenv`, and any file or network access. Available libraries are
  read-only.
- Budget per callback call: 10 ms of real time, enforced by the Luau interrupt (200 ms for
  running the file at load time), and 25 ms for all the calls of one tick (timers,
  events, `tick`). Exceeding it counts as a runtime error.
- Mode memory: 16 MB maximum.
- What GameViber keeps for a mode is bounded too: 64 timers, 64 pulses and 16 patterns
  at once (more is a runtime error), 256 points per pattern, 32 `plot()` series (names
  cut at 40 characters), `hud()` labels cut at 24 characters, 4 `hud_event()` messages
  per tick (the others are dropped), 20 `log()` lines per second of 500 characters at
  most.
- Script global variables are reset on every (re)load, except `persist`.

## 11. Errors and hot reload

- The editor saves, and a watcher also detects changes made in an external editor. Every
  save triggers a reload.
- **Load error** (syntax, missing or invalid `mode {}`): the previous version of the mode
  keeps running, and the error is shown with its line number.
- **Successful reload**: `on_stop` is called on the old version, then `on_start` on the
  new one. Compatible parameter values and `persist` are kept.
- **Runtime error** in a callback: all outputs go to 0 immediately, the mode is suspended,
  and the error is shown with the traceback. The "Resume" button calls `on_start` again.

## 12. Safety (outside the script)

- **Per-toy response**, adjustable on the Toys page: a channel value below 0.01 leaves
  the toy off; above, it goes through the toy's curve (`value ^ curve`) and is spread
  between its weakest and strongest intensity. Defaults: 0, 1 and 1 (unchanged).
- **Global intensity cap**, adjustable in the GUI: default 1.0, applied last, after the
  per-toy response.
- **Panic button**: BACK + START held for 0.5 s. The combo is configurable on the
  Keybindings page (any 2 or more buttons of §6.2).
  - It stops all toys and suspends the mode until it is re-enabled from the GUI.
  - v1: the combo's presses are still forwarded to the callbacks.
- **Source loss**: if the gamepad is disconnected or the capture stops, all outputs go to
  0 until a gamepad is back. The mode keeps running: held buttons get a release event
  and the effects the game was playing are dropped, so the rumble reads 0. The standard
  (proxy) source looks for the gamepad again every 2 s; the kernel probe picks up
  gamepads by itself.
- **Strokers**: GameViber does not know where a stroker is. Each one has a range, a
  fastest and a slowest move and a shortest time between two changes of direction,
  adjustable on the Toys page; its strokes never go faster, nor turn more often, nor
  leave the range. A move from where the toy might be anywhere (the first one, after
  a stop) is timed for the farthest point, at a gentle speed. A channel value below
  0.01, the panic button and source loss stop it where it is.
- **Output rate**: 20 sends/s maximum per toy. A change smaller than 0.01 is not sent,
  except going to 0, which is always sent.

## 13. Planned evolutions (not in v1)

- **Chaining**: `input.upstream` would expose the output of the previous mode.
- **Per-game mode**: detecting the game process and mapping it to a mode.
- **Strokes written by the mode** (strokers already play the intensity, §8.2):
  `stroke(speed, range)`, funscripts played on an event.
- **Multiple gamepads**: `ev.pad` and `input.pads[i]`.
- **Block editor** generating Luau.
- **The game's image without the overlay**: through the desktop's screen capture, for
  games the overlay does not reach.

## 14. Examples

The reference versions, shipped with the application, are in `gameviber/modes/`.

### 14.1 Simple (GHR parity, default mode)

```lua
mode {
  api = 1,
  name = "Simple",
  description = "Forwards the game's rumble, like the Game Haptics Router.",
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
```

### 14.2 Accumulation

```lua
mode {
  api = 1,
  name = "Accumulation",
  description = "Vibrations and bonus presses add points; the rumble is scaled by points, "
             .. "which drain while idle.",
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
```

### 14.3 Per-genre modes

The other built-in modes target a game genre. Their heuristics (parry window, "hit"
threshold) need tuning per game: the rumble does not tell who took the hit.

| Mode | File | Genre | Mechanics |
|---|---|---|---|
| Combo | `combo.luau` | fighting | vibrations less than `window` apart form a combo; each hit is stronger, a long enough combo ends with a burst; button presses add a light tension |
| Overheat | `overheat.luau` | action / shooter | continuous rumble scaled down, rising edges above `peak` become pulses; firing heats a gauge, a full gauge triggers an overheat |
| Tension | `tension.luau` | turn-based with QTEs | a wave builds while rumble happened in the last `combat_timeout` s; a vibration within `window` after a parry/dodge press gets a reward pulse; long strong vibrations are amplified |
| Engine | `engine.luau` | racing | RT/LT drive an RPM value that sets the rate and strength of a pulsing vibration; rumble adds road texture and impacts |
| Heartbeat | `heartbeat.luau` | horror | heartbeat whose tempo and strength follow a stress gauge raised by vibrations; optional random jump scares |
| All or Nothing | `all_or_nothing.luau` | souls-like | gauge rising while the player is active, cut by a vibration above `hit`, with a punishment pulse |
| Ambient | `ambient.luau` | exploration / cosy | slow wave under the rumble, fading out after `fade_after` s of inactivity |
| Surge | `surge.luau` | action metroidvania | quiet outside fights (rumble seen in the last `combat_timeout` s); vibrations fill a gauge, a vibration within `window` after a parry press fills it more and gets a reward pulse; the gauge drives a pulsing glow during fights; parry held + a surge button spends `surge_cost` on a crescendo |
