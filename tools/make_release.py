#!/usr/bin/env python3
"""3.4.1: the public release (Nexus Mods) in one command.

  python3 tools/make_release.py [--deploy]

1. Builds the DLL with every build-machine path rewritten (no home folder or
   user name inside the binary) in work/ascension-release-target.
   --deploy also installs that DLL as dist/ascension.dll (same code).
2. Stages only what the mod reads at run time: the me3 profile, the DLL,
   mod/ (regulation + texts), config/, the card art, plus README, CHANGELOG,
   launchers, LICENSE, third-party licenses and notices. Never state/, local/,
   logs/, saves, sources or tools.
3. Refuses to finish if any staged file contains the home folder, the user
   name or a Steam account id from local/.
4. Writes work/release/ELDEN RING Ascension <v>.zip and copies the zip, the
   mod page text and the images to ~/Pulpit/Ascension-Nexus-<v>/.
No tests, no game launch.
"""
from __future__ import annotations

import argparse
import glob
import hashlib
import json
import os
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
WORK = PROJECT.parents[1] / "work"
TARGET = "x86_64-pc-windows-msvc"
HOME = Path.home()


def version() -> str:
    for line in (PROJECT / "Cargo.toml").read_text().splitlines():
        if line.startswith("version"):
            return line.split('"')[1]
    raise SystemExit("no version in Cargo.toml")


def build(target_dir: Path) -> Path:
    flags = [f"--remap-path-prefix={HOME}/.cargo=/cargo", f"--remap-path-prefix={PROJECT}=/ascension",
             f"--remap-path-prefix={HOME}=/build", "-C", "link-arg=/PDBALTPATH:ascension.pdb"]
    config = "target.%s.rustflags=%s" % (TARGET, json.dumps(flags))
    env = dict(os.environ, CARGO_TARGET_DIR=str(target_dir))
    subprocess.run(["cargo", "xwin", "build", "--release", "--offline", "--target", TARGET, "--config", config],
                   cwd=PROJECT, env=env, check=True)
    return target_dir / TARGET / "release/ascension.dll"


def crate_licenses(dest: Path) -> list[str]:
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--offline", "--format-version", "1", "--filter-platform", TARGET],
        cwd=PROJECT, check=True, capture_output=True, text=True).stdout)
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = meta["resolve"]["root"]
    seen, stack = set(), [root]
    while stack:
        node = stack.pop()
        if node in seen:
            continue
        seen.add(node)
        for dep in nodes[node]["deps"]:
            if any(k["kind"] in (None, "normal") for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    seen.discard(root)
    rows = []
    for pid in sorted(seen, key=lambda i: (packages[i]["name"], packages[i]["version"])):
        p = packages[pid]
        lic = p.get("license") or "see its license file"
        rows.append(f"{p['name']} {p['version']}  -  {lic}" + (f"  -  {p['repository']}" if p.get("repository") else ""))
        folder = Path(p["manifest_path"]).parent
        files = [f for f in folder.iterdir() if f.is_file() and
                 f.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE"))]
        if p["name"] == "imgui-sys":
            files += [Path(f) for f in glob.glob(str(folder / "third-party/imgui-master/imgui/LICENSE.txt"))]
        if files:
            out = dest / f"{p['name']}-{p['version']}"
            out.mkdir(parents=True, exist_ok=True)
            for f in files:
                name = "DearImGui-LICENSE.txt" if "third-party" in str(f) else f.name
                shutil.copy(f, out / name)
    # Standard texts for crates that ship none of their own (their Cargo.toml
    # names MIT and/or Apache-2.0); fromsoftware-rs' own are in licenses/.
    serde = sorted(glob.glob(str(HOME / ".cargo/registry/src/*/serde-1.*")))[-1]
    for name in ("LICENSE-MIT", "LICENSE-APACHE"):
        shutil.copy(Path(serde) / name, dest.parent / f"standard-{name}.txt")
    return rows


NOTICE_HEAD = """ELDEN RING: ASCENSION - third-party notices
==========================================================================

The mod's own code and data are MIT licensed (LICENSE.txt). It contains or
uses the following, each under its own license (full texts in licenses/):

- fromsoftware-rs / eldenring-rs (Vincent Swarte and contributors):
  MIT NON-AI or Apache 2.0 NON-AI, see licenses/fromsoftware-rs-*.txt.
  Its code may not be used to train or improve machine learning models.
- hudhook (Andrea Venuta), MIT; Dear ImGui (Omar Cornut), MIT;
  imgui-rs, MIT or Apache 2.0.
- EROverlay boss data (Soar Qin), MIT, licenses/EROverlay-LICENSE.txt.
- Noto Serif Display, SIL Open Font License 1.1, licenses/NotoSerifDisplay-OFL.txt.
- DejaVu Sans, Bitstream Vera / DejaVu license, licenses/DejaVu-LICENSE.txt.
- Mod Engine 3 (me3) is a separate download and is not included.

Crates compiled into dist/ascension.dll (license files in licenses/crates;
crates without their own file use licenses/standard-LICENSE-*.txt or the
fromsoftware-rs texts):

"""


def privacy_terms() -> list[bytes]:
    terms = {str(HOME), HOME.name}
    context = PROJECT / "local/launch-context.json"
    if context.is_file():
        account = json.loads(context.read_text()).get("steam_account")
        if account:
            terms.add(str(account))
    out = []
    for t in terms:
        if len(t) >= 4:
            out += [t.encode(), t.encode("utf-16-le")]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--deploy", action="store_true", help="also install the clean DLL as dist/ascension.dll")
    ap.add_argument("--no-desktop", action="store_true")
    args = ap.parse_args()
    v = version()
    compat = json.loads((PROJECT / "config/compatibility.json").read_text())
    buildinfo = json.loads((PROJECT / "dist/BUILDINFO.json").read_text())
    # The title screen's "App Ver." for each supported eldenring.exe file version.
    app_versions = {"2.7.2.0": "1.17.2"}
    values = {"{VERSION}": v, "{GAME_VERSION}": compat["product_version"],
              "{APP_VERSION}": app_versions[compat["product_version"]], "{ME3_VERSION}": buildinfo.get("me3_version", "0.13.0")}

    def fill(text: str) -> str:
        for k, val in values.items():
            text = text.replace(k, val)
        return text

    dll = build(WORK / "ascension-release-target")
    if args.deploy:
        shutil.copy(dll, PROJECT / "dist/ascension.dll.new")
        os.replace(PROJECT / "dist/ascension.dll.new", PROJECT / "dist/ascension.dll")
        digest = hashlib.sha256((PROJECT / "dist/ascension.dll").read_bytes()).hexdigest()
        (PROJECT / "dist/SHA256SUMS").write_text(f"{digest}  dist/ascension.dll\n")
        subprocess.run([sys.executable, str(PROJECT / "tools/record_build.py")], cwd=PROJECT, check=True,
                       stdout=subprocess.DEVNULL)
    rel = WORK / "release"
    stage_root = rel / f"ELDEN-RING-ASCENSION-{v}"
    stage = stage_root / "Ascension"
    shutil.rmtree(stage_root, ignore_errors=True)
    (stage / "dist").mkdir(parents=True)
    shutil.copy(PROJECT / "ascension.me3", stage)
    shutil.copy(dll, stage / "dist/ascension.dll")
    digest = hashlib.sha256((stage / "dist/ascension.dll").read_bytes()).hexdigest()
    (stage / "dist/SHA256SUMS").write_text(f"{digest}  ascension.dll\n")
    info = dict(buildinfo, version=v, native_sha256=digest)
    info.pop("verification", None)
    (stage / "dist/BUILDINFO.json").write_text(json.dumps(info, indent=2) + "\n")
    shutil.copytree(PROJECT / "mod", stage / "mod")
    shutil.copytree(PROJECT / "config", stage / "config")
    shutil.copytree(PROJECT / "assets/cards", stage / "assets/cards", ignore=shutil.ignore_patterns("override"))
    shutil.copytree(PROJECT / "assets/icon", stage / "assets/icon")
    for name in ("README.txt", "CHANGELOG.txt"):
        (stage / name).write_text(fill((PROJECT / "release" / name).read_text()))
    shutil.copy(PROJECT / "release/Launch Ascension.bat", stage)
    shutil.copy(PROJECT / "release/launch-ascension-linux.sh", stage)
    shutil.copy(PROJECT / "LICENSE", stage / "LICENSE.txt")
    lic = stage / "licenses"
    lic.mkdir()
    for f in (PROJECT / "docs/third-party").glob("*.txt"):
        shutil.copy(f, lic)
    shutil.copy(PROJECT / "assets/fonts/NotoSerifDisplay-OFL.txt", lic)
    rows = crate_licenses(lic / "crates")
    (stage / "THIRD-PARTY-NOTICES.txt").write_text(NOTICE_HEAD + "\n".join(rows) + "\n")

    # Privacy: nothing of the build machine or the player's account inside.
    terms = privacy_terms()
    leaks = []
    for f in stage.rglob("*"):
        if f.is_file():
            data = f.read_bytes()
            leaks += [f"{f.relative_to(stage)}" for t in terms if t in data]
    if leaks:
        raise SystemExit("private data found, release not written: " + ", ".join(sorted(set(leaks))))

    zip_path = rel / f"ELDEN RING Ascension {v}.zip"
    zip_path.unlink(missing_ok=True)
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for f in sorted(stage.rglob("*")):
            if f.is_file():
                zi = zipfile.ZipInfo.from_file(f, f.relative_to(stage_root).as_posix())
                zi.compress_type = zipfile.ZIP_DEFLATED
                mode = 0o755 if f.suffix == ".sh" else 0o644
                zi.external_attr = (0o100000 | mode) << 16
                with open(f, "rb") as src:
                    z.writestr(zi, src.read())
    files = sum(1 for f in stage.rglob("*") if f.is_file())
    print(f"release: {zip_path} ({zip_path.stat().st_size / 1e6:.1f} MB, {files} files, {len(rows)} crates listed)")

    if not args.no_desktop:
        out = HOME / "Pulpit" / f"Ascension-Nexus-{v}"
        shutil.rmtree(out, ignore_errors=True)
        (out / "images").mkdir(parents=True)
        shutil.copy(zip_path, out)
        (out / "NEXUS-PAGE.txt").write_text(fill((PROJECT / "release/NEXUS-PAGE.txt").read_text()))
        for img in (PROJECT / "release/images").glob("*.png"):
            shutil.copy(img, out / "images")
        print(f"desktop: {out}")


if __name__ == "__main__":
    main()
