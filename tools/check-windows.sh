#!/bin/sh
# Checks that everything outside the `linux` modules still builds for Windows
# (AGENTS.md, Platforms). Nothing is linked, so no Windows toolchain is needed:
# the C/C++ dependencies (Luau) are compiled with the host's gcc, and ONNX
# Runtime is not downloaded (DOCS_RS).
# Needs: rustup target add x86_64-pc-windows-gnu
set -e
cd "$(dirname "$0")/.."
export CC_x86_64_pc_windows_gnu=gcc CXX_x86_64_pc_windows_gnu=g++ AR_x86_64_pc_windows_gnu=ar DOCS_RS=1
exec cargo check --target x86_64-pc-windows-gnu -p gameviber -p gameviber-overlay "$@"
