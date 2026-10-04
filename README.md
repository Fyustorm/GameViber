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

- a status bar with the gamepad / rumble capture / Intiface state, the global
  maximum intensity and **STOP ALL** (also: BACK + START held for 0.5 s on the
  gamepad, configurable on the Keybindings page);
- **Play**: your modes (or the built-in ones) as tiles, then a page for the
  chosen mode with its explanation, its main settings, all its settings and
  named **presets** (e.g. one per game). It opens on the last session's mode;
- **Toys**: the connection to Intiface Central (status, address), the toys it
  found, a test buzz, which mode channels each one plays, and how it renders
  them: weakest and strongest intensity and a response curve, with buttons to
  feel each;
- **Gamepad**: gamepad and rumble capture status, the buttons and sticks
  received right now, the capture method (standard proxy or kernel probe, which
  needs root) with their pros and cons, gamepad hiding, troubleshooting;
- **Sound**: which sound modes hear (automatic: the game showing the overlay,
  else everything; one application; off), what is heard right now (loudness,
  bass / mids / treble, hits), and the **scene model** to download (see below);
- **Keybindings**: the panic stop and mark-a-moment gamepad combos;
- **Creator**: mode editing with hot reload (Ctrl+S), graphs of the rumble,
  outputs and `plot()` values, a simulator (fake rumble and buttons), recorded
  play **sessions** (the game's rumble and your inputs, replayed into a freshly
  restarted mode, with or without the toys) and the log. Recordings are saved
  in `~/.config/gameviber/recordings/`.

### A mode for your game

The best experience is a mode written for the game you play. **Play → Make a mode
for my game** guides you through getting one from any AI assistant (ChatGPT,
Claude, Gemini, Le Chat...):

1. type the game's name;
2. copy the request GameViber builds and paste it in a new conversation. It holds
   the context, the full mode API, the rules (every button the mode uses is a
   setting defaulting to the game's own binding, heuristics stay tunable) and an
   example; an assistant with web search can check the game's default controls;
3. paste the answer (or just its code, or drop the `.luau` file on the window):
   GameViber checks that the mode loads, creates it and activates it. If it does
   not load, copy the fix request and send it back to the assistant.

The request template is [`gameviber/prompts/new-mode.md`](gameviber/prompts/new-mode.md).

When a mode does not feel right, **Doesn't feel right?** (Play page) asks the
mode's own questions ("Parry detection: Often missed | Good | Also on hits
taken", declared with `ask()`, or generic ones). Many answers come with a
**quick fix** that adjusts the matching setting in one click. Otherwise pick a
recorded session (or save the last 2 minutes of play, which GameViber always
keeps in memory) and copy the request.

While playing, hold **BACK + RS** (configurable on the Keybindings page) to
**mark a moment** that felt wrong: the overlay confirms, and the last 2
minutes are saved 15 s later with the marks, which the request points out. GameViber replays the session into the mode with your settings, so the
assistant sees when the game vibrated, what you pressed and what the mode
output, along with the earlier rounds (answers, quick fixes, applied fixes) so
it does not go back and forth. Pasting the answer back updates a user mode in
place (the previous version is kept as `.luau.bak`) or creates a tuned copy of
a built-in one.
Template: [`gameviber/prompts/fix-feel.md`](gameviber/prompts/fix-feel.md).

### The game's sound

The rumble and the buttons do not say whether you are fighting, exploring or
watching a cutscene; the music usually does. GameViber listens to the game's
sound through PipeWire (`pw-record`, no root) and gives modes its loudness per
band, the hits it hears, and **scenes** each mode describes in words ("intense
battle music", "calm exploration music"). Scenes are recognized by a sound
model (LAION's CLAP, quantized, about 200 MB) downloaded from the **Sound**
page into `~/.local/share/gameviber/models/`; it runs on the CPU (about 0.1 s
every 2 s, on 2 threads) and only while the active mode declares scenes.
Nothing leaves the computer and the sound itself is never saved. Scenes come a
few seconds late: modes use them for the mood of a phase, not to time effects.

### In-game overlay

Like MangoHud, GameViber can draw a small panel over the game: the active mode
and preset, the scene recognized in the game's sound, how strong the toys run (with the global cap), the mode's gauges
and what it detects ("Parry!"), and warnings (toy lost, Intiface disconnected,
mode error, panic stop). Install it from the **Overlay** page, then enable it
per game with a Steam launch option:

- Proton and Vulkan games: `GAMEVIBER_OVERLAY=1 %command%` (or turn it on for
  every Vulkan game). It is an implicit Vulkan layer
  ([`gameviber-overlay/`](gameviber-overlay/)), so it covers every Proton game
  (DXVK / VKD3D).
- Native OpenGL games: `~/.local/share/gameviber/gameviber-overlay %command%`.
  The launcher preloads the same library, which then hooks `glXSwapBuffers` /
  `eglSwapBuffers` (GLX and EGL, OpenGL 3.0+ and OpenGL ES 3.0+), like MangoHud.

It works fullscreen or not, on any desktop, in 64-bit and 32-bit games (see
Build). Not supported yet: Flatpak Steam.
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
