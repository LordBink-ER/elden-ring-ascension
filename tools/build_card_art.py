#!/usr/bin/env python3
"""0.7 "Wygląd": card illustrations for every power, packed into texture atlases.

Each card gets a painterly scene (2:1): a sky in the card's element palette,
light and god rays, distant landscape silhouettes and a central motif chosen
from its name and category (sword, shield, flask, moon, meteor, Erdtree, flame,
lightning, frost crystals, blood drop, rot flower, sleep moth, madness eye ...).
Everything is procedural and seeded by the card id, so a rerun is identical.
An oil-paint look comes from a Kuwahara filter, canvas grain and a vignette.

Hand-made or generated art can replace any card: put assets/cards/override/<id>.png
(any size, 2:1 is best; it is centre-cropped) and rerun.

Output (read by the DLL, src/art.rs):
  assets/cards/atlas-N.qoi   2048 px wide atlases of 320x160 tiles (QOI, RGB)
  assets/cards/index.json    tile size, atlases, card id -> tile, card back tile
Run: python3 tools/build_card_art.py
"""
from __future__ import annotations

import hashlib
import json
import math
import struct
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

PROJECT = Path(__file__).resolve().parents[1]
OUT = PROJECT / "assets/cards"
OVERRIDE = OUT / "override"
TILE_W, TILE_H = 320, 160
ATLAS_W, ATLAS_H = 2048, 2048
COLS, ROWS = ATLAS_W // TILE_W, ATLAS_H // TILE_H
SS = 2
W, H = TILE_W * SS, TILE_H * SS

# --------------------------------------------------------------- palettes ---
# shadow, mid, light, accent (linear-ish RGB 0..1)
PALETTES = {
    "fire": ((0.10, 0.03, 0.02), (0.46, 0.13, 0.04), (1.00, 0.60, 0.22), (1.00, 0.80, 0.45)),
    "frost": ((0.02, 0.05, 0.10), (0.12, 0.30, 0.45), (0.68, 0.90, 1.00), (0.88, 0.98, 1.00)),
    "lightning": ((0.03, 0.03, 0.08), (0.17, 0.19, 0.40), (0.95, 0.92, 0.62), (1.00, 1.00, 0.88)),
    "holy": ((0.08, 0.06, 0.03), (0.42, 0.32, 0.14), (1.00, 0.85, 0.48), (1.00, 0.95, 0.76)),
    "magic": ((0.02, 0.03, 0.09), (0.10, 0.18, 0.42), (0.48, 0.70, 1.00), (0.82, 0.92, 1.00)),
    "blood": ((0.08, 0.01, 0.02), (0.40, 0.04, 0.06), (0.95, 0.24, 0.24), (1.00, 0.58, 0.52)),
    "poison": ((0.03, 0.06, 0.03), (0.20, 0.32, 0.10), (0.64, 0.85, 0.30), (0.86, 1.00, 0.56)),
    "rot": ((0.08, 0.02, 0.02), (0.45, 0.12, 0.08), (0.96, 0.44, 0.25), (1.00, 0.68, 0.48)),
    "madness": ((0.08, 0.06, 0.00), (0.45, 0.32, 0.02), (1.00, 0.84, 0.15), (1.00, 0.96, 0.58)),
    "sleep": ((0.05, 0.03, 0.09), (0.28, 0.18, 0.42), (0.78, 0.62, 0.95), (0.93, 0.86, 1.00)),
    "death": ((0.03, 0.01, 0.05), (0.18, 0.08, 0.25), (0.62, 0.40, 0.86), (0.86, 0.72, 1.00)),
    "physical": ((0.05, 0.05, 0.05), (0.28, 0.26, 0.22), (0.86, 0.78, 0.62), (0.98, 0.92, 0.78)),
    "stamina": ((0.03, 0.06, 0.03), (0.18, 0.30, 0.12), (0.70, 0.88, 0.45), (0.88, 1.00, 0.72)),
    "grace": ((0.07, 0.05, 0.02), (0.36, 0.27, 0.12), (0.95, 0.78, 0.40), (1.00, 0.92, 0.66)),
    "curse": ((0.06, 0.00, 0.01), (0.30, 0.02, 0.05), (0.86, 0.12, 0.18), (1.00, 0.38, 0.36)),
    "night": ((0.02, 0.02, 0.04), (0.10, 0.12, 0.21), (0.56, 0.66, 0.86), (0.86, 0.91, 1.00)),
}

ELEMENT_KEYS = [
    ("rot", ["rot", "aeonia", "scarlet", "zgnil", "pest_", "pestilent"]),
    ("frost", ["frost", "ice", "cold", "zamor", "freez", "frozen", "mróz", "odmroż", "lód", "glacier"]),
    ("lightning", ["lightning", "thunder", "storm", "bolt", "spark", "fortissax", "błyskaw"]),
    ("madness", ["madness", "frenz", "insight", "szaleń"]),
    ("sleep", ["sleep", "dream", "lucid", "uśpi"]),
    ("death", ["death", "rancor", "ghost", "umbral", "black_blade", "destined"]),
    ("blood", ["blood", "bleed", "crimson", "hemorrh", "vein", "sanguin", "krw"]),
    ("poison", ["poison", "venom", "fly", "flies", "trucizn"]),
    ("fire", ["fire", "flame", "pyre", "cinder", "burn", "inferno", "ogn", "płom"]),
    ("magic", ["glint", "magic", "moon", "star", "carian", "sorcer", "crystal", "rennala", "astel",
               "loretta", "adula", "scholar", "cerulean", "memory"]),
    ("holy", ["holy", "gold", "erdtree", "grace", "divine", "faith", "wrath", "elden", "święt", "blessing"]),
    ("stamina", ["stamina", "breath", "green", "endur", "wind", "stride", "calm"]),
]
CATEGORY_ELEMENT = {
    "offense": "physical", "defense": "grace", "recovery": "blood", "endurance": "stamina",
    "magic": "magic", "arcane_arsenal": "magic", "mobility": "night", "exploration": "grace",
    "status": "poison", "elemental": "fire", "economy": "grace", "roguelite": "grace",
    "attribute_synergy": "holy", "cursed": "curse", "reflex": "night", "evolution": "holy",
    "experimental": "magic", "command": "death", "signature": "holy", "keystone": "holy",
    "prismatic": "holy",
}
PROC_ELEMENT = {"magic": "magic", "fire": "fire", "lightning": "lightning", "holy": "holy", "physical": "physical"}

MOTIF_KEYS = [
    ("moon", ["moon", "lunar", "rennala"]),
    ("meteor", ["meteor", "astel", "comet", "quake", "gravity", "crushing"]),
    ("stars", ["star", "constellation", "chorus"]),
    ("tree", ["erdtree", "tree", "root", "sapling", "sovereign_light"]),
    ("shield", ["shield", "guard", "bulwark", "fortress", "aegis", "ward", "wall", "bastion"]),
    ("flask", ["flask", "draught", "vessel", "drought", "reservoir", "tear", "physick", "pulse", "well"]),
    ("cards", ["fate", "thread", "ledger", "dice", "luck", "reroll", "affinity", "destiny", "gambl", "fortune"]),
    ("rune", ["rune", "ember", "trophy", "wealth", "greed", "golden_tithe", "soul", "echo"]),
    ("swarm", ["fly", "flies", "swarm"]),
    ("eye", ["madness", "frenz", "insight", "eye"]),
    ("parry", ["parry", "retort", "deflect", "riposte_tide"]),
    ("dagger", ["backstab", "assassin", "shadow", "knife", "ambush", "kiss"]),
    ("moth", ["dream", "sleep", "lucid", "moth"]),
    ("flower", ["rot", "bloom", "aeonia", "pest", "scarlet"]),
    ("droplet", ["blood", "bleed", "crimson", "hemorrh", "vein", "sanguin", "price"]),
    ("crystal", ["frost", "ice", "cold", "glacier", "zamor", "frozen", "crystal", "icecrag", "shatter"]),
    ("bolt", ["lightning", "thunder", "storm", "bolt", "spark", "fortissax", "capacitor"]),
    ("flame", ["flame", "fire", "pyre", "cinder", "burn", "ember"]),
    ("ghost", ["ghost", "rancor", "death", "spirit", "umbral", "phantom_reprisal"]),
    ("wind", ["wind", "step", "evasion", "footwork", "dodge", "swift", "dash", "feather", "phantom", "breath"]),
    ("helm", ["endur", "iron", "stone", "heavy", "weight", "unyield", "frame", "tenacity"]),
    ("sword", ["blade", "sword", "edge", "strike", "executioner", "headsman", "riposte", "reprisal",
               "counter", "duel", "slash", "fury", "rage"]),
    ("compass", ["explor", "wayfar", "pilgrim", "map", "path", "scout", "seek", "landing"]),
]
CATEGORY_MOTIF = {
    "offense": "crossed", "defense": "shield", "recovery": "flask", "endurance": "helm", "magic": "stars",
    "arcane_arsenal": "circle", "mobility": "wind", "exploration": "compass", "status": "droplet",
    "elemental": "flame", "economy": "rune", "roguelite": "cards", "attribute_synergy": "rings",
    "cursed": "cursed_eye", "reflex": "reflex", "evolution": "tree", "experimental": "gem",
    "command": "ghost", "signature": "tree", "keystone": "rings", "prismatic": "stars",
}
EMISSIVE = {"moon", "meteor", "stars", "tree", "rune", "eye", "moth", "flower", "droplet", "crystal", "bolt",
            "flame", "ghost", "wind", "circle", "rings", "cursed_eye", "gem"}


def seed_of(text: str) -> int:
    return int.from_bytes(hashlib.md5(text.encode()).digest()[:8], "little")


def tokens(text):
    out = []
    word = ""
    for ch in text:
        if ch.isalnum():
            word += ch
        else:
            if word:
                out.append(word)
            word = ""
    if word:
        out.append(word)
    return out


def pick(keys, text):
    """First key with a word that starts a token of the text ("pest" matches
    "pestilent", not "tempest"); words ending in "_" must match a whole token."""
    toks = tokens(text)
    for name, words in keys:
        for w in words:
            if w.endswith("_"):
                if w[:-1] in toks:
                    return name
            elif any(t.startswith(w.replace("_", "")) for t in toks) or ("_" in w and w in text):
                return name
    return None


# ------------------------------------------------------------------ noise ---
def fbm(rng, w, h, base, octaves=5, persistence=0.5):
    total = np.zeros((h, w), np.float32)
    amp, norm, scale = 1.0, 0.0, float(base)
    for _ in range(octaves):
        gw, gh = max(2, int(w / scale) + 2), max(2, int(h / scale) + 2)
        grid = rng.random((gh, gw), dtype=np.float32)
        layer = np.asarray(Image.fromarray(grid).resize((w, h), Image.BICUBIC), dtype=np.float32)
        total += layer * amp
        norm += amp
        amp *= persistence
        scale = max(2.0, scale / 2.0)
    return np.clip(total / norm, 0.0, 1.0)


def fbm1(rng, n, base, octaves=4):
    total = np.zeros(n, np.float32)
    amp, norm, scale = 1.0, 0.0, float(base)
    for _ in range(octaves):
        k = max(2, int(n / scale) + 2)
        pts = rng.random(k).astype(np.float32)
        xs = np.linspace(0, k - 1, n)
        total += np.interp(xs, np.arange(k), pts).astype(np.float32) * amp
        norm += amp
        amp *= 0.5
        scale = max(2.0, scale / 2.0)
    return total / norm


# ----------------------------------------------------------------- shapes ---
def rot(points, angle, cx, cy):
    c, s = math.cos(angle), math.sin(angle)
    return [(cx + x * c - y * s, cy + x * s + y * c) for x, y in points]


def sword_pts(s):
    blade = [(0, -1.15 * s), (0.08 * s, -0.96 * s), (0.08 * s, 0.42 * s), (-0.08 * s, 0.42 * s), (-0.08 * s, -0.96 * s)]
    guard = [(-0.40 * s, 0.40 * s), (-0.34 * s, 0.36 * s), (0.34 * s, 0.36 * s), (0.40 * s, 0.40 * s),
             (0.34 * s, 0.52 * s), (-0.34 * s, 0.52 * s)]
    grip = [(-0.045 * s, 0.52 * s), (0.045 * s, 0.52 * s), (0.045 * s, 0.86 * s), (-0.045 * s, 0.86 * s)]
    return blade, guard, grip


def draw_sword(d, cx, cy, s, angle):
    blade, guard, grip = sword_pts(s)
    for poly in (blade, guard, grip):
        d.polygon(rot(poly, angle, cx, cy), fill=255)
    px, py = rot([(0, 0.92 * s)], angle, cx, cy)[0]
    r = 0.075 * s
    d.ellipse([px - r, py - r, px + r, py + r], fill=255)


def draw_dagger(d, cx, cy, s, angle):
    blade = [(0, -0.75 * s), (0.07 * s, -0.40 * s), (0.06 * s, 0.25 * s), (-0.06 * s, 0.25 * s), (-0.02 * s, -0.55 * s)]
    guard = [(-0.22 * s, 0.24 * s), (0.22 * s, 0.24 * s), (0.18 * s, 0.32 * s), (-0.18 * s, 0.32 * s)]
    grip = [(-0.04 * s, 0.32 * s), (0.04 * s, 0.32 * s), (0.04 * s, 0.62 * s), (-0.04 * s, 0.62 * s)]
    for poly in (blade, guard, grip):
        d.polygon(rot(poly, angle, cx, cy), fill=255)


def flame_poly(cx, cy, s, rng, tongues=5, lean=0.0):
    pts = []
    n = 48
    for i in range(n + 1):
        t = i / n
        a = math.pi * 2 * t
        # teardrop: wide bottom, pointed top
        x = math.sin(a) * 0.5 * s * (1 - 0.35 * math.cos(a))
        y = -math.cos(a) * 0.62 * s
        if y < 0:
            wob = 1 + 0.18 * math.sin(a * tongues + rng.random() * 0.5)
            x *= wob
            y *= 1.0 + 0.55 * (abs(math.sin(a * tongues * 0.5)))
        x += lean * (-y) * 0.4
        pts.append((cx + x, cy + y))
    return pts


def tongue(cx, base_y, width, height, lean, curl=0.0):
    """One flame tongue: wide round base, curved pointed tip."""
    pts = []
    n = 18
    for i in range(n + 1):
        t = i / n
        y = base_y - height * t
        w = width * (1 - t) ** 0.8 * (0.6 + 0.4 * math.sin(math.pi * min(1, t * 1.6)))
        x = cx + lean * height * t * t + curl * height * math.sin(t * math.pi) * 0.15
        pts.append((x - w, y))
    for i in range(n, -1, -1):
        t = i / n
        y = base_y - height * t
        w = width * (1 - t) ** 0.8 * (0.6 + 0.4 * math.sin(math.pi * min(1, t * 1.6)))
        x = cx + lean * height * t * t + curl * height * math.sin(t * math.pi) * 0.15
        pts.append((x + w, y))
    return pts


def draw_flame(d, cx, cy, s, rng, fill=255, scale=1.0):
    base = cy + 0.75 * s * scale
    d.ellipse([cx - 0.5 * s * scale, base - 0.6 * s * scale, cx + 0.5 * s * scale, base + 0.12 * s * scale], fill=fill)
    for k, (dx, h, lean) in enumerate(((0.0, 1.75, 0.05), (-0.28, 1.15, -0.22), (0.28, 1.25, 0.2),
                                       (-0.42, 0.75, -0.35), (0.44, 0.8, 0.3))):
        h = h * (0.9 + rng.random() * 0.2)
        d.polygon(tongue(cx + dx * s * scale, base, 0.26 * s * scale, h * s * scale, lean + (rng.random() - 0.5) * 0.12,
                         curl=(rng.random() - 0.5)), fill=fill)


def star4(cx, cy, r_long, r_short, angle=0.0, points=4):
    pts = []
    for i in range(points * 2):
        a = angle + math.pi * i / points - math.pi / 2
        r = r_long if i % 2 == 0 else r_short
        pts.append((cx + math.cos(a) * r, cy + math.sin(a) * r))
    return pts


def motif_mask(kind, cx, cy, s, rng, element):
    """(body mask, emissive extra mask, aura mask) at W x H."""
    body = Image.new("L", (W, H), 0)
    extra = Image.new("L", (W, H), 0)
    aura = Image.new("L", (W, H), 0)
    d = ImageDraw.Draw(body)
    e = ImageDraw.Draw(extra)
    jitter = (rng.random() - 0.5) * 0.25
    if kind == "sword":
        draw_sword(d, cx, cy, s * 1.2, 0.55 + jitter)
    elif kind == "crossed":
        draw_sword(d, cx, cy, s * 1.15, 0.62 + jitter * 0.5)
        draw_sword(d, cx, cy, s * 1.15, -0.62 + jitter * 0.5)
    elif kind == "dagger":
        side = 1 if rng.random() < 0.5 else -1
        draw_dagger(d, cx - side * 0.15 * s, cy + 0.1 * s, s * 1.7, side * (0.75 + jitter))
        e.ellipse([cx + side * 0.75 * s - 0.16 * s, cy - 0.95 * s, cx + side * 0.75 * s + 0.16 * s, cy - 0.63 * s], fill=230)
        for k in range(3):
            r = s * (1.0 + 0.2 * k)
            e.arc([cx - r, cy - r * 0.7, cx + r, cy + r * 0.7], 200 if side > 0 else 280, 260 if side > 0 else 340,
                  fill=150 - 40 * k, width=max(1, int(0.03 * s)))
    elif kind == "parry":
        # a buckler meeting a blade, with a spark at the contact point
        side = 1 if rng.random() < 0.5 else -1
        bx = cx - side * 0.35 * s
        r = 0.66 * s
        d.ellipse([bx - r, cy - r, bx + r, cy + r], fill=255)
        d.ellipse([bx - r * 0.66, cy - r * 0.66, bx + r * 0.66, cy + r * 0.66], fill=150)
        d.ellipse([bx - r * 0.22, cy - r * 0.22, bx + r * 0.22, cy + r * 0.22], fill=255)
        draw_sword(d, cx + side * 0.55 * s, cy + 0.05 * s, s * 1.1, -side * (0.85 + jitter))
        sx, sy = cx + side * 0.15 * s, cy - 0.45 * s
        e.polygon(star4(sx, sy, 0.75 * s, 0.06 * s, 0.35), fill=255)
        e.polygon(star4(sx, sy, 0.38 * s, 0.05 * s, 0.35 + math.pi / 4), fill=220)
        for k in range(10):
            a = rng.random() * math.tau
            q = s * (0.25 + rng.random() * 0.45)
            e.line([(sx, sy), (sx + math.cos(a) * q, sy + math.sin(a) * q)], fill=170, width=max(1, int(0.02 * s)))
    elif kind == "reflex":
        draw_sword(d, cx, cy, s, 0.75 + jitter)
        for k in range(3):
            r = s * (0.9 + 0.18 * k)
            e.arc([cx - r, cy - r, cx + r, cy + r], 200 + k * 8, 300 - k * 8, fill=170 - 40 * k, width=int(0.04 * s))
        e.polygon(star4(cx - 0.35 * s, cy - 0.45 * s, 0.6 * s, 0.05 * s, 0.2), fill=255)
    elif kind == "shield":
        shape = int(rng.random() * 3)
        if shape == 0:  # heater
            top, mid, tip = -0.85 * s, 0.12 * s, 0.98 * s
            poly = [(-0.72 * s, top), (0.72 * s, top), (0.70 * s, mid)]
            for i in range(1, 12):
                t = i / 12
                poly.append((0.70 * s * (1 - t) ** 1.2, mid + (tip - mid) * math.sin(t * math.pi / 2)))
            poly.append((0, tip))
            for x, y in reversed(poly[3:-1]):
                poly.append((-x, y))
            poly.append((-0.70 * s, mid))
        elif shape == 1:  # round buckler
            poly = [(math.cos(math.tau * i / 48) * 0.85 * s, math.sin(math.tau * i / 48) * 0.85 * s) for i in range(48)]
        else:  # tower
            poly = [(-0.62 * s, -0.55 * s)]
            for i in range(13):
                a = math.pi + math.pi * i / 12
                poly.append((math.cos(a) * 0.62 * s, -0.55 * s + math.sin(a) * 0.45 * s))
            poly += [(0.62 * s, 0.95 * s), (-0.62 * s, 0.95 * s)]
        d.polygon([(cx + x, cy + y) for x, y in poly], fill=255)
        inner = [(cx + x * 0.78, cy + y * 0.78 + 0.02 * s) for x, y in poly]
        d.polygon(inner, fill=150)
        emblem = int(rng.random() * 5)
        ey = cy - 0.02 * s
        if emblem == 0:
            e.polygon(star4(cx, ey, 0.42 * s, 0.10 * s, 0.0, 4), fill=255)
            e.ellipse([cx - 0.12 * s, ey - 0.12 * s, cx + 0.12 * s, ey + 0.12 * s], fill=255)
        elif emblem == 1:  # cross
            e.rectangle([cx - 0.07 * s, ey - 0.45 * s, cx + 0.07 * s, ey + 0.45 * s], fill=255)
            e.rectangle([cx - 0.32 * s, ey - 0.20 * s, cx + 0.32 * s, ey - 0.06 * s], fill=255)
        elif emblem == 2:  # sun
            r = 0.17 * s
            e.ellipse([cx - r, ey - r, cx + r, ey + r], fill=255)
            for k in range(12):
                a = math.tau * k / 12
                e.line([(cx + math.cos(a) * 0.24 * s, ey + math.sin(a) * 0.24 * s),
                        (cx + math.cos(a) * 0.40 * s, ey + math.sin(a) * 0.40 * s)], fill=255, width=max(2, int(0.04 * s)))
        elif emblem == 3:  # crescent
            r = 0.32 * s
            e.ellipse([cx - r, ey - r, cx + r, ey + r], fill=255)
            e.ellipse([cx - r + 0.16 * s, ey - r - 0.06 * s, cx + r + 0.16 * s, ey + r - 0.06 * s], fill=0)
        else:  # little tree
            e.line([(cx, ey + 0.40 * s), (cx, ey - 0.15 * s)], fill=255, width=max(2, int(0.06 * s)))
            for side in (-1, 1):
                e.line([(cx, ey - 0.05 * s), (cx + side * 0.28 * s, ey - 0.38 * s)], fill=255, width=max(2, int(0.04 * s)))
                e.line([(cx, ey + 0.12 * s), (cx + side * 0.22 * s, ey - 0.12 * s)], fill=255, width=max(2, int(0.03 * s)))
            e.ellipse([cx - 0.36 * s, ey - 0.55 * s, cx + 0.36 * s, ey - 0.05 * s], outline=255, width=max(1, int(0.025 * s)))
    elif kind == "flask":
        d.ellipse([cx - 0.56 * s, cy - 0.30 * s, cx + 0.56 * s, cy + 0.82 * s], fill=255)
        d.rectangle([cx - 0.15 * s, cy - 0.62 * s, cx + 0.15 * s, cy - 0.22 * s], fill=255)
        d.rectangle([cx - 0.22 * s, cy - 0.72 * s, cx + 0.22 * s, cy - 0.60 * s], fill=255)
        liquid = Image.new("L", (W, H), 0)
        ImageDraw.Draw(liquid).ellipse([cx - 0.48 * s, cy - 0.22 * s, cx + 0.48 * s, cy + 0.74 * s], fill=255)
        cut = np.asarray(liquid).copy()
        cut[: int(cy + 0.06 * s), :] = 0
        extra = Image.fromarray(np.maximum(np.asarray(extra), cut))
        body = Image.fromarray((np.asarray(body) * 0.55).astype(np.uint8))
    elif kind == "helm":
        d.chord([cx - 0.62 * s, cy - 0.85 * s, cx + 0.62 * s, cy + 0.45 * s], 180, 360, fill=255)
        d.rectangle([cx - 0.62 * s, cy - 0.21 * s, cx + 0.62 * s, cy + 0.62 * s], fill=255)
        d.polygon([(cx - 0.62 * s, cy + 0.62 * s), (cx + 0.62 * s, cy + 0.62 * s), (cx, cy + 0.95 * s)], fill=255)
        d.rectangle([cx - 0.50 * s, cy + 0.02 * s, cx + 0.50 * s, cy + 0.10 * s], fill=0)
        d.rectangle([cx - 0.04 * s, cy + 0.10 * s, cx + 0.04 * s, cy + 0.70 * s], fill=0)
        for side in (-1, 1):
            horn = [(cx + side * 0.50 * s, cy - 0.45 * s), (cx + side * 1.05 * s, cy - 0.95 * s),
                    (cx + side * 0.95 * s, cy - 0.60 * s), (cx + side * 0.60 * s, cy - 0.20 * s)]
            d.polygon(horn, fill=255)
        e.rectangle([cx - 0.46 * s, cy + 0.03 * s, cx + 0.46 * s, cy + 0.09 * s], fill=230)
    elif kind == "stars":
        d.polygon(star4(cx, cy, 0.95 * s, 0.13 * s), fill=255)
        d.polygon(star4(cx, cy, 0.45 * s, 0.10 * s, math.pi / 4), fill=255)
        pts = []
        for _ in range(7):
            a = rng.random() * math.tau
            r = s * (0.9 + rng.random() * 1.1)
            x, y = cx + math.cos(a) * r * 1.5, cy + math.sin(a) * r * 0.7
            pts.append((x, y))
            k = s * (0.10 + rng.random() * 0.14)
            e.polygon(star4(x, y, k, k * 0.2), fill=255)
        for a, b in zip(pts, pts[1:]):
            e.line([a, b], fill=90, width=max(1, int(0.015 * s)))
    elif kind == "moon":
        r = 0.78 * s
        d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=255)
        for _ in range(5):
            a = rng.random() * math.tau
            rr = r * rng.random() * 0.7
            k = r * (0.08 + rng.random() * 0.12)
            x, y = cx + math.cos(a) * rr, cy + math.sin(a) * rr
            d.ellipse([x - k, y - k, x + k, y + k], fill=205)
    elif kind == "meteor":
        r = 0.36 * s
        x0, y0 = cx + 0.35 * s, cy + 0.15 * s
        tail = [(x0 - r * 0.9, y0 - r * 0.4), (x0 - 2.6 * s, y0 - 1.8 * s), (x0 + r * 0.4, y0 - r * 0.9)]
        e.polygon(tail, fill=170)
        d.ellipse([x0 - r, y0 - r, x0 + r, y0 + r], fill=255)
        for _ in range(6):
            a = rng.random() * math.tau
            k = s * (0.04 + rng.random() * 0.06)
            x, y = x0 + math.cos(a) * s * 0.7, y0 + math.sin(a) * s * 0.5
            d.ellipse([x - k, y - k, x + k, y + k], fill=255)
    elif kind == "tree":
        base_y = cy + 1.25 * s

        def branch(x, y, angle, length, width, depth):
            x2, y2 = x + math.cos(angle) * length, y + math.sin(angle) * length
            d.line([(x, y), (x2, y2)], fill=255, width=max(1, int(width)))
            if depth <= 0:
                return
            for side in (-1, 1):
                branch(x2, y2, angle + side * (0.32 + rng.random() * 0.3), length * (0.66 + rng.random() * 0.12),
                       width * 0.68, depth - 1)
            if rng.random() < 0.4:
                branch(x2, y2, angle + (rng.random() - 0.5) * 0.3, length * 0.6, width * 0.6, depth - 1)

        branch(cx, base_y, -math.pi / 2, 0.95 * s, 0.20 * s, 7)
        ImageDraw.Draw(aura).ellipse([cx - 1.5 * s, cy - 1.15 * s, cx + 1.5 * s, cy + 0.25 * s], fill=255)
    elif kind == "rune":
        w = max(2, int(0.08 * s))
        d.line([(cx, cy - 0.85 * s), (cx, cy + 0.95 * s)], fill=255, width=w)
        d.arc([cx - 0.75 * s, cy - 0.55 * s, cx + 0.15 * s, cy + 0.65 * s], 110, 250, fill=255, width=w)
        d.arc([cx - 0.15 * s, cy - 0.55 * s, cx + 0.75 * s, cy + 0.65 * s], 290, 430, fill=255, width=w)
        d.line([(cx - 0.35 * s, cy + 0.80 * s), (cx + 0.35 * s, cy + 0.80 * s)], fill=255, width=w)
        k = 0.13 * s
        d.ellipse([cx - k, cy - 1.05 * s - k, cx + k, cy - 1.05 * s + k], fill=255)
        for _ in range(5):
            a = rng.random() * math.tau
            q = 0.05 * s
            x, y = cx + math.cos(a) * 1.2 * s, cy + math.sin(a) * 0.8 * s
            e.ellipse([x - q, y - q, x + q, y + q], fill=255)
    elif kind == "cards":
        for i, a in enumerate((-0.35, 0.0, 0.35)):
            cw, ch = 0.42 * s, 0.66 * s
            poly = [(-cw, -ch), (cw, -ch), (cw, ch), (-cw, ch)]
            ox = (i - 1) * 0.35 * s
            d.polygon(rot(poly, a, cx + ox, cy + abs(i - 1) * 0.12 * s), fill=255 if i == 1 else 200)
        e.polygon(star4(cx, cy, 0.25 * s, 0.10 * s, 0.0, 4), fill=255)
    elif kind == "wind":
        for k in range(4):
            r = s * (0.45 + 0.28 * k)
            off = (rng.random() - 0.5) * 0.3 * s
            d.arc([cx - r * 1.6 + off, cy - r, cx + r * 1.6 + off, cy + r], 190 + 10 * k, 330 - 8 * k,
                  fill=255, width=max(2, int(0.06 * s * (1 - 0.15 * k))))
        feather = [(cx - 0.9 * s, cy + 0.6 * s), (cx + 0.7 * s, cy - 0.5 * s), (cx + 0.5 * s, cy - 0.2 * s),
                   (cx - 0.5 * s, cy + 0.65 * s)]
        d.polygon(feather, fill=230)
    elif kind == "eye":
        pts = []
        for i in range(41):
            t = -1 + 2 * i / 40
            pts.append((cx + t * 0.95 * s, cy - 0.42 * s * (1 - t * t)))
        for i in range(41):
            t = 1 - 2 * i / 40
            pts.append((cx + t * 0.95 * s, cy + 0.42 * s * (1 - t * t)))
        d.polygon(pts, fill=150)
        r = 0.30 * s
        e.ellipse([cx - r, cy - r, cx + r, cy + r], fill=255)
        d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=255)
        d.ellipse([cx - 0.07 * s, cy - 0.24 * s, cx + 0.07 * s, cy + 0.24 * s], fill=40)
        for k in range(12):
            a = math.tau * k / 12
            e.line([(cx + math.cos(a) * 1.05 * s, cy + math.sin(a) * 0.6 * s),
                    (cx + math.cos(a) * 1.45 * s, cy + math.sin(a) * 0.85 * s)], fill=180, width=int(0.03 * s))
    elif kind == "moth":
        for side in (-1, 1):
            upper = [(cx, cy - 0.15 * s), (cx + side * 0.55 * s, cy - 0.85 * s), (cx + side * 1.15 * s, cy - 0.75 * s),
                     (cx + side * 1.05 * s, cy - 0.25 * s), (cx + side * 0.4 * s, cy + 0.05 * s)]
            lower = [(cx, cy + 0.05 * s), (cx + side * 0.75 * s, cy + 0.15 * s), (cx + side * 0.8 * s, cy + 0.55 * s),
                     (cx + side * 0.35 * s, cy + 0.75 * s), (cx + side * 0.1 * s, cy + 0.35 * s)]
            d.polygon(upper, fill=255)
            d.polygon(lower, fill=225)
            for (ox, oy, k) in ((0.62, -0.45, 0.13), (0.45, 0.38, 0.08)):
                x, y = cx + side * ox * s, cy + oy * s
                d.ellipse([x - k * s, y - k * s, x + k * s, y + k * s], fill=120)
                e.ellipse([x - k * 0.5 * s, y - k * 0.5 * s, x + k * 0.5 * s, y + k * 0.5 * s], fill=255)
        d.ellipse([cx - 0.08 * s, cy - 0.45 * s, cx + 0.08 * s, cy + 0.50 * s], fill=255)
        for _ in range(9):
            x, y = cx + (rng.random() - 0.5) * 3 * s, cy + (rng.random() - 0.5) * 1.6 * s
            k = s * 0.035
            e.ellipse([x - k, y - k, x + k, y + k], fill=230)
    elif kind == "swarm":
        for k in range(70):
            a = k * 0.45 + rng.random() * 0.4
            r = s * (0.15 + 0.012 * k) * (0.8 + rng.random() * 0.4)
            x, y = cx + math.cos(a) * r * 1.5, cy + math.sin(a) * r * 0.8
            q = s * (0.03 + rng.random() * 0.03)
            d.ellipse([x - q, y - q * 0.7, x + q, y + q * 0.7], fill=255)
            e.line([(x - q * 1.8, y - q * 1.2), (x, y)], fill=150, width=max(1, int(q * 0.7)))
            e.line([(x + q * 1.8, y - q * 1.2), (x, y)], fill=150, width=max(1, int(q * 0.7)))
    elif kind == "flower":
        petals = 6 + int(rng.random() * 3)
        for k in range(petals):
            a = 360 * k / petals
            petal = Image.new("L", (W, H), 0)
            ImageDraw.Draw(petal).ellipse([cx - 0.22 * s, cy - 0.95 * s, cx + 0.22 * s, cy], fill=255)
            petal = petal.rotate(a, center=(cx, cy))
            body = Image.fromarray(np.maximum(np.asarray(body), np.asarray(petal)))
        d = ImageDraw.Draw(body)
        r = 0.2 * s
        e.ellipse([cx - r, cy - r, cx + r, cy + r], fill=255)
    elif kind == "droplet":
        def drop(x, y, k):
            d.ellipse([x - 0.5 * k, y - 0.1 * k, x + 0.5 * k, y + 0.9 * k], fill=255)
            d.polygon([(x - 0.47 * k, y + 0.25 * k), (x, y - 0.75 * k), (x + 0.47 * k, y + 0.25 * k)], fill=255)
        drop(cx, cy - 0.2 * s, 1.15 * s)
        drop(cx - 0.9 * s, cy + 0.35 * s, 0.4 * s)
        drop(cx + 0.8 * s, cy + 0.5 * s, 0.3 * s)
    elif kind == "crystal":
        for k in range(5):
            a = -math.pi / 2 + (k - 2) * 0.38 + (rng.random() - 0.5) * 0.15
            length = s * (1.25 - abs(k - 2) * 0.22)
            w = s * 0.16
            shard = [(-w, 0), (-w * 0.8, -length * 0.8), (0, -length), (w * 0.8, -length * 0.8), (w, 0), (0, w * 0.6)]
            d.polygon(rot(shard, a + math.pi / 2, cx, cy + 0.75 * s), fill=255 if k % 2 == 0 else 210)
    elif kind == "bolt":
        x, y = cx + 0.2 * s, cy - 1.3 * s
        pts_l, pts_r = [], []
        for i in range(7):
            w = 0.12 * s * (1 - i / 8)
            pts_l.append((x - w, y))
            pts_r.append((x + w, y))
            x += (rng.random() - 0.5) * 0.7 * s + (-0.25 * s if i % 2 == 0 else 0.25 * s)
            y += 0.38 * s
        d.polygon(pts_l + [(x, y)] + list(reversed(pts_r)), fill=255)
        for _ in range(3):
            bx, by = pts_l[int(rng.random() * 5) + 1]
            e.line([(bx, by), (bx + (rng.random() - 0.5) * s, by + 0.5 * s)], fill=200, width=int(0.03 * s))
    elif kind == "flame":
        draw_flame(d, cx, cy, s, rng, fill=210)
        draw_flame(e, cx, cy + 0.12 * s, s, rng, fill=255, scale=0.55)
    elif kind == "ghost":
        pts = []
        for i in range(25):
            a = math.pi + math.pi * i / 24
            pts.append((cx + math.cos(a) * 0.55 * s, cy - 0.35 * s + math.sin(a) * 0.62 * s))
        hem = 1.05 * s
        for i in range(13):
            t = i / 12
            x = cx + 0.55 * s - t * 1.1 * s + 0.25 * s * math.sin(t * math.pi) * (rng.random() - 0.3)
            y = cy + hem * (0.75 + 0.25 * (i % 2)) + (rng.random() - 0.5) * 0.1 * s
            pts.append((x + 0.35 * s * t * t, y))
        d.polygon(pts, fill=220)
        for side in (-1, 1):
            ex = cx + side * 0.2 * s
            d.ellipse([ex - 0.09 * s, cy - 0.40 * s, ex + 0.09 * s, cy - 0.18 * s], fill=25)
            e.ellipse([ex - 0.05 * s, cy - 0.34 * s, ex + 0.05 * s, cy - 0.24 * s], fill=255)
        for side in (-1, 1):
            d.polygon(tongue(cx + side * 0.95 * s, cy + 0.9 * s, 0.14 * s, 0.9 * s, side * 0.3), fill=150)
    elif kind == "compass":
        d.polygon(star4(cx, cy, 1.05 * s, 0.14 * s, 0.0, 4), fill=255)
        d.polygon(star4(cx, cy, 0.55 * s, 0.10 * s, math.pi / 4, 4), fill=200)
        r = 0.85 * s
        d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=200, width=int(0.04 * s))
        e.polygon(flame_poly(cx, cy - 0.05 * s, 0.35 * s, rng), fill=255)
    elif kind == "circle":
        for r, wdt in ((1.0, 0.05), (0.82, 0.03)):
            d.ellipse([cx - r * s, cy - r * s, cx + r * s, cy + r * s], outline=255, width=max(2, int(wdt * s)))
        for k in range(24):
            a = math.tau * k / 24
            r1, r2 = 0.84 * s, (0.98 if k % 2 else 0.92) * s
            d.line([(cx + math.cos(a) * r1, cy + math.sin(a) * r1), (cx + math.cos(a) * r2, cy + math.sin(a) * r2)],
                   fill=255, width=max(1, int(0.02 * s)))
        for off in (0, math.pi):
            tri = [(cx + math.cos(off + math.tau * i / 3 - math.pi / 2) * 0.7 * s,
                    cy + math.sin(off + math.tau * i / 3 - math.pi / 2) * 0.7 * s) for i in range(3)]
            d.polygon(tri, outline=255, width=max(2, int(0.03 * s)))
        r = 0.2 * s
        e.ellipse([cx - r, cy - r, cx + r, cy + r], fill=255)
    elif kind == "rings":
        for dx in (-0.38, 0.38):
            r = 0.62 * s
            d.ellipse([cx + dx * s - r, cy - r, cx + dx * s + r, cy + r], outline=255, width=int(0.11 * s))
    elif kind == "cursed_eye":
        r = 0.95 * s
        d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=230, width=int(0.07 * s))
        pts = []
        for i in range(41):
            t = -1 + 2 * i / 40
            pts.append((cx + t * 0.7 * s, cy - 0.32 * s * (1 - t * t)))
        for i in range(41):
            t = 1 - 2 * i / 40
            pts.append((cx + t * 0.7 * s, cy + 0.32 * s * (1 - t * t)))
        d.polygon(pts, fill=160)
        k = 0.22 * s
        e.ellipse([cx - k, cy - k, cx + k, cy + k], fill=255)
        x, y = cx + 0.1 * s, cy - 1.0 * s
        for _ in range(5):
            nx, ny = x + (rng.random() - 0.5) * 0.5 * s, y + 0.4 * s
            e.line([(x, y), (nx, ny)], fill=220, width=int(0.035 * s))
            x, y = nx, ny
    else:  # gem
        hexagon = [(cx + math.cos(math.tau * i / 6) * 0.8 * s, cy + math.sin(math.tau * i / 6) * 0.8 * s) for i in range(6)]
        d.polygon(hexagon, fill=255)
        for x, y in hexagon:
            e.line([(cx, cy), (x, y)], fill=120, width=max(1, int(0.02 * s)))
    return body, extra, aura


# ------------------------------------------------------------- landscape ---
def landscape(rng, horizon, kind):
    """(far mask, near mask) float 0..1."""
    xs = np.arange(W)
    far = np.zeros((H, W), np.float32)
    near = np.zeros((H, W), np.float32)
    ys = np.arange(H)[:, None]
    hill_far = horizon - (0.04 + 0.10 * fbm1(rng, W, W / 3)) * H
    far = (ys >= hill_far[None, :]).astype(np.float32)
    if kind in ("ruins", "castle"):
        img = Image.fromarray((far * 255).astype(np.uint8))
        d = ImageDraw.Draw(img)
        for _ in range(int(3 + rng.random() * 4)):
            x = rng.random() * W
            w = W * (0.008 + rng.random() * 0.02)
            top = horizon - H * (0.12 + rng.random() * (0.35 if kind == "castle" else 0.18))
            d.rectangle([x - w, top, x + w, horizon], fill=255)
            if kind == "castle":
                d.polygon([(x - w * 1.6, top), (x, top - H * 0.08), (x + w * 1.6, top)], fill=255)
        far = np.asarray(img, np.float32) / 255
    near_line = horizon + (0.06 + 0.14 * fbm1(rng, W, W / 2)) * H - 0.10 * H
    near = (ys >= near_line[None, :]).astype(np.float32)
    if kind == "trees":
        img = Image.fromarray((near * 255).astype(np.uint8))
        d = ImageDraw.Draw(img)
        for _ in range(int(4 + rng.random() * 5)):
            x = rng.random() * W
            base = near_line[int(min(W - 1, max(0, x)))]
            h = H * (0.08 + rng.random() * 0.16)
            d.polygon([(x - h * 0.25, base + 2), (x, base - h), (x + h * 0.25, base + 2)], fill=255)
        near = np.asarray(img, np.float32) / 255
    far = np.asarray(Image.fromarray((far * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(1.2)), np.float32) / 255
    near = np.asarray(Image.fromarray((near * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(0.8)), np.float32) / 255
    return far, near


# --------------------------------------------------------------- painting ---
def kuwahara(img, r):
    h, w, _ = img.shape
    lum = img.mean(axis=2)
    n = float((r + 1) ** 2)

    def box(a):
        p = np.pad(a, ((r, r), (r, r)), mode="edge")
        s = np.zeros((p.shape[0] + 1, p.shape[1] + 1), np.float64)
        s[1:, 1:] = p.cumsum(0).cumsum(1)
        k = r + 1
        return (s[k:, k:] - s[:-k, k:] - s[k:, :-k] + s[:-k, :-k]) / n

    lm = box(lum)
    lv = box(lum * lum) - lm * lm
    quads = [(0, 0), (0, r), (r, 0), (r, r)]
    variances = np.stack([lv[dy:dy + h, dx:dx + w] for dy, dx in quads])
    best = variances.argmin(axis=0)
    out = np.zeros_like(img)
    for c in range(3):
        m = box(img[:, :, c])
        means = np.stack([m[dy:dy + h, dx:dx + w] for dy, dx in quads])
        out[:, :, c] = np.take_along_axis(means, best[None], axis=0)[0]
    return out


def blur(mask, radius):
    img = Image.fromarray(np.clip(mask * 255, 0, 255).astype(np.uint8))
    return np.asarray(img.filter(ImageFilter.GaussianBlur(radius)), np.float32) / 255


# 3.0 Keystones: an explicit look each (element, motif).
KEYSTONE_LOOK = {
    "key_colossus_grip": ("physical", "helm"), "key_wrath_unbound": ("fire", "sword"),
    "key_flurry_covenant": ("stamina", "crossed"), "key_windborne": ("stamina", "wind"),
    "key_hexblade_oath": ("frost", "sword"), "key_rot_harvest": ("rot", "flower"),
    "key_arcane_constellation": ("magic", "stars"), "key_spellblade_echo": ("magic", "moon"),
    "key_living_fortress": ("grace", "helm"), "key_aegis_oath": ("holy", "shield"),
    "key_duelists_edge": ("night", "sword"), "key_phantom_veil": ("night", "wind"),
    "key_sanguine_covenant": ("blood", "droplet"), "key_crimson_frenzy": ("blood", "flame"),
    "key_golden_wager": ("holy", "rune"), "key_fates_hand": ("grace", "cards"),
    "key_warlords_banner": ("death", "ghost"), "key_shared_fate": ("death", "tree"),
    # 3.2: blows on a foe getting up.
    "no_quarter": ("blood", "crossed"),
    # 3.4 Prismatic (the rainbow is painted on top).
    "prism_elden_overflow": ("holy", "sword"), "prism_tireless_titan": ("stamina", "wind"),
    "prism_chaos_sovereign": ("madness", "eye"), "prism_starfall_apocalypse": ("magic", "meteor"),
    "prism_deathless": ("holy", "tree"), "prism_ascendant_form": ("grace", "rune"),
    "prism_phantom_sovereign": ("lightning", "bolt"), "prism_endless_vigor": ("blood", "droplet"),
    "prism_golden_apotheosis": ("holy", "cards"), "prism_lord_of_the_legion": ("death", "ghost"),
}


def paint(power, procs):
    pid = power["id"]
    rng = np.random.default_rng(seed_of(pid))
    text_id = (pid + " " + power["name"]).lower()
    text_all = text_id + " " + power.get("description", "").lower()
    category = power["category"]
    element = None
    if category == "cursed":
        element = "curse"
    element = element or pick(ELEMENT_KEYS, text_id)
    proc_id = (power["levels"][0].get("proc") or {}).get("proc")
    if not element and proc_id and proc_id in procs:
        el = max(procs[proc_id].get("element", {}).items(), key=lambda kv: kv[1], default=(None, 0))[0]
        element = PROC_ELEMENT.get(el)
    element = element or pick(ELEMENT_KEYS, text_all) or CATEGORY_ELEMENT.get(category, "grace")
    motif = pick(MOTIF_KEYS, text_id)
    if category in ("cursed",) and motif is None:
        motif = "cursed_eye"
    if motif is None and proc_id:
        motif = {"meteorite": "meteor", "astel_meteor": "meteor", "full_moon": "moon", "elden_stars": "stars",
                 "ruin_stars": "stars", "fly_swarm": "moth", "pest_threads": "flower", "loretta_greatbow": "stars",
                 "black_blade": "sword", "adula_moonblade": "moon",
                 # 2.0 signature cards: the boss's spell decides the picture.
                 "wrath_of_gold": "sword", "catch_flame": "flame", "glintstone_bolt": "stars", "rancor_spirits": "ghost",
                 "black_flame": "flame", "rejection": "shield", "fell_pyre": "flame", "zamor_frost": "crystal",
                 "placidusax_ruin": "bolt", "scarlet_aeonia": "flower", "bloodboon": "droplet",
                 "fortissax_spear": "bolt", "frozen_lightning": "bolt", "ghostflame_burst": "ghost",
                 "frenzied_burst": "eye", "ancient_lightning": "bolt"}.get(proc_id)
    motif = motif or CATEGORY_MOTIF.get(category, "gem")
    if pid in KEYSTONE_LOOK:
        element, motif = KEYSTONE_LOOK[pid]
    shadow, mid, light, accent = (np.array(c, np.float32) for c in PALETTES[element])
    # time of day / mood jitter
    shift = (rng.random(3).astype(np.float32) - 0.5) * 0.06
    mid = np.clip(mid + shift, 0, 1)

    yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
    v = yy / H
    focus_x = W * (0.38 + rng.random() * 0.24)
    focus_y = H * (0.40 + rng.random() * 0.10)
    horizon = H * (0.66 + rng.random() * 0.12)

    # Sky gradient and clouds.
    t = np.clip(v / (horizon / H), 0, 1)[..., None]
    sky = shadow * 0.9 * (1 - t) + (mid * 0.85 + light * 0.18) * t
    clouds = fbm(rng, W, H, 160, 5)
    streak = fbm(rng, W, H // 4 + 1, 90, 3)
    streak = np.asarray(Image.fromarray(streak).resize((W, H), Image.BICUBIC), np.float32)
    cloud_a = np.clip((clouds * 0.7 + streak * 0.5 - 0.55) * 2.2, 0, 1) * (1 - t[..., 0] * 0.4)
    cloud_col = mid * 0.6 + light * 0.25
    img = sky * (1 - cloud_a[..., None] * 0.7) + cloud_col * cloud_a[..., None] * 0.7

    # Light: soft glow and god rays around the focus.
    dx, dy = xx - focus_x, yy - focus_y
    dist = np.sqrt(dx * dx + dy * dy) / H
    glow = np.exp(-(dist ** 2) / (2 * 0.28 ** 2))
    theta = np.arctan2(dy, dx)
    ray_noise = fbm1(rng, 720, 18, 3)
    idx = ((theta + math.pi) / math.tau * 719).astype(np.int32)
    rays = np.clip(ray_noise[idx] * 1.6 - 0.55, 0, 1) ** 2 * np.exp(-dist * 2.2)
    strength = 1.35 if category in ("evolution", "signature", "keystone", "prismatic") else 1.0
    img = img + accent * (glow[..., None] * 0.55 + rays[..., None] * 0.35) * strength

    # Landscape.
    kind = ["hills", "ruins", "castle", "trees", "hills"][int(rng.random() * 5)]
    far, near = landscape(rng, horizon, kind)
    fog = np.clip(1 - (yy - horizon) / (H * 0.4), 0, 1)
    far_col = mid * 0.45 + light * 0.10
    near_col = shadow * 0.55
    img = img * (1 - far[..., None]) + (far_col * (0.7 + 0.3 * fog[..., None])) * far[..., None]
    img = img * (1 - near[..., None]) + near_col * near[..., None]

    # The background gets the full oil-paint pass; the motif a light one.
    img = np.clip(img, 0, None)
    img = kuwahara(np.clip(img * (1 + img / 4.0) / (1 + img), 0, 1).astype(np.float32), 3)

    # Motif.
    s = H * (0.30 + rng.random() * 0.05)
    cx, cy = focus_x, focus_y + H * 0.04
    body, extra, aura = motif_mask(motif, cx, cy, s, rng, element)
    if motif in ("shield", "cards", "helm", "rune", "crystal", "droplet", "gem", "crossed", "eye"):
        tilt = (rng.random() - 0.5) * 16
        body, extra = (m.rotate(tilt, center=(cx, cy), resample=Image.BICUBIC) for m in (body, extra))
    body = np.asarray(body, np.float32) / 255
    extra = np.asarray(extra, np.float32) / 255
    aura = np.asarray(aura, np.float32) / 255
    if aura.any():
        img = img + accent * blur(aura, 30)[..., None] * 0.75
    body = blur(body, 0.8)
    emissive = motif in EMISSIVE
    halo = blur(np.maximum(body, extra), 22) * (1.0 if emissive else 0.55)
    if category in ("evolution", "signature", "keystone", "prismatic"):
        ring = Image.new("L", (W, H), 0)
        r = s * 1.45
        ImageDraw.Draw(ring).ellipse([cx - r, cy - r, cx + r, cy + r], outline=255, width=int(0.05 * s))
        halo = halo + blur(np.asarray(ring, np.float32) / 255, 6) * 0.9
    img = img + accent * halo[..., None] * 0.45
    texture = fbm(rng, W, H, 40, 4)
    shade = np.clip(1.0 - (yy - (cy - s)) / (2.4 * s), 0.25, 1.0)
    if emissive:
        core = light * (0.75 + 0.35 * texture[..., None]) + accent * 0.35 * shade[..., None]
    else:
        metal = np.array([0.30, 0.29, 0.27], np.float32) * (0.6 + 0.6 * texture[..., None])
        core = metal * (0.55 + 0.75 * shade[..., None]) + mid * 0.25
    img = img * (1 - body[..., None]) + core * body[..., None]
    edge = np.clip(body - blur(body, 2.5), 0, 1)
    img = img + accent * edge[..., None] * (1.2 if not emissive else 0.6)
    if motif == "flask":
        liquid = blur(extra, 1.0)[..., None]
        fluid = light * (0.85 + 0.3 * texture[..., None]) + mid * 0.2
        img = img * (1 - liquid) + fluid * liquid + light * blur(extra, 12)[..., None] * 0.5
    else:
        img = img + (accent * 0.8 + light * 0.35) * blur(extra, 1.2)[..., None] + accent * blur(extra, 9)[..., None] * 0.6

    # Particles: embers rise, snow falls, stars glitter.
    pm = Image.new("L", (W, H), 0)
    pd = ImageDraw.Draw(pm)
    count = {"fire": 60, "frost": 70, "magic": 45, "night": 45, "holy": 40, "lightning": 25, "madness": 40}.get(element, 25)
    for _ in range(count):
        x = rng.random() * W
        y = rng.random() * H * (0.7 if element in ("magic", "night") else 1.0)
        k = 0.6 + rng.random() * 2.2 * SS
        pd.ellipse([x - k, y - k, x + k, y + k], fill=int(120 + rng.random() * 135))
    particles = blur(np.asarray(pm, np.float32) / 255, 1.0)
    img = img + light * particles[..., None] * 0.9

    # Tone map, grade, light paint pass.
    img = np.clip(img, 0, None)
    img = np.minimum(img, 1.0) * 0.82 + np.clip(img - 1.0, 0, None) * 0.08 + np.minimum(img, 1.0) ** 2 * 0.18
    gray = img.mean(axis=2, keepdims=True)
    img = gray + (img - gray) * 1.12
    img = np.clip(img * 1.06, 0, 1).astype(np.float32)
    # 3.4 Prismatic: every colour at once, a diagonal rainbow over the scene
    # and brighter on the motif, with white sparks.
    if category == "prismatic":
        hh, ww = img.shape[:2]
        gy, gx = np.mgrid[0:hh, 0:ww].astype(np.float32)
        hue = ((gx / ww) * 0.8 + (gy / hh) * 0.5 + rng.random() * 0.3) % 1.0
        k = (hue[..., None] * 6.0 + np.array([0.0, 4.0, 2.0], np.float32)) % 6.0
        rainbow = np.clip(np.abs(k - 3.0) - 1.0, 0, 1)
        weight = 0.38 + 0.4 * blur(np.maximum(body, extra), 10)[..., None]
        img = img * (1 - weight) + (img * 0.35 + rainbow * 0.75) * weight
        sparks = (rng.random((hh, ww)) > 0.9975).astype(np.float32)
        img = img + blur(sparks, 1.4)[..., None] * 1.6
        img = np.clip(img, 0, 1).astype(np.float32)
    img = kuwahara(img, 1)
    out = Image.fromarray((np.clip(img, 0, 1) * 255).astype(np.uint8)).resize((TILE_W, TILE_H), Image.LANCZOS)
    arr = np.asarray(out, np.float32) / 255
    grain = fbm(rng, TILE_W, TILE_H, 3, 2)
    vy, vx = np.mgrid[0:TILE_H, 0:TILE_W].astype(np.float32)
    r2 = ((vx / TILE_W - 0.5) * 1.6) ** 2 + ((vy / TILE_H - 0.48) * 1.9) ** 2
    vignette = np.clip(1 - 0.42 * r2, 0.35, 1)
    arr = arr * (0.93 + 0.07 * grain[..., None]) * vignette[..., None]
    return (np.clip(arr, 0, 1) * 255).astype(np.uint8), element, motif


def card_back():
    rng = np.random.default_rng(seed_of("ascension card back"))
    yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
    base = np.array([0.045, 0.040, 0.030], np.float32)
    n = fbm(rng, W, H, 60, 5)
    img = base + n[..., None] * np.array([0.06, 0.05, 0.03], np.float32)
    cx, cy = W / 2, H / 2
    dist = np.sqrt((xx - cx) ** 2 + (yy - cy) ** 2) / H
    img = img + np.array([0.9, 0.7, 0.35], np.float32) * np.exp(-(dist ** 2) / (2 * 0.22 ** 2))[..., None] * 0.35
    sig = Image.new("L", (W, H), 0)
    d = ImageDraw.Draw(sig)
    s = H * 0.40
    for r, wdt in ((1.0, 0.035), (0.86, 0.02), (0.42, 0.025)):
        d.ellipse([cx - r * s, cy - r * s, cx + r * s, cy + r * s], outline=255, width=max(2, int(wdt * s)))
    for k in range(4):
        a = math.pi / 4 + k * math.pi / 2
        d.line([(cx + math.cos(a) * 0.42 * s, cy + math.sin(a) * 0.42 * s),
                (cx + math.cos(a) * 1.0 * s, cy + math.sin(a) * 1.0 * s)], fill=255, width=max(2, int(0.025 * s)))
    d.polygon(star4(cx, cy, 0.62 * s, 0.07 * s), fill=255)
    d.arc([cx - 2.2 * s, cy - 0.9 * s, cx + 2.2 * s, cy + 2.9 * s], 230, 310, fill=200, width=max(2, int(0.02 * s)))
    d.arc([cx - 2.2 * s, cy - 2.9 * s, cx + 2.2 * s, cy + 0.9 * s], 50, 130, fill=200, width=max(2, int(0.02 * s)))
    m = np.asarray(sig, np.float32) / 255
    gold = np.array([0.95, 0.76, 0.40], np.float32)
    img = img + gold * blur(m, 10)[..., None] * 0.5
    img = img * (1 - m[..., None]) + gold * (0.75 + 0.3 * n[..., None]) * m[..., None]
    img = np.clip(img, 0, 1).astype(np.float32)
    img = kuwahara(img, 2)
    out = Image.fromarray((img * 255).astype(np.uint8)).resize((TILE_W, TILE_H), Image.LANCZOS)
    return np.asarray(out)


def override(pid):
    path = OVERRIDE / f"{pid}.png"
    if not path.exists():
        return None
    img = Image.open(path).convert("RGB")
    w, h = img.size
    target = TILE_W / TILE_H
    if w / h > target:
        nw = int(h * target)
        img = img.crop(((w - nw) // 2, 0, (w - nw) // 2 + nw, h))
    else:
        nh = int(w / target)
        img = img.crop((0, (h - nh) // 2, w, (h - nh) // 2 + nh))
    return np.asarray(img.resize((TILE_W, TILE_H), Image.LANCZOS))


# -------------------------------------------------------------------- QOI ---
def qoi_encode(rgb: np.ndarray) -> bytes:
    h, w, _ = rgb.shape
    out = bytearray(b"qoif" + struct.pack(">IIBB", w, h, 3, 0))
    index = [None] * 64
    pr, pg, pb = 0, 0, 0
    run = 0
    flat = rgb.reshape(-1, 3).tolist()
    last = len(flat) - 1
    append = out.append
    for i, (r, g, b) in enumerate(flat):
        if r == pr and g == pg and b == pb:
            run += 1
            if run == 62 or i == last:
                append(0xC0 | (run - 1))
                run = 0
            continue
        if run:
            append(0xC0 | (run - 1))
            run = 0
        hsh = (r * 3 + g * 5 + b * 7 + 255 * 11) % 64
        px = (r, g, b)
        if index[hsh] == px:
            append(hsh)
        else:
            index[hsh] = px
            vr = ((r - pr + 128) & 255) - 128
            vg = ((g - pg + 128) & 255) - 128
            vb = ((b - pb + 128) & 255) - 128
            vg_r, vg_b = vr - vg, vb - vg
            if -2 <= vr <= 1 and -2 <= vg <= 1 and -2 <= vb <= 1:
                append(0x40 | ((vr + 2) << 4) | ((vg + 2) << 2) | (vb + 2))
            elif -32 <= vg <= 31 and -8 <= vg_r <= 7 and -8 <= vg_b <= 7:
                append(0x80 | (vg + 32))
                append(((vg_r + 8) << 4) | (vg_b + 8))
            else:
                out += bytes((0xFE, r, g, b))
        pr, pg, pb = r, g, b
    out += bytes((0, 0, 0, 0, 0, 0, 0, 1))
    return bytes(out)


def qoi_decode(data: bytes) -> np.ndarray:
    """Spec decoder, used to check every atlas right after writing it."""
    if data[:4] != b"qoif":
        raise ValueError("not QOI")
    w, h, channels, _ = struct.unpack(">IIBB", data[4:14])
    out = bytearray(w * h * 3)
    index = [(0, 0, 0, 0)] * 64
    r, g, b, a = 0, 0, 0, 255
    p, run, end = 14, 0, len(data) - 8
    for i in range(w * h):
        if run:
            run -= 1
        elif p < end:
            b1 = data[p]
            p += 1
            if b1 == 0xFE:
                r, g, b = data[p], data[p + 1], data[p + 2]
                p += 3
            elif b1 == 0xFF:
                r, g, b, a = data[p], data[p + 1], data[p + 2], data[p + 3]
                p += 4
            elif b1 >> 6 == 0:
                r, g, b, a = index[b1]
            elif b1 >> 6 == 1:
                r = (r + ((b1 >> 4) & 3) - 2) & 255
                g = (g + ((b1 >> 2) & 3) - 2) & 255
                b = (b + (b1 & 3) - 2) & 255
            elif b1 >> 6 == 2:
                b2 = data[p]
                p += 1
                vg = (b1 & 0x3F) - 32
                r = (r + vg - 8 + ((b2 >> 4) & 15)) & 255
                g = (g + vg) & 255
                b = (b + vg - 8 + (b2 & 15)) & 255
            else:
                run = b1 & 0x3F
            index[(r * 3 + g * 5 + b * 7 + a * 11) % 64] = (r, g, b, a)
        out[i * 3:i * 3 + 3] = bytes((r, g, b))
    return np.frombuffer(bytes(out), np.uint8).reshape(h, w, 3)


def main():
    powers = [p for p in json.loads((PROJECT / "config/powers.json").read_text())["powers"] if not p.get("hidden")]
    procs = {p["id"]: p for p in json.loads((PROJECT / "config/procs.json").read_text())["procs"]}
    OUT.mkdir(parents=True, exist_ok=True)
    tiles = [card_back()]
    cards = {}
    looks = {}
    for power in powers:
        art = override(power["id"])
        if art is None:
            art, element, motif = paint(power, procs)
            looks[power["id"]] = f"{motif}/{element}"
        else:
            looks[power["id"]] = "override"
        cards[power["id"]] = len(tiles)
        tiles.append(art)
    per_atlas = COLS * ROWS
    atlases = []
    for a in range((len(tiles) + per_atlas - 1) // per_atlas):
        chunk = tiles[a * per_atlas:(a + 1) * per_atlas]
        rows = (len(chunk) + COLS - 1) // COLS
        height = rows * TILE_H
        sheet = np.zeros((height, ATLAS_W, 3), np.uint8)
        for i, tile in enumerate(chunk):
            r, c = divmod(i, COLS)
            sheet[r * TILE_H:(r + 1) * TILE_H, c * TILE_W:(c + 1) * TILE_W] = tile
        name = f"atlas-{a}.qoi"
        encoded = qoi_encode(sheet)
        if not np.array_equal(qoi_decode(encoded), sheet):
            raise SystemExit(f"{name}: QOI round trip failed")
        (OUT / name).write_bytes(encoded)
        atlases.append({"file": name, "width": ATLAS_W, "height": height})
    for stale in OUT.glob("atlas-*.qoi"):
        if stale.name not in {a["file"] for a in atlases}:
            stale.unlink()
    index = {
        "schema": 1,
        "tile": [TILE_W, TILE_H],
        "columns": COLS,
        "per_atlas": per_atlas,
        "atlases": atlases,
        "back": 0,
        "cards": cards,
        "looks": looks,
    }
    (OUT / "index.json").write_text(json.dumps(index, indent=1, sort_keys=True) + "\n")
    size = sum((OUT / a["file"]).stat().st_size for a in atlases)
    print(f"{len(cards)} card illustrations + back in {len(atlases)} atlases ({size / 1e6:.1f} MB)")


if __name__ == "__main__":
    main()
