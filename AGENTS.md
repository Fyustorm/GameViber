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
Linux only.

Pipeline: **source** (interception), **audio** (the game's sound), **screen** (its
image, the profile's zones) and **inputs** (other programs) → **mode** (Luau
script) → **safety layer** → **Intiface output**.

## Layout

| Path | Contents |
|---|---|
| `gameviber/src/source/` | interception sources: `proxy` (uinput virtual gamepad) and `ebpf` |
| `gameviber/src/audio/` | the game's sound: `capture` (PipeWire `pw-record` / `pw-dump`), `features` (levels, hits), `clap` (sound scene model: mel spectrogram, encoder) |
| `gameviber/src/models.rs` | scene models downloaded on demand (CLAP, CLIP): download, ONNX sessions, text embeddings |
| `gameviber/src/rumble.rs` | evdev force-feedback semantics (ff-memless) → strong/weak levels |
| `gameviber/src/gamepad.rs` | button/axis normalization (Xbox layout), panic combo |
| `gameviber/src/mode/` | Luau runtime: `library.rs` (script API), `outputs.rs` (channels, pulses, patterns), `rumble_events.rs`, `scenes.rs` (scenes fused from the sound, the image and the profile's examples), `prompt.rs` (AI requests: a per-game mode, a fix for a mode that feels wrong), `report.rs` (a session replayed offline into a mode, for the fix request), `tests.rs` |
| `gameviber/src/screen/` | the game's image: frames copied by the overlay, measures (brightness, motion, flashes), `clip` (image scene model: PIL-exact preprocessing, encoder thread), `zones` (shown-or-not and bar zones) |
| `gameviber/src/profile.rs` | game profiles (`~/.config/gameviber/games/<exe>.json`, captures as PNG in `games/<exe>/`): captures per scene (also the scene examples), zones drawn on them, declared inputs |
| `gameviber/src/inputs.rs` | values and events other programs send: local WebSocket (browsers refused) and named pipe |
| `gameviber/src/engine.rs` | engine thread: sources, audio, image, mode, safety layer, routing, output |
| `gameviber/src/session.rs` | recorded play sessions (rumble, buttons, axes, sound and image measures, hits, flashes, zones, scene embeddings, values from other programs) and their replay |
| `gameviber/src/gui/` | egui GUI: setup guide (`onboarding`), pages (`play`, `toys`, `gamepad`, `audio`, `screen` (the Game page: live image, captures, zoomable zone editor checked on every capture, inputs), `keybindings`, `overlay`, `creator`, `settings`), AI requests (`generator` dialog: a mode for a game; `feedback` page: a fix for the active mode), Luau highlighting (`luau`), `theme` |
| `gameviber/src/helper/` | privileged helper (`gameviber helper`, started through pkexec) |
| `gameviber/src/config.rs` | config files, built-in mode registry (`BUILTIN_MODES`) |
| `gameviber/modes/` | built-in modes, embedded in the binary |
| `gameviber/prompts/new-mode.md` | template of the request asking an AI assistant to write a mode for one game |
| `gameviber/prompts/fix-feel.md` | template of the request asking an AI assistant to fix a mode that does not feel right (`<!-- full -->` blocks are left out of the short request for the conversation that wrote the mode) |
| `gameviber/prompts/rules.md` | what makes a mode feel good and the script rules, shared by both requests; players can override the three templates from the Settings page (`~/.config/gameviber/prompts/`) |
| `gameviber-ebpf/`, `gameviber-common/` | eBPF probe and types shared with it; `gameviber-common/src/overlay.rs`: overlay protocol and the shared frame memory (`frames`) |
| `gameviber-overlay/` | in-game overlay: implicit Vulkan layer (`layer.rs`, `render.rs`), OpenGL swap hooks when preloaded (`gl/`), panel layout with epaint (`hud.rs`), socket client, copies of the game's image (`capture.rs`, `render.rs`, `gl/capture.rs`) |
| `gameviber/src/overlay.rs` | overlay socket server (with the games' frame memory) and layer installation |
| `docs/spec-modes.md` | mode API specification (source of truth for the script API) |
| `prototype/` | original Python prototype (reference only) |
| `tools/` | test helpers: fake gamepad, SDL rumble game |

## Build and test

```sh
cargo build --release
cargo test
SKIP_EBPF_BUILD=1 cargo test   # without the eBPF toolchain (nightly + bpf-linker)
cargo build-overlay32          # 32-bit overlay layer (i686 target + 32-bit glibc headers)
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
