#!/bin/sh
# Lays out the files a GameViber package installs under DESTDIR (prefix /usr),
# for deb / rpm / AUR packaging. Build first:
#   cargo build --release && cargo build-overlay32
# Needs cargo-about, for the third-party licenses.
# Usage: packaging/linux/stage.sh DESTDIR
# The 32-bit overlay is optional: without it, 32-bit games get no overlay.
set -eu
[ $# -eq 1 ] || { echo "usage: $0 DESTDIR" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
target=${CARGO_TARGET_DIR:-$root/target}
dest=$1
lib=libgameviber_overlay.so

install -Dm755 "$target/release/gameviber" "$dest/usr/bin/gameviber"
install -Dm755 "$here/gameviber-overlay" "$dest/usr/bin/gameviber-overlay"

# Libraries in directories named like the dynamic linker's $PLATFORM, so that
# one LD_PRELOAD entry suits both architectures (glibc may expand x86_64 to
# haswell or xeon_phi: links to x86_64).
install -Dm755 "$target/release/$lib" "$dest/usr/lib/gameviber/x86_64/$lib"
ln -sfn x86_64 "$dest/usr/lib/gameviber/haswell"
ln -sfn x86_64 "$dest/usr/lib/gameviber/xeon_phi"
install -Dm644 "$here/gameviber_overlay.x86_64.json" "$dest/usr/share/vulkan/implicit_layer.d/gameviber_overlay.x86_64.json"
if [ -f "$target/i686-unknown-linux-gnu/release/$lib" ]; then
    install -Dm755 "$target/i686-unknown-linux-gnu/release/$lib" "$dest/usr/lib/gameviber/i686/$lib"
    install -Dm644 "$here/gameviber_overlay.x86.json" "$dest/usr/share/vulkan/implicit_layer.d/gameviber_overlay.x86.json"
else
    echo "warning: no 32-bit overlay (cargo build-overlay32): left out" >&2
fi

# How GameViber was installed, for its updates: one of our packages,
# installed through the package manager (a store build writes its own name).
printf 'package\n' > "$dest/usr/lib/gameviber/distribution"

# Licenses: GameViber's, and those of what it includes.
install -Dm644 "$root/LICENSE" "$dest/usr/share/licenses/gameviber/LICENSE"
install -Dm644 "$root/LICENSE-MIT" "$dest/usr/share/licenses/gameviber/LICENSE-MIT"
"$root/packaging/third-party/licenses.sh" "$dest/usr/share/licenses/gameviber/THIRD-PARTY-LICENSES.txt"

install -Dm644 "$here/60-gameviber-uinput.rules" "$dest/usr/lib/udev/rules.d/60-gameviber-uinput.rules"
install -Dm644 "$here/io.github.gameviber.GameViber.policy" "$dest/usr/share/polkit-1/actions/io.github.gameviber.GameViber.policy"
install -Dm644 "$here/io.github.gameviber.GameViber.desktop" "$dest/usr/share/applications/io.github.gameviber.GameViber.desktop"
install -Dm644 "$root/gameviber/icons/gameviber.svg" "$dest/usr/share/icons/hicolor/scalable/apps/io.github.gameviber.GameViber.svg"
install -Dm644 "$root/gameviber/icons/gameviber-256.png" "$dest/usr/share/icons/hicolor/256x256/apps/io.github.gameviber.GameViber.png"
