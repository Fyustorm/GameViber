#!/bin/sh
# Runs a command in the Windows build container (Containerfile), with the
# repository and the host's rustup and cargo at the same paths. The xwin SDK,
# the Wine prefix and other caches go in ~/.cache/gameviber-windows (seen as
# ~/.cache inside); Wine's temporary files are emptied first.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
cache=$HOME/.cache/gameviber-windows
mkdir -p "$cache"
exec podman run --rm -i --userns=keep-id \
  -v "$root:$root:z" -v "$HOME/.rustup:$HOME/.rustup:z" -v "$HOME/.cargo:$HOME/.cargo:z" -v "$cache:$HOME/.cache:z" \
  -e HOME="$HOME" -e RUSTUP_HOME="$HOME/.rustup" -e CARGO_HOME="$HOME/.cargo" \
  -e PATH="$HOME/.cargo/bin:$HOME/.cache/bin:/usr/local/bin:/usr/bin:/bin" \
  -e CARGO_TARGET_DIR="$root/target/xwin" -e XWIN_ACCEPT_LICENSE=1 \
  -e WINEPREFIX="$HOME/.cache/wineprefix" -e WINEDEBUG=-all -e CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER=wine \
  -w "$root" "${GAMEVIBER_WINDOWS_IMAGE:-gameviber-windows}" \
  sh -c "rm -rf \$WINEPREFIX/drive_c/users/*/Temp/* 2>/dev/null; $*"
