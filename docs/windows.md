# GameViber on Windows

The Windows version, its plan and where it stands. Windows gets fewer
features than Linux (AGENTS.md, Platforms): what needs a kernel probe or code
injected into games is left out, the rest works the same.

## What it uses

| Part | Linux | Windows |
|---|---|---|
| Paths | XDG | `%APPDATA%\GameViber` (config), `%LOCALAPPDATA%\GameViber` (data) |
| File dialogs | desktop portal | the system's file dialogs (`IFileOpenDialog`) |
| Rumble ("proxy" source) | uinput virtual gamepad | ViGEmBus virtual Xbox 360 controller; the real gamepad read through XInput (Xbox layout) or HID (mapped like on Linux) |
| Hiding the real gamepad | helper through pkexec | HidHide, set up through an elevated GameViber (UAC) |
| Kernel probe ("ebpf" source) | eBPF | not available |
| Game sound | PipeWire | WASAPI: process loopback (Windows 10 2004+) or the default output's loopback |
| Game image | in-game overlay's copies | Windows.Graphics.Capture of the game's window (Windows 10 1903+), read like an overlay's copies: the window in front when it covers its screen, or a game library's (Steam, Epic, GOG, Xbox...) |
| In-game overlay panel | Vulkan layer, `LD_PRELOAD` | not available yet |
| Keyboard shortcuts | global shortcuts portal | `RegisterHotKey` |
| `gameviber://` links, single instance | desktop entry, Unix socket | `HKCU\Software\Classes\gameviber`, named pipe |
| External inputs pipe | FIFO | `\\.\pipe\gameviber-<user>-external` |
| Steam app id | the game's environment | Steam's `RunningAppID` in the registry |
| Updates | packages through pkexec, archive | the installer run silently, or the zip archive replaced in place |
| Packaging | deb, rpm, Arch, tar.gz | Inno Setup installer (ViGEmBus and HidHide offered), zip |

## Plan

1. Base: platform (paths, local time, stop signals, file dialogs, Steam app
   id), links and single instance, shortcuts, external inputs pipe, updates,
   Windows build in CI, installer.
2. Sound: WASAPI capture.
3. Rumble: ViGEmBus source, HidHide.
4. Image: Windows.Graphics.Capture.
5. Later: an in-game panel (transparent window or injected overlay).

## Building from Linux

`cargo-xwin` cross-compiles the MSVC build (Luau with clang-cl, the prebuilt
ONNX Runtime), and Wine runs the tests that do not need a real Windows. The
container of `tools/windows/` has all it needs (clang 19: the MSVC headers
refuse older ones; Wine with its 32-bit part, for Inno Setup):

```sh
podman build -t gameviber-windows tools/windows
tools/windows/run.sh 'cargo install cargo-xwin'     # once, into ~/.cargo
tools/windows/run.sh 'cargo xwin build --release --target x86_64-pc-windows-msvc -p gameviber'
tools/windows/run.sh 'cargo xwin test --target x86_64-pc-windows-msvc -p gameviber'   # under Wine
tools/windows/run.sh 'xvfb-run -a wine target/xwin/x86_64-pc-windows-msvc/debug/gameviber.exe'
```

ONNX Runtime asks for `DirectML.lib` and `PathCch.lib` with capitals: link
them to the lowercase files xwin puts in
`~/.cache/gameviber-windows/cargo-xwin/xwin/sdk/lib/um/x86_64` once it has
downloaded them. `packaging/windows/package.sh` also runs there with Inno
Setup installed in Wine (`ISCC` set to a script running `wine ISCC.exe`).

Under Wine, `std::fs::remove_dir_all` fails (Wine's read-only files): tests
must not depend on it.

ViGEmBus, HidHide, process loopback and Windows.Graphics.Capture are not in
Wine: they are tested on Windows only.

## Status

Everything of the plan's first four steps is written, **nothing has run on a
real Windows yet**. Under Wine: the build links, the tests pass (unit tests of
the Windows parts included: sample conversion, HID values, XInput layout,
HidHide's lists, frame downscaling, the game window heuristic, pipes), the GUI
starts, the installer builds and installs, the sound and gamepad sources fail
cleanly (Wine has no sound device, ViGEmBus nor capture).

- [x] Base: platform, links, shortcuts, external inputs pipe, updates, CI, installer
- [x] Sound
- [x] Rumble
- [x] Image

### To check on Windows

1. The installer: ViGEmBus and HidHide offered and installed, the
   `gameviber://` links open GameViber, a second start hands its link over.
2. An Xbox controller: Setup › Gamepad says it is passed on, a game sees the
   virtual controller (`joy.cpl` lists it), its rumble shows on the Rumble
   capture card and still reaches the controller.
3. A DualShock or a generic gamepad: known (SDL's database) or set up on the
   Gamepad page, games get it as an Xbox 360 controller; its buttons and axes
   in the right places (the HID numbering is SDL's on Windows: check it
   against a mapping of SDL_GameControllerDB).
4. Hiding: Windows asks to allow it, games only see the virtual controller,
   the real one comes back when GameViber stops (and after killing it, at the
   next start). Unknown: whether HidHide also hides Xbox controllers this way
   (their HID side and its parent are listed).
5. The sound: the game's own sound in Auto (Setup › Sound), or everything.
6. The image: a borderless game shows in Setup › In-game overlay, Creator ›
   Captures & indicators gets live images, no yellow border on Windows 11;
   the HDR and multi-monitor cases.
7. Keyboard shortcuts, the external inputs pipe
   (`echo {"event":"kill"} > \\.\pipe\gameviber-%USERNAME%-external`), the
   file dialogs, an update from the installer and from the zip.
8. The CI's Windows jobs (tests, packages) on GitHub.
