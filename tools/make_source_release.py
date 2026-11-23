#!/usr/bin/env python3
"""3.4.1: a clean, public copy of the source (for Nexus Mods' review and GitHub).

  python3 tools/make_source_release.py [--build]

Copies an allow-list only: Cargo files, src/, examples/, tests/, config/, the mod's
own assets, tools/, release/ texts and images, the me3 profile and licenses. Never
state/, local/, logs/, dist/, mod/ (game data), saves, internal notes or work/.
Writes README.md and BUILDING.md, then refuses to finish if any file contains the
home folder, the user name, a Steam account id or the e-mail address.
--build compiles the DLL from the copy (separate target dir) to prove it builds.
Output: work/source-release/elden-ring-ascension (no git; publishing is separate).
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
WORK = PROJECT.parents[1] / "work"
OUT = WORK / "source-release" / "elden-ring-ascension"
HOME = Path.home()
# This machine's workspace folder (relative to home), never published.
LOCAL_ROOT = str(PROJECT.parents[3].relative_to(HOME)) if PROJECT.is_relative_to(HOME) else str(PROJECT.parents[3])

FILES = ["Cargo.toml", "Cargo.lock", ".cargo/config.toml", "ascension.me3", "LICENSE"]
TREES = ["src", "examples", "tests", "config", "assets/fonts", "assets/sounds", "assets/icon",
         "assets/cards", "tools", "release"]
SKIP = shutil.ignore_patterns("__pycache__", "*.pyc", "override", "launch-ascension.sh")


def version() -> str:
    return next(l.split('"')[1] for l in (PROJECT / "Cargo.toml").read_text().splitlines() if l.startswith("version"))


def private_terms() -> list[bytes]:
    terms = {str(HOME), HOME.name, LOCAL_ROOT}
    ctx = PROJECT / "local/launch-context.json"
    if ctx.is_file():
        terms.add(str(json.loads(ctx.read_text()).get("steam_account", "")))
    try:
        email = subprocess.run(["git", "config", "--global", "user.email"], capture_output=True, text=True).stdout.strip()
        if email:
            terms.add(email)
    except OSError:
        pass
    out = []
    for t in terms:
        if len(t) >= 4:
            out += [t.encode(), t.encode().lower(), t.encode("utf-16-le")]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build", action="store_true")
    args = ap.parse_args()
    v = version()
    shutil.rmtree(OUT, ignore_errors=True)
    OUT.mkdir(parents=True)
    for f in FILES:
        dst = OUT / f
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(PROJECT / f, dst)
    for t in TREES:
        shutil.copytree(PROJECT / t, OUT / t, ignore=SKIP)
    lic = OUT / "licenses"
    lic.mkdir()
    for f in (PROJECT / "docs/third-party").glob("*.txt"):
        shutil.copy(f, lic)
    shutil.copy(PROJECT / "assets/fonts/NotoSerifDisplay-OFL.txt", lic)
    rustc = subprocess.run(["rustc", "--version"], capture_output=True, text=True).stdout.split()[1]
    (OUT / "README.md").write_text((PROJECT / "release/SOURCE-README.md").read_text().replace("{VERSION}", v))
    (OUT / "BUILDING.md").write_text((PROJECT / "release/BUILDING.md").read_text().replace("RUSTC_VERSION", rustc))
    for f in ("SOURCE-README.md", "BUILDING.md"):
        (OUT / "release" / f).unlink(missing_ok=True)
    notices = WORK / "release" / f"ELDEN-RING-ASCENSION-{v}" / "Ascension" / "THIRD-PARTY-NOTICES.txt"
    if notices.is_file():
        shutil.copy(notices, OUT / "THIRD-PARTY-NOTICES.txt")
    # Local tool defaults point into this machine's folders; the public copy
    # uses neutral ones (set ASCENSION_DOTNET / ASCENSION_SOULSFORMATS).
    bm = OUT / "tools/build_messages.py"
    root = re.escape(LOCAL_ROOT)
    text = re.sub(r"Path\.home\(\) / '" + root + r"/[^']*/dotnet'", "'dotnet'", bm.read_text())
    text = re.sub(r"Path\.home\(\) / '" + root + r"/[^']*/SoulsFormats\.dll'", "'SoulsFormats.dll'", text)
    bm.write_text(text)
    (OUT / ".gitignore").write_text("target/\n/state/\n/local/\n/logs/\n/dist/\n/mod/\n__pycache__/\n*.sl2\n")

    terms = private_terms()
    leaks = sorted({str(f.relative_to(OUT)) for f in OUT.rglob("*") if f.is_file()
                    for t in terms if t in f.read_bytes()})
    if leaks:
        raise SystemExit("private data found, fix before publishing: " + ", ".join(leaks))
    files = sum(1 for f in OUT.rglob("*") if f.is_file())
    size = sum(f.stat().st_size for f in OUT.rglob("*") if f.is_file()) / 1e6
    print(f"source: {OUT} ({files} files, {size:.1f} MB), privacy scan clean")

    if args.build:
        env = dict(os.environ, CARGO_TARGET_DIR=str(WORK / "source-check-target"))
        subprocess.run(["cargo", "xwin", "build", "--release", "--offline", "--target", "x86_64-pc-windows-msvc"],
                       cwd=OUT, env=env, check=True)
        dll = WORK / "source-check-target/x86_64-pc-windows-msvc/release/ascension.dll"
        print(f"built from the clean copy: {dll} ({dll.stat().st_size / 1e6:.1f} MB)")


if __name__ == "__main__":
    main()
