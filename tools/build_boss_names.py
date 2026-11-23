#!/usr/bin/env python3
"""Map boss health-bar name ids (NpcName FMG) to Ascension boss tiers.

Input: an uncompressed item message binder (item_dlc02.msgbnd, BND4) from the
user's own game (decompress the DCX with any Kraken tool, e.g. the CS2
conversion's dcxtool under Wine) and config/bosses.json. Output:
config/boss-tiers-by-name.json = {"schema": 1, "tiers": {"<fmg id>": "<tier>"}}.
Only ids and tiers are written, never game text.
Usage: build_boss_names.py <item.bnd>
"""
from __future__ import annotations
import collections
import json
import re
import struct
import sys
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
ORDER = ["minor", "greater", "major", "remembrance"]


def bnd4_files(b):
    assert b[:4] == b"BND4"
    count = struct.unpack_from("<i", b, 0x0C)[0]
    hsize = struct.unpack_from("<q", b, 0x20)[0]
    unicode = b[0x30]
    for i in range(count):
        h = 0x40 + i * hsize
        size = struct.unpack_from("<q", b, h + 8)[0]
        off, _fid, noff = struct.unpack_from("<Iii", b, h + 0x18)
        if unicode:
            e = noff
            while b[e:e + 2] != b"\0\0":
                e += 2
            name = b[noff:e].decode("utf-16-le")
        else:
            name = b[noff:b.index(b"\0", noff)].decode("shift-jis")
        yield name, b[off:off + size]


def fmg_entries(b):
    groups = struct.unpack_from("<i", b, 0x0C)[0]
    str_off = struct.unpack_from("<q", b, 0x18)[0]
    out = {}
    for g in range(groups):
        idx, first, last = struct.unpack_from("<iii", b, 0x28 + g * 0x10)
        for k, sid in enumerate(range(first, last + 1)):
            o = struct.unpack_from("<q", b, str_off + (idx + k) * 8)[0]
            if o:
                e = o
                while b[e:e + 2] != b"\0\0":
                    e += 2
                out[sid] = b[o:e].decode("utf-16-le")
    return out


def main():
    binder = Path(sys.argv[1]).read_bytes()
    names = collections.defaultdict(set)
    for name, data in bnd4_files(binder):
        if "NpcName" in name:
            for key, text in fmg_entries(data).items():
                if text:
                    names[text.strip()].add(key)
    bosses = json.loads((PROJECT / "config/bosses.json").read_text())
    votes = collections.defaultdict(collections.Counter)
    for boss in bosses:
        plain = re.sub(r"\s*\(.*?\)\s*", "", boss["name"]).strip()
        for part in re.split(r"\s*(?:&|\+| and )\s*", plain):
            for key in names.get(part.strip(), ()):
                votes[key][boss["tier"]] += 1
    tiers = {}
    for key, counter in votes.items():
        best = max(counter.items(), key=lambda kv: (kv[1], ORDER.index(kv[0])))
        tiers[str(key)] = best[0]
    out = {"schema": 1, "tiers": dict(sorted(tiers.items(), key=lambda kv: int(kv[0])))}
    (PROJECT / "config/boss-tiers-by-name.json").write_text(json.dumps(out, indent=1) + "\n")
    print(f"{len(tiers)} name ids mapped; tiers {collections.Counter(tiers.values())}")


if __name__ == "__main__":
    main()
