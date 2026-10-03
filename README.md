# GameViber

Linux equivalent of the Intiface Game Haptics Router: intercepts the rumble
games send to the gamepad, transforms it through a **Lua-scriptable mode** and
drives the toys connected to Intiface Central.

## Usage

Start Intiface Central ("Start Server"), then, **before the game**:

```sh
./target/release/gameviber                      # GUI
./target/release/gameviber --headless -v        # no GUI, logs only
```

Do not run GameViber with `sudo`: the eBPF source and gamepad hiding go
through a **privileged helper** started on demand via `pkexec` (one password
prompt per session).

At first launch, a setup guide walks through Intiface Central, the toys, the
gamepad (with the choice of capture method) and a first mode. It can be run
again from the Connection page. Then the GUI provides:

- a status bar with the gamepad / rumble capture / Intiface state, the global
  maximum intensity and **STOP ALL** (also: BACK + START held for 0.5 s on the
  gamepad);
- **Play**: the modes as tiles, and the active mode's explanation, its main
  settings, all its settings and named **presets** (e.g. one per game);
- **Toys**: the toys Intiface found, a test buzz, and which mode channels each
  one plays;
- **Connection**: status, capture method (standard proxy or kernel probe) with
  their pros and cons, gamepad hiding, Intiface address, troubleshooting;
- **Creator**: mode editing with hot reload (Ctrl+S), graphs of the rumble,
  outputs and `plot()` values, a simulator (fake rumble and buttons) and the log.

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

Luau is built from source (a C++ compiler is required).

## Known limitations

- Only the evdev force-feedback path is covered: gamepads driven through
  hidraw by SDL (DualShock/DualSense/Switch) need `SDL_JOYSTICK_HIDAPI=0`.
- ebpf source: effects uploaded before GameViber started are invisible until
  the game's next upload.

`tools/sdl_rumble.py` simulates an SDL3 game, `tools/fake_gamepad.py` a physical gamepad. The original Python prototype is in `prototype/`.
