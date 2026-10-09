#!/bin/sh
# Makes the Windows release files in target/package/, from the release build
# (on Windows, in Git Bash: cargo build --release -p gameviber; from Linux:
# cargo xwin build --release --target x86_64-pc-windows-msvc -p gameviber):
#   packaging/windows/package.sh [gameviber.exe]
# The installer (Inno Setup's iscc must be in PATH, or ISCC set; it bundles
# the ViGEmBus and HidHide installers, downloaded and checked here), and a
# zip archive for running GameViber without installing it. cargo-about must be
# in PATH (THIRD-PARTY-LICENSES.txt). The release workflow lists them all in
# SHA256SUMS.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' gameviber/Cargo.toml | head -n1)
exe=${1:-}
if [ -z "$exe" ]; then
    for candidate in target/release/gameviber.exe target/x86_64-pc-windows-msvc/release/gameviber.exe; do
        [ -f "$candidate" ] && exe=$candidate && break
    done
fi
[ -n "$exe" ] && [ -f "$exe" ] || { echo "gameviber.exe not found: build it first" >&2; exit 1; }
out=target/package
name=gameviber-$version-windows-x86_64
mkdir -p "$out"
work=target/package-windows
rm -rf "$work"
mkdir -p "$work/stage" "$work/drivers"

# The drivers the installer offers, as their authors release them.
download() {
    curl -sSfL -o "$work/drivers/$1" "$2"
    echo "$3  $work/drivers/$1" | sha256sum -c --quiet - || { echo "$2 is not the file expected" >&2; exit 1; }
}
download ViGEmBus_setup.exe https://github.com/nefarius/ViGEmBus/releases/download/v1.22.0/ViGEmBus_1.22.0_x64_x86_arm64.exe \
    89220a7865076b342892f98865f3499fb7c4cfd673159e89d352c360fd014c6a
download HidHide_setup.exe https://github.com/nefarius/HidHide/releases/download/v1.5.230.0/HidHide_1.5.230_x64.exe \
    f4bbbcb82e6258641b887c74bc81c4c5f66e4aa811808dfc304347687b7605f6

cp "$exe" "$work/stage/gameviber.exe"
cp LICENSE LICENSE-MIT "$work/stage/"
packaging/third-party/licenses.sh "$work/stage/THIRD-PARTY-LICENSES.txt"
# The drivers the installer carries.
for driver in "ViGEmBus 1.22.0 (https://github.com/nefarius/ViGEmBus):vigembus" "HidHide 1.5.230 (https://github.com/nefarius/HidHide):hidhide"; do
    printf '\n================================================================================\n'
    printf '%s, whose installer the GameViber installer carries\n' "${driver%:*}"
    printf -- '--------------------------------------------------------------------------------\n'
    cat "packaging/third-party/${driver##*:}-LICENSE"
done >> "$work/stage/THIRD-PARTY-LICENSES.txt"

# The archive. How GameViber was installed, for its updates: it replaces these files itself.
mkdir -p "$work/$name"
cp "$work/stage/"* "$work/$name/"
cp "$here/archive-readme.txt" "$work/$name/README.txt"
printf 'archive\r\n' > "$work/$name/distribution"
rm -f "$out/$name.zip"
# zip, or 7-Zip (GitHub's Windows runners have it).
(cd "$work" && if command -v zip >/dev/null; then zip -qr "../package/$name.zip" "$name"; else 7z a -tzip -bd "../package/$name.zip" "$name" >/dev/null; fi)

# The installer.
iscc=${ISCC:-iscc}
command -v "$iscc" >/dev/null || iscc="/c/Program Files (x86)/Inno Setup 6/ISCC.exe"
# Windows paths for iscc: Git Bash's cygpath, or Wine's winepath when made from Linux.
path() {
    if command -v cygpath >/dev/null; then cygpath -w "$1"
    elif command -v winepath >/dev/null; then winepath -w "$1" 2>/dev/null
    else printf '%s' "$1"; fi
}
"$iscc" -Q "-DVersion=$version" "-DStage=$(path "$root/$work/stage")" "-DDrivers=$(path "$root/$work/drivers")" \
    "-DOutput=$(path "$root/$out")" "-DOutputName=$name-setup" "$(path "$here/gameviber.iss")"
rm -rf "$work"
ls -l "$out"
