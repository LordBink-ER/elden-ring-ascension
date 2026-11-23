"""Pinned project-local WitchyBND tooling. No system installation or game launch."""
from __future__ import annotations
import concurrent.futures
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import shutil
import struct
import subprocess
import termios
import time
import urllib.parse
import urllib.request
import zipfile

WITCHY_VERSION = "3.0.1.0"
WITCHY_URL = "https://github.com/ividyon/WitchyBND/releases/download/3.0.1.0/WitchyBND-3.0.1.0-linux-x64.zip"
WITCHY_ARCHIVE_SHA256 = "8405239acf90f4a4e5821f221175b48be01601648ec61921ce644b8f0469a6f9"
SMITHBOX_REVISION = "6625a5857120b5dfe3b342f1dce5d953754d420a"
SMITHBOX_PREFIX = "src/Smithbox.Data/Assets/PARAM/ER/"


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def fetch(url, destination):
    destination = Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    if not destination.is_file():
        with urllib.request.urlopen(url, timeout=60) as source, destination.open("xb") as target:
            shutil.copyfileobj(source, target)
    return destination


def extract_archive(archive, destination):
    destination = Path(destination).resolve()
    with zipfile.ZipFile(archive) as source:
        for entry in source.infolist():
            target = (destination / entry.filename).resolve()
            if target != destination and destination not in target.parents:
                raise ValueError("Unsafe tooling archive path")
            if (entry.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError("Tool archive contains a symlink")
        source.extractall(destination)


def prepare_tooling(tooling_root: Path):
    """Download exact official release + current supported ER schemas into work/."""
    tooling_root = Path(tooling_root).resolve()
    root = tooling_root / f"witchybnd-{WITCHY_VERSION}"
    archive = fetch(WITCHY_URL, tooling_root / f"WitchyBND-{WITCHY_VERSION}-linux-x64.zip")
    if digest(archive) != WITCHY_ARCHIVE_SHA256:
        raise RuntimeError("Official Witchy archive SHA256 differs from pinned source")
    exe = root / "WitchyBND"
    if not exe.is_file():
        root.mkdir(parents=True, exist_ok=True)
        extract_archive(archive, root)
    exe.chmod(exe.stat().st_mode | 0o100)
    bundled = root / "Assets/Paramdex.zip"
    if bundled.is_file():
        extract_archive(bundled, root / "Assets/Paramdex")
        # Witchy itself otherwise replaces the folder and deletes this archive at first use.
        bundled.unlink()
    cache = tooling_root / f"smithbox-paramdefs-{SMITHBOX_REVISION}"
    tree_file = fetch(f"https://api.github.com/repos/vawser/Smithbox/git/trees/{SMITHBOX_REVISION}?recursive=1", cache / "tree.json")
    tree = json.loads(tree_file.read_text())["tree"]
    definitions = [entry["path"] for entry in tree if entry["path"].startswith(SMITHBOX_PREFIX + "Defs/") and entry["path"].endswith(".xml")]
    if len(definitions) < 100:
        raise RuntimeError("Incomplete official Smithbox ER definition tree")
    paths = definitions + [SMITHBOX_PREFIX + "Upgrader Information.json"]

    def download(path):
        relative = path.removeprefix(SMITHBOX_PREFIX)
        url = f"https://raw.githubusercontent.com/vawser/Smithbox/{SMITHBOX_REVISION}/" + urllib.parse.quote(path)
        local = fetch(url, cache / relative)
        return relative, local

    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        sources = dict(pool.map(download, paths))
    paramdex = root / "Assets/Paramdex/ER"
    for relative, source in sources.items():
        if relative.startswith("Defs/"):
            target = paramdex / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
    information = json.loads(sources["Upgrader Information.json"].read_text())
    maximum = information["MaxVersion"]
    if not re.fullmatch(r"\d+_\d+_\d+_\d+L", maximum):
        raise RuntimeError("Unrecognized upstream regulation-version format")
    (paramdex / "Upgrader").mkdir(parents=True, exist_ok=True)
    (paramdex / "Upgrader/version.txt").write_text(maximum)
    provenance = {"witchybnd_version": WITCHY_VERSION, "archive_url": WITCHY_URL,
                  "archive_sha256": WITCHY_ARCHIVE_SHA256, "smithbox_revision": SMITHBOX_REVISION,
                  "smithbox_max_version": maximum,
                  "definitions": {relative: digest(source) for relative, source in sources.items()}}
    (root / "ascension-tooling-provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    # Complete XML values are intentional. Witchy threshold=0 omits common values without storing defaults.
    (root / "appsettings.override.json").write_text(json.dumps({"Offline": True, "PauseOnError": False,
        "EndDelay": 0, "GitBackup": False, "Parallel": False, "ParamDefaultValueThreshold": 1.01}))
    return exe, provenance


def run_witchy(executable: Path, arguments: list[str], log: Path, config_home: Path):
    """PTY handles console-position queries; no human input, shell, tests or game."""
    master, slave = pty.openpty()
    attributes = termios.tcgetattr(slave)
    attributes[3] &= ~termios.ECHO
    termios.tcsetattr(slave, termios.TCSANOW, attributes)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 150, 0, 0))
    environment = os.environ.copy()
    environment["XDG_CONFIG_HOME"] = str(Path(config_home).resolve())
    environment["TERM"] = "xterm"
    command = [str(Path(executable).resolve()), "--passive", *arguments]
    process = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave, env=environment)
    os.close(slave)
    pending = b""
    started = time.monotonic()
    log.parent.mkdir(parents=True, exist_ok=True)
    try:
        with log.open("xb") as output:
            while True:
                ready, _, _ = select.select([master], [], [], 0.1)
                if ready:
                    try:
                        chunk = os.read(master, 1 << 16)
                    except OSError:
                        break
                    if not chunk:
                        break
                    output.write(chunk)
                    pending += chunk
                    requests = pending.count(b"\x1b[6n")
                    if requests:
                        os.write(master, b"\x1b[1;1R" * requests)
                    pending = pending[-3:]
                elif process.poll() is not None:
                    break
                if time.monotonic() - started > 300:
                    process.terminate()  # only this tool process created above
                    raise RuntimeError(f"Witchy generation timed out; see {log}")
        code = process.wait()
        if code:
            raise RuntimeError(f"Witchy generation exited {code}; see {log}")
    finally:
        os.close(master)
