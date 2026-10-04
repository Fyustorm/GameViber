# GameViber

Linux equivalent of the Intiface Game Haptics Router: intercepts the rumble
games send to the gamepad, listens to the game's sound, transforms both through
a **Lua-scriptable mode** and drives the toys connected to Intiface Central.

## Usage

Start Intiface Central ("Start Server"), then, **before the game**:

```sh
./target/release/gameviber                      # GUI
./target/release/gameviber --headless -v        # no GUI, logs only
```

Do not run GameViber with `sudo`: the eBPF source and gamepad hiding go
through a **privileged helper** started on demand via `pkexec` (root access,
granted with your password once per session).

At first launch, a setup guide walks through Intiface Central, the toys, the
gamepad (with the choice of capture method and its live buttons) and a first
mode, preferably one made for your game by an AI assistant. It can be run
again from **Setup** at the bottom of the left bar. Then the GUI provides:

- a status bar with **the game being played** (pick another there), the
  gamepad / rumble capture / Intiface state, the global maximum intensity and
  **STOP ALL** (also: BACK + START held for 0.5 s on the gamepad, configurable
  in Setup);
- **Games**: the library of your games, created by their name without
  launching them, then each game by breadcrumb:
  - **Modes**: the game's modes as a compact list (made by an AI assistant,
    built-in or your own), each with **a page of its own**: its explanation,
    its main settings, all its settings and named **presets**;
  - **Signals**: what GameViber reads from the game for all its modes, as
    guided steps: its **scenes** (named once, optionally with how they sound),
    **captures** of each scene, **zones** of its screen, which sound to listen
    to, and values other programs send (see below);
  - **Sessions**: the sessions recorded while playing it;

  a game linked to its executable becomes the game being played by itself;
  built-in modes can also be played without a game;
- **Toys**: the connection to Intiface Central (status, address), the toys it
  found, a test buzz, which mode channels each one plays, and how it renders
  them: weakest and strongest intensity and a response curve, with buttons to
  feel each;
- **Setup**, what does not depend on the game: the **gamepad** (capture status,
  live buttons and sticks, the capture method, standard proxy or kernel probe,
  hiding, troubleshooting), the **gamepad combos** (panic stop, mark a moment,
  capture the screen), the **in-game overlay**, the **sound** listened to by
  default (automatic: the game showing the overlay, else everything; everything;
  one application, all its streams; off) with what is heard right now and the
  sound scene model, and **other programs** (the local port);
- **Settings** (bottom of the left bar): the language AI assistants answer and
  write modes in, the templates of the requests sent to them (editable, with
  GameViber's version one click away), and the setup guide;
- **Creator**: mode editing with hot reload (Ctrl+S), graphs of the rumble,
  outputs and `plot()` values, a simulator (fake rumble and buttons), recorded
  play **sessions** (the game's rumble and your inputs, replayed into a freshly
  restarted mode, with or without the toys) and the log. Recordings are saved
  in `~/.config/gameviber/recordings/`.

### A mode for your game

The best experience is a mode written for the game you play. In a game's
**Modes**, **New mode** (Quick or Advanced) guides you through getting one from
any AI assistant (ChatGPT, Claude, Gemini, Le Chat...); it is added to the game:

1. type the game's name;
2. copy the request GameViber builds and paste it in a new conversation. It holds
   the context, the mode API, what makes a mode feel good (a continuous, varying
   vibration rather than isolated buzzes) and the rules (every button the mode uses
   is a setting defaulting to the game's own binding, heuristics stay tunable); an
   assistant with web search can check the game's default controls;
3. paste the answer (or just its code, or drop the `.luau` file on the window):
   GameViber checks that the mode loads, creates it and activates it. If it does
   not load, copy the fix request and send it back to the assistant.

The request template is [`gameviber/prompts/new-mode.md`](gameviber/prompts/new-mode.md),
with [`rules.md`](gameviber/prompts/rules.md); both can be edited from Settings.

When a mode does not feel right, **Doesn't feel right?** (on its page) asks the
mode's own questions ("Parry detection: Often missed | Good | Also on hits
taken", declared with `ask()`, or generic ones). Many answers come with a
**quick fix** that adjusts the matching setting in one click. Otherwise pick a
recorded session (or save the last 2 minutes of play, which GameViber always
keeps in memory) and copy the request: the page lists what goes into it, and
it can be a full one for a new conversation or a short one for the
conversation that wrote the mode.

While playing, hold **BACK + RS** (configurable in Setup) to
**mark a moment** that felt wrong: the overlay confirms, and the last 2
minutes are saved 15 s later with the marks, which the request points out. GameViber replays the session into the mode with your settings, so the
assistant sees when the game vibrated, what you pressed and what the mode
output, along with the earlier rounds (answers, quick fixes, applied fixes) so
it does not go back and forth. Pasting the answer back updates a user mode in
place (the previous version is kept as `.luau.bak`) or creates a tuned copy of
a built-in one.
Template: [`gameviber/prompts/fix-feel.md`](gameviber/prompts/fix-feel.md).

### The game's sound and image

The rumble and the buttons do not say whether you are fighting, exploring or
watching a cutscene; the music and the screen usually do. GameViber listens to
the game's sound through PipeWire (`pw-record`, no root) and looks at its image
through the in-game overlay, and gives modes:

- **scenes**, defined once per game in its Signals (or described by a mode
  that suits any game), recognized from how they sound by a sound model
  (LAION's CLAP, about 200 MB) and from their captures by an image model
  (OpenAI's CLIP, about 150 MB). Both run on the CPU, only while the active
  mode uses them, and are downloaded into `~/.local/share/gameviber/models/`;
- **impacts** (strong hits heard, flashes seen) and an overall **intensity**;
- the raw measures behind them, for advanced modes.

Scenes come a few seconds late: modes use them for the mood of a phase, not to
time effects. Nothing leaves the computer; the sound is never saved, and of the
image only the captures you take.

### A game's signals

A game's **Signals** teach GameViber about the game, for all its modes (saved
in `~/.config/gameviber/games/`), step by step:

- **scenes**: name the phases that should not feel the same (battle,
  exploration, story, menu), once; describe how each sounds if the music
  changes between them. Modes made for the game read their names;

- **captures**: hold the capture combo on the gamepad (BACK + LS by default,
  Setup) while the game shows a battle, an exploration, a dialogue...: the
  image is captured without leaving the game, so it keeps the game's gamepad
  prompts, and the in-game overlay confirms. Captures go to the scene picked on
  the captures page, or "to sort" and filed there later. A few
  captures per scene make the image recognition much more reliable;
- **zones**: on a capture (zoom in as needed), draw a rectangle around something
  shown only at times (the battle interface) or around a bar (health, with the
  colors of its full and empty parts picked on the image; a bar that moves is
  found anywhere in its zone). The page shows what
  each zone reads on every capture and suggests a threshold. Modes read whether
  it is shown, or how full the bar is, ten times per second;
- **values from other programs**: a game's existing mod, or a script reading a
  game's API, can send JSON to `ws://127.0.0.1:12350` or to the pipe
  `$XDG_RUNTIME_DIR/gameviber/inputs` (`{"set": {"hp": 0.4}}`,
  `{"event": "kill"}`); declare what it sends so AI assistants know it.

When asking an AI assistant for a mode, pick **Quick** (the rumble, the buttons
and the scenes, impacts and intensity: a couple of minutes) or **Advanced**
(also the raw measures and the game's signals; the assistant may ask you to
draw zones).

### In-game overlay

Like MangoHud, GameViber can draw a small panel over the game: the active mode
and preset, the scene recognized in the game's sound, how strong the toys run (with the global cap), the mode's gauges
and what it detects ("Parry!"), and warnings (toy lost, Intiface disconnected,
mode error, panic stop). Install it from **Setup › In-game overlay**, then enable it
per game with a Steam launch option (once installed, GameViber updates it when
it starts with a newer version; restart running games to get it):

- Proton and Vulkan games: `GAMEVIBER_OVERLAY=1 %command%` (or turn it on for
  every Vulkan game). It is an implicit Vulkan layer
  ([`gameviber-overlay/`](gameviber-overlay/)), so it covers every Proton game
  (DXVK / VKD3D).
- Native OpenGL games: `~/.local/share/gameviber/gameviber-overlay %command%`.
  The launcher preloads the same library, which then hooks `glXSwapBuffers` /
  `eglSwapBuffers` (GLX and EGL, OpenGL 3.0+ and OpenGL ES 3.0+), like MangoHud.

It works fullscreen or not, on any desktop, in 64-bit and 32-bit games (see
Build). It also copies small images of the game for GameViber (480 pixels
wide, 10 per second), shrunk on the GPU and passed through shared memory; they
are never saved (turn this off on the **Game** page). Not supported yet: Flatpak Steam.
`DISABLE_GAMEVIBER_OVERLAY=1` turns it off for one game;
`GAMEVIBER_OVERLAY_DEBUG=1` prints its errors on the game's stderr.

**Anti-cheats**: the overlay is code loaded into the game's process (a Vulkan
layer, or `LD_PRELOAD` for OpenGL), the same techniques MangoHud, vkBasalt and
the Steam overlay use. An anti-cheat could still mistake it for a cheat, so do
not enable it in online games with an anti-cheat (EasyAntiCheat, BattlEye,
VAC), and keep it per game rather than on for every Vulkan game. The rest of
GameViber never touches the game: the eBPF source watches its `ioctl` calls
from the kernel and the proxy source only exposes a virtual gamepad, as Steam
Input does.

### Modes

User modes are `.luau` files in `~/.config/gameviber/modes/` (they can also be
edited in an external editor: they are reloaded on save).
API: [`docs/spec-modes.md`](docs/spec-modes.md). Built-in modes
([`gameviber/modes/`](gameviber/modes/)) are fallbacks designed per game genre:

| Mode | For | Idea |
|---|---|---|
| Simple | anything | forwards the rumble (GHR parity) |
| Accumulation | anything | points earned per vibration, draining while idle |
| Combo | fighting (Tekken, SF6) | chained hits grow stronger, final burst |
| Overheat | action / shooter (Ratchet & Clank, Doom) | compressed rumble, peaks stand out, overheat gauge |
| Tension | turn-based with QTEs (Clair Obscur) | wave building up during fights, parries rewarded |
| Engine | racing (Forza, GT) | trigger = engine RPM, rumble = road and impacts |
| Heartbeat | horror (RE, Silent Hill) | heart speeding up with every vibration |
| All or Nothing | souls-like (Elden Ring, Sekiro) | gauge rising while you survive, emptied by a big hit |
| Ambient | exploration, platformer, cosy | slow wave under the rumble, fading out when idle |
| Surge | action metroidvania (PoP: The Lost Crown, Hollow Knight) | hits and parries fill a gauge that glows in fights, spent on a crescendo |

The heuristics (parry window, "hit" threshold) need tuning per game: the
rumble does not tell who took the hit.

Options: `--source proxy|ebpf|none` (remembered afterwards), `--device /dev/input/eventX`,
`--hide`, `--no-passthrough`, `--url`, `--no-intiface`, `--mode <file or name>`,
`--preset <name>`, `-v`.

## Interception sources

| `--source` | How it works | Root | The game sees |
|---|---|---|---|
| `proxy` (default) | uinput virtual gamepad; the real one is grabbed, inputs are forwarded, rumble is captured then sent back to the real gamepad | to hide the real one (helper) | a copy (same name, VID/PID), and the real one unless hidden |
| `ebpf` | eBPF probe on the `EVIOCSFF`/`EVIOCRMFF` ioctls + passive evdev reading of play/stop | yes (helper) | the real gamepad, nothing changes |
| `none` | No interception (simulator only) | no | — |

The eBPF probe is derived from
[linux-game-haptics-router](https://github.com/madrigal-eschat/linux-game-haptics-router)
(Apache-2.0, see `LICENSE-APACHE-linux-game-haptics-router`), with the addition
of capturing the ioctl's fd to know which gamepad is targeted.

### Privileged helper

`gameviber helper` is the only code running as root. It only loads the eBPF
probe (and forwards its raw events) and hides / restores a gamepad's nodes
(`/dev/input/eventN` only). It talks JSON over stdin/stdout with the user
process, ignores Ctrl+C, and restores everything then exits as soon as stdin
closes (GameViber exiting or crashing). Fd resolution, gamepad reading and
everything else run unprivileged. When started directly as root
(`sudo ... --headless`), GameViber does without the helper.

## Build

```sh
rustup toolchain install nightly --component rust-src   # for the eBPF probe
# bpf-linker: prebuilt binary at https://github.com/aya-rs/bpf-linker/releases
cargo build --release          # SKIP_EBPF_BUILD=1 to build without the probe
cargo test
```

The 32-bit in-game overlay (for 32-bit games) is built separately; GameViber
finds it in `target/i686-unknown-linux-gnu/release/` (or `lib32/` next to the
executable) and installs it with the 64-bit one:

```sh
rustup target add i686-unknown-linux-gnu
sudo dnf install glibc-devel.i686     # Debian/Ubuntu: sudo apt install gcc-multilib
cargo build-overlay32
```

Luau is built from source (a C++ compiler is required). ONNX Runtime (for the
scene model) is downloaded prebuilt by the `ort` crate at build time and linked
statically.

## Known limitations

- Only the evdev force-feedback path is covered: gamepads driven through
  hidraw by SDL (DualShock/DualSense/Switch) need `SDL_JOYSTICK_HIDAPI=0`.
- ebpf source: effects uploaded before GameViber started are invisible until
  the game's next upload.
- Sound: needs PipeWire (`pw-record`, `pw-dump`). In automatic mode without
  the overlay, music or voice chat playing next to the game is heard too.

`tools/sdl_rumble.py` simulates an SDL3 game, `tools/fake_gamepad.py` a physical gamepad. The original Python prototype is in `prototype/`.
