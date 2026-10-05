#!/bin/sh
# Writes the licenses of everything compiled into GameViber that it did not
# write itself, which its packages must carry (packaging/linux/stage.sh):
#   packaging/third-party/licenses.sh OUTPUT
# The Rust crates come from cargo-about (in PATH; see about.toml), then what it
# does not see: Luau (C++, built by a build dependency of mlua) and ONNX
# Runtime (a prebuilt library, with the notices of what it contains).
set -eu
[ $# -eq 1 ] || { echo "usage: $0 OUTPUT" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
out=$1
cd "$root"
# Every crate of Cargo.lock, those of other systems included: cargo-about reads
# them all (cargo metadata), a Linux build only downloads its own.
cargo fetch --locked --quiet

# The ONNX Runtime notices in this directory are those of the version ort
# downloads: refresh them when it changes.
onnxruntime=1.28.0
ort_version=$(awk '/^name = "ort-sys"/ { getline; gsub(/version = |"/, ""); print }' Cargo.lock)
ort_dist=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/"ort-sys-$ort_version"/build/download/dist.tsv | head -n1)
grep -q "ms@$onnxruntime/" "$ort_dist" || {
    echo "ort-sys $ort_version no longer downloads ONNX Runtime $onnxruntime: update onnxruntime-*.txt and this script" >&2
    exit 1
}

luau_version=$(awk '/^name = "luau0-src"/ { getline; gsub(/version = |"/, ""); print }' Cargo.lock)
luau_license=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/"luau0-src-$luau_version"/LICENSE | head -n1)
[ -f "$luau_license" ] || { echo "Luau's license not found (cargo fetch first)" >&2; exit 1; }

{
    cat <<'HEAD'
GameViber: licenses of the software it includes
================================================

GameViber itself is under the GNU General Public License version 3 or later
(LICENSE), some parts also under the MIT license (LICENSE-MIT). It includes
the software below, under these licenses. Its source code, and the source of
the versions of these libraries it uses, are at
https://github.com/Fyustorm/GameViber (Cargo.lock lists the exact versions).

HEAD
    cargo about generate --locked -c "$here/about.toml" --workspace "$here/about.hbs"
    printf '\n================================================================================\n'
    printf 'Luau %s (https://github.com/luau-lang/luau), via luau0-src\n' "${luau_version#*+luau}"
    printf -- '--------------------------------------------------------------------------------\n'
    cat "$luau_license"
    printf '\n================================================================================\n'
    printf 'ONNX Runtime %s (https://github.com/microsoft/onnxruntime), via ort-sys\n' "$onnxruntime"
    printf -- '--------------------------------------------------------------------------------\n'
    cat "$here/onnxruntime-LICENSE"
    printf '\nONNX Runtime includes the components below. Eigen is under the Mozilla Public\n'
    printf 'License 2.0: its source code is at https://gitlab.com/libeigen/eigen.\n\n'
    cat "$here/onnxruntime-ThirdPartyNotices.txt"
} > "$out"
