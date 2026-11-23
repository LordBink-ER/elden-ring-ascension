#!/usr/bin/env python3
"""Generate a local Ascension regulation package from the user's owned source.

This command compiles data. It never launches the game, changes installed files,
opens a save, runs a test suite, or round-trips the generated package.
"""
from __future__ import annotations
import argparse
import copy
import datetime as dt
import json
import math
import os
from pathlib import Path
import re
import shutil
import struct
import sys
import uuid
import xml.etree.ElementTree as ET

from environment_probe import PROJECT, WORKSPACE, locate, pe_version, steam_root
from params_tooling import digest, prepare_tooling, run_witchy

SOURCE_REGULATION_SHA256 = "b8cdab84c0772f66811fdc5176a381877ca6657b37d420cc2c1562a90d63870e"
GOODS_BASE = 6_960_000
LOT_BASE = 97_000_000


def write_json(path, document):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".new-" + uuid.uuid4().hex)
    with temporary.open("x", encoding="utf-8") as stream:
        json.dump(document, stream, indent=2, ensure_ascii=False, allow_nan=False)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


class Param:
    def __init__(self, path):
        self.path = Path(path)
        self.tree = ET.parse(path)
        self.root = self.tree.getroot()
        if self.root.findtext("cellStyle") != "0":
            raise ValueError(f"Attribute XML required: {path}")
        self.fields = {f.attrib["name"]: f.attrib for f in self.root.findall("./fields/field")}
        self.row_parent = self.root.find("rows")
        if self.row_parent is None:
            raise ValueError(f"Missing rows in {path}")
        self.rows = {}
        for row in self.row_parent:
            identifier = int(row.attrib["id"])
            if identifier in self.rows:
                raise ValueError(f"Duplicate source row {identifier} in {path}")
            self.rows[identifier] = row

    def clone(self, source_id, target_id, name):
        if target_id in self.rows:
            raise ValueError(f"Row ID collision in {self.path.name}: {target_id}")
        if not 0 < target_id < 2**31:
            raise ValueError(f"Invalid row ID: {target_id}")
        row = copy.deepcopy(self.rows[source_id])
        for key in ("id", "name", "paramdexName"):
            row.attrib.pop(key, None)
        row.set("id", str(target_id))
        row.set("name", name)
        self.rows[target_id] = row
        self.row_parent.append(row)
        return row

    def set(self, row, name, value):
        if name not in self.fields:
            raise ValueError(f"Unknown field {self.path.name}:{name}")
        definition = self.fields[name]
        kind = definition["type"]
        if kind in {"dummy8", "fixstr", "fixstrW"}:
            raise ValueError(f"Manifest cannot edit padding/string field {name}")
        if isinstance(value, bool):
            value = int(value)
        if not isinstance(value, (int, float)) or not math.isfinite(value):
            raise ValueError(f"Nonfinite/nonnumeric field {name}: {value}")
        if kind.startswith(("s", "u")):
            if value != int(value):
                raise ValueError(f"Fractional integer field {name}")
            bits = int(definition.get("bitsize", "-1"))
            if bits < 0:
                bits = int(kind[1:])
            lower = -(1 << (bits - 1)) if kind.startswith("s") else 0
            upper = (1 << (bits - (1 if kind.startswith("s") else 0))) - 1
            if not lower <= value <= upper:
                raise ValueError(f"Field overflow {name}={value}, range {lower}..{upper}")
            encoded = str(int(value))
        elif kind in {"f32", "angle32"}:
            try:
                struct.pack("<f", value)
            except (OverflowError, struct.error):
                raise ValueError(f"Float32 overflow {name}") from None
            encoded = format(float(value), ".9g")
        else:
            raise ValueError(f"Unsupported field type {name}: {kind}")
        row.set(name, encoded)

    def save(self):
        self.row_parent[:] = sorted(self.row_parent, key=lambda row: int(row.attrib["id"]))
        self.tree.write(self.path, encoding="utf-8", xml_declaration=True)


def new_effect(param, definition):
    identifier = int(definition["id"])
    template = int(definition.get("template_id", 350200))  # Blessed Dew: passive, self-only, no triggered graph
    row = param.clone(template, identifier, definition["name"])
    # Rune income is multiplicative. Every owned effect preserves normal
    # rewards unless it explicitly increases them; zero is not neutral.
    param.set(row, "soulRate", 1)
    if "template_id" not in definition:
        # Own perpetual neutral row; all other cells come from a concrete passive vanilla row.
        neutral = {"effectEndurance": -1, "motionInterval": 1, "iconId": -1, "vfxId": -1,
                   "changeHpPoint": 0, "changeHpRate": 0, "changeMpPoint": 0, "changeMpRate": 0,
                   "changeStaminaPoint": 0, "changeStaminaRate": 0, "soulRate": 1,
                   "stateInfo": 0, "spCategory": 0, "applyIdOnGetSoul": 0}
        for name, value in neutral.items():
            param.set(row, name, value)
    for name, value in definition["fields"].items():
        if name == "soulRate" and value < 1:
            raise ValueError(f"Ascension effect {identifier} must preserve base rune income: soulRate >= 1")
        param.set(row, name, value)
    return identifier


DAMAGE_FIELDS = {"physical": "atkPhys", "magic": "atkMag", "fire": "atkFire",
                 "lightning": "atkThun", "holy": "atkDark"}
# Hit behaviour copied from the vanilla attack a bullet used; damage itself always
# comes from the fixed-damage goods template and is rewritten by the DLL.
ATK_COPIED = ("dmgLevel", "atkSuperArmor", "atkStam", "knockbackDist", "atkAttribute",
              "spAttribute", "spEffectId0", "spEffectId1", "spEffectId2", "spEffectId3",
              "spEffectId4", "Material_AttackType", "Material_AttackMaterial")


def cell(param, row, name):
    value = row.attrib.get(name)
    if value is None:
        value = param.fields[name].get("defaultValue", "0")
    return value


def atk_total(param, row):
    return sum(float(cell(param, row, field)) for field in DAMAGE_FIELDS.values())


def build_procs(bullets, atks):
    """Clone each proc's vanilla bullet chain into its own rows (config/procs.json)."""
    spec = json.loads((PROJECT / "config/procs.json").read_text())
    if spec.get("schema") != 1:
        raise ValueError("procs manifest requires schema=1")
    template = int(spec["atk_template"])
    if template not in atks.rows:
        raise ValueError(f"proc AtkParam template {template} missing")
    resolved = []
    ids = set()
    for index, proc in enumerate(spec["procs"]):
        base = int(spec["row_base"]) + index * int(spec["row_stride"])
        order, queue = [], [int(proc["root"])]
        while queue:
            vanilla = queue.pop(0)
            if vanilla <= 0 or vanilla in order:
                continue
            if vanilla not in bullets.rows:
                raise ValueError(f"proc {proc['id']}: bullet {vanilla} missing")
            order.append(vanilla)
            if len(order) > min(48, int(spec["row_stride"])):
                raise ValueError(f"proc {proc['id']}: bullet chain too long")
            for link in ("HitBulletID", "intervalCreateBulletId"):
                child = int(cell(bullets, bullets.rows[vanilla], link))
                if child > 0:
                    queue.append(child)
        mapping = {vanilla: base + i for i, vanilla in enumerate(order)}
        reference = None
        for vanilla in order:
            atk = int(cell(bullets, bullets.rows[vanilla], "atkId_Bullet"))
            if atk > 1 and atk in atks.rows and atk_total(atks, atks.rows[atk]) > 0:
                reference = atk_total(atks, atks.rows[atk])
                break
        shares = proc["element"]
        if not shares or any(kind not in DAMAGE_FIELDS for kind in shares) or abs(sum(shares.values()) - 1.0) > 1e-6:
            raise ValueError(f"proc {proc['id']}: element shares must name damage kinds and sum to 1")
        atk_rows = []
        for i, vanilla in enumerate(order):
            target = mapping[vanilla]
            if target in ids:
                raise ValueError(f"proc row collision {target}")
            ids.add(target)
            source = bullets.rows[vanilla]
            row = bullets.clone(vanilla, target, f"Ascension proc {proc['id']} {i}")
            for link in ("HitBulletID", "intervalCreateBulletId"):
                child = int(cell(bullets, source, link))
                bullets.set(row, link, mapping.get(child, -1) if child > 0 else child)
            bullets.set(row, "spEffectIDForShooter", -1)
            bullets.set(row, "createLimitGroupId", 0)
            bullets.set(row, "isHitBothTeam", 0)
            if i == 0:
                if int(cell(bullets, source, "EmittePosType")) == 5:
                    bullets.set(row, "EmittePosType", 0)
                for name, value in proc.get("overrides", {}).items():
                    bullets.set(row, name, value)
            atk = int(cell(bullets, source, "atkId_Bullet"))
            if atk > 1 and atk in atks.rows:
                vanilla_atk = atks.rows[atk]
                weight = atk_total(atks, vanilla_atk) / reference if reference else 1.0
                clone = atks.clone(template, target, f"Ascension proc {proc['id']} {i}")
                for name in ATK_COPIED:
                    if name in atks.fields:
                        atks.set(clone, name, float(cell(atks, vanilla_atk, name)) if atks.fields[name]["type"] in {"f32", "angle32"} else int(float(cell(atks, vanilla_atk, name))))
                for kind, field in DAMAGE_FIELDS.items():
                    # Fallback damage if the DLL never rewrites the row.
                    atks.set(clone, field, round(100 * weight * shares.get(kind, 0.0)))
                bullets.set(row, "atkId_Bullet", target)
                atk_rows.append([target, round(weight, 4)])
        resolved.append({"id": proc["id"], "root_bullet": mapping[order[0]],
                         "bullets": [mapping[v] for v in order], "atk_rows": atk_rows,
                         "speed": float(cell(bullets, bullets.rows[order[0]], "initVellocity")),
                         "source_chain": order})
        if not atk_rows:
            raise ValueError(f"proc {proc['id']}: chain has no damaging attack")
    return {"schema": 1, "row_base": spec["row_base"], "procs": resolved}


def build_identity(goods, lots):
    payload = [GOODS_BASE + i for i in range(16)]
    commit = GOODS_BASE + 16
    grants = [LOT_BASE + i * 10 for i in range(16)]
    commit_lot = LOT_BASE + 160
    # Reserve each whole ten-ID lot group so an adjacent mod row cannot be granted too.
    lot_collisions = set(lots.rows) & set(range(LOT_BASE, LOT_BASE + 170))
    if lot_collisions:
        raise ValueError(f"Reserved identity ItemLot group collision: {sorted(lot_collisions)}")
    if commit >= 0x800000:
        raise ValueError("Goods IDs must fit native non-indexed 23-bit selector")
    for index, identifier in enumerate(payload + [commit]):
        # Memory Stone is an actual permanent, non-sellable important Goods row.
        row = goods.clone(10030, identifier, f"Ascension identity {'commit v1' if index == 16 else f'byte {index:02d}'}")
        fields = {"maxNum": 1 if index == 16 else 256, "maxRepositoryNum": 0,
                  "sellValue": -1, "basicPrice": 0, "weight": 0,
                  "isOnlyOne": 0, "isRemoveItem_forGameClear": 0,
                  "isDiscard": 0, "isDrop": 0, "isDeposit": 0, "isEquip": 0,
                  "isConsume": 0, "isAutoEquip": 0, "isApplySpecialEffect": 0,
                  "disableMultiDropShare": 1, "enable_live": 0, "enable_gray": 0,
                  "enable_friendly": 0, "enable_hostile": 0, "enable_multi": 0,
                  "disable_offline": 0, "isUseMultiPlayPreparation": 0,
                  "refId_default": -1, "refId_1": -1, "refVirtualWepId": -1,
                  "behaviorId": 0, "replaceItemId": -1, "appearanceReplaceItemId": -1,
                  "yesNoDialogMessageId": -1, "itemGetTutorialFlagId": 0,
                  "showLogCondType": 0, "showDialogCondType": 0,
                  "sortId": 6960000 + index}
        for name, value in fields.items():
            goods.set(row, name, value)
    for identifier, good_id in zip(grants + [commit_lot], payload + [commit]):
        # Golden Seed's map lot establishes Goods category=1, weight=1000, quantity=1.
        row = lots.clone(10280, identifier, f"Ascension identity grant {good_id}")
        for slot in range(1, 9):
            suffix = f"{slot:02d}"
            values = {"lotItemId": good_id if slot == 1 else 0,
                      "lotItemCategory": 1 if slot == 1 else 0,
                      "lotItemBasePoint": 1000 if slot == 1 else 0,
                      "lotItemNum": 1 if slot == 1 else 0,
                      "getItemFlagId": 0, "cumulateLotPoint": 0,
                      "enableLuck": 0, "cumulateReset": 0}
            for prefix, value in values.items():
                lots.set(row, prefix + suffix, value)
        # Explicit native/default GameClearOffset=-1, matching Golden Seed;
        # do not leave the identity protocol dependent on template inheritance.
        for name, value in {"getItemFlagId": 0, "cumulateNumFlagId": 0,
                            "cumulateNumMax": 0, "lotItem_Rarity": 0,
                            "GameClearOffset": -1}.items():
            lots.set(row, name, value)
    return {"schema": 1, "payload_goods": payload, "commit_good": commit,
            "init_item_lots": grants, "commit_item_lot": commit_lot,
            "goods_itemlot_category": 1, "quantity_offset": 1, "commit_quantity": 1,
            "goods_template_id": 10030, "itemlot_template_id": 10280}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-regulation", type=Path)
    parser.add_argument("--game-exe", type=Path)
    parser.add_argument("--source-sha256", default=SOURCE_REGULATION_SHA256,
                        help="Explicit allowed source hash; defaults to the discovered owned install")
    parser.add_argument("--effects", type=Path, default=PROJECT / "config/effects.json")
    parser.add_argument("--witchy", type=Path)
    parser.add_argument("--tooling-root", type=Path, default=WORKSPACE / "work/tooling")
    parser.add_argument("--prepare-tools", action="store_true", help="Fetch pinned official tooling/schemas into work only")
    parser.add_argument("--build-root", type=Path, default=WORKSPACE / "work/params")
    args = parser.parse_args()
    try:
        _library, game, _state, _manifest = locate(steam_root())
        for destination in (args.build_root.resolve(), args.tooling_root.resolve()):
            if destination == game.resolve() or game.resolve() in destination.parents:
                raise ValueError("Build/tooling destinations cannot be inside the installed game directory")
        source = (args.source_regulation or game / "regulation.bin").resolve()
        executable = (args.game_exe or game / "eldenring.exe").resolve()
        compatibility = json.loads((PROJECT / "config/compatibility.json").read_text())
        if digest(executable) not in compatibility["sha256"] or pe_version(executable)["product_version"] != compatibility["product_version"]:
            raise ValueError("Unsupported executable fingerprint; no generation performed")
        source_hash = digest(source)
        if source_hash != args.source_sha256.lower():
            raise ValueError("Source regulation hash differs; inspect source and pass its explicit --source-sha256")
        effects_manifest_hash = digest(args.effects)
        definitions = json.loads(args.effects.read_text())
        if definitions.get("schema") != 1 or not definitions.get("effects"):
            raise ValueError("effects manifest requires schema=1 and nonempty effects")
        effect_ids = [int(row["id"]) for row in definitions["effects"]]
        if len(set(effect_ids)) != len(effect_ids):
            raise ValueError("Duplicate effect IDs in manifest")
        if args.prepare_tools:
            executable_tool, tooling = prepare_tooling(args.tooling_root)
        else:
            executable_tool = args.witchy or args.tooling_root / "witchybnd-3.0.1.0/WitchyBND"
            provenance = executable_tool.parent / "ascension-tooling-provenance.json"
            if not executable_tool.is_file() or not provenance.is_file():
                raise ValueError("Pinned local Witchy/schemas missing; use --prepare-tools")
            tooling = json.loads(provenance.read_text())
        stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        build = args.build_root.resolve() / f"build-{stamp}-{uuid.uuid4().hex[:8]}"
        build.mkdir(parents=True, exist_ok=False)
        staged = build / "regulation.bin"
        with source.open("rb") as src, staged.open("xb") as dst:
            shutil.copyfileobj(src, dst)
        if digest(source) != source_hash or digest(staged) != source_hash:
            raise ValueError("Source changed while copying")
        config_home = args.tooling_root / "witchy-config"
        run_witchy(executable_tool, ["--unpack", str(staged)], build / "01-unpack.log", config_home)
        folder = build / "regulation-bin"
        binder = folder / "_witchy-bnd4.xml"
        if not binder.is_file():
            raise RuntimeError("Witchy did not emit regulation binder; see 01-unpack.log")
        binder_version = int(ET.parse(binder).getroot().findtext("version"))
        maximum = int(tooling["smithbox_max_version"].rstrip("L").replace("_", ""))
        if binder_version > maximum:
            raise ValueError("Source regulation is newer than pinned official schema metadata")
        names = ["SpEffectParam", "EquipParamGoods", "ItemLotParam_map", "Bullet", "AtkParam_Pc"]
        run_witchy(executable_tool, ["--unpack", *[str(folder / (name + ".param")) for name in names]],
                   build / "02-xml.log", config_home)
        params = [Param(folder / (name + ".param.xml")) for name in names]
        effects, goods, lots, bullets, atks = params
        for definition in definitions["effects"]:
            new_effect(effects, definition)
        identity = build_identity(goods, lots)
        proc_rows = build_procs(bullets, atks)
        for param in params:
            param.save()
        param_times = {param.path.with_suffix(""): param.path.with_suffix("").stat().st_mtime_ns for param in params}
        run_witchy(executable_tool, ["--repack", *[str(param.path) for param in params]],
                   build / "03-param-compile.log", config_home)
        for compiled, previous_time in param_times.items():
            if compiled.stat().st_mtime_ns <= previous_time:
                raise RuntimeError(f"PARAM compilation did not emit {compiled.name}; inspect 03-param-compile.log")
        # Binder compilation consumes the three edited PARAMs and all untouched original members.
        before = staged.stat().st_mtime_ns
        run_witchy(executable_tool, ["--repack", str(folder)], build / "04-regulation-compile.log", config_home)
        if staged.stat().st_mtime_ns <= before or digest(staged) == source_hash:
            raise RuntimeError("Regulation compilation did not produce a new package; inspect build logs")
        package_hash = digest(staged)
        if digest(args.effects) != effects_manifest_hash:
            raise ValueError("Effects manifest changed during generation; package not published")
        with staged.open("rb") as candidate:
            if candidate.read(4) in {b"BND4", b"DCX\0"}:
                raise ValueError("Witchy did not encrypt the regulation output")
        identity.update({"source_regulation_sha256": source_hash,
                         "generated_regulation_sha256": package_hash,
                         "source_regulation_version": binder_version})
        output = PROJECT / "mod/regulation.bin"
        output.parent.mkdir(parents=True, exist_ok=True)
        if output.is_symlink():
            raise ValueError("Refusing to replace a symlink package")
        if output.is_file():
            shutil.copyfile(output, build / "previous-ascension-regulation.bin")
        temporary = output.with_name("regulation.bin.new-" + uuid.uuid4().hex)
        with staged.open("rb") as src, temporary.open("xb") as dst:
            shutil.copyfileobj(src, dst)
            dst.flush()
            os.fsync(dst.fileno())
        os.replace(temporary, output)
        write_json(PROJECT / "config/identity-marker.json", identity)
        write_json(PROJECT / "config/procs-rows.json", proc_rows)
        report = {"schema": 1, "created_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
                  "source_path": str(source), "source_sha256": source_hash,
                  "source_regulation_version": binder_version, "game_exe_sha256": digest(executable),
                  "effects_manifest_sha256": effects_manifest_hash, "effect_count": len(effect_ids),
                  "effect_ids": effect_ids, "identity": identity, "tooling": tooling,
                  "output": str(output), "output_sha256": package_hash,
                  "status": "generated; no game test or round-trip performed"}
        write_json(build / "build-manifest.json", report)
        write_json(args.build_root / "manifest-resolved.json", report)
        print(json.dumps({"output": str(output), "sha256": package_hash, "effect_count": len(effect_ids),
                          "identity_goods": 17, "identity_itemlots": 17, "build": str(build), "status": report["status"]}, indent=2))
        return 0
    except (OSError, ValueError, KeyError, RuntimeError, ET.ParseError) as error:
        print(f"PARAM BUILD ERROR: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
