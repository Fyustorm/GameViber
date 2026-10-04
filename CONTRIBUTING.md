# Contributing to GameViber

Code, comments, docs and commit messages are in English. [AGENTS.md](AGENTS.md)
holds the project's layout, rules and known pitfalls (written for AI coding
agents, but true for everyone): read it before changing the code.

## Build and test

```sh
rustup toolchain install nightly --component rust-src   # for the eBPF probe
# bpf-linker: prebuilt binary at https://github.com/aya-rs/bpf-linker/releases
cargo build --release          # SKIP_EBPF_BUILD=1: without the eBPF probe
cargo test                     # after any change to the runtime or a mode
tools/check-windows.sh         # the code outside `linux` modules builds for Windows
```

Luau is built from source (needs a C++ compiler). ONNX Runtime is downloaded
prebuilt by the `ort` crate and linked statically.

The 32-bit overlay (for 32-bit games) is built apart; GameViber finds it in
`target/i686-unknown-linux-gnu/release/` or in `lib32/` next to its executable:

```sh
rustup target add i686-unknown-linux-gnu
sudo dnf install glibc-devel.i686     # Debian/Ubuntu: sudo apt install gcc-multilib
cargo build-overlay32
```

Run it with `./target/release/gameviber`, or `--headless -v` for logs only.
Options: `--source proxy|ebpf|none` (remembered), `--device /dev/input/eventX`,
`--hide`, `--no-passthrough`, `--url`, `--no-intiface`,
`--mode <file or built-in name>`, `--preset <name>`, `-v`.

Test helpers: `tools/sdl_rumble.py` simulates an SDL3 game,
`tools/fake_gamepad.py` a physical gamepad. `prototype/` is the original
Python prototype (reference only).

## How it works

Pipeline: **source** (the gamepad's rumble and buttons), **audio** (the game's
sound), **screen** (its image, through the overlay) and **inputs** (other
programs) → **mode** (Luau script) → **safety layer** (global cap, panic stop,
zero on source loss) → **Intiface** (Buttplug protocol).

### Modes

Modes are Luau scripts: [`docs/spec-modes.md`](docs/spec-modes.md) is the API
specification. Built-in modes are in [`gameviber/modes/`](gameviber/modes/);
user modes in `~/.config/gameviber/modes/`. The **Creator** page edits them
with hot reload, graphs, a simulator and session replay. The requests sent to
AI assistants are built from [`gameviber/prompts/`](gameviber/prompts/).

### Interception sources

| Source | How it works | Root | The game sees |
|---|---|---|---|
| `proxy` (default) | uinput virtual gamepad; the real one is grabbed, inputs are forwarded, the rumble is captured then sent back to the real gamepad | only to hide the real one | a copy (same name, VID/PID), and the real one unless hidden |
| `ebpf` | eBPF probe on the `EVIOCSFF` / `EVIOCRMFF` ioctls, plus passive evdev reading of play / stop | yes | the real gamepad, unchanged |
| `none` | no interception (simulator only) | no | — |

The eBPF probe is derived from
[linux-game-haptics-router](https://github.com/madrigal-eschat/linux-game-haptics-router)
(Apache-2.0, see `LICENSE-APACHE-linux-game-haptics-router`), with the
addition of capturing the ioctl's fd to know which gamepad is targeted.

### Privileged helper

`gameviber helper`, started through `pkexec`, is the only code running as root.
It only loads the eBPF probe (forwarding its raw events) and hides / restores a
gamepad's nodes (`/dev/input/eventN` only). It talks JSON lines over
stdin/stdout, ignores Ctrl+C, and restores everything then exits when stdin
closes (GameViber exiting or crashing). Started as root (`sudo ... --headless`),
GameViber does without it. Packages install a polkit action for it that only
covers `/usr/bin/gameviber helper`.

### In-game overlay

[`gameviber-overlay/`](gameviber-overlay/) is an implicit Vulkan layer and,
when preloaded (`LD_PRELOAD`, through the `gameviber-overlay` launcher),
OpenGL swap hooks (GLX and EGL). It talks to GameViber over a Unix socket and
shares small copies of the game's image (480 pixels wide, 10 per second,
shrunk on the GPU) through sealed shared memory. `GAMEVIBER_OVERLAY_DEBUG=1`
prints its errors on the game's stderr.

### Sound and image

The sound comes from PipeWire (`pw-record`, `pw-dump`, `pw-link`). Scenes are
recognized by CLAP (sound) and CLIP (image) running on ONNX Runtime on the CPU;
the models are downloaded on demand into `~/.local/share/gameviber/models/`.

### Platforms

Linux only for now, but every OS-specific piece lives in `linux` modules with
fallbacks for other systems, so that Windows can be added later. See
AGENTS.md, Platforms.

## Packaging and releases

[`packaging/linux/`](packaging/linux/) holds the files a package installs under
`/usr`, and the scripts that build the packages:

| Path | What |
|---|---|
| `/usr/bin/gameviber`, `/usr/bin/gameviber-overlay` | the app, and the launcher for OpenGL games |
| `/usr/lib/gameviber/{x86_64,i686}/libgameviber_overlay.so` | the overlay library (`haswell`, `xeon_phi`: links to `x86_64`, for `$PLATFORM`) |
| `/usr/share/vulkan/implicit_layer.d/gameviber_overlay.{x86_64,x86}.json` | the Vulkan layer, on in games with `GAMEVIBER_OVERLAY=1` |
| `/usr/lib/udev/rules.d/60-gameviber-uinput.rules` | lets the logged-in user create the virtual gamepad (`/dev/uinput`) |
| `/usr/share/polkit-1/actions/io.github.gameviber.GameViber.policy` | the helper's polkit action |
| `/usr/share/applications/io.github.gameviber.GameViber.desktop` | the menu entry, also needed by the keyboard shortcuts portal |

Without a package (the `.tar.gz`), GameViber installs the overlay for the user
under `~/.local/share` itself.

```sh
cargo build --release && cargo build-overlay32
packaging/linux/package.sh     # deb, rpm, Arch package, .tar.gz and SHA256SUMS in target/package/ (needs nfpm)
```

On GitHub, the **CI** workflow runs the tests and the Windows check; the
**Packages** workflow builds the packages on Ubuntu 24.04 (glibc 2.39, the
oldest the prebuilt ONNX Runtime links with). To release, set the version in
`gameviber/Cargo.toml` and `gameviber-overlay/Cargo.toml` (`0.1.0-alpha.1`
while in alpha), then push a matching tag:

```sh
git tag v0.1.0-alpha.1 && git push origin v0.1.0-alpha.1
```

The packages are attached to a draft release, published by hand.
