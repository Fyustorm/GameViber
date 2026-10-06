# GameViber user guide

The [README](../README.md) covers installing and the first steps. This guide
goes through the rest.

## The app

The top bar shows **the game being played** (pick another there), the state of
the gamepad, of the rumble capture and of Intiface, the global maximum
intensity and **STOP ALL**.

| Page | What it is for |
|---|---|
| **Games** | Your games, added by name. Each game has its **Modes**, its **Signals** (what GameViber reads from it, see below) and its recorded **Sessions**. A game linked to its executable becomes the game being played by itself. |
| **Live** | What happens while you play, to keep on a second screen: the mode and its main settings, what goes to the toys, the scene, the sound, the gamepad. |
| **Toys** | The connection to Intiface Central, the toys it found, a test buzz, and how each toy plays: which mode channels, weakest and strongest intensity, response curve. |
| **Setup** | What does not depend on the game: the gamepad and how it is captured, the shortcuts (panic stop, mark a moment, capture the screen) on the gamepad or the keyboard, the in-game overlay, the sound listened to by default, other programs. |
| **Settings** | The language AI assistants answer in, the requests sent to them (editable). |
| **Creator** | Writing modes by hand: hot reload, graphs, a simulator, replay of recorded sessions, the log. |

A mode's own page shows its explanation, its main settings, all its settings
and named **presets**.

## A mode for your game

The best experience is a mode written for the game you play. In a game's
**Modes**, **New mode** guides you through getting one from any AI assistant:

1. Type the game's name, and pick **Quick** (a couple of minutes) or
   **Advanced** (the assistant may also use the game's signals, and ask you to
   draw zones).
2. Copy the request GameViber builds and paste it into a new conversation. An
   assistant with web search can check the game's default controls.
3. Paste the answer back (or just its code, or drop the `.luau` file on the
   window). GameViber checks that the mode loads and activates it. If it does
   not load, copy the fix request and send it back to the assistant.

### When it does not feel right

**Doesn't feel right?** on the mode's page asks the mode's own questions
("Parry detection: Often missed | Good | Also on hits taken"). Many answers
come with a **quick fix** that adjusts the matching setting in one click.

Otherwise, send a fix request to the assistant with a recorded session:

- While playing, hold **BACK + RS** to **mark a moment** that felt wrong. The
  last 2 minutes are saved with the marks.
- Or pick a session, or save the last 2 minutes (GameViber always keeps them in
  memory).

GameViber replays the session into the mode, so the assistant sees when the
game vibrated, what you pressed and what the mode did. Pasting the answer back
updates your mode (the previous version is kept as `.luau.bak`), or creates a
tuned copy of a built-in one.

### Sharing a mode

A mode written for a game reads that game's scenes and zones, so it is shared
with them. **Export** on a mode's page saves it in a `.gameviber` file with the
game's signals: its scenes, zones, captures (filed under a scene; captures to
sort stay home), values from other programs and executables. Captures are
images of your screen: look at them before sharing the file.

**Import a mode** in the library adds the mode to the game of the same name,
with the signals it lacks (what you set up is kept), or adds the game. Captures
are analysed again once the game is played. Modes run in a sandbox: they can
only drive your toys, never reach your files or the network.

## A game's signals

The rumble and the buttons do not say whether you are fighting, exploring or
watching a cutscene; the music and the screen usually do. A game's **Signals**
teach GameViber about the game, for all its modes, step by step:

- **Scenes**: name the phases that should not feel the same (battle,
  exploration, story, menu), and describe how each sounds if the music
  changes between them. GameViber recognizes them from the sound and from your
  captures, with models (about 350 MB) downloaded the first time a mode needs
  them, running on your computer. Scenes come a few seconds late: modes use
  them for the mood of a phase.
- **Captures**: while the game shows a battle, a dialogue..., hold **BACK + LS**
  on the gamepad (or press the capture key, Ctrl + Alt + C suggested). The image
  is captured without leaving the game. A few captures per scene make the
  recognition much more reliable. Needs the in-game overlay. Screenshots on
  your computer can be added too (**From files...** under the captures).
- **Zones**: on a capture, draw a rectangle around something shown only at
  times (the battle interface) or around a bar (health). Modes read whether it
  is shown, or how full the bar is, ten times a second. A bar of one color is
  read by its colors (pick its full and empty parts); a bar in a gradient, in
  segments or made of hearts by its look: draw the rectangle exactly on it
  (not its icon), take its look on a capture where it is full, then add its
  empty look from captures where it is low.
- **Sound**: which application to listen to (by default, the game showing the
  overlay, otherwise everything the computer plays).
- **Values from other programs**: a mod of the game, or a script reading its
  API, can send values to GameViber (`{"set": {"hp": 0.4}}`,
  `{"event": "kill"}`) on `ws://127.0.0.1:12350`. Declare what it sends so AI
  assistants know about it.

## In-game overlay

A small panel over the game: the mode and preset, the scene, how strong the
toys run, the mode's gauges and what it detects ("Parry!"), and warnings (toy
lost, Intiface disconnected, panic stop). It also lets GameViber see the
game's image (for scenes, captures and zones).

The package installs it; with the `.tar.gz`, install it from
**Setup › In-game overlay**. Then turn it on per game, in Steam's launch
options (Lutris, Heroic: the same as an environment variable or a command
prefix):

- Proton (Windows) and Vulkan games: `GAMEVIBER_OVERLAY=1 %command%`. You can
  also turn it on for every Vulkan game in Setup.
- Native OpenGL games: `gameviber-overlay %command%`.

`DISABLE_GAMEVIBER_OVERLAY=1 %command%` turns it off for one game. Games
started before installing it need a restart.

**Anti-cheats**: the overlay is loaded into the game's process, like MangoHud,
vkBasalt or the Steam overlay. An anti-cheat could still mistake it for a
cheat: do not enable it in online games with an anti-cheat (EasyAntiCheat,
BattlEye, VAC), and keep it per game. The rest of GameViber never touches the
game.

## Gamepad capture

**Setup › Gamepad** offers two methods:

- **Standard** (default): GameViber shows games a copy of your gamepad and
  passes everything through. Games see two gamepads unless you hide the real
  one (asks for your password).
- **Kernel probe**: games see your real gamepad, nothing changes for them;
  GameViber watches the rumble from the kernel (asks for your password).

## Built-in modes

Fallbacks per game genre, for when you have no mode made for your game:

| Mode | For | Idea |
|---|---|---|
| Simple | anything | forwards the rumble as it is |
| Accumulation | anything | points earned per vibration, draining while idle |
| Combo | fighting (Tekken, SF6) | chained hits grow stronger, final burst |
| Overheat | action / shooter (Ratchet & Clank, Doom) | compressed rumble, peaks stand out, overheat gauge |
| Tension | turn-based with QTEs (Clair Obscur) | wave building up during fights, parries rewarded |
| Engine | racing (Forza, GT) | trigger = engine RPM, rumble = road and impacts |
| Heartbeat | horror (RE, Silent Hill) | heart speeding up with every vibration |
| All or Nothing | souls-like (Elden Ring, Sekiro) | gauge rising while you survive, emptied by a big hit |
| Ambient | exploration, platformer, cosy | slow wave under the rumble, fading out when idle |
| Surge | action metroidvania (PoP: The Lost Crown, Hollow Knight) | hits and parries fill a gauge, spent on a crescendo |

The rumble does not say who took a hit, so settings such as a parry window
need tuning per game.

## Updates

GameViber looks for a new version when it starts, then every few hours (turn
this off in **Settings › Updates**, which also has **Check now**). When one is
out, a banner says so; **See the update** shows what's new. Then, depending on
how you installed GameViber:

- **`.deb`, `.rpm` or Arch package**: **Download and install** downloads it,
  checks it, and installs it with your package manager after asking for your
  password.
- **`.tar.gz` archive**: **Download and install** replaces the files where you
  extracted it (keep the `distribution` file there).
- **Installed from a store or a package repository**: GameViber only tells
  you; update it there.

Then **Restart GameViber now**: toys stop during the restart, and running
games keep the old overlay until you restart them. Your settings, games and
modes are kept. While GameViber is an alpha, alpha versions are offered too.

## Where things are saved

- Settings, games, your modes and recorded sessions: `~/.config/gameviber/`
  (your modes in `modes/`, reloaded when saved from any editor).
- Downloaded models: `~/.local/share/gameviber/models/`.

## Known limitations

- PlayStation and Switch gamepads driven directly by SDL may not report their
  rumble: set `SDL_JOYSTICK_HIDAPI=0` in the game's launch options.
- Kernel probe: a rumble set up before GameViber started is only seen at the
  game's next one.
- In automatic sound mode without the overlay, music or voice chat playing next
  to the game is heard too.
- The overlay does not work with the Flatpak version of Steam yet.
