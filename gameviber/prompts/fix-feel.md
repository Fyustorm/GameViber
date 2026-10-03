The GameViber mode below does not feel right to the player{{GAME}}. Help fix it.

# Context

GameViber is a Linux application that intercepts the rumble (force feedback) a game sends
to a gamepad, together with the player's button presses and stick/trigger positions, and
turns them into vibrations on adult toys connected through Intiface Central (Buttplug
protocol). The transformation is done by a **mode**: a small Luau script. The full mode
API specification is at the end of this message: it is the only API you may use.

# What the player says

{{PROBLEMS}}

# The mode

Mode **{{NAME}}**, with the player's current settings:

{{PARAMS}}

```lua
{{SOURCE}}
```

# What happened

{{SESSION}}

# What to do

1. **Find the cause.** Explain in a few sentences what in the mode, or in its settings,
   makes the player feel what they describe. When a session is given above, use it:
   compare when the game vibrated, when buttons were pressed, and what the mode output.
   Remember that **the rumble does not say who took a hit**: the mode can only guess.
2. **Fix it.**
   - If changing settings is enough, list the new values (with their labels) and stop
     there: the player sets them on the Play page.
   - Otherwise send the **complete corrected `.luau` file** in a single code block. Keep
     the names and types of the existing parameters where you can, so the player keeps
     their settings, and bump `version`.
3. After the fix, give a short **tuning guide**: which parameters to adjust first if it
   still feels off.

# Rules for the script

- Use only the API v1 described in the specification below (§13 lists features that do
  **not** exist). Luau sandbox: no `io`, `os`, `require`, files or network.
- **Every gamepad button the mode reacts to must be a `button_param`**, whose label says
  what the button does in the game. Never compare against a hard-coded button name.
- Every timing or threshold heuristic is a parameter with its unit in the label, e.g.
  `"Parry window (s)"`, `"Hit = vibration above"`.
- Output to each toy is limited to 20 updates per second, so waveforms faster than about
  5 Hz blur. Keep patterns and pulses at least 0.1 s long.
- Safety is not the script's job: the global cap, the panic stop and zeroing on gamepad
  loss are handled by GameViber. Do not reimplement them.
- Quiet moments (menus, pauses, cutscenes without rumble) should fade to 0: use
  `input.idle`, `input.input_idle` or `input.rumble_idle`. Note that `rumble_idle`
  counts from mode activation when no vibration has happened yet.
- Keep the game's own rumble perceptible (usually `math.max(effect, rumble * weight)`).
- Keep `plot()` for the internal state worth tuning, and `hud()` / `hud_event()` for what
  the in-game overlay should show.
- Clamp every gauge to its range; `set()` and `pulse()` already clamp to 0..1.
- English only, readable code, short comments where the intent is not obvious.

# Mode API specification

{{SPEC}}
