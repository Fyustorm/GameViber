# GameViber on Windows

The Windows version, its plan and where it stands. Windows gets fewer
features than Linux (AGENTS.md, Platforms): what needs a kernel probe or code
injected into games is left out, the rest works the same.

## What it uses

| Part | Linux | Windows |
|---|---|---|
| Paths | XDG | `%APPDATA%\GameViber` (config), `%LOCALAPPDATA%\GameViber` (data) |
| File dialogs | desktop portal | the system's file dialogs (`rfd`) |
| Rumble ("proxy" source) | uinput virtual gamepad | ViGEmBus virtual Xbox 360 controller; the real gamepad read through XInput (Xbox layout) or HID (mapped like on Linux) |
| Hiding the real gamepad | helper through pkexec | HidHide, set up through an elevated GameViber (UAC) |
| Kernel probe ("ebpf" source) | eBPF | not available |
| Game sound | PipeWire | WASAPI: process loopback (Windows 10 2004+) or the default output's loopback |
| Game image | in-game overlay's copies | Windows.Graphics.Capture of the game's window (Windows 10 1903+), read like an overlay's copies |
| In-game overlay panel | Vulkan layer, `LD_PRELOAD` | not available yet |
| Keyboard shortcuts | global shortcuts portal | `RegisterHotKey` |
| `gameviber://` links, single instance | desktop entry, Unix socket | `HKCU\Software\Classes\gameviber`, named pipe |
| External inputs pipe | FIFO | `\\.\pipe\gameviber-external` |
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
ONNX Runtime), and Wine runs the tests that do not need a real Windows:

```sh
cargo install cargo-xwin
cargo xwin build --release --target x86_64-pc-windows-msvc -p gameviber
CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER=wine cargo xwin test --target x86_64-pc-windows-msvc -p gameviber
```

It needs clang 19 or newer (the MSVC headers refuse older ones), and links
named `DirectML.lib` and `PathCch.lib` next to the lowercase files xwin puts in
`~/.cache/cargo-xwin/xwin/sdk/lib/um/x86_64` (ONNX Runtime asks for them
with capitals). `packaging/windows/package.sh` also runs from Linux with
Inno Setup installed in Wine (`ISCC` set to a script running `wine ISCC.exe`).

ViGEmBus, HidHide, process loopback and Windows.Graphics.Capture are not in
Wine: they are tested on Windows only.

## Status

Nothing tested on a real Windows yet. Under Wine: the tests pass, the GUI
starts, the installer builds and installs.

- [x] Base: platform, links, shortcuts, external inputs pipe, updates, CI, installer
- [x] Sound
- [ ] Rumble
- [ ] Image
