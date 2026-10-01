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

The GUI provides:

- a status bar with **STOP ALL** (also: BACK + START held for 0.5 s on the
  gamepad), the global maximum intensity, the **source selector**
  (proxy / eBPF / none) and the "hide the real gamepad" checkbox;
- the list of modes and their parameters, generated from the script;
- **Monitor**: graphs of the rumble, the outputs and `plot()` values;
- **Editor**: mode editing with hot reload (Ctrl+S);
- **Routing**: which toys each mode channel drives;
- **Simulator**: fake rumble and fake buttons to test without a game;
- **Log**.

User modes are `.luau` files in `~/.config/gameviber/modes/` (they can also be
edited in an external editor: they are reloaded on save).
API: [`docs/spec-modes.md`](docs/spec-modes.md). Built-in modes
([`gameviber/modes/`](gameviber/modes/)), designed per game genre:

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

The heuristics (parry window, "hit" threshold) need tuning per game: the
rumble does not tell who took the hit.

Options: `--source proxy|ebpf|none` (remembered afterwards), `--device /dev/input/eventX`,
`--hide`, `--no-passthrough`, `--url`, `--no-intiface`, `--mode <file or name>`, `-v`.

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
