#!/usr/bin/env python3
"""Read-only local Steam/Proton/ER inventory; optional explicit exclusive backup."""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import sys
import uuid

APP_ID = "1245620"
PROJECT = Path(__file__).resolve().parents[1]
WORKSPACE = PROJECT.parent.parent
DEFAULT_BACKUPS = WORKSPACE / "work" / "backups"
PROXY_NAMES = {"dinput8.dll", "winhttp.dll", "version.dll", "dxgi.dll", "d3d11.dll",
               "d3d12.dll", "xinput1_3.dll", "xinput1_4.dll", "dsound.dll", "winmm.dll"}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for data in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(data)
    return digest.hexdigest()


def vdf(path: Path) -> dict:
    """Minimal Valve KeyValues reader; does not evaluate code or decode \n escapes."""
    source = path.read_text(encoding="utf-8-sig", errors="replace")
    tokens = re.findall(r'//[^\n]*|"(?:\\.|[^"\\])*"|[{}]|[^\s{}]+', source)
    tokens = [x for x in tokens if not x.startswith("//")]
    index = 0

    def token_value(value):
        if value.startswith('"'):
            return value[1:-1].replace('\\"', '"').replace('\\\\', '\\')
        return value

    def obj(nested=False):
        nonlocal index
        out = {}
        while index < len(tokens):
            key = tokens[index]
            index += 1
            if key == "}":
                if not nested:
                    raise ValueError(f"Unexpected closing brace: {path}")
                return out
            if key == "{" or index >= len(tokens):
                raise ValueError(f"Invalid KeyValues document: {path}")
            value = tokens[index]
            index += 1
            out[token_value(key)] = obj(True) if value == "{" else token_value(value)
        if nested:
            raise ValueError(f"Missing closing brace: {path}")
        return out

    return obj()


def pe_version(path: Path) -> dict:
    """Read PE fixed version fields (separate file and product version labels)."""
    data = path.read_bytes()
    if len(data) < 64 or data[:2] != b"MZ":
        raise ValueError(f"Not a PE image: {path}")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if pe + 24 > len(data) or data[pe:pe + 4] != b"PE\0\0":
        raise ValueError(f"Invalid PE signature: {path}")
    machine, = struct.unpack_from("<H", data, pe + 4)
    signature = b"\xbd\x04\xef\xfe\x00\x00\x01\x00"
    pos = data.find(signature)
    if pos < 0 or pos + 24 > len(data):
        return {"machine": hex(machine), "file_version": None, "product_version": None}
    fields = struct.unpack_from("<6I", data, pos)

    def version(high, low):
        return ".".join(map(str, (high >> 16, high & 65535, low >> 16, low & 65535)))

    return {"machine": hex(machine), "file_version": version(fields[2], fields[3]),
            "product_version": version(fields[4], fields[5])}


def steam_root(requested=None) -> Path:
    options = [Path(requested).expanduser()] if requested else [
        Path.home() / ".local/share/Steam", Path.home() / ".steam/steam",
        Path.home() / ".var/app/com.valvesoftware.Steam/.local/share/Steam"]
    for option in options:
        if (option / "steamapps/libraryfolders.vdf").is_file():
            return option.resolve()
    raise FileNotFoundError("Steam libraryfolders.vdf not found; pass --steam-root PATH")


def libraries(steam: Path) -> list[Path]:
    raw = vdf(steam / "steamapps/libraryfolders.vdf").get("libraryfolders", {})
    result = [steam]
    for entry in raw.values():
        name = entry.get("path") if isinstance(entry, dict) else entry
        if isinstance(name, str):
            candidate = Path(name).expanduser().resolve()
            if candidate not in result:
                result.append(candidate)
    return result


def locate(steam: Path, requested_game=None):
    installs = []
    for library in libraries(steam):
        manifest = library / f"steamapps/appmanifest_{APP_ID}.acf"
        if manifest.is_file():
            state = vdf(manifest).get("AppState", {})
            game = library / "steamapps/common" / state.get("installdir", "ELDEN RING") / "Game"
            installs.append((library, game.resolve(), state, manifest))
    if requested_game:
        requested = Path(requested_game).expanduser().resolve()
        if requested.name.lower() == "eldenring.exe":
            requested = requested.parent
        if (requested / "Game/eldenring.exe").is_file():
            requested = requested / "Game"
        installs = [x for x in installs if x[1] == requested]
    if len(installs) != 1:
        raise ValueError(f"Expected one Steam ER install; found {len(installs)}. Pass --game-dir PATH")
    library, game, state, manifest = installs[0]
    if not (game / "eldenring.exe").is_file():
        raise FileNotFoundError(f"Missing {game / 'eldenring.exe'}")
    return library, game, state, manifest


def find_me3(requested=None):
    options = [requested] if requested else [shutil.which("me3"), str(Path.home() / ".local/bin/me3")]
    for item in options:
        if item and Path(item).expanduser().is_file():
            return Path(item).expanduser().resolve()
    return None


def find_windows_binaries(requested=None, me3_path=None):
    if requested:
        candidates = [Path(requested).expanduser()]
    else:
        data_home = Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))
        candidates = [data_home / "me3/windows-bin"]
        if me3_path:
            candidates.append(Path(me3_path).parent.parent / "share/me3/windows-bin")
    for candidate in candidates:
        if all((candidate / name).is_file() for name in ("me3-launcher.exe", "me3_mod_host.dll")):
            return {"path": str(candidate.resolve()), "files": {
                name: {"sha256": sha256(candidate / name), **pe_version(candidate / name)}
                for name in ("me3-launcher.exe", "me3_mod_host.dll")}}
    return None


def process_names() -> list[dict]:
    """Check only process executable labels; never dump command lines."""
    found = []
    for entry in Path("/proc").glob("[0-9]*"):
        try:
            comm = (entry / "comm").read_text().strip()
            if comm.lower() in {"eldenring.exe", "start_protected", "start_protected_game.exe"} or comm.lower().startswith("easyanticheat"):
                found.append({"pid": int(entry.name), "name": comm})
                continue
            raw = (entry / "cmdline").read_bytes().split(b"\0")
            candidates = [Path(x.decode(errors="replace").replace("\\", "/")).name.lower() for x in raw if x]
            if any(x.startswith("easyanticheat") and x.endswith(".exe") for x in candidates):
                found.append({"pid": int(entry.name), "name": comm})
        except (OSError, ValueError):
            pass
    return found


def inventory(steam=None, game=None, me3=None, windows_binaries_dir=None) -> dict:
    steam = steam_root(steam)
    library, game, state, manifest = locate(steam, game)
    exe = game / "eldenring.exe"
    prefix_candidates = [lib / f"steamapps/compatdata/{APP_ID}/pfx" for lib in libraries(steam)
                         if (lib / f"steamapps/compatdata/{APP_ID}/pfx").is_dir()]
    prefix = prefix_candidates[0] if prefix_candidates else library / f"steamapps/compatdata/{APP_ID}/pfx"
    save_roots = sorted({root for candidate in prefix_candidates
                         for root in candidate.glob("drive_c/users/*/AppData/Roaming/EldenRing")})
    proxies = sorted(str(p) for p in game.iterdir() if p.name.lower() in PROXY_NAMES)
    mapping = None
    steam_config = steam / "config/config.vdf"
    if steam_config.is_file():
        config = vdf(steam_config).get("InstallConfigStore", {}).get("Software", {}).get("Valve", {}).get("Steam", {})
        mappings = config.get("CompatToolMapping", {})
        mapping = mappings.get(APP_ID, mappings.get("0"))
    me3_path = find_me3(me3)
    me3_version = None
    if me3_path:
        result = subprocess.run([str(me3_path), "--version"], capture_output=True, text=True, timeout=15, check=True)
        me3_version = result.stdout.strip()
    runtimes = []
    for lib in libraries(steam):
        common = lib / "steamapps/common"
        if common.is_dir():
            for item in common.iterdir():
                if (item / "proton").is_file():
                    version = item / "version"
                    runtimes.append({"path": str(item), "proton_script_sha256": sha256(item / "proton"), "version": version.read_text(errors="replace").strip()[:300] if version.is_file() else None})
    compat_dir = steam / "compatibilitytools.d"
    if compat_dir.is_dir():
        for item in compat_dir.iterdir():
            if (item / "proton").is_file():
                version = item / "version"
                runtimes.append({"path": str(item), "proton_script_sha256": sha256(item / "proton"), "version": version.read_text(errors="replace").strip()[:300] if version.is_file() else None})
    return {
        "schema": 1, "captured_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "host": {"os": platform.system(), "machine": platform.machine(), "kernel": platform.release()},
        "steam_root": str(steam), "library": str(library), "game_dir": str(game),
        "steam_manifest": str(manifest), "steam_build_id": state.get("buildid"),
        "steam_account": state.get("LastOwner"),
        "installed_depots": state.get("InstalledDepots", {}),
        "executable": {"path": str(exe), "size": exe.stat().st_size, "sha256": sha256(exe), **pe_version(exe)},
        "regulation": {"path": str(game / "regulation.bin"), "sha256": sha256(game / "regulation.bin")} if (game / "regulation.bin").is_file() else None,
        "dlc_archives_present": all((game / p).is_file() for p in ["DLC.bhd", "DLC.bdt"]),
        "prefix": str(prefix), "prefix_candidates": [str(x) for x in prefix_candidates], "save_roots": [str(x) for x in save_roots],
        "proton_mapping": mapping, "proton_runtimes": runtimes,
        "me3": {"path": str(me3_path), "version": me3_version, "sha256": sha256(me3_path)} if me3_path else None,
        "me3_windows_binaries": find_windows_binaries(windows_binaries_dir, me3_path),
        "proxy_dlls": proxies, "proxy_sha256": {str(p): sha256(Path(p)) for p in proxies}, "running_game_processes": process_names(),
        "note": "Read-only inventory. Archive/depot presence does not prove DLC entitlement or runtime success."
    }


def backup_sources(report: dict) -> list[tuple[str, Path, list[Path]]]:
    result = []
    for number, save_root in enumerate(report["save_roots"]):
        root = Path(save_root)
        result.append((f"eldenring-{number}", root, sorted(root.rglob("*"))))
    me3_root = Path.home() / ".config/me3"
    if me3_root.is_dir():
        result.append(("me3-config", me3_root, sorted(me3_root.rglob("*"))))
    game = Path(report["game_dir"])
    result.append(("game-config", game, sorted(game.glob("*.ini"))))
    # Persisted mod data stays beside the mod, never in the native game folder.
    for name in ("state", "saves", "data"):
        root = PROJECT / name
        if root.is_dir():
            result.append((f"ascension-{name}", root, sorted(root.rglob("*"))))
    return result


def create_backup(report: dict, destination=DEFAULT_BACKUPS) -> dict:
    if process_names():
        raise RuntimeError("Close Elden Ring before backing up saves")
    sources = backup_sources(report)
    if not report["save_roots"]:
        raise RuntimeError("No EldenRing save/config folder found; cannot confirm backup coverage")
    destination = Path(destination).expanduser().resolve()
    for _label, root, _files in sources:
        if destination == root.resolve() or root.resolve() in destination.parents:
            raise ValueError("Backup destination cannot be inside a source directory")
    destination.mkdir(parents=True, exist_ok=True)
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    snapshot = destination / f"ascension-{stamp}-{uuid.uuid4().hex[:12]}"
    snapshot.mkdir(mode=0o700)  # exclusive: no overwrite or reuse
    manifest = {"schema": 1, "created_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
                "steam_build_id": report["steam_build_id"], "exe_sha256": report["executable"]["sha256"], "files": []}
    for label, root, files in sources:
        for source in files:
            if source.is_symlink():
                raise RuntimeError(f"Backup refuses symlink: {source}; incomplete snapshot: {snapshot}")
            if not source.is_file():
                continue
            before = source.stat()
            relative = Path("sources") / label / source.relative_to(root)
            target = snapshot / relative
            target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            digest = hashlib.sha256()
            with source.open("rb") as src, target.open("xb") as dst:
                for data in iter(lambda: src.read(1024 * 1024), b""):
                    dst.write(data)
                    digest.update(data)
                dst.flush()
                os.fsync(dst.fileno())
            after = source.stat()
            if (before.st_size, before.st_mtime_ns, before.st_ino) != (after.st_size, after.st_mtime_ns, after.st_ino):
                raise RuntimeError(f"Source changed while backing up: {source}; incomplete snapshot: {snapshot}")
            checksum = digest.hexdigest()
            if sha256(target) != checksum or sha256(source) != checksum:
                raise RuntimeError(f"Backup checksum mismatch: {source}; incomplete snapshot: {snapshot}")
            manifest["files"].append({"source": str(source), "backup": relative.as_posix(), "size": before.st_size,
                                       "mtime_ns": before.st_mtime_ns, "sha256": checksum})
    with (snapshot / "manifest.json").open("x", encoding="utf-8") as stream:
        json.dump(manifest, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    # Persist directory entries bottom-up before declaring the snapshot complete.
    for directory in sorted([p for p in snapshot.rglob("*") if p.is_dir()] + [snapshot], key=lambda p: len(p.parts), reverse=True):
        descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    with (snapshot / "COMPLETE").open("x") as stream:
        stream.write(sha256(snapshot / "manifest.json") + "\n")
        stream.flush()
        os.fsync(stream.fileno())
    directory_fd = os.open(snapshot, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory_fd)
    finally:
        os.close(directory_fd)
    for directory in (destination, destination.parent):
        descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    return {"snapshot": str(snapshot), "file_count": len(manifest["files"]), "manifest_sha256": sha256(snapshot / "manifest.json")}


def verify_backup(snapshot: Path) -> dict:
    snapshot = Path(snapshot).resolve()
    expected = (snapshot / "COMPLETE").read_text().strip()
    if sha256(snapshot / "manifest.json") != expected:
        raise RuntimeError("Backup manifest checksum mismatch")
    manifest = json.loads((snapshot / "manifest.json").read_text())
    for entry in manifest["files"]:
        path = snapshot / entry["backup"]
        if snapshot not in path.resolve().parents or path.is_symlink():
            raise RuntimeError("Invalid backup path")
        if path.stat().st_size != entry["size"] or sha256(path) != entry["sha256"]:
            raise RuntimeError(f"Backup file corrupted: {entry['backup']}")
    return {"snapshot": str(snapshot), "verified_files": len(manifest["files"])}


def parser():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--steam-root")
    p.add_argument("--game-dir")
    p.add_argument("--me3")
    p.add_argument("--windows-binaries-dir", help="Directory containing the installed ME3 Windows launcher and host DLL")
    p.add_argument("--backup", action="store_true", help="Explicitly create save/config snapshot; never launches game")
    p.add_argument("--backup-root", type=Path, default=DEFAULT_BACKUPS)
    p.add_argument("--verify-backup", type=Path)
    return p


def main():
    args = parser().parse_args()
    try:
        if args.verify_backup:
            result = verify_backup(args.verify_backup)
        else:
            result = inventory(args.steam_root, args.game_dir, args.me3, args.windows_binaries_dir)
            if args.backup:
                result["backup"] = create_backup(result, args.backup_root)
        print(json.dumps(result, ensure_ascii=False, indent=2))
        return 0
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
