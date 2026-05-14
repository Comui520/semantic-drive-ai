#!/usr/bin/env bash
# Build script for Semantic Drive AI (Windows GNU)
# Automatically copies MinGW DLLs needed at runtime.

set -e

MINGW_BIN="/c/Users/Comui/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin"

export PATH="$HOME/.cargo/bin:$MINGW_BIN:$PATH"

echo "==> Building..."
cargo build "$@"

echo "==> Copying runtime DLLs..."
cp "$MINGW_BIN/libstdc++-6.dll" target/debug/ 2>/dev/null || true
cp "$MINGW_BIN/libgcc_s_seh-1.dll" target/debug/ 2>/dev/null || true
cp "$MINGW_BIN/libwinpthread-1.dll" target/debug/ 2>/dev/null || true

echo "==> Done. Binary at target/debug/semantic-drive-ai.exe"
