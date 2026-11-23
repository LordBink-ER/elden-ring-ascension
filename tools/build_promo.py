#!/usr/bin/env python3
"""3.4.1: images for the mod page (release/images), drawn from the mod's own
card art, fonts and rarity colours. Key art, not in-game screenshots.

  cover.png   1920x1080  title + a fan of five cards
  cards.png   1920x1080  a gallery of twelve cards
  prismatic.png 1920x1080  the ten Prismatic cards
Run: python3 tools/build_promo.py
"""
from __future__ import annotations

import colorsys
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

PROJECT = Path(__file__).resolve().parents[1]
OUT = PROJECT / "release/images"
SERIF = str(PROJECT / "assets/fonts/NotoSerifDisplay-Regular.ttf")
SANS = str(PROJECT / "assets/fonts/DejaVuSans.ttf")
W, H = 1920, 1080
GOLD = (236, 200, 120)
RARITY = {
    "common": (168, 171, 163), "uncommon": (99, 184, 112), "rare": (94, 158, 242),
    "epic": (184, 120, 230), "legendary": (240, 191, 79), "mythic": (235, 79, 71),
}


def font(path, size):
    return ImageFont.truetype(path, size)


class Art:
    def __init__(self):
        self.index = json.loads((PROJECT / "assets/cards/index.json").read_text())
        self.atlases = [Image.open(PROJECT / "assets/cards" / a["file"]).convert("RGB") for a in self.index["atlases"]]
        self.tw, self.th = self.index["tile"]

    def tile(self, card_id):
        i = self.index["cards"][card_id]
        per, cols = self.index["per_atlas"], self.index["columns"]
        j = i % per
        x, y = (j % cols) * self.tw, (j // cols) * self.th
        return self.atlases[i // per].crop((x, y, x + self.tw, y + self.th))


def powers():
    data = json.loads((PROJECT / "config/powers.json").read_text())
    rows = data if isinstance(data, list) else data["powers"]
    return {p["id"]: p for p in rows}


def rainbow(k):
    r, g, b = colorsys.hsv_to_rgb(k % 1.0, 0.62, 1.0)
    return int(r * 255), int(g * 255), int(b * 255)


def background(seed=3, size=(W, H), focus=(0.62, 0.62), embers=170):
    W, H = size
    y, x = np.mgrid[0:H, 0:W].astype(np.float32)
    d = np.sqrt(((x - W * focus[0]) / W) ** 2 + ((y - H * focus[1]) / H) ** 2)
    k = np.clip(d / 0.75, 0, 1)[..., None]
    base = np.array((70, 48, 24), np.float32) * (1 - k) + np.array((9, 7, 6), np.float32) * k
    img = Image.fromarray(base.astype(np.uint8), "RGB").convert("RGBA")
    rays = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(rays)
    cx, cy = W * focus[0], H * 1.05
    for i in range(22):
        a = math.pi + math.pi * (i + 0.5) / 22
        w = 0.025 if i % 2 else 0.045
        d.polygon([(cx, cy), (cx + math.cos(a - w) * 2400, cy + math.sin(a - w) * 2400),
                   (cx + math.cos(a + w) * 2400, cy + math.sin(a + w) * 2400)], fill=(255, 210, 130, 16 if i % 2 else 26))
    img = Image.alpha_composite(img, rays.filter(ImageFilter.GaussianBlur(10)))
    rng = np.random.default_rng(seed)
    em = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    ed = ImageDraw.Draw(em)
    for _ in range(embers):
        x, y = rng.uniform(0, W), rng.uniform(0, H)
        s = rng.uniform(1.2, 4.2) * (0.6 + y / H)
        ed.ellipse([x - s, y - s, x + s, y + s], fill=(255, int(rng.uniform(150, 225)), 90, int(rng.uniform(70, 210))))
    img = Image.alpha_composite(img, em.filter(ImageFilter.GaussianBlur(1.2)))
    return img


def card(art, power, w=300, level=0):
    """A card in the mod's style: art, name, rarity, the level's text."""
    h = int(w * 1.2)
    prism = power["category"] == "prismatic"
    rarity = power["level_rarities"][level] if power.get("level_rarities") else power["base_rarity"]
    color = rainbow(0.0) if prism else RARITY.get(rarity, GOLD)
    img = Image.new("RGBA", (w + 40, h + 40), (0, 0, 0, 0))
    glow = Image.new("RGBA", img.size, (0, 0, 0, 0))
    ImageDraw.Draw(glow).rounded_rectangle([14, 14, w + 26, h + 26], 22, fill=color + (150,))
    img = Image.alpha_composite(img, glow.filter(ImageFilter.GaussianBlur(14)))
    body = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    bd = ImageDraw.Draw(body)
    bd.rounded_rectangle([0, 0, w - 1, h - 1], 18, fill=(22, 18, 14, 255))
    pic = art.tile(power["id"]).resize((w - 24, (w - 24) // 2), Image.LANCZOS)
    body.paste(pic, (12, 12))
    # Frame: rarity colour, or every colour around a Prismatic card.
    if prism:
        steps = 240
        for s in range(steps):
            t0, t1 = s / steps, (s + 1) / steps

            def at(t):
                p = t * 2 * (w + h)
                if p < w:
                    return (p, 0)
                if p < w + h:
                    return (w - 1, p - w)
                if p < 2 * w + h:
                    return (w - 1 - (p - w - h), h - 1)
                return (0, h - 1 - (p - 2 * w - h))
            bd.line([at(t0), at(t1)], fill=rainbow(t0), width=7)
    else:
        bd.rounded_rectangle([0, 0, w - 1, h - 1], 18, outline=color, width=5)
    bd.rectangle([12, 12, w - 13, 12 + (w - 24) // 2], outline=(0, 0, 0), width=2)
    y = 24 + (w - 24) // 2
    name_font = font(SERIF, int(w * 0.095))
    name = power["name"]
    while bd.textlength(name, font=name_font) > w - 30 and name_font.size > 14:
        name_font = font(SERIF, name_font.size - 1)
    bd.text((w / 2, y + 8), name, font=name_font, fill=(245, 236, 214), anchor="mt")
    label = "PRISMATIC" if prism else ("KEYSTONE" if power["category"] == "keystone" else
                                       "SIGNATURE" if power["category"] == "signature" else rarity.upper())
    small = font(SANS, int(w * 0.044))
    spaced = "  ".join(label)
    lx = w / 2 - bd.textlength(spaced, font=small) / 2
    ly = y + 16 + name_font.size
    for k, ch in enumerate(spaced):
        bd.text((lx, ly), ch, font=small, fill=rainbow(k / max(1, len(spaced))) if prism else color)
        lx += bd.textlength(ch, font=small)
    text = power["levels"][level]["description"]
    tf = font(SANS, int(w * 0.056))
    lines = []
    for para in text.split("\n"):
        line = ""
        for word in para.split():
            test = (line + " " + word).strip()
            if bd.textlength(test, font=tf) > w - 34 and line:
                lines.append(line)
                line = word
            else:
                line = test
        if line:
            lines.append(line)
    ty = ly + small.size + 14
    for line in lines[:5]:
        bd.text((w / 2, ty), line, font=tf, fill=(214, 204, 186), anchor="mt")
        ty += int(tf.size * 1.32)
    pips = level + 1
    for i in range(5):
        cx = w / 2 + (i - 2) * 22
        pip = (rainbow(i / 5) if prism else color) if i < pips else (60, 54, 46)
        bd.ellipse([cx - 6, h - 28, cx + 6, h - 16], fill=pip)
    img.alpha_composite(body, (20, 20))
    return img


def title_block(img, x, y, scale=1.0, version=None):
    d = ImageDraw.Draw(img)
    over = font(SERIF, int(34 * scale))
    big = font(SERIF, int(150 * scale))
    sub = font(SERIF, int(36 * scale))
    d.text((x + 6, y), "E L D E N   R I N G", font=over, fill=(214, 186, 128))
    glow = Image.new("RGBA", img.size, (0, 0, 0, 0))
    ImageDraw.Draw(glow).text((x, y + 40 * scale), "ASCENSION", font=big, fill=(255, 200, 110, 200))
    img.alpha_composite(glow.filter(ImageFilter.GaussianBlur(18)))
    d = ImageDraw.Draw(img)
    d.text((x, y + 40 * scale), "ASCENSION", font=big, fill=(250, 232, 190))
    ry = y + 40 * scale + big.size * 1.2
    d.line([(x + 6, ry), (x + 640 * scale, ry)], fill=GOLD, width=2)
    cx = x + 323 * scale
    d.polygon([(cx, ry - 9), (cx + 9, ry), (cx, ry + 9), (cx - 9, ry)], fill=GOLD)
    d.text((x + 6, ry + 26), "Roguelite cards for the Lands Between", font=sub, fill=(232, 220, 196))
    if version:
        d.text((x + 6, ry + 22 + sub.size * 1.5), version, font=font(SANS, int(26 * scale)), fill=(190, 172, 140))
    return ry + 22 + sub.size * 2.6


def cover(art, p, version):
    img = background(3)
    after = title_block(img, 100, 250, 0.9, None)
    d = ImageDraw.Draw(img)
    bullets = ["Beat a boss, pick 1 of 3 cards", "261 cards  ·  9 Paths  ·  31 boss Signature cards",
               "18 Keystones  ·  10 ultra-rare Prismatic cards", "Every attribute gets a new role"]
    bf = font(SANS, 30)
    for i, b in enumerate(bullets):
        yy = after + 10 + i * 50
        d.polygon([(118, yy + 15), (127, yy + 24), (118, yy + 33), (109, yy + 24)], fill=GOLD)
        d.text((145, yy + 6), b, font=bf, fill=(236, 226, 206))
    d.text((112, H - 70), version, font=font(SANS, 26), fill=(190, 172, 140))
    sig = next((pid for pid, x in p.items() if x["category"] == "signature" and x["name"] == "Starscourge"),
               next(pid for pid, x in p.items() if x["category"] == "signature"))
    myth = next((pid for pid, x in p.items() if x["category"] == "evolution"), "key_aegis_oath")
    fan = [("key_wrath_unbound", 2), (sig, 0), ("prism_starfall_apocalypse", 4), (myth, 0), ("key_phantom_veil", 3)]
    angles = [-16, -8, 0, 8, 16]
    for i in [0, 4, 1, 3, 2]:
        pid, lvl = fan[i]
        c = card(art, p[pid], 300, min(lvl, len(p[pid]["levels"]) - 1))
        if i == 2:
            c = card(art, p[pid], 340, min(lvl, len(p[pid]["levels"]) - 1))
        r = c.rotate(-angles[i], resample=Image.BICUBIC, expand=True)
        cx = 1420 + (i - 2) * 132
        cy = 540 + abs(i - 2) * 34
        shadow = Image.new("RGBA", r.size, (0, 0, 0, 0))
        shadow.paste((0, 0, 0, 160), (0, 0), r.split()[3])
        img.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(16)), (int(cx - r.width / 2 + 14), int(cy - r.height / 2 + 22)))
        img.alpha_composite(r, (int(cx - r.width / 2), int(cy - r.height / 2)))
    return img


def gallery(art, p, ids, title, subtitle, seed, levels=None):
    img = background(seed)
    d = ImageDraw.Draw(img)
    d.text((W / 2, 52), title, font=font(SERIF, 64), fill=(250, 232, 190), anchor="mt")
    d.text((W / 2, 132), subtitle, font=font(SANS, 26), fill=(214, 196, 160), anchor="mt")
    cols = (len(ids) + 1) // 2
    cw = min(300, int((W - 120) / cols) - 34)
    for k, pid in enumerate(ids):
        row, col = divmod(k, cols)
        lvl = (levels or {}).get(pid, 0)
        c = card(art, p[pid], cw, min(lvl, len(p[pid]["levels"]) - 1))
        n = min(cols, len(ids) - row * cols)
        x = W / 2 + (col - (n - 1) / 2) * (cw + 34) - c.width / 2
        rows = (len(ids) + cols - 1) // cols
        top = 190 + (H - 190 - rows * (cw * 1.2 + 40)) / 2
        y = top + row * (cw * 1.2 + 40)
        img.alpha_composite(c, (int(x), int(y)))
    return img


def header(art, p):
    """The mod page banner: 1300x372, drawn at 2x and scaled down."""
    S = 2
    hw, hh = 1300 * S, 372 * S
    img = background(11, (hw, hh), (0.72, 0.75), embers=150)
    shade = Image.new("RGBA", (hw, hh), (0, 0, 0, 0))
    sd = ImageDraw.Draw(shade)
    for i in range(60):
        sd.rectangle([0, 0, int(hw * 0.55 * (1 - i / 60)), hh], fill=(0, 0, 0, 3))
    img = Image.alpha_composite(img, shade)
    title_block(img, 110, 150, 1.22, None)
    sig = next((pid for pid, x in p.items() if x["category"] == "signature" and x["name"] == "Starscourge"),
               next(pid for pid, x in p.items() if x["category"] == "signature"))
    loretta = next((pid for pid, x in p.items() if x["name"] == "Loretta's Mastery"), sig)
    myth = next((pid for pid, x in p.items() if x["category"] == "evolution"), "key_aegis_oath")
    fan = [("key_colossus_grip", 2), (loretta, 1), ("key_wrath_unbound", 3), ("prism_starfall_apocalypse", 4),
           (myth, 0), (sig, 2), ("key_phantom_veil", 4)]
    n = len(fan)
    mid = (n - 1) / 2
    order = sorted(range(n), key=lambda i: -abs(i - mid))
    for i in order:
        pid, lvl = fan[i]
        big = i == int(mid)
        c = card(art, p[pid], 300 if big else 250, min(lvl, len(p[pid]["levels"]) - 1))
        r = c.rotate(-(i - mid) * 5.5, resample=Image.BICUBIC, expand=True)
        cx = 1890 + (i - mid) * 150
        cy = 380 + abs(i - mid) ** 1.6 * 16
        shadow = Image.new("RGBA", r.size, (0, 0, 0, 0))
        shadow.paste((0, 0, 0, 160), (0, 0), r.split()[3])
        img.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(14)), (int(cx - r.width / 2 + 12), int(cy - r.height / 2 + 18)))
        img.alpha_composite(r, (int(cx - r.width / 2), int(cy - r.height / 2)))
    return img.convert("RGB").resize((1300, 372), Image.LANCZOS)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    version = "version " + next(l.split('"')[1] for l in (PROJECT / "Cargo.toml").read_text().splitlines() if l.startswith("version"))
    art, p = Art(), powers()
    cover(art, p, version).convert("RGB").save(OUT / "cover.png", optimize=True)
    header(art, p).save(OUT / "header.png", optimize=True)
    pick = []
    for cat in ["offense", "reflex", "magic", "status", "defense", "command", "signature", "signature",
                "keystone", "keystone", "evolution", "cursed"]:
        pid = next(i for i, x in p.items() if x["category"] == cat and i not in pick and i in art.index["cards"])
        pick.append(pid)
    levels = {pid: i % 5 for i, pid in enumerate(pick)}
    gallery(art, p, pick, "Every boss is a card draw",
            "Common to Mythic, Signature cards from the great bosses, Keystones that change how you play", 5, levels
            ).convert("RGB").save(OUT / "cards.png", optimize=True)
    prism = [i for i, x in p.items() if x["category"] == "prismatic"]
    gallery(art, p, prism, "PRISMATIC  ·  MAX WIN",
            "Ten ultra-rare cards in every colour at once, about two per playthrough", 9,
            {pid: 4 for pid in prism}).convert("RGB").save(OUT / "prismatic.png", optimize=True)
    print("images:", ", ".join(sorted(x.name for x in OUT.glob("*.png"))))


if __name__ == "__main__":
    main()
