#!/bin/sh
# Renders GameViber's icon (gameviber/icons/gameviber.svg, the one to edit)
# into the files made from it: the PNG of the window and the GUI, Windows'
# .ico (embedded in gameviber.exe by build.rs), the website's favicon and
# its touch icon. Needs ImageMagick with librsvg.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
icons="$root/gameviber/icons"
site="$root/server/src/main/webui-public/public"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# Each size drawn from the SVG, not scaled down from a large one: sharper.
render() {
    magick -background none -density "$(($1 * 72 / 512 * 4))" "$icons/gameviber.svg" -resize "$1x$1" "$2"
}
render 256 "$icons/gameviber-256.png"
for size in 16 24 32 48 64 128 256; do
    render "$size" "$tmp/$size.png"
done
magick "$tmp/16.png" "$tmp/24.png" "$tmp/32.png" "$tmp/48.png" "$tmp/64.png" "$tmp/128.png" "$tmp/256.png" "$icons/gameviber.ico"
cp "$icons/gameviber.svg" "$site/favicon.svg"
# iOS rounds the corners itself and fills transparency with black.
magick -background "#121419" -density 144 "$icons/gameviber.svg" -resize 180x180 -flatten "$site/apple-touch-icon.png"
