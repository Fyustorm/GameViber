Help the player set up GameViber for the game **{{GAME}}**, before a mode is written for
it. Do not write the mode yet: the player will ask for it in this conversation once
GameViber is set up.

GameViber (Linux) turns what a game does into vibrations on adult toys connected through
Intiface Central: the rumble the game sends to the gamepad, the player's buttons and
sticks, the game's sound and its image. A **mode** is a small Luau script doing this
transformation; its API is specified at the end of this message. Besides the rumble and
the buttons, a mode reads what the player sets up for it in GameViber (§6.3, §6.5):

- **phases**: parts of the game that should not feel the same (battle, exploration,
  story). GameViber recognizes them from the sound (a description of what is only heard
  there) and from captures of the screen the player takes in each one, a few seconds
  late;
- **indicators**: parts of the game's interface the player draws on those captures,
  read on every frame: a **visibility** (an element shown or not: a battle menu, a
  dialogue box) or a **gauge** (how full a bar is: health, stamina). Indicators can be a
  **sure sign** of a phase, exact at once: the phase is current while all the
  indicators of its sign are shown. An indicator can be in the signs of several phases
  (battle: the health gauge and the battle menu; exploration: the gauge alone): the
  sign of the most indicators shown wins. One phase can be the phase of **none of the
  others** (story: neither the menu nor the gauge on screen).

**Language**: answer in {{LANGUAGE}}, and write the descriptions the player reads in
{{LANGUAGE}} too. Keep the names (phases, indicators) in English.

# What to do

1. **Research the game.** If you can browse the web, look up {{GAME}}'s main mechanics,
   its default controller bindings (Xbox layout), when it rumbles, how its music changes
   and which parts of its interface show only at certain moments. Say what you are
   unsure about.
2. **Propose 2 to 4 phases**, the most contrasted ones: two or three that feel very
   different beat many close ones. For each: what only its sound has (for the sound
   model; leave it out when the music does not change), and its sure sign when the
   interface gives one.
3. **Propose the indicators** worth drawing: a sure sign of a phase, or a gauge a mode
   can follow (health, a combo meter). For each: what it is, where it is on the screen
   and in which phase to capture it. Prefer fixed parts of the interface (a frame, an
   icon) over text that changes.
4. **Say which moments to capture**: a few captures per phase, of different places and
   moments, make the image recognition much more reliable.
5. Explain in a few sentences what a mode could do with all this (2 to 4 mechanics that
   map well to vibrations), so the player sees what the setup is for.
6. **End with the setup** in a single `json` code block, which GameViber reads to create
   the phases and list the indicators to draw:

```json
{
  "phases": [
    { "name": "battle", "sound": "fast battle music with heavy drums", "indicators": ["hp", "battle_menu"] },
    { "name": "exploration", "sound": "calm ambient music", "indicators": ["hp"] },
    { "name": "story", "otherwise": true }
  ],
  "indicators": [
    { "name": "hp", "kind": "gauge", "where": "top left, the green health bar; capture it while exploring" },
    { "name": "battle_menu", "kind": "visibility", "where": "bottom right, the frame of the command menu; capture it in a battle" }
  ]
}
```

Names are letters, digits and underscores, starting with a letter. `sound`,
`indicators` and `otherwise` are optional; one phase at most is `otherwise`, and it has
no indicators.

# What GameViber knows about this game

{{INPUTS}}

{{RULES}}

# Mode API specification

{{SPEC}}
