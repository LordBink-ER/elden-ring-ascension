#!/usr/bin/env python3
"""3.4: the Ascension launcher icon (assets/icon/ascension.png, 512 px).

A gold medallion: a dark bronze disc in a double gold rim, light rays, a card
rising out of the light with a gold frame and a radiant diamond sigil, three
rising chevrons and embers. Drawn at 4x and scaled down. Run:
python3 tools/build_icon.py
"""
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

PROJECT = Path(__file__).resolve().parents[1]
OUT = PROJECT / "assets/icon/ascension.png"
S = 2048
C = S / 2
GOLD = (236, 196, 112)
GOLD_DARK = (150, 108, 48)


def radial(size, inner, outer, power=1.0):
    y, x = np.mgrid[0:size, 0:size].astype(np.float32)
    d = np.sqrt((x - size / 2) ** 2 + (y - size / 2) ** 2) / (size / 2)
    k = np.clip(d, 0, 1) ** power
    return np.asarray(inner, np.float32) * (1 - k[..., None]) + np.asarray(outer, np.float32) * k[..., None]


def main():
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    # Disc.
    disc = radial(S, (58, 44, 26), (12, 9, 6), 1.2)
    disc_img = Image.fromarray(np.clip(disc, 0, 255).astype(np.uint8), "RGB").convert("RGBA")
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).ellipse([C - 940, C - 940, C + 940, C + 940], fill=255)
    img.paste(disc_img, (0, 0), mask)
    # Rays from the card's light.
    rays = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(rays)
    for i in range(28):
        a = math.tau * i / 28
        w = 0.035 if i % 2 else 0.06
        r = 900
        d.polygon([(C, C + 60), (C + math.cos(a - w) * r, C + 60 + math.sin(a - w) * r),
                   (C + math.cos(a + w) * r, C + 60 + math.sin(a + w) * r)], fill=(255, 214, 140, 46 if i % 2 else 70))
    rays = rays.filter(ImageFilter.GaussianBlur(6))
    img = Image.alpha_composite(img, Image.composite(rays, Image.new("RGBA", (S, S), (0, 0, 0, 0)), mask))
    # Glow behind the card.
    glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(glow).ellipse([C - 420, C - 380, C + 420, C + 460], fill=(255, 190, 90, 150))
    img = Image.alpha_composite(img, glow.filter(ImageFilter.GaussianBlur(120)))
    # The card, tilted a little, rising.
    card = Image.new("RGBA", (760, 1060), (0, 0, 0, 0))
    cd = ImageDraw.Draw(card)
    cd.rounded_rectangle([20, 20, 740, 1040], 60, fill=(26, 20, 13, 255), outline=GOLD, width=26)
    cd.rounded_rectangle([70, 70, 690, 990], 38, outline=GOLD_DARK, width=8)
    # Sigil: a diamond with a ring, and three chevrons rising.
    cx, cy = 380, 470
    cd.ellipse([cx - 210, cy - 210, cx + 210, cy + 210], outline=GOLD, width=14)
    cd.polygon([(cx, cy - 260), (cx + 150, cy), (cx, cy + 260), (cx - 150, cy)], fill=(255, 226, 160, 255))
    cd.polygon([(cx, cy - 170), (cx + 92, cy), (cx, cy + 170), (cx - 92, cy)], fill=(196, 142, 60, 255))
    for k in range(3):
        y = 900 - k * 70
        cd.line([(cx - 150, y), (cx, y - 70), (cx + 150, y)], fill=GOLD, width=22, joint="curve")
    card = card.rotate(-8, resample=Image.BICUBIC, expand=True)
    shadow = Image.new("RGBA", card.size, (0, 0, 0, 0))
    shadow.paste((0, 0, 0, 170), (0, 0), card.split()[3])
    shadow = shadow.filter(ImageFilter.GaussianBlur(30))
    ox, oy = int(C - card.size[0] / 2), int(C - card.size[1] / 2 + 40)
    img.alpha_composite(shadow, (ox + 30, oy + 40))
    img.alpha_composite(card, (ox, oy))
    # A prismatic glint on the card's corner.
    glint = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glint)
    for i, col in enumerate([(255, 90, 90), (255, 200, 80), (120, 240, 120), (90, 180, 255), (200, 120, 255)]):
        gd.line([(C + 120 + i * 26, C - 520), (C + 320 + i * 26, C - 320)], fill=col + (150,), width=16)
    img = Image.alpha_composite(img, glint.filter(ImageFilter.GaussianBlur(4)))
    # Embers.
    rng = np.random.default_rng(7)
    em = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ed = ImageDraw.Draw(em)
    for _ in range(70):
        a = rng.uniform(0, math.tau)
        r = rng.uniform(250, 860)
        x, y = C + math.cos(a) * r, C + math.sin(a) * r
        s = rng.uniform(5, 16)
        ed.ellipse([x - s, y - s, x + s, y + s], fill=(255, int(rng.uniform(160, 230)), 90, int(rng.uniform(120, 230))))
    em = Image.composite(em, Image.new("RGBA", (S, S), (0, 0, 0, 0)), mask)
    img = Image.alpha_composite(img, em.filter(ImageFilter.GaussianBlur(2)))
    # Double gold rim.
    rim = ImageDraw.Draw(img)
    rim.ellipse([C - 960, C - 960, C + 960, C + 960], outline=GOLD, width=44)
    rim.ellipse([C - 880, C - 880, C + 880, C + 880], outline=GOLD_DARK, width=14)
    for i in range(12):
        a = math.tau * i / 12 - math.pi / 2
        x, y = C + math.cos(a) * 920, C + math.sin(a) * 920
        rim.polygon([(x, y - 30), (x + 22, y), (x, y + 30), (x - 22, y)], fill=(255, 226, 160))
    OUT.parent.mkdir(parents=True, exist_ok=True)
    img.resize((512, 512), Image.LANCZOS).save(OUT)
    print(f"icon: {OUT} (512 px)")


if __name__ == "__main__":
    main()
