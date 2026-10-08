<!-- after-analysis -->
The player set up in GameViber what you proposed (or what they kept of it), as listed
below. Now write the mode for **{{GAME}}**, following the rules and the API given earlier
in this conversation.
<!-- /after-analysis -->
<!-- full -->
Write a GameViber mode made specifically for the game **{{GAME}}**.

GameViber (Linux) turns what a game does into vibrations on adult toys connected through
Intiface Central: the rumble the game sends to the gamepad, the player's buttons and
sticks, the game's sound and its image. A **mode** is a small Luau script doing this
transformation; its API is specified at the end of this message.

**Language**: answer in {{LANGUAGE}}, and write the texts players see in the mode
(name, description, help, parameter labels, feedback questions, overlay messages) in
{{LANGUAGE}} too. Keep identifiers and code comments in English.
<!-- /full -->

# What to do

<!-- direct -->
1. **Research the game.** If you can browse the web, look up {{GAME}}'s default
   controller bindings (Xbox layout) and main mechanics; otherwise say which bindings
   you are unsure about. Find out when it rumbles, and which phases (battles,
   exploration, dialogue) its music or its screen tells apart.
2. **Design the mode** around 2 to 4 mechanics that map well to vibrations, and explain
   it in a few sentences.
3. **Write the mode** (below).
<!-- /direct -->
<!-- conversation -->
1. **Research the game.** If you can browse the web, look up {{GAME}}'s default
   controller bindings (Xbox layout) and main mechanics; otherwise say which bindings
   you are unsure about. Find out when it rumbles, and which phases (battles,
   exploration, dialogue) its music or its screen tells apart.
2. **Ask the player before writing anything**, in one short message:
   - 3 to 5 numbered questions about what they want, each with lettered choices they can
     answer in a few characters ("1b 2a 3c"): which moments should vibrate (attacks
     landed, hits taken, special moves, running, quiet moments), continuous or in bursts,
     how strong, how the phases should differ;
   - 2 or 3 **contrasted designs** for this game, each a name and 2 sentences (a tension
     rising through battles, rewards for the hits landed, punishment for the hits taken),
     with the mechanics each one uses.

   Then stop and wait for the answer.
3. Once they answered, **write the mode** (below) following their choices, and say in a
   few sentences how it does.
<!-- /conversation -->
<!-- after-analysis -->
1. **Design the mode** around 2 to 4 mechanics, using what was set up: the phases for
   the mood, the indicators where they say exactly what the phases only guess. Explain
   it in a few sentences.
2. **Write the mode** (below).
<!-- /after-analysis -->

The mode is the complete `.luau` file in a single code block: a header comment naming the
game, then `mode { }` with `api = 1`, a short `name`, a one-line `description`,
`category = "{{GAME}}"`, a plain-language `help` (2-3 sentences),
`author = "AI for GameViber"`, `version = "1.0"` and 1 to 3 `main_params`. After it,
give a short **tuning guide**: which parameters to adjust first if something feels off.

# What GameViber knows about this game

{{INPUTS}}

<!-- full -->
{{RULES}}

# Mode API specification

{{SPEC}}
<!-- /full -->
