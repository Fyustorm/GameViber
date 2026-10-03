# AGENTS.md

Guidance for AI coding agents working on GameViber.

## Language: everything in English

**Everything in this repository is written in English**: code identifiers, comments,
log messages, GUI strings, mode scripts (`.luau`), commit messages, README, specs, docs
and tools. Do not add text in any other language, even if the user talks to you in
another language.

## What the project is

GameViber is the Linux equivalent of the Intiface Game Haptics Router. It intercepts
the rumble games send to a gamepad, transforms it through a Lua (Luau) **mode**, and
drives toys connected to Intiface Central (Buttplug protocol). Linux only.

Pipeline: **source** (interception) → **mode** (Luau script) → **safety layer** →
**Intiface output**.

## Layout

| Path | Contents |
|---|---|
| `gameviber/src/source/` | interception sources: `proxy` (uinput virtual gamepad) and `ebpf` |
| `gameviber/src/rumble.rs` | evdev force-feedback semantics (ff-memless) → strong/weak levels |
| `gameviber/src/gamepad.rs` | button/axis normalization (Xbox layout), panic combo |
| `gameviber/src/mode/` | Luau runtime: `library.rs` (script API), `outputs.rs` (channels, pulses, patterns), `rumble_events.rs`, `tests.rs` |
| `gameviber/src/engine.rs` | engine thread: sources, mode, safety layer, routing, output |
| `gameviber/src/gui/` | egui GUI: setup guide (`onboarding`), pages (`play`, `toys`, `connection`, `creator`), `theme` |
| `gameviber/src/helper/` | privileged helper (`gameviber helper`, started through pkexec) |
| `gameviber/src/config.rs` | config files, built-in mode registry (`BUILTIN_MODES`) |
| `gameviber/modes/` | built-in modes, embedded in the binary |
| `gameviber-ebpf/`, `gameviber-common/` | eBPF probe and types shared with it |
| `docs/spec-modes.md` | mode API specification (source of truth for the script API) |
| `prototype/` | original Python prototype (reference only) |
| `tools/` | test helpers: fake gamepad, SDL rumble game |

## Build and test

```sh
cargo build --release
cargo test
SKIP_EBPF_BUILD=1 cargo test   # without the eBPF toolchain (nightly + bpf-linker)
```

Run `cargo test` after any change to the runtime or to a mode.

## Rules

- **Mode API**: any change to the script API (functions, callbacks, `input` fields,
  parameters) must be reflected in `docs/spec-modes.md`, and the spec's version / API
  number bumped if the change is incompatible.
- **Built-in modes**: a new built-in mode goes in `gameviber/modes/<name>.luau`, is
  registered in `BUILTIN_MODES` (`gameviber/src/config.rs`), added to the load test and
  given a behaviour test in `gameviber/src/mode/tests.rs`, and listed in the README
  table and in `docs/spec-modes.md` §14.3. Follow the style of the existing modes: a
  header comment, `author = "GameViber"`, a `category`, a plain-language `help` and
  1 to 3 `main_params` for players, parameters with units in their labels, and
  `plot()` for the internal state worth tuning.
- **Safety stays outside scripts**: the global cap, STOP ALL / panic combo and
  zeroing outputs on source loss live in the engine, never in a mode.
- **Privileges**: the GUI and the main process must never run as root. Root-only work
  (eBPF probe, hiding gamepad nodes) goes through the helper, whose scope must stay
  minimal.
- Match the surrounding code: comment density, naming and idioms.

## Known pitfalls

- `evdev` 0.13: `VirtualDeviceBuilder::with_phys` mis-encodes the `UI_SET_PHYS` ioctl
  (EINVAL). Do not use it.
- The rumble does not say who took a hit: heuristics such as parry windows or hit
  thresholds must stay tunable parameters.
- Output to each toy is limited to 20 updates/s: waveforms faster than ~5 Hz blur.
- `input.idle` is `min(rumble_idle, input_idle)`, and `rumble_idle` counts from mode
  activation when no vibration has happened yet.
