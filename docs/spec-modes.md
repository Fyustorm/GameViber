# GameViber — Mode specification (API v1)

Status: implemented (v1) · API version: `1`

## 1. Goal and scope

A **mode** is a Lua script that turns what happens in the game (rumble sent to the
gamepad, player inputs, time) into commands for the toys connected to Intiface.
Modes are created and edited on the fly from the built-in editor, without recompiling.

**Included in v1**

- A single active mode at a time.
- A single intercepted gamepad.
- Scalar Buttplug outputs: `Vibrate`, `Rotate`, `Oscillate`.
- Hot reload, parameters adjustable from the GUI, debug graphs, simulator.

**Not in v1** (see §13): mode chaining, automatic per-game mode, linear outputs
(strokers), multiple gamepads, block editor.

## 2. Architecture

```
Gamepad proxy ──► event queue ──► Lua runtime (active mode) ──► Safety layer ──► Buttplug output
 (uinput, FF,     (timestamped)     fixed 50 Hz tick               (not scriptable)   (rate limited)
  buttons, axes)
```

- The **proxy** produces raw events: FF effects (upload, play, stop), buttons, axes.
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

## 5. Callbacks

All callbacks are optional, except `tick`.

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
```

The d-pad is converted to buttons even when the driver exposes it as axes (hat). Triggers
stay axes (§7), but `LT` and `RT` also generate a button event when they cross 0.5 up or
down.

Axis movements do not generate callbacks, to avoid a flood of events. Their current state
is read from `input.axes` on every tick.

## 7. The `input` table (current state, read-only)

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
```

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
  mapped to it. For the script, it is always an intensity between 0 and 1.
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
  running the file at load time). Exceeding it counts as a runtime error.
- Mode memory: 16 MB maximum.
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

- **Global intensity cap**, adjustable in the GUI: default 1.0, applied after the mode.
- **Panic button**: BACK + START held for 0.5 s. The combo is configurable on the
  Connection page (any 2 or more buttons of §6.2).
  - It stops all toys and suspends the mode until it is re-enabled from the GUI.
  - v1: the combo's presses are still forwarded to the callbacks.
- **Source loss**: if the gamepad is disconnected or the capture stops, all outputs go to
  0 until a gamepad is back. The mode keeps running: held buttons get a release event
  and the effects the game was playing are dropped, so the rumble reads 0. The standard
  (proxy) source looks for the gamepad again every 2 s; the kernel probe picks up
  gamepads by itself.
- **Output rate**: 20 sends/s maximum per toy. A change smaller than 0.01 is not sent,
  except going to 0, which is always sent.

## 13. Planned evolutions (not in v1)

- **Chaining**: `input.upstream` would expose the output of the previous mode.
- **Per-game mode**: detecting the game process and mapping it to a mode.
- **Linear outputs** (`LinearCmd`, for strokers): `stroke(speed, range)`.
- **Multiple gamepads**: `ev.pad` and `input.pads[i]`.
- **Block editor** generating Luau.

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
