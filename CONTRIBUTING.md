# Contributing to GameViber

Code, comments, docs and commit messages are in English. [AGENTS.md](AGENTS.md)
holds the project's layout, rules and known pitfalls (written for AI coding
agents, but true for everyone): read it before changing the code.

## License and sign-off

GameViber is under the GNU GPL version 3 or later ([LICENSE](LICENSE)). The
built-in modes, `docs/spec-modes.md` and `gameviber/prompts/` are under the MIT
license, and the eBPF probe (`gameviber-ebpf/`, `gameviber-common/`) under MIT
or the GPL version 2 or later ([LICENSE-MIT](LICENSE-MIT)); contributions to
those parts are under the same terms.

Sign off your commits (`git commit -s`): the `Signed-off-by:` line certifies
the [Developer Certificate of Origin](https://developercertificate.org/), that
you wrote the change or have the right to contribute it under these licenses.
No other agreement is needed.

New dependencies must be under a license compatible with the GPL version 3
(MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0... not GPL-2.0-only, not
proprietary): `packaging/third-party/about.toml` lists the accepted ones.

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
`tools/fake_gamepad.py` a physical gamepad. `tools/update-gamecontrollerdb.sh`
refreshes the gamepad mappings embedded from SDL_GameControllerDB. `prototype/`
is the original Python prototype (reference only).

## How it works

Pipeline: **source** (the gamepad's rumble and buttons), **audio** (the game's
sound), **screen** (its image, through the overlay) and **inputs** (other
programs) → **mode** (Luau script) → **safety layer** (global cap, panic stop,
zero on source loss) → **Intiface** (Buttplug protocol).

### Modes

Modes are Luau scripts: [`docs/spec-modes.md`](docs/spec-modes.md) is the API
specification. Built-in modes are in [`gameviber/modes/`](gameviber/modes/);
user modes are packages in `~/.config/gameviber/modes/<name>/` (`package.rs`:
the script `mode.luau`, its variants in `variants/`, and `mode.json` with the
inputs the player set up for it, its captures in `captures/`). The **Creator**
page edits them
with hot reload, graphs, a simulator and session replay. The requests sent to
AI assistants are built from [`gameviber/prompts/`](gameviber/prompts/).
A mode is shared with its game as a `.gameviber` file (`sharing.rs`): a zip
archive of `gameviber.json` (format version, the game's name, executables and
Steam app id, the mode's inputs without capture embeddings), `mode.luau`,
`variants/*.luau` and `captures/*.png`. Importing checks the limits, the file
names and that every script loads, then makes a package of the mode, joined to
the same game (same Steam app id, or same name).

### Interception sources

| Source | How it works | Root | The game sees |
|---|---|---|---|
| `proxy` (default) | uinput virtual gamepad; the real one is grabbed, inputs are forwarded, the rumble is captured then sent back to the real gamepad | only to hide the real one | a copy (same name, VID/PID), and the real one unless hidden |
| `ebpf` | eBPF probe on the `EVIOCSFF` / `EVIOCRMFF` ioctls, plus passive evdev reading of play / stop | yes | the real gamepad, unchanged |
| `none` | no interception (simulator only) | no | — |

The eBPF source was first based on
[linux-game-haptics-router](https://github.com/madrigal-eschat/linux-game-haptics-router),
whose idea it follows (watching the `EVIOCSFF` / `EVIOCRMFF` ioctls); its probe
was since rewritten. It is loaded into the kernel as "Dual MIT/GPL", which lets
it read the effect from the game's memory (`bpf_probe_read_user`).

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

The sound comes from PipeWire (`pw-record`, `pw-dump`, `pw-link`). Phases are
recognized by CLAP (sound) and CLIP (image) running on ONNX Runtime on the CPU;
the models are downloaded on demand into `~/.local/share/gameviber/models/`.

### Platforms

Linux only for now, but every OS-specific piece lives in `linux` modules with
fallbacks for other systems, so that Windows can be added later. See
AGENTS.md, Platforms.

## Community server

`server/` is the service players publish modes to: Quarkus, one SQLite file,
built as a native binary. See [`server/README.md`](server/README.md) for its
API, dev mode and deployment. A package it accepts must stay one the app
imports: when the `.gameviber` format changes (`sharing.rs`), change
`SharedPackage` too.

The app talks to the server set at build time by `GAMEVIBER_COMMUNITY_URL`
(`community::URL`; `http://localhost:8080` without it, `https://api.gameviber.fyustorm.ovh`
for the released packages).

```sh
cd server
./mvnw test          # the API, against a database of its own
./mvnw quarkus:dev   # http://localhost:8080, back-office token "dev"
```

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

Packages and the archive carry GameViber's licenses and
`THIRD-PARTY-LICENSES.txt`, made by
[`packaging/third-party/licenses.sh`](packaging/third-party/licenses.sh): the
crates' licenses ([cargo-about](https://github.com/EmbarkStudios/cargo-about)),
Luau's, and ONNX Runtime's with its third-party notices (stored in
`packaging/third-party/` for the version `ort` downloads; the script fails
when that version changes).

```sh
cargo build --release && cargo build-overlay32
packaging/linux/package.sh     # deb, rpm, Arch package, .tar.gz and SHA256SUMS in target/package/ (needs nfpm and cargo-about)
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

### Updates

GameViber checks the repository's GitHub releases (`gameviber/src/update/`):
pre-releases only while it is one itself, drafts never. How it updates comes
from a `distribution` file shipped with it: `package` in
`/usr/lib/gameviber/` (our packages: downloaded, checked against
`SHA256SUMS`, installed with the package manager through pkexec), `archive`
next to the executable (the archive's files are replaced in place). Any other
value names the store or repository that updates it (a future Flathub or
distribution package writes its own), and GameViber only announces new
versions; so does a build from source, which has no such file. A release must
therefore keep its file names (`.deb`, `.rpm`, `.pkg.tar.zst`, `.tar.gz`) and
its `SHA256SUMS`. `GAMEVIBER_RELEASES_URL` points the check at another server
answering like GitHub's API (the tests run one).
