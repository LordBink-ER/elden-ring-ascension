# Building ELDEN RING: ASCENSION

The released mod is one native DLL (`dist/ascension.dll`) plus data files. Mod Engine 3
loads the DLL through the `ascension.me3` profile. Everything here builds it from source.

## Requirements

- A recent stable Rust (the release was built with 1.100.0) with the `x86_64-pc-windows-msvc` target.
- On Windows: Visual Studio Build Tools (MSVC). On Linux: `cargo-xwin`.
- Internet access on the first build (crates.io and the pinned fromsoftware-rs git revision).

## Build the DLL

Windows:

```
rustup target add x86_64-pc-windows-msvc
cargo build --release --target x86_64-pc-windows-msvc
```

Linux (cross-compile):

```
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin
cargo xwin build --release --target x86_64-pc-windows-msvc
```

Output: `target/x86_64-pc-windows-msvc/release/ascension.dll`. All dependencies are pinned in
`Cargo.lock`.

The Nexus release is built with `python3 tools/make_release.py`, which adds only
`--remap-path-prefix` flags (so no build-machine paths end up in the binary) and
`/PDBALTPATH:ascension.pdb`. The code is the same.

## Check the card data

```
cargo run --release --example validate_data -- .
```

It audits all 261 cards, their 1305 levels, synergies, bosses and procs.

## What the DLL does (for reviewers)

- Loaded by Mod Engine 3 as a native mod (`[[natives]]` in `ascension.me3`), offline only.
- Draws its menus with hudhook (a DirectX 12 present hook) and Dear ImGui.
- Reads game state and registers game tasks through fromsoftware-rs; applies the mod's own
  SpEffect rows to the player and enemies.
- Writes files only inside its own folder: `state/` (card progress and backups),
  `logs/`, `local/` and `config/ascension.toml` (settings).
- Reads the game's save file (`%APPDATA%\EldenRing\<SteamID>\ER0000.ascension.sl2`) to recognise
  the character. It never writes the save directly.
- No network access, no downloads, no process launching. Its imports are only Windows system
  DLLs (kernel32, user32, ntdll, bcrypt, winmm, xinput1_4, dxgi, d3d12, d3dcompiler_47,
  oleaut32).
- `Launch Ascension.bat` and `launch-ascension-linux.sh` are one-line scripts that call
  `me3 launch --game eldenring --profile ascension.me3`.

## Data package (`mod/`)

`mod/regulation.bin` and `mod/msg/` are edited copies of the game's own files, generated from
the builder's game install by `tools/build_params.py` and `tools/build_messages.py`. They are
FromSoftware data, so they are not in this repository; the Nexus download contains them.

## Generators

- `tools/author_powers.py` writes `config/powers.json` (the cards), then `tools/card_text.py`
  writes their English texts.
- `tools/build_card_art.py` paints the card artwork procedurally into `assets/cards`.
- `tools/build_sounds.py` synthesises the sounds in `assets/sounds`.
- `tools/build_icon.py` and `tools/build_promo.py` draw the icon and the mod page images.
