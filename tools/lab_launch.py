#!/usr/bin/env python3
"""Review, back up and launch only the verified offline Ascension lab profile."""
from __future__ import annotations

import argparse
import json
import os
import re
from pathlib import Path
import shlex
import subprocess
import sys
import tomllib

from environment_probe import (PROJECT, DEFAULT_BACKUPS, create_backup, inventory,
                               pe_version, process_names, sha256)


def launch_environment(proxies: list[str], allow_builtin: bool, current: dict) -> tuple[dict, list[str]]:
    env = dict(current)
    env["ME3_PROTON_LAUNCH_VERB"] = "waitforexitandrun"
    errors = []
    names = {Path(p).name.lower() for p in proxies}
    supported = {"dinput8.dll", "winhttp.dll"}
    if names - supported:
        errors.append("Unsupported existing proxy DLLs: " + ", ".join(sorted(names - supported)))
    bypassed = names & supported
    if bypassed and not allow_builtin:
        errors.append("Existing proxy DLLs require explicit --builtin-proxies: " + ", ".join(sorted(bypassed)))
    if bypassed and allow_builtin:
        stems = {Path(name).stem.lower() for name in bypassed}
        existing = env.get("WINEDLLOVERRIDES", "")
        # Refuse ambiguous/wildcard/path overrides instead of silently changing prior user choices.
        for clause in existing.split(";"):
            if not clause.strip():
                continue
            if "=" not in clause:
                errors.append("Cannot safely merge malformed WINEDLLOVERRIDES")
                continue
            raw_names, raw_mode = clause.rsplit("=", 1)
            mode = raw_mode.strip().lower()
            for name in raw_names.lower().split(","):
                name = name.strip()
                stem = name.replace("\\", "/").rsplit("/", 1)[-1].removesuffix(".dll").lstrip("*")
                if name == "*" or (stem in stems and (name.startswith("*") or "\\" in name or "/" in name or mode != "b")):
                    errors.append("Conflicting WINEDLLOVERRIDES for " + name + "; review/unset it in this terminal first")
        addition = ",".join(sorted(stems)) + "=b"
        env["WINEDLLOVERRIDES"] = existing.rstrip(";") + (";" if existing.rstrip(";") else "") + addition
    return env, errors


def validate_profile(profile: Path) -> tuple[dict, list[str]]:
    errors = []
    document = tomllib.loads(profile.read_text(encoding="utf-8"))
    if set(document) - {"profileVersion", "savefile", "start_online", "disable_arxan", "supports", "natives", "packages"}:
        errors.append("Unexpected profile properties; review profile before launch")
    expected = {"profileVersion": "v1", "savefile": "ER0000.ascension.sl2", "start_online": False, "disable_arxan": False}
    for key, value in expected.items():
        if document.get(key) != value:
            errors.append(f"Profile must specify {key}={value!r}")
    supports = document.get("supports", [])
    if supports != [{"game": "eldenring"}]:
        errors.append("Profile must support only Elden Ring")
    natives = document.get("natives", [])
    if len(natives) != 1:
        errors.append("Profile must load exactly one Ascension DLL")
    else:
        native = natives[0]
        path = (profile.parent / native.get("path", "")).resolve()
        expected_dll = (PROJECT / "dist/ascension.dll").resolve()
        if path != expected_dll:
            errors.append("Native path must resolve to this project's dist/ascension.dll")
        if native.get("initializer") != {"function": "ascension_init"}:
            errors.append("DLL initializer must be ascension_init")
        if native.get("load_early", False) or not native.get("enabled", True):
            errors.append("Ascension DLL must be enabled and not loaded early")
        if not path.is_file():
            errors.append(f"DLL has not been built: {path}")
        elif pe_version(path)["machine"] != "0x8664":
            errors.append("Ascension DLL must be Windows x64 PE")
    if document.get("packages") != [{"id": "ascension-data", "path": "mod"}]:
        errors.append("Profile must load exactly package ascension-data at the local mod directory")
    return document, errors


MESSAGE_LOCALES = frozenset({
    "engus", "frafr", "deude", "itait", "jpnjp", "korkr", "polpl", "rusru",
    "spaes", "spaar", "zhotw", "zhocn", "thath", "porbr", "araae",
})
MESSAGE_CONTAINER = "item_dlc02.msgbnd.dcx"
SOURCE_ARCHIVE_NAMES = frozenset({"Data0.bhd", "Data1.bhd", "Data2.bhd", "Data3.bhd", "DLC.bhd"})


def valid_sha256(value) -> bool:
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def validate_messages(report: dict, marker: dict, information: dict, errors: list[str]) -> set[Path]:
    """Read an exact generated-message allowlist; never inspect whole BDT data."""
    manifest_path = PROJECT / "config/messages-manifest.json"
    information.update({"messages_manifest_path": str(manifest_path),
                        "messages_manifest_sha256": None, "message_files": [],
                        "source_archive_headers": []})
    allowed = set()
    if not manifest_path.is_file() or manifest_path.is_symlink():
        errors.append("Generated config/messages-manifest.json is missing or is a symlink")
        return allowed
    information["messages_manifest_sha256"] = sha256(manifest_path)
    messages = json.loads(manifest_path.read_text(encoding="utf-8"))
    if not isinstance(messages, dict):
        errors.append("Messages manifest must be a JSON object")
        return allowed
    if type(messages.get("schema")) is not int or messages["schema"] != 1:
        errors.append("Messages manifest schema must be 1")
    executable_hash = messages.get("source_executable_sha256")
    if not valid_sha256(executable_hash) or executable_hash != report.get("executable", {}).get("sha256"):
        errors.append("Messages manifest source executable differs from the installed executable")
    if messages.get("active_container") != MESSAGE_CONTAINER:
        errors.append("Messages manifest must select only item_dlc02.msgbnd.dcx")
    payload = marker.get("payload_goods")
    commit = marker.get("commit_good")
    if (not isinstance(payload, list) or len(payload) != 16
            or any(type(identifier) is not int or not 0 < identifier < 0x800000 for identifier in payload)
            or type(commit) is not int or not 0 < commit < 0x800000
            or len(set(payload + [commit])) != 17):
        errors.append("Identity manifest must declare sixteen distinct payload Goods and one commit Goods")
    elif messages.get("goods_ids") != payload + [commit]:
        errors.append("Messages Goods IDs differ from the identity marker payload/commit IDs")

    archive_headers = messages.get("source_archive_headers")
    declared_archives = set()
    game_dir = Path(report["game_dir"])
    if not isinstance(archive_headers, list) or not 1 <= len(archive_headers) <= len(SOURCE_ARCHIVE_NAMES):
        errors.append("Messages manifest requires a bounded source archive header list")
        archive_headers = []
    for header in archive_headers:
        if (not isinstance(header, dict) or set(header) != {"name", "sha256", "bdt_size"}
                or not isinstance(header.get("name"), str) or header["name"] not in SOURCE_ARCHIVE_NAMES
                or not valid_sha256(header.get("sha256"))
                or type(header.get("bdt_size")) is not int or header["bdt_size"] <= 0):
            errors.append("Messages manifest contains an invalid source archive header")
            continue
        name = header["name"]
        if name in declared_archives:
            errors.append(f"Duplicate messages source archive header: {name}")
            continue
        declared_archives.add(name)
        bhd = game_dir / name
        bdt = bhd.with_suffix(".bdt")
        actual_hash = sha256(bhd) if bhd.is_file() else None
        actual_size = bdt.stat().st_size if bdt.is_file() else None
        if actual_hash != header["sha256"]:
            errors.append(f"Messages source archive header changed: {name}")
        if actual_size != header["bdt_size"]:
            errors.append(f"Messages source archive data size changed: {bdt.name}")
        information["source_archive_headers"].append({
            "name": name, "path": str(bhd), "sha256": actual_hash,
            "bdt_path": str(bdt), "bdt_size": actual_size,
        })

    generated = messages.get("generated_files")
    if not isinstance(generated, list) or not 2 <= len(generated) <= len(MESSAGE_LOCALES):
        errors.append("Messages manifest must declare between two and fifteen generated binders")
        generated = []
    languages = set()
    required_fields = {"path", "sha256", "source_sha256", "source_archive", "language", "localization", "tables"}
    for entry in generated:
        if not isinstance(entry, dict) or set(entry) != required_fields:
            errors.append("Messages manifest generated-file record has unexpected fields")
            continue
        relative = entry.get("path")
        match = re.fullmatch(r"msg/([a-z]{5})/item_dlc02\.msgbnd\.dcx", relative) if isinstance(relative, str) else None
        if match is None or match.group(1) not in MESSAGE_LOCALES:
            errors.append("Messages manifest contains a path outside the native item-text allowlist")
            continue
        language = match.group(1)
        path = PROJECT / "mod" / relative
        if path in allowed or language in languages:
            errors.append(f"Duplicate generated messages binder: {relative}")
            continue
        allowed.add(path)
        languages.add(language)
        localization = "polish" if language == "polpl" else "english" if language == "engus" else "english_fallback"
        if entry.get("language") != language or entry.get("localization") != localization:
            errors.append(f"Messages language/localization metadata is inconsistent: {relative}")
        if not valid_sha256(entry.get("sha256")) or not valid_sha256(entry.get("source_sha256")):
            errors.append(f"Messages binder hashes are invalid: {relative}")
        if not isinstance(entry.get("source_archive"), str) or entry["source_archive"] not in declared_archives:
            errors.append(f"Messages binder has no matching source archive header: {relative}")
        tables = entry.get("tables")
        roles = set()
        table_ids = set()
        if not isinstance(tables, list) or not tables:
            errors.append(f"Messages binder must declare its merged Goods tables: {relative}")
            tables = []
        for table in tables:
            if (not isinstance(table, dict) or set(table) != {"id", "role", "before", "after", "added"}
                    or type(table.get("id")) is not int
                    or table.get("role") not in ("title", "summary", "description")
                    or any(type(table.get(key)) is not int for key in ("before", "after", "added"))
                    or table["before"] < 0 or table["added"] != 17 or table["after"] != table["before"] + 17):
                errors.append(f"Invalid generated Goods text table metadata: {relative}")
                continue
            if table["id"] in table_ids:
                errors.append(f"Duplicate generated Goods text table ID: {relative}")
            table_ids.add(table["id"])
            roles.add(table["role"])
        if roles != {"title", "summary", "description"}:
            errors.append(f"Messages binder must include Goods names, summaries and descriptions: {relative}")
        parents = (PROJECT / "mod", PROJECT / "mod/msg", path.parent)
        if path.is_symlink() or any(parent.is_symlink() for parent in parents):
            errors.append(f"Generated messages binder or parent is a symlink: {relative}")
            actual_hash = None
        else:
            actual_hash = sha256(path) if path.is_file() else None
        if actual_hash != entry.get("sha256") or actual_hash is None:
            errors.append(f"Generated messages binder missing or changed: {relative}")
        information["message_files"].append({
            "path": str(path), "relative_path": relative, "sha256": actual_hash,
            "source_sha256": entry.get("source_sha256"), "source_archive": entry.get("source_archive"),
            "language": language, "localization": entry.get("localization"), "tables": tables,
        })
    if not {"engus", "polpl"}.issubset(languages):
        errors.append("Generated messages must include native English and Polish binders")
    return allowed


def validate_generated_package(report: dict) -> tuple[dict, list[str]]:
    errors = []
    package_dir = PROJECT / "mod"
    package = package_dir / "regulation.bin"
    manifest_path = PROJECT / "config/identity-marker.json"
    information = {"path": str(package), "sha256": None, "manifest_path": str(manifest_path),
                   "manifest_sha256": None, "source_path": None, "source_sha256": None,
                   "messages_manifest_path": str(PROJECT / "config/messages-manifest.json"),
                   "messages_manifest_sha256": None, "message_files": [], "source_archive_headers": []}
    if package_dir.is_symlink() or package.is_symlink():
        errors.append("Generated package directory and regulation must not be symlinks")
    if not package.is_file():
        errors.append("Generated mod/regulation.bin is missing; run tools/build_params.py")
    else:
        information["sha256"] = sha256(package)
    if not manifest_path.is_file() or manifest_path.is_symlink():
        errors.append("Generated config/identity-marker.json is missing or is a symlink")
        return information, errors
    marker = json.loads(manifest_path.read_text(encoding="utf-8"))
    information["manifest_sha256"] = sha256(manifest_path)
    if not isinstance(marker, dict):
        errors.append("Identity marker manifest must be a JSON object")
        return information, errors
    if type(marker.get("schema")) is not int or marker["schema"] != 1:
        errors.append("Identity marker manifest schema must be 1")
    generated_hash = marker.get("generated_regulation_sha256")
    source_hash = marker.get("source_regulation_sha256")
    if not valid_sha256(generated_hash) or generated_hash != information["sha256"]:
        errors.append("Generated regulation hash differs from config/identity-marker.json; rebuild the package")
    source = report.get("regulation")
    if not source or not valid_sha256(source_hash) or source.get("sha256") != source_hash:
        errors.append("Installed source regulation differs from the generated package source; regenerate from the intended source")
    if source:
        information["source_path"] = source["path"]
        information["source_sha256"] = source.get("sha256")
    allowed_files = {package} | validate_messages(report, marker, information, errors)
    allowed_directories = {parent for path in allowed_files for parent in path.parents
                           if parent != package_dir and package_dir in parent.parents}
    if package_dir.is_dir() and not package_dir.is_symlink():
        for path in package_dir.rglob("*"):
            relative = str(path.relative_to(package_dir))
            if path.is_symlink():
                errors.append(f"Generated package contains a symlink: {relative}")
            elif path.is_dir():
                if path not in allowed_directories:
                    errors.append(f"Unexpected generated-package directory: {relative}")
            elif not path.is_file() or path not in allowed_files:
                errors.append(f"Unexpected generated-package file: {relative}")
    return information, errors


def make_plan(args):
    report = inventory(args.steam_root, args.game_dir, args.me3, getattr(args, "windows_binaries_dir", None))
    profile = (PROJECT / "ascension.me3").resolve()
    _document, errors = validate_profile(profile)
    regulation_package, package_errors = validate_generated_package(report)
    errors.extend(package_errors)
    compatibility_file = PROJECT / "config/compatibility.json"
    if not compatibility_file.is_file():
        errors.append("Missing config/compatibility.json; cannot verify game build")
    else:
        compatibility = json.loads(compatibility_file.read_text())
        if report["executable"]["sha256"] not in compatibility.get("sha256", []):
            errors.append("Game SHA256 is unsupported; update/reverify adapter before launch")
        if report["executable"]["product_version"] != compatibility.get("product_version"):
            errors.append("Game ProductVersion is unsupported")
    if not report["me3"]:
        errors.append("ME3 is missing; pass --me3 /path/to/me3")
    elif report["me3"]["version"].split()[-1] != "0.13.0":
        errors.append("ME3 must be the verified version 0.13.0")
    if len(report.get("prefix_candidates", [])) > 1:
        errors.append("Multiple ER Proton prefixes found; determine the intended ME3 prefix before launch")
    windows_bins = report.get("me3_windows_binaries")
    if not windows_bins:
        errors.append("Missing ME3 Windows runtime files; pass --windows-binaries-dir containing me3-launcher.exe and me3_mod_host.dll")
    elif any(info.get("machine") != "0x8664" for info in windows_bins["files"].values()):
        errors.append("ME3 Windows runtime must contain Windows x64 PE files")
    elif any(info.get("product_version") != "0.13.0.0" for info in windows_bins["files"].values()):
        errors.append("ME3 Windows runtime files must match version 0.13.0.0")
    if not report["save_roots"]:
        errors.append("Cannot locate game saves/config for a complete prelaunch backup")
    account = report.get("steam_account") or ""
    account_roots = [Path(root) / account for root in report["save_roots"] if account and (Path(root) / account).is_dir()]
    if not account.isdecimal() or len(account_roots) != 1:
        errors.append("Cannot unambiguously bind the Steam account to the isolated save")
    if report["running_game_processes"]:
        errors.append("Elden Ring or EAC is already running; close it normally before launch")
    if not report["proton_mapping"]:
        errors.append("No explicit Steam Proton mapping found; select the intended Proton runtime in Steam first")
    env, env_errors = launch_environment(report["proxy_dlls"], args.builtin_proxies, os.environ)
    errors.extend(env_errors)
    me3 = report["me3"]["path"] if report["me3"] else "me3"
    command = [me3, "--windows-binaries-dir", windows_bins["path"] if windows_bins else "/missing-me3-windows-runtime",
               "--steam-dir", report["steam_root"], "launch", "--game", "eldenring", "--profile", str(profile),
               "--exe", report["executable"]["path"], "--online", "false", "--disable-arxan", "false",
               "--no-mem-patch", "true"]
    plan = {
        "regulation_package": regulation_package,
        "mode": "offline laboratory", "me3_windows_binaries": windows_bins, "launch_authorized_by_cli": bool(args.launch),
        "game_product_version": report["executable"]["product_version"],
        "steam_build_id": report["steam_build_id"], "exe_sha256": report["executable"]["sha256"],
        "profile": str(profile), "profile_sha256": sha256(profile), "native_dll": str(PROJECT / "dist/ascension.dll"),
        "native_sha256": sha256(PROJECT / "dist/ascension.dll") if (PROJECT / "dist/ascension.dll").is_file() else None,
        "command": command, "command_display": shlex.join(command),
        "environment_changes": {key: value for key, value in env.items() if os.environ.get(key) != value},
        "existing_proxy_dlls": report["proxy_dlls"],
        "proxy_strategy": "builtin-only dinput8/winhttp for this process; no native fallback; existing files untouched" if args.builtin_proxies else "refuse existing proxies",
        "saves": report["save_roots"], "backup_root": str(args.backup_root.resolve()),
        "sidecar_state": str(PROJECT / "state"),
        "launch_context": {"schema": 1, "steam_account": account,
                           "profile_namespace": "ER0000.ascension.sl2",
                           "save_path": "Z:" + str(account_roots[0] / "ER0000.ascension.sl2").replace("/", "\\") if len(account_roots) == 1 else ""},
        "errors": errors,
        "note": "No game, save, profile, EAC or proxy files are renamed, removed or replaced. This tool does not restore backups automatically."
    }
    return report, plan, env


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--steam-root")
    parser.add_argument("--game-dir")
    parser.add_argument("--me3")
    parser.add_argument("--windows-binaries-dir", help="Explicit installed ME3 Windows runtime directory; avoids source-tree target autodetection")
    parser.add_argument("--check", action="store_true", help="Read-only prerequisite validation (default)")
    parser.add_argument("--dry-run", action="store_true", help="Show exact argv/environment; creates no backup and launches nothing")
    parser.add_argument("--backup", action="store_true", help="Create exclusive save/config snapshot")
    parser.add_argument("--backup-root", type=Path, default=DEFAULT_BACKUPS)
    parser.add_argument("--builtin-proxies", action="store_true", help="Use Wine builtins for existing dinput8/winhttp DLL proxies only, without modifying files")
    parser.add_argument("--launch", action="store_true", help="Actually launch ME3 after passing checks and making a fresh backup")
    args = parser.parse_args()
    if args.launch and (args.check or args.dry_run):
        parser.error("--launch cannot be combined with --check or --dry-run")
    if args.launch and not args.backup:
        parser.error("--launch requires --backup for a fresh prelaunch snapshot")
    try:
        report, plan, env = make_plan(args)
        print(json.dumps(plan, ensure_ascii=False, indent=2), flush=True)
        if plan["errors"]:
            return 2
        if args.dry_run or args.check or not (args.backup or args.launch):
            return 0
        backup = create_backup(report, args.backup_root)
        print(json.dumps({"backup": backup}, indent=2), flush=True)
        if args.launch:
            if process_names():
                raise RuntimeError("Elden Ring/EAC appeared after backup; launch cancelled")
            for path, checksum in [(Path(report["executable"]["path"]), plan["exe_sha256"]),
                                   (Path(plan["native_dll"]), plan["native_sha256"]),
                                   (Path(plan["profile"]), plan["profile_sha256"]),
                                   (Path(plan["regulation_package"]["path"]), plan["regulation_package"]["sha256"]),
                                   (Path(plan["regulation_package"]["manifest_path"]), plan["regulation_package"]["manifest_sha256"]),
                                   (Path(plan["regulation_package"]["messages_manifest_path"]), plan["regulation_package"]["messages_manifest_sha256"]),
                                   (Path(plan["regulation_package"]["source_path"]), plan["regulation_package"]["source_sha256"])]:
                if sha256(path) != checksum:
                    raise RuntimeError(f"File changed after validation; review and retry: {path}")
            # Rehash every declared message binder/BHD and restat BDT sizes
            # after backup. The manifest itself was pinned above, so this also
            # rejects files or directories added outside its exact allowlist.
            package_now, package_errors = validate_generated_package(report)
            if package_errors:
                raise RuntimeError("Generated package changed after backup: " + "; ".join(package_errors))
            for field in ("message_files", "source_archive_headers"):
                if package_now[field] != plan["regulation_package"][field]:
                    raise RuntimeError(f"Generated package provenance changed after backup: {field}")
            for name, info in plan["me3_windows_binaries"]["files"].items():
                if sha256(Path(plan["me3_windows_binaries"]["path"]) / name) != info["sha256"]:
                    raise RuntimeError(f"ME3 Windows runtime changed after validation: {name}")
            context_path = PROJECT / "local/launch-context.json"
            context_path.parent.mkdir(parents=True, exist_ok=True)
            context_path.write_text(json.dumps(plan["launch_context"], indent=2) + "\n", encoding="utf-8")
            # subprocess argv only: never interpolate shell text or change game files.
            return subprocess.run(plan["command"], env=env, cwd=PROJECT, check=False).returncode
        return 0
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
