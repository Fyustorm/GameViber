Write a GameViber mode made specifically for the game **{{GAME}}**.

GameViber (Linux) turns what a game does into vibrations on adult toys connected through
Intiface Central: the rumble the game sends to the gamepad, the player's buttons and
sticks, and the game's sound. A **mode** is a small Luau script doing this
transformation; its API is specified at the end of this message.

**Language**: answer in {{LANGUAGE}}, and write the texts players see in the mode
(name, description, help, parameter labels, feedback questions, overlay messages) in
{{LANGUAGE}} too. Keep identifiers and code comments in English.

# What to do

1. **Research the game.** If you can browse the web, look up {{GAME}}'s default
   controller bindings (Xbox layout) and main mechanics; otherwise say which bindings
   you are unsure about. Find out when it rumbles, and which phases (battles,
   exploration, dialogue) its music tells apart.
2. **Design the mode** around 2 to 4 mechanics that map well to vibrations, and explain
   it in a few sentences.
3. **Write the complete `.luau` file** in a single code block: a header comment naming
   the game, then `mode { }` with `api = 1`, a short `name`, a one-line `description`,
   `category = "{{GAME}}"`, a plain-language `help` (2-3 sentences),
   `author = "AI for GameViber"`, `version = "1.0"` and 1 to 3 `main_params`.
4. Give a short **tuning guide**: which parameters to adjust first if something feels
   off.

{{RULES}}

# Mode API specification

{{SPEC}}
