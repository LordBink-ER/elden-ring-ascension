#!/usr/bin/env python3
"""Safety/integrity tests for tooling; never execute ME3 or access real saves."""
import contextlib
import io
import json
import struct
import sys
from types import SimpleNamespace
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import environment_probe as probe
import lab_launch
from lab_launch import launch_environment


class LabSafetyTests(unittest.TestCase):
    def test_keyvalues_escaped_paths_and_nested_depots(self):
        with tempfile.TemporaryDirectory(dir=probe.WORKSPACE / "work") as td:
            path = Path(td) / "manifest.vdf"
            path.write_text('"AppState" { "installdir" "C:\\\\Games\\\\ER" // ignored\n "InstalledDepots" { "2778580" { "manifest" "123" } } }')
            document = probe.vdf(path)["AppState"]
            self.assertEqual(document["installdir"], "C:\\Games\\ER")
            self.assertEqual(document["InstalledDepots"]["2778580"]["manifest"], "123")

    def test_backup_exclusive_verified_and_corruption_detected(self):
        with tempfile.TemporaryDirectory(dir=probe.WORKSPACE / "work") as td:
            base = Path(td)
            source = base / "source"
            source.mkdir()
            save = source / "ER0000.sl2"
            save.write_bytes(bytes(range(256)) * 1000)
            original_hash = probe.sha256(save)
            report = {"save_roots": [str(source)], "steam_build_id": "test", "executable": {"sha256": "test"}}
            with patch.object(probe, "backup_sources", return_value=[("test", source, [save])]), patch.object(probe, "process_names", return_value=[]):
                first = probe.create_backup(report, base / "backups")
                second = probe.create_backup(report, base / "backups")
            self.assertNotEqual(first["snapshot"], second["snapshot"])
            self.assertEqual(probe.verify_backup(Path(first["snapshot"]))["verified_files"], 1)
            self.assertEqual(probe.sha256(save), original_hash)
            manifest = json.loads((Path(first["snapshot"]) / "manifest.json").read_text())
            (Path(first["snapshot"]) / manifest["files"][0]["backup"]).write_bytes(b"corrupt")
            with self.assertRaises(RuntimeError):
                probe.verify_backup(Path(first["snapshot"]))
            self.assertEqual(probe.verify_backup(Path(second["snapshot"]))["verified_files"], 1)

    def test_backup_refuses_running_game(self):
        with patch.object(probe, "process_names", return_value=[{"pid": 7, "name": "eldenring.exe"}]):
            with self.assertRaisesRegex(RuntimeError, "Close Elden Ring"):
                probe.create_backup({}, probe.DEFAULT_BACKUPS)

    def test_proxy_policy_builtin_only_preserves_unrelated(self):
        proxies = ["/game/dinput8.dll", "/game/winhttp.dll"]
        _env, errors = launch_environment(proxies, False, {})
        self.assertTrue(errors)
        env, errors = launch_environment(proxies, True, {"WINEDLLOVERRIDES": "dxgi=n"})
        self.assertEqual(errors, [])
        self.assertEqual(env["WINEDLLOVERRIDES"], "dxgi=n;dinput8,winhttp=b")
        for override in ["dinput8=n,b", "*dinput8=b", "C:\\game\\dinput8=n", "*=n", "invalid"]:
            _env, errors = launch_environment(proxies, True, {"WINEDLLOVERRIDES": override})
            self.assertTrue(errors, override)
        _env, errors = launch_environment(["/game/version.dll"], True, {})
        self.assertTrue(errors)


    def test_backup_symlink_never_marks_complete(self):
        with tempfile.TemporaryDirectory(dir=probe.WORKSPACE / "work") as td:
            base = Path(td)
            source = base / "source"
            source.mkdir()
            outside = base / "outside.sl2"
            outside.write_bytes(b"original")
            link = source / "linked.sl2"
            link.symlink_to(outside)
            report = {"save_roots": [str(source)], "steam_build_id": "test", "executable": {"sha256": "test"}}
            with patch.object(probe, "backup_sources", return_value=[("test", source, [link])]), patch.object(probe, "process_names", return_value=[]):
                with self.assertRaisesRegex(RuntimeError, "symlink"):
                    probe.create_backup(report, base / "backups")
            self.assertEqual(outside.read_bytes(), b"original")
            self.assertEqual(list((base / "backups").rglob("COMPLETE")), [])

    def test_profile_hash_gate_and_dry_run_cannot_launch(self):
        with tempfile.TemporaryDirectory(dir=probe.WORKSPACE / "work") as td:
            root = Path(td)
            (root / "dist").mkdir()
            (root / "config").mkdir()
            data = bytearray(128)
            data[:2] = b"MZ"
            struct.pack_into("<I", data, 0x3C, 64)
            data[64:68] = b"PE\0\0"
            struct.pack_into("<H", data, 68, 0x8664)
            (root / "dist/ascension.dll").write_bytes(data)
            (root / "ascension.me3").write_text((probe.PROJECT / "ascension.me3").read_text())
            (root / "config/compatibility.json").write_text(json.dumps({"product_version": "2.7.2.0", "sha256": ["fixture"]}))
            report = {"executable": {"path": "/fixture with spaces/eldenring.exe", "sha256": "fixture", "product_version": "2.7.2.0"},
                      "steam_build_id": "test", "steam_root": "/fixture/Steam",
                      "me3_windows_binaries": {"path": "/fixture/me3-runtime", "files": {"me3-launcher.exe": {"machine": "0x8664", "product_version": "0.13.0.0"}, "me3_mod_host.dll": {"machine": "0x8664", "product_version": "0.13.0.0"}}},
                      "me3": {"path": "/fixture/me3", "version": "me3 0.13.0"},
                      "save_roots": [str(root)], "running_game_processes": [], "proton_mapping": {"name": "fixture"},
                      "proxy_dlls": [], "prefix_candidates": ["fixture"]}
            args = SimpleNamespace(steam_root=None, game_dir=None, me3=None, launch=False, builtin_proxies=False, backup_root=root / "backups")
            with patch.object(lab_launch, "PROJECT", root), patch.object(lab_launch, "inventory", return_value=report):
                _, plan, _ = lab_launch.make_plan(args)
                self.assertEqual(plan["errors"], [])
                self.assertIn("/fixture with spaces/eldenring.exe", plan["command"])
                self.assertLess(plan["command"].index("--windows-binaries-dir"), plan["command"].index("launch"))
                self.assertIn("/fixture/me3-runtime", plan["command"])
                before = sorted(str(p) for p in root.rglob("*"))
                with patch.object(sys, "argv", ["lab_launch.py", "--dry-run", "--backup"]), patch.object(lab_launch.subprocess, "run", side_effect=AssertionError("must not execute")), patch.object(lab_launch, "create_backup", side_effect=AssertionError("must not write backup")), contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(lab_launch.main(), 0)
                self.assertEqual(before, sorted(str(p) for p in root.rglob("*")))
                report["executable"]["sha256"] = "unsupported"
                _, plan, _ = lab_launch.make_plan(args)
                self.assertTrue(any("SHA256" in error for error in plan["errors"]))

if __name__ == "__main__":
    unittest.main()
