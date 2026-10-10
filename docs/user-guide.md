# GameViber user guide

The [README](../README.md) covers installing and the first steps. This guide
goes through the rest.

## The app

The top bar shows **the game being played** (pick another there), the state of
the gamepad, of the rumble capture and of Intiface, the global maximum
intensity and **STOP ALL**.

| Page | What it is for |
|---|---|
| **Community** | Where GameViber opens: type the game you play (it searches as you type), and install a mode other players made for it in a click (the best rated first), open one shared with you by its code, report one that a game update broke. No mode for your game yet? **Create a mode for it**: an AI assistant writes it, then share it from its page. Creating a mode for a game shows its community modes too, and the library tells when the game you play has some and you have no mode of your own. |
| **Library** | Your modes, by game, and the built-in ones (**Any game**). **Create a mode** asks for its game, then opens the Creator. Opening a mode plays it and shows its page: its settings, what it reads, sharing it, and its **game** (how GameViber recognizes it, which sound it listens to). A game linked to its executable becomes the game being played by itself, and so does a game Steam starts once GameViber saw its Steam app id. |
| **Live** | What happens while you play, to keep on a second screen: the mode and its main settings, what goes to the toys, the phase, the sound, the gamepad. |
| **Toys** | The connection to Intiface Central (disconnect, reconnect, start or stop its scan for toys, its address), the toys it found, a test buzz, and how each toy plays: which mode channels, weakest and strongest intensity, response curve, and for strokers what they can do (see Strokers). |
| **Setup** | What does not depend on the game: the gamepad and how it is captured, the shortcuts (panic stop, mark a moment, capture the screen) on the gamepad or the keyboard, the in-game overlay, the sound listened to by default, other programs. |
| **Settings** | The language AI assistants answer in, the requests sent to them (editable). |
| **Creator** | The workspace of the mode being played, in tabs to visit in any order: the **✨ AI assistant** (what you ask of it: the game's phases and indicators proposed, the script written), **Phases**, **Sessions** (the easiest way to get captures: record yourself playing through every phase and situation, then work from its replay; recorded sessions to watch again like a video, replayed into the mode as it is now (its script, settings, phases, captures and indicators: change one, in any tab, and the player offers to apply it: it pauses while the session is replayed again, then plays on; indicators drawn or moved since the recording are read from its images) — the game's images, its rumble, your buttons, the phases, what the mode's inputs said and what it sent to the toys, at any moment — while the toys play it; images to add to the captures, picked with the keyboard too (**⌨ Shortcuts** under the timeline lists the keys); one indicator to follow in the timeline, chosen under it (a gauge as a line, a visibility as the background where it is shown)), **Captures & indicators**, **Other programs** (see A mode's inputs), its **Script** (started from a built-in mode, or written by hand, with hot reload) and **Logs**. A recorded session keeps 1 to 10 images of the game a second (2 by default, about 250 MB an hour, written to disk as they come; a recording stops after an hour); the last 2 minutes, always kept in memory, keep 2 a second. The mode runs while you edit it: every change can be felt at once, and **Live ›** shows what it does; **? How a mode works** shows how the parts fit. |

A mode's own page shows its explanation, its main
settings, all its settings and named **presets**, and its **variants**: other scripts of the same mode
(a "boss only" version, a calmer one), reading the same inputs, each with its
own settings and presets. **+ Variant** copies the script played now, to change
in Creator; a variant is shared with its mode.

### Strokers

Strokers (The Handy, Kiiroo Keon, OSR2...) play every mode: GameViber turns the
intensity a mode asks for into strokes, faster and longer as it rises (**Stronger**
on the toy's card: speed, length or both). A stroker does not tell where it is,
so GameViber never asks one for more than it can do, and moves it slowly when it
cannot know where it is (first move, after a stop). **🎯 Calibrate** on its card
finds what it can do with you: the toy goes where you put its range, then strokes
faster, turns more often and strokes slower, step by step, until you press the
button when it stops following; GameViber keeps a margin. Or set them on its card:

- **Lowest** / **Highest**: the part of its length it goes over.
- **Fastest**: the time of its fastest move over its whole length. Raise it if the
  toy stops following (shorter strokes, stutters).
- **Slowest**: lower it if slow strokes jerk.
- **Turns**: the shortest time between two changes of direction.

When the mode sends several feelings, **Length** on its card can give its stroke
length to one of them: the feelings it plays then set only their speed. Where it
is sent shows on its card, on Live and in the session player,
to check a mode without watching the toy. A mode can also say what the strokes
should be, add single strokes for its big moments (`stroke()` and `thrust()`), and
play funscripts on an event: add `.funscript` files to it in Creator › **Funscripts**
(▶ Try plays one on every toy; rename them there), or write one in its editor (**✏ New**,
**✏ Edit**): a click puts the cursor (or adds a point, with **Click adds points**), drag a
box to pick points (Shift adds to them) and drag them to move them together, right-click
removes one, insert strokes at the cursor, select a part on the timeline to try it, delete or cut it out, or make a
new funscript of it. As in usual funscript editors, the keyboard does most of it: 0 to 9
set a point at the cursor (0%, 100%), ← → move the cursor a step (Shift: ten), ↑ ↓ go
to the next or previous point, Delete removes the points picked, Ctrl+A picks them all,
Space tries. With **Grid** on, times snap
to its step (the cursor, points added or moved, the selection) and ← → move one step.
Each segment is colored for how fast it moves against what your strokers can do (the
strictest of those connected, as set or calibrated on the Toys page): gray still, blue
too slow (the toy may jerk), green to orange fine, red too fast (shortened when played);
a ring marks a turn too soon after the previous one (skipped). The pointer shows a
segment's speed in units per second. Its script plays them with
`play(funscript("name"))` (see the mode specification §8.5). They travel with the
mode when it is shared. **↕ Stroke** and the **Feel** buttons try it. Panic, a lost gamepad and silence
stop it where it is. The global cap slows and shortens its strokes.

## A mode for your game

The best experience is a mode written for the game you play. In the
**Library**, **Create a mode**:

1. Type the game's name (the game running is filled in) and choose how to
   start: **An AI assistant writes it** (recommended), a built-in mode, or
   writing it yourself. **How a mode works ?** shows what a mode reads, the
   phases, the script and your toys.
2. For an AI assistant, tick what you want:
   - your **toys** (vibrators, strokers, both);
   - **what you should feel**: moments (damage you take, hits you land,
     button presses, big moments), backgrounds (a wave in fights, a calm wave
     exploring, a heartbeat on low health, silence in menus) and moments
     **depending on the game** (parries, charged attacks, combos, recoil,
     speed), which the assistant leaves out where the game has none. The
     common ones are ticked at first; 🖥 marks those read from the screen;
   - **how the assistant works**: **Propose the game's phases and screen
     indicators first** (ticked at first, and needed by what reads the screen):
     pasting its answer sets the phases up (with their sure signs) and lists
     the indicators under **To draw** in **Captures & indicators**, with where
     to find them; once they are drawn, send the short script request in the
     same conversation. **Ask me questions and propose designs** first: the
     assistant asks what you want, with choices, and proposes 2 or 3 designs
     before writing the script.

   **Anything else?** takes your own instructions. All of it is kept with the
   mode: **Change** in the Creator's **✨ AI assistant** tab ticks it again.
3. **Open the Creator**: its **✨ AI assistant** tab shows the request to copy
   and paste into a new conversation, and the steps: its answer pasted
   under its request sets the phases up; then **Record a session**: play
   until you have been through every phase and situation (full and low
   health, dying, menus, story, bosses, exploring), and stop; then pick
   images of each in its replay and draw the indicators on them (the replay
   also shows whether the phases are recognized at the right moments); then
   the script request and its answer. The bar on top shows where you are; a step
   done folds up, and **↺ Ask again** opens it again. An assistant with web search can
   check the game's default controls. Indicators or values from other
   programs make it an advanced request, which uses them. You can also write
   more at the end of the request once pasted.
4. Paste the answer back (or just its code, or drop the `.luau` file on the
   window) and **Use this script**. GameViber checks that it loads first; if it
   does not, copy the fix request and send it back to the assistant.

There is no fixed order: the mode runs while you make it, so play, feel,
change the phases or the script, and play again. In the **Script** tab,
**Start from a built-in mode** instead copies one made for a genre, and
**Write it yourself** opens the script.

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

A mode written for a game reads the phases and indicators set up for it, so it
is shared with them. **Export**, on a mode's page, saves it in a `.gameviber` file
with its inputs (phases, indicators, captures filed under a phase, captures to
sort stay home; external inputs) and the game's name and executables. Captures
are images of your screen: look at them before sharing the file.

**Import a mode** in the library (or **Import a file** in Community) adds the mode, with its inputs, to the game of
the same name, or adds the game. Captures are analysed again once the mode is
played. Modes run in a sandbox: they can
only drive your toys, never reach your files or the network.

### Publishing a mode

On a mode's page, **In the community**: pick a name players see
and a password (there is no email yet: a lost password cannot be recovered),
then publish it for the people you give its code to (testers, first) or for
everyone. Look at the captures it sends, and leave out those showing your name,
a chat or a notification. Modes are published under the MIT license. Once
published, the same place gives its tester code (and a new one, the old one then
stops working), lists it for everyone or makes it private again, publishes its
next version with a word on what changed, or withdraws it.

The community's modes are also on the website, <https://gameviber.fyustorm.ovh>:
**Open in GameViber** on a mode's page opens it in GameViber, ready to install
(your browser asks first). A share code reads as a link too:
`https://gameviber.fyustorm.ovh/m/<code>`. Only one GameViber runs at a time:
starting it again brings the running one to the front.

A mode installed from the community offers its updates (on its page), unless you keep its version. If you changed it, the
update installs beside your version, which stays as it is. After a few hours
of play with a mode of your own that you did not change, GameViber suggests
publishing it (never while you play; **Later**, or **Don't ask for this mode**).

### Stats and votes

At first launch, GameViber asks whether to share your stats with the
community: how long you play the modes you installed from it (minutes and
sessions, sent every ten minutes or so) and your votes, under an id made up for
this installation. Never your name, your games' image or sound, your toys or
how they ran, and nothing linked to your author account. A game's modes
come the best rated first (a few votes counting little, then the most played),
each with its share of likes, its players and its median play time. Once you played a mode you installed, say whether you liked it on its
page. Change your mind in **Settings › Community**.

## A mode's inputs

The rumble and the buttons do not say whether you are fighting, exploring or
watching a cutscene; the music and the screen usually do. GameViber reads the
rumble, the gamepad, the sound and the image by itself; the Creator's tabs
teach it more about the game, for the mode being played: each mode of yours
keeps its own (a new mode for a game starts with those of the game's active
mode), and shares them with the mode. Built-in modes read none: duplicate one
to set some up.

- **Phases**: name the parts of the game that should not feel the same
  (battle, exploration, story, menu), and describe how each sounds if the music
  changes between them. GameViber recognizes them from the sound and from your
  captures, with models (about 350 MB) downloaded the first time a mode needs
  them, running on your computer. Phases come a few seconds late: modes use
  them for the mood of a phase. A phase can have a **sure sign**: indicators
  shown only in it (its menu), all shown together, exact at once. An indicator
  can be in the signs of several phases (battle: the health gauge and the
  battle menu; exploration: the gauge alone; the sign of the most indicators
  wins), and one phase can be **none of the others** (story: no menu nor gauge
  on screen). A phase can **ignore** the sound's hits or the
  image's flashes, for a menu whose clicks and music the mode would take for
  hits: exact when the phase has a sure sign (an indicator), else a few seconds
  late.
- **Captures**: the easiest is **From a session**: record a session playing
  through every phase, then pick images in its replay (**P** on the image
  shown, **A** to add them under a phase). Or, while the game shows a battle, a dialogue..., hold **BACK + LS**
  on the gamepad (or press the capture key, Ctrl + Alt + C suggested). The image
  is captured without leaving the game. A few captures per phase make the
  recognition much more reliable. Needs the in-game overlay. Screenshots on
  your computer can be added too (**From files...** under the captures).
- **Indicators**: on a capture, draw a zone (a rectangle) around something
  shown only at times (the battle interface: a visibility indicator) or around
  a bar (health: a gauge). Modes read whether it is shown, or how full the bar
  is, ten times a second; an indicator shown in several places has a zone for
  each. Whether a visibility indicator shows on a capture is guessed from the
  phases (shown in the phase of the capture it was drawn on); when it shows in
  several phases, or a phase mixes screens (cutscenes and menus), mark the
  captures **👁 Shown** or **⊘ Not shown** (under the zone's threshold, or
  right-click a capture): the threshold is then suggested from them. An
  element whose inside changes (a minimap and its map) is told by what stays:
  with three varied captures where it is shown or more (and some where it is
  not), **Compare only what stays** learns which parts of the zone (its frame)
  tell it, highlighted on the image. A bar of one color is
  read by its colors (pick its full and empty parts); a bar filled again over
  itself in another color once full (green, then yellow over the green) by
  its colors with a **tier** for each color (**+ Tier**), each an equal share
  of its value. When a menu over the bar looks like it empty (a dark empty
  color), set **Read only when** a visibility indicator is shown (an icon next
  to the bar) or hidden (the menu's button): the gauge is unknown otherwise.
  A bar in a gradient, in
  segments or made of hearts is read by its look: draw the rectangle exactly on it
  (not its icon), take its look on a capture where it is full, then add its
  empty look from captures where it is low.
  A curved bar (an arc, a ring) is drawn as a **〰 Path** rather than a
  rectangle: click points along its middle, from the end it fills from to its
  full end, and set its thickness; only the pixels along it are read, by its
  colors or its look.
- **Sound**: which application to listen to (by default, the game showing the
  overlay, otherwise everything the computer plays), shared by the game's
  modes.
- **External inputs**: a mod of the game, or a script reading its
  API, can send values to GameViber (`{"set": {"hp": 0.4}}`,
  `{"event": "kill"}`) on `ws://127.0.0.1:12350`. Declare what it sends so AI
  assistants know about it.

## In-game overlay

A small panel over the game: the mode and preset, the phase, how strong the
toys run, the mode's gauges and what it detects ("Parry!"), and warnings (toy
lost, Intiface disconnected, panic stop). It also lets GameViber see the
game's image (for phases, captures and indicators).

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

**Setup › Gamepad** offers two methods on Linux (Windows: see below):

- **Standard** (default): GameViber shows games a copy of your gamepad and
  passes everything through. Games see two gamepads unless you hide the real
  one (asks for your password).
- **Kernel probe**: games see your real gamepad, nothing changes for them;
  GameViber watches the rumble from the kernel (asks for your password).

A gamepad that cannot vibrate on Linux (many in their DInput mode) gets no
rumble from games: use the Standard method, which shows games one that can.
When such a gamepad does not tell which button is which, the Gamepad page asks
you to **set up its buttons** once, one press at a time: a drawn gamepad shows
the button to press (Xbox or PlayStation names), lights up what you press,
and sets again any button you click on it. Games then get it as an Xbox 360
controller (gamepads SDL's community database knows need nothing).
Your setups are in `~/.config/gameviber/gamecontrollerdb.txt`, in SDL's format:
a line for your gamepad from SDL_GameControllerDB works too.

On **Windows**, the Standard method is the only one: games get a virtual Xbox
360 controller (it needs [ViGEmBus](https://github.com/nefarius/ViGEmBus/releases),
which GameViber's installer offers). An Xbox controller is passed on as it is,
and still vibrates; any other gamepad gets the Xbox layout from SDL's database
or from the buttons you set up, and does not vibrate itself. To keep games
from seeing two controllers, tick **Hide the real gamepad from games**: it
needs [HidHide](https://github.com/nefarius/HidHide/releases) (also offered by
the installer), and Windows asks you to allow it each time GameViber hides or
gives back the gamepad.

## On Windows

Windows is in preview and gets fewer features than Linux:

- No in-game panel (overlay). Modes still see the game's image: GameViber
  reads the window of the game in front when it is fullscreen or borderless,
  or any window of a game from Steam, Epic, GOG or Xbox. A game in exclusive
  fullscreen may give a black image: set it to borderless. **Setup › In-game
  overlay** shows the game being read.
- No kernel probe: the rumble comes from the virtual controller only.
- The game's sound is its own (Windows 10 2004 or later), whatever plays next
  to it; or everything the computer plays.
- Keyboard shortcuts are Ctrl+Alt+X (stop every toy), Ctrl+Alt+M (mark a
  moment) and Ctrl+Alt+C (capture the screen); a key another program holds is
  left without one.
- Other programs send their values to `\\.\pipe\gameviber-<your user name>-external`.

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
- **Windows installer**: **Download and install** downloads it; the new
  version installs when GameViber restarts (Windows asks you to allow it),
  then GameViber starts again.
- **Windows `.zip` archive**: like the `.tar.gz`, its files are replaced in place.
- **Installed from a store or a package repository**: GameViber only tells
  you; update it there.

Then **Restart GameViber now**: toys stop during the restart, and running
games keep the old overlay until you restart them. Your settings, games and
modes are kept. While GameViber is an alpha, alpha versions are offered too.

## Where things are saved

- Settings, games, your modes and recorded sessions: `~/.config/gameviber/`
  (your modes in `modes/`, reloaded when saved from any editor); on Windows
  `%APPDATA%\GameViber\`.
- Downloaded models: `~/.local/share/gameviber/models/`; on Windows
  `%LOCALAPPDATA%\GameViber\models\`.

## Known limitations

- PlayStation and Switch gamepads driven directly by SDL may not report their
  rumble: set `SDL_JOYSTICK_HIDAPI=0` in the game's launch options.
- Kernel probe: a rumble set up before GameViber started is only seen at the
  game's next one.
- In automatic sound mode without the overlay, music or voice chat playing next
  to the game is heard too.
- The overlay does not work with the Flatpak version of Steam yet.
