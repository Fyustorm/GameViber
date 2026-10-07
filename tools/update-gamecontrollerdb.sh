#!/bin/sh
# Refreshes the gamepad mappings GameViber embeds (gameviber/gamepads/): the
# Linux lines of SDL_GameControllerDB, and its license.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
dir="$root/gameviber/gamepads"
repo=https://raw.githubusercontent.com/mdqinc/SDL_GameControllerDB
sha=$(curl -sSf https://api.github.com/repos/mdqinc/SDL_GameControllerDB/commits/master | sed -n 's/^  "sha": "\([0-9a-f]*\)",$/\1/p')
[ -n "$sha" ] || { echo "cannot read the latest commit" >&2; exit 1; }
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
curl -sSfL -o "$tmp" "$repo/$sha/gamecontrollerdb.txt"
{
    echo "# SDL_GameControllerDB (https://github.com/mdqinc/SDL_GameControllerDB), commit $sha:"
    echo "# its Linux mappings, under the zlib license (LICENSE beside this file)."
    echo "# Refresh with: tools/update-gamecontrollerdb.sh"
    grep 'platform:Linux' "$tmp"
} > "$dir/gamecontrollerdb.txt"
curl -sSfL -o "$dir/LICENSE" "$repo/$sha/LICENSE"
echo "$(grep -vc '^#' "$dir/gamecontrollerdb.txt") mappings from $sha"
