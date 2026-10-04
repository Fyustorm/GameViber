The GameViber mode below does not feel right to the player{{GAME}}. Help fix it.

<!-- full -->
GameViber (Linux) turns what a game does into vibrations on adult toys connected through
Intiface Central: the rumble the game sends to the gamepad, the player's buttons and
sticks, and the game's sound. A **mode** is a small Luau script doing this
transformation; its API is specified at the end of this message.
<!-- /full -->

**Language**: answer in {{LANGUAGE}}, and write the texts players see in the mode
(name, description, help, parameter labels, feedback questions, overlay messages) in
{{LANGUAGE}} too. Keep identifiers and code comments in English.

# What the player says

{{PROBLEMS}}

# Earlier attempts

{{HISTORY}}

# The mode

Mode **{{NAME}}**, with the player's current settings:

{{PARAMS}}

<!-- full -->
```lua
{{SOURCE}}
```
<!-- /full -->

# What happened

{{SESSION}}

# What to do

1. **Find the cause** in a few sentences. When a session is given, compare when the game
   vibrated, when buttons were pressed and what the mode output, starting with the
   marked moments. Do not go back and forth on the same setting.
2. **Fix it.** If new settings are enough, list them (with their labels) and stop there.
   Otherwise send the **complete corrected `.luau` file** in a single code block, keeping
   the existing parameters' names and types where you can, and bump `version`. A scene
   that is often wrong is usually fixed by rewording it as sound only heard there, or by
   merging scenes the music cannot tell apart.
3. Give a short **tuning guide**: which parameters to adjust first if it still feels off.

<!-- full -->
{{RULES}}

# Mode API specification

{{SPEC}}
<!-- /full -->
