#!/bin/sh
# Makes every Linux release file in target/package/, from the release builds:
#   cargo build --release && cargo build-overlay32
#   packaging/linux/package.sh
# deb, rpm and Arch packages (nfpm, which must be in PATH), an archive for
# installing without a package (immutable distros, SteamOS), and SHA256SUMS.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' gameviber/Cargo.toml | head -n1)
out=target/package
lib=libgameviber_overlay.so
rm -rf "$out"
mkdir -p "$out"

packaging/linux/stage.sh "$out/root"
[ -f "$out/root/usr/lib/gameviber/i686/$lib" ] || { echo "the 32-bit overlay is missing: cargo build-overlay32" >&2; exit 1; }
# 0.1.0-alpha.1: version 0.1.0, prerelease alpha.1. nfpm leaves the
# prerelease out of Arch's pkgver: there it is 0.1.0alpha.1, which pacman
# also orders before 0.1.0.
base=${version%%-*}
pre=${version#"$base"}
pre=${pre#-}
for packager in deb rpm; do
    VERSION=$base PRERELEASE=$pre nfpm package -f packaging/linux/nfpm.yaml -p "$packager" -t "$out/"
done
VERSION=$base$pre PRERELEASE= nfpm package -f packaging/linux/nfpm.yaml -p archlinux -t "$out/"

# The archive: GameViber finds the overlay libraries next to its executable
# and installs them for the user (Setup › In-game overlay).
name=gameviber-$version-x86_64
mkdir -p "$out/$name/lib32"
cp target/release/gameviber "$out/$name/"
cp "target/release/$lib" "$out/$name/"
cp "target/i686-unknown-linux-gnu/release/$lib" "$out/$name/lib32/"
cp packaging/linux/archive-readme.txt "$out/$name/README.txt"
# How GameViber was installed, for its updates: it replaces these files itself.
printf 'archive\n' > "$out/$name/distribution"
tar -C "$out" -czf "$out/$name.tar.gz" "$name"
rm -rf "$out/root" "$out/$name"

(cd "$out" && sha256sum -- * > SHA256SUMS)
ls -l "$out"
