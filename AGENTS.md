# AGENTS.md

Guidance for AI coding agents working on GameViber.

## Language: everything in English

**Everything in this repository is written in English**: code identifiers, comments,
log messages, GUI strings, mode scripts (`.luau`), commit messages, README, specs, docs
and tools. Do not add text in any other language, even if the user talks to you in
another language.

## What the project is

GameViber is the Linux equivalent of the Intiface Game Haptics Router. It intercepts
the rumble games send to a gamepad, listens to the game's sound, looks at its image
through the in-game overlay, takes values other programs send, transforms all of it
through a Lua (Luau) **mode**, and drives toys connected to Intiface Central (Buttplug
protocol).
Linux only for now; everything OS-specific is isolated so that other systems
(Windows first) can be added later (see Platforms).

Pipeline: **source** (interception), **audio** (the game's sound), **screen** (its
image, the profile's zones) and **inputs** (other programs) → **mode** (Luau
script) → **safety layer** → **Intiface output**.

## Layout

| Path | Contents |
|---|---|
| `gameviber/src/platform/` | what differs between operating systems for the rest of the code (paths, local time, stop signals, the GUI window system dialogs open over, file dialogs); `linux/`: XDG paths, the privileged helper (`helper/`, `gameviber helper`, started through pkexec), the device hider, the desktop's portals (`portal.rs`; file chooser: `files.rs`) |
| `gameviber/src/source/` | interception sources, OS backends started through `Sources`: `linux/proxy` (uinput virtual gamepad) and `linux/ebpf` |
| `gameviber/src/audio/` | the game's sound: `capture` (what to listen to; `linux`: PipeWire `pw-record` / `pw-dump`), `features` (levels, hits), `clap` (sound scene model: mel spectrogram, encoder) |
| `gameviber/src/models.rs` | scene models downloaded on demand (CLAP, CLIP): download, ONNX sessions, text embeddings |
| `gameviber/src/rumble.rs` | force-feedback semantics (evdev's, ff-memless) → strong/weak levels |
| `gameviber/src/gamepad.rs` | button/axis normalization (Xbox layout) from `codes` (Linux's numbering, which every source translates to), panic combo |
| `gameviber/src/mode/` | Luau runtime: `library.rs` (script API), `outputs.rs` (channels, pulses, patterns), `rumble_events.rs`, `scenes.rs` (scenes fused from the sound, the image and the game's captures; the game's scenes replace the mode's own), `prompt.rs` (AI requests: a per-game mode, a fix for a mode that feels wrong), `report.rs` (a session replayed offline into a mode, for the fix request), `tests.rs` |
| `gameviber/src/screen/` | the game's image: frames copied by the overlay, measures (brightness, motion, flashes), `clip` (image scene model: PIL-exact preprocessing, encoder thread), `zones` (shown-or-not and bar zones) |
| `gameviber/src/game.rs` | games (`~/.config/gameviber/games/<id>.json`), identified by name: linked executables (optional), sound source, modes; migration of the older exe-keyed profiles |
| `gameviber/src/package.rs` | mode packages (`~/.config/gameviber/modes/<name>/`): the script (`mode.luau`) and the inputs the player set up for the mode (`mode.json`: scenes, captures per scene, also the scene examples, as PNG in `captures/`, zones drawn on them, declared inputs); migration of the older mode files and game-held inputs |
| `gameviber/src/sharing.rs` | a mode shared with its game: `.gameviber` files (zip: `gameviber.json` with the game and the mode's inputs, `mode.luau`, `captures/`), exported from a mode's page, imported from the library (joined to the game of the same name); `gui/sharing.rs` |
| `gameviber/src/shortcuts/` | keyboard shortcuts for the combos' actions; `linux`: the desktop's global shortcuts portal (needs a desktop entry for the app id, written on first use) |
| `gameviber/src/update/` | new versions from the GitHub releases, installed according to how GameViber was installed (the `distribution` file of `packaging/linux/`): package through pkexec, archive in place, only announced for stores and source builds; `gui/updates.rs`: banner and Settings card |
| `gameviber/src/inputs/` | values and events other programs send: local WebSocket (browsers refused) and, `linux`, a named pipe |
| `gameviber/src/engine.rs` | engine thread: sources, audio, image, mode, safety layer, routing, output |
| `gameviber/src/session.rs` | recorded play sessions (rumble, buttons, axes, sound and image measures, hits, flashes, zones, scene embeddings, values from other programs) and their replay |
| `gameviber/src/gui/` | egui GUI: setup guide (`onboarding`), pages: `live` (while playing: mode, output, signals, gamepad combos), `games` (library, then each game by breadcrumb: modes, sessions), `play` (a mode's page, mode tiles), `signals` (a game's guided Signals, for the active mode), `screen` (the active mode's captures and zoomable zone editor checked on every capture), `toys`, `setup` (tabs: `gamepad`, `keybindings`, `overlay`, `audio` (default sound), other programs), `creator`, `settings`; AI requests (`generator` dialog: a mode for a game; `feedback` page: a fix for the active mode), Luau highlighting (`luau`), `theme` |
| `gameviber/src/config.rs` | config files, built-in mode registry (`BUILTIN_MODES`) |
| `gameviber/modes/` | built-in modes, embedded in the binary |
| `gameviber/prompts/new-mode.md` | template of the request asking an AI assistant to write a mode for one game |
| `gameviber/prompts/fix-feel.md` | template of the request asking an AI assistant to fix a mode that does not feel right (`<!-- full -->` blocks are left out of the short request for the conversation that wrote the mode) |
| `gameviber/prompts/rules.md` | what makes a mode feel good and the script rules, shared by both requests; players can override the three templates from the Settings page (`~/.config/gameviber/prompts/`) |
| `gameviber-ebpf/`, `gameviber-common/` | eBPF probe and types shared with it; `gameviber-common/src/overlay.rs`: overlay protocol and the shared frame memory (`frames`) |
| `gameviber-overlay/` | in-game overlay: panel layout with epaint (`hud.rs`); `linux/`: implicit Vulkan layer (`layer.rs`, `render.rs`), OpenGL swap hooks when preloaded (`gl/`), socket client, copies of the game's image (`capture.rs`, `render.rs`, `gl/capture.rs`) |
| `gameviber/src/overlay/` | the games' overlays and their frame memory; `linux/`: the Unix socket and sealed memfds, `linux/install.rs`: layer and launcher installation |
| `docs/spec-modes.md` | mode API specification (source of truth for the script API) |
| `README.md`, `docs/user-guide.md`, `CONTRIBUTING.md` | for players: install and first steps (keep it short), then the full guide; for developers: build, architecture, packaging and releases |
| `packaging/linux/` | files a package installs under `/usr` (Vulkan layer manifests, OpenGL launcher, udev rule, polkit action, desktop entry), `stage.sh` laying them out with the built binaries, `postinstall.sh`; tests check they match what the code expects. `package.sh` makes the release files (deb, rpm, Arch with `nfpm.yaml`, an archive for systems without packages) |
| `packaging/third-party/` | `licenses.sh` writing `THIRD-PARTY-LICENSES.txt` for the packages: cargo-about (`about.toml`: accepted licenses), Luau, ONNX Runtime's notices |
| `LICENSE`, `LICENSE-MIT` | GPL-3.0-or-later for GameViber; MIT for the built-in modes, the mode spec and the prompts; MIT or GPL-2.0-or-later for the eBPF probe and `gameviber-common` |
| `.github/workflows/` | `ci.yml`: tests and the Windows check; `packages.yml`: packages built on Ubuntu 24.04 (glibc 2.39, the oldest the prebuilt ONNX Runtime links with), attached to a draft release on a `v*` tag |
| `prototype/` | original Python prototype (reference only) |
| `tools/` | test helpers: fake gamepad, SDL rumble game, `check-windows.sh` (Platforms) |

## Build and test

```sh
cargo build --release
cargo test
SKIP_EBPF_BUILD=1 cargo test   # without the eBPF toolchain (nightly + bpf-linker)
cargo build-overlay32          # 32-bit overlay layer (i686 target + 32-bit glibc headers)
tools/check-windows.sh         # the code outside `linux` modules still builds for Windows
```

Run `cargo test` after any change to the runtime or to a mode.

## Rules

- **Mode API**: any change to the script API (functions, callbacks, `input` fields,
  parameters) must be reflected in `docs/spec-modes.md`, and the spec's version / API
  number bumped if the change is incompatible.
- **Built-in modes**: a new built-in mode goes in `gameviber/modes/<name>.luau` (MIT, see Licenses), is
  registered in `BUILTIN_MODES` (`gameviber/src/config.rs`), added to the load test and
  given a behaviour test in `gameviber/src/mode/tests.rs`, and listed in the table of
  `docs/user-guide.md` and in `docs/spec-modes.md` §14.3. Follow the style of the existing modes: a
  header comment, `author = "GameViber"`, a `category`, a plain-language `help` and
  1 to 3 `main_params` for players, parameters with units in their labels,
  `plot()` for the internal state worth tuning, `hud()` / `hud_event()` for what
  the in-game overlay should show, and 2 to 5 `feedback` questions (`ask()`), each
  linked to the parameter that fixes it.
- **Per-game modes come first**: built-in modes are genre fallbacks; players are steered
  towards a mode written by an AI assistant for their game. The requests
  (`gameviber/prompts/new-mode.md`, and `fix-feel.md` for fixes) share
  `prompts/rules.md` (what feels good, script rules) and embed `docs/spec-modes.md`
  without the sections listed in `SPEC_LEFT_OUT` (`mode/prompt.rs`). Keep the rules
  short, in line with the mode style rules and the known pitfalls below, and do not
  repeat in them what the spec already says.
- **Safety stays outside scripts**: the global cap, STOP ALL / panic combo and
  zeroing outputs on source loss live in the engine, never in a mode.
- **Platforms**: see the section below; run `tools/check-windows.sh` after
  touching OS-specific code or adding a dependency.
- **Packaging**: a file a package installs (`packaging/linux/`) that the code
  also generates or relies on (layer manifests, launcher, desktop entry, polkit
  action) must stay identical to it: change both, the tests compare them. Paths
  are under `/usr`. Without a package, the app keeps installing for the user
  in `~/.local/share` (`overlay/linux/install.rs`).
- **Licenses**: GameViber is GPL-3.0-or-later. Built-in modes carry
  `-- SPDX-License-Identifier: MIT` on their first line (they are meant to be
  copied); the eBPF probe declares "Dual MIT/GPL" to the kernel and must not
  use code under any other license. A new dependency must be GPL-3-compatible
  and its license listed in `packaging/third-party/about.toml`. The Settings
  page shows the GPL notice (`gui/settings.rs`, `about`).
- **Releases**: the version is in `gameviber/Cargo.toml` (and
  `gameviber-overlay/Cargo.toml`), semver with a prerelease while in alpha
  (`0.1.0-alpha.1`); tag `v<version>` to build the packages. Nothing is
  published to stores or package repositories yet.
- **Privileges**: the GUI and the main process must never run as root. Root-only work
  (eBPF probe, hiding gamepad nodes) goes through the helper, whose scope must stay
  minimal.
- Match the surrounding code: comment density, naming and idioms.

## Platforms

GameViber only runs on Linux today, but a Windows version (with fewer features:
no eBPF, an overlay of its own) is planned. Keep the way open:

- **OS-specific code lives only in modules named after the OS**: `linux.rs` or
  `linux/`, declared with `#[cfg(target_os = "linux")]`. That means any use
  of `libc`, `evdev`, `aya`, `zbus`, `wayland-*`, `std::os::unix`, `/dev`,
  `/proc`, XDG paths, PipeWire tools, pkexec or Unix sockets. Everything else
  must build on every OS.
- **Where it goes**: things the whole app needs (paths, time, signals, the
  privileged helper) in `platform/`; a part's backend next to that part
  (`source/linux/`, `audio/capture/linux.rs`, `overlay/linux/`,
  `shortcuts/linux.rs`, `inputs/linux.rs`, `update/linux.rs`,
  `gameviber-overlay/src/linux/`).
  The part's `mod.rs` holds the neutral types and logic and re-exports the
  backend's items under the same names for every OS.
- **Other systems** get each part's `unsupported.rs` (same API, "not available
  on this system yet"), so the rest still builds and runs. A Windows backend
  is a `windows.rs` / `windows/` next to `linux`, with the fallback's `cfg`
  narrowed to `not(any(target_os = "linux", target_os = "windows"))`.
- **Data stays neutral**: sources translate their gamepad to Linux's key and
  axis numbering (`gamepad::codes`) and to evdev force-feedback effects
  (`rumble::Effect`), and audio to mono f32 samples, whatever the OS.
- **Linux-only crates** go in `[target.'cfg(target_os = "linux")'.dependencies]`.
- `tools/check-windows.sh` checks the Windows build without a Windows toolchain;
  it must stay free of errors and warnings.

## Known pitfalls

- `evdev` 0.13: `VirtualDeviceBuilder::with_phys` mis-encodes the `UI_SET_PHYS` ioctl
  (EINVAL). Do not use it.
- The rumble does not say who took a hit: heuristics such as parry windows or hit
  thresholds must stay tunable parameters.
- Output to each toy is limited to 20 updates/s: waveforms faster than ~5 Hz blur.
- `input.idle` is `min(rumble_idle, input_idle)`, and `rumble_idle` counts from mode
  activation when no vibration has happened yet.
- Scenes come seconds late, can be wrong, and need the downloaded models: modes use
  them for the mood of a phase, never to time effects, and must work without them.
  Each sense only speaks about the scenes it describes; their likelihoods multiply.
  Phases sharing their music cannot be told apart by the sound (Metaphor: dungeon
  exploration and fights); the image can, poorly from descriptions alone (zero-shot
  CLIP, ~70 %), well from the profile's example images (~85 % with ten per scene) or a
  zone. A scene sets the tension (a faded background in tense phases: a slow wave in
  battles, a heartbeat in tense games, whose low point never goes to 0), the rumble and
  buttons make the peaks. Two or three contrasted scenes beat many close ones.
- The API has two levels: scenes, impacts and intensity (§6.3) for every request;
  raw sound and image, zones and other programs' values (§6.4, §6.5, §7.1) only in
  advanced requests (`SPEC_ADVANCED` in `mode/prompt.rs`). Keep new inputs in the
  advanced level unless every mode should use them.
- Image copies (overlay): made before the panel is drawn, so the panel never
  shows in them; one frame's copy is read when that swapchain image (Vulkan) or
  the pixel buffer's fence (OpenGL) comes back, never by waiting on the GPU.
  GameViber maps a game's frame memory only once its size is sealed: a game
  shrinking it would otherwise crash GameViber. Not handled yet: multisampled
  OpenGL back buffers, and HDR swapchains (values are clamped to 8 bits).
- The CLAP mel spectrogram (`audio/clap.rs`) must match `transformers`'
  `ClapFeatureExtractor`, and the CLIP preprocessing (`screen/clip.rs`) PIL's bicubic
  resize and center crop; their tests hold reference values computed with them. The
  quantized encoders give slightly different vectors across ONNX Runtime versions
  (cosine ~0.996 for CLIP).
