#!/usr/bin/env bash
set -euo pipefail
ASCENSION_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ASCENSION_ROOT"
ascension_target_dir="${CARGO_TARGET_DIR:-$ASCENSION_ROOT/../../work/ascension-target}"
export CARGO_TARGET_DIR="$ascension_target_dir"
# 2.0: card and synergy texts from their numbers (after tools/author_powers.py).
python3 tools/card_text.py
# 0.7: card pictures (atlases) and interface sounds; the DLL embeds the sounds.
python3 tools/build_card_art.py
python3 tools/build_sounds.py
python3 tools/build_params.py
python3 tools/build_messages.py
# The DLL's own validation on the shipped data: a bad card/synergy/config file
# fails here instead of stopping Ascension at game start (0.4.0 did).
CARGO_TARGET_DIR="$ascension_target_dir-host" cargo run --locked --release --example validate_data -- "$ASCENSION_ROOT"
cargo xwin build --locked --release --target x86_64-pc-windows-msvc
mkdir -p dist
cp "$ascension_target_dir/x86_64-pc-windows-msvc/release/ascension.dll" dist/ascension.dll
sha256sum dist/ascension.dll > dist/SHA256SUMS
python3 tools/record_build.py
