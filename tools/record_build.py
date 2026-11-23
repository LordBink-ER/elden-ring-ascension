#!/usr/bin/env python3
"""Record the assembled release; never launches or tests the game."""
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import tomllib

project = Path(__file__).resolve().parent.parent

def digest(relative):
    return hashlib.sha256((project / relative).read_bytes()).hexdigest()

metadata = {
    "version": tomllib.loads((project / "Cargo.toml").read_text())["package"]["version"],
    "assembled_utc": datetime.now(timezone.utc).isoformat(),
    "target": "x86_64-pc-windows-msvc",
    "build_command": "cargo xwin build --locked --release --target x86_64-pc-windows-msvc",
    "native_sha256": digest("dist/ascension.dll"),
    "regulation_sha256": digest("mod/regulation.bin"),
    "identity_manifest_sha256": digest("config/identity-marker.json"),
    "messages_manifest_sha256": digest("config/messages-manifest.json"),
    "game_product_version": json.loads((project / "config/compatibility.json").read_text())["product_version"],
    "me3_version": "0.13.0",
    "verification": "Cross-compiled successfully. No further tests or game launches, as requested. New gameplay is source-derived and game-unverified.",
}
(project / "dist/BUILDINFO.json").write_text(json.dumps(metadata, indent=2) + "\n")
