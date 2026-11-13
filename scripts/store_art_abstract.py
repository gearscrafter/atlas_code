"""Abstract images for the SideQuest listing: "diving into the net". Data rain made of code characters,
a network of nodes, translucent planes fading into depth (the Atlas panels) and the L-shaped grips,
over a deep blue-green, with scanlines, chromatic aberration and grain.

Usage: python scripts/store_art_abstract.py .   ->  dist/store/abstract/
"""
import math
import os
import random
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = sys.argv[1] if len(sys.argv) > 1 else "."
OUT = os.path.join(ROOT, "dist", "store", "abstract")
os.makedirs(OUT, exist_ok=True)
FONT = os.path.join(ROOT, "assets", "fonts", "RobotoMono-Regular.ttf")

DEEP = (2, 9, 11)
TEAL = (12, 48, 50)
DATA = (110, 255, 205)
CYAN = (120, 225, 255)
ORANGE = (255, 150, 70)
GLYPHS = "0123456789ABCDEF{}[]<>/=:;|_+*#$&λ"


def font(px):
    return ImageFont.truetype(FONT, max(6, int(px)))


def glow(base, layer, radius, strength=2, keep=True):
    """A halo of the same color as `layer` (the premultiplied color and the transparency are blurred separately)."""
    arr = np.asarray(layer).astype(np.float32) / 255.0
    a = arr[..., 3:4]
    premul = Image.fromarray((arr[..., :3] * a * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(radius))
    ba = np.asarray(layer.getchannel("A").filter(ImageFilter.GaussianBlur(radius))).astype(np.float32)[..., None] / 255
    col = np.asarray(premul).astype(np.float32) / 255 / np.maximum(ba, 1e-4)
    halo = Image.fromarray((np.concatenate([np.clip(col, 0, 1), np.clip(ba * 0.9, 0, 1)], -1) * 255).astype(np.uint8), "RGBA")
    for _ in range(strength):
        base.alpha_composite(halo)
    if keep:
        base.alpha_composite(layer)


def backdrop(w, h, rng):
    """Background: a very dark blue-green radial gradient, with a lighter haze toward the center."""
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    cx, cy = w * 0.58, h * 0.46
    r = np.sqrt(((x - cx) / w) ** 2 + ((y - cy) / h) ** 2)
    t = np.clip(1.0 - r * 1.6, 0, 1)[..., None]
    rgb = np.array(DEEP, np.float32) * (1 - t) + np.array(TEAL, np.float32) * t
    return Image.fromarray(np.concatenate([rgb, np.full((h, w, 1), 255, np.float32)], -1).astype(np.uint8), "RGBA")


def data_rain(base, rng, columns, size_range, alpha_range, blur, tint=DATA):
    """Columns of falling characters that fade out; the far ones small, dim and blurred."""
    w, h = base.size
    layer = Image.new("RGBA", base.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    for _ in range(columns):
        px = rng.uniform(*size_range)
        f = font(px)
        x = rng.uniform(0, w)
        length = rng.randint(6, 26)
        head = rng.uniform(-h * 0.2, h * 1.1)
        for k in range(length):
            y = head - k * px * 1.15
            if y < -px or y > h:
                continue
            fade = (1 - k / length) ** 1.6
            a = int(rng.uniform(*alpha_range) * fade)
            color = (230, 255, 245) if k == 0 else tint
            d.text((x, y), rng.choice(GLYPHS), font=f, fill=color + (a,))
    if blur:
        layer = layer.filter(ImageFilter.GaussianBlur(blur))
    glow(base, layer, max(2, blur * 2 + 2), 1)


VANISH = (0.5, 0.5)


def project(p, w, h, focal):
    x, y, z = p
    return (w * VANISH[0] + focal * x / z, h * VANISH[1] + focal * y / z)


def network(base, rng, n, w, h, focal, accent_every=7):
    """Nodes in space joined to their neighbors: the network."""
    pts = [(rng.uniform(-5, 5), rng.uniform(-2.2, 1.8), rng.uniform(6.0, 16.0)) for _ in range(n)]
    layer = Image.new("RGBA", base.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    proj = [project(p, w, h, focal) for p in pts]
    for i, p in enumerate(pts):
        near = sorted(range(n), key=lambda j: sum((a - b) ** 2 for a, b in zip(p, pts[j])))[1:4]
        for j in near:
            depth = (p[2] + pts[j][2]) / 2
            a = int(120 * (1 - (depth - 6) / 11))
            d.line((proj[i], proj[j]), fill=CYAN + (max(20, a),), width=1 if depth > 5 else 2)
    for i, p in enumerate(pts):
        r = max(1.5, 9 / p[2])
        c = ORANGE if i % accent_every == 0 else (200, 245, 255)
        x, y = proj[i]
        d.ellipse((x - r, y - r, x + r, y + r), fill=c + (230,))
    glow(base, layer, 6, 2)


def l_corners(d, quad, arm, color, width):
    """L-shaped grips at the four corners of a quadrilateral (in perspective)."""
    for i in range(4):
        p = np.array(quad[i], np.float32)
        for nb in (quad[(i + 1) % 4], quad[(i - 1) % 4]):
            v = np.array(nb, np.float32) - p
            q = p + v / (np.linalg.norm(v) + 1e-6) * arm
            d.line((tuple(p), tuple(q)), fill=color, width=width)


def planes(base, rng, w, h, focal, count):
    """A corridor of translucent planes (the Atlas panels, abstracted) that recedes toward the vanishing point,
    alternating left and right and turned toward the center. Each one with its L-shaped grips."""
    layer = Image.new("RGBA", base.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    for k in reversed(range(count)):
        z = 2.1 + k * 1.25
        side = -1 if k % 2 == 0 else 1
        cx = side * rng.uniform(1.0, 1.2)
        cy = rng.uniform(-0.35, 0.15)
        hw, hh = 0.7, 0.42
        yaw = -side * rng.uniform(0.55, 0.8)
        corners = []
        for sx, sy in [(-1, -1), (1, -1), (1, 1), (-1, 1)]:
            x = cx + sx * hw * math.cos(yaw)
            zz = z + sx * hw * math.sin(yaw)
            corners.append(project((x, cy + sy * hh, zz), w, h, focal))
        a = int(230 * (1 - k / count) ** 1.2) + 25
        d.polygon(corners, fill=(20, 70, 80, int(a * 0.22)), outline=CYAN + (int(a * 0.45),))
        for row in range(6):
            t = 0.18 + row * 0.12
            l = [corners[0][i] * (1 - t) + corners[3][i] * t for i in range(2)]
            r = [corners[1][i] * (1 - t) + corners[2][i] * t for i in range(2)]
            f0, f1 = 0.1, rng.uniform(0.35, 0.85)
            s0 = (l[0] + (r[0] - l[0]) * f0, l[1] + (r[1] - l[1]) * f0)
            s1 = (l[0] + (r[0] - l[0]) * f1, l[1] + (r[1] - l[1]) * f1)
            d.line((s0, s1), fill=(DATA if row % 2 else CYAN) + (int(a * 0.6),), width=2 if k < 3 else 1)
        arm = max(8, 0.16 * focal / z)
        l_corners(d, corners, arm, (ORANGE if k == 1 else CYAN) + (min(255, a + 30),), 3 if k < 4 else 2)
    glow(base, layer, 5, 1)


def light(base, w, h):
    """The point of light at the end of the corridor."""
    layer = Image.new("RGBA", base.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    x, y = w * VANISH[0], h * VANISH[1]
    d.ellipse((x - 10, y - 10, x + 10, y + 10), fill=(255, 235, 210, 255))
    d.ellipse((x - 4, y - 4, x + 4, y + 4), fill=(255, 255, 255, 255))
    glow(base, layer, 40, 3)
    halo = Image.new("RGBA", base.size, (0, 0, 0, 0))
    ImageDraw.Draw(halo).ellipse((x - 260, y - 160, x + 260, y + 160), fill=(60, 200, 190, 60))
    base.alpha_composite(halo.filter(ImageFilter.GaussianBlur(90)))


def grid_floor(base, w, h):
    layer = Image.new("RGBA", base.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    horizon = h * 0.7
    for i in range(-30, 31):
        d.line((w / 2 + i * 12, horizon, w / 2 + i * 160, h), fill=DATA + (26,), width=1)
    for k in range(1, 14):
        y = horizon + (h - horizon) * (k / 13) ** 2
        d.line((0, y, w, y), fill=DATA + (int(18 + 20 * k / 13),), width=1)
    base.alpha_composite(layer)


def finish(img, rng, scan=3, aberration=3, grain=10, vignette=0.55):
    """Scanlines, chromatic aberration, grain and vignette: the look of an old screen."""
    arr = np.asarray(img.convert("RGB")).astype(np.float32)
    h, w, _ = arr.shape
    if aberration:
        arr[..., 0] = np.roll(arr[..., 0], aberration, axis=1)
        arr[..., 2] = np.roll(arr[..., 2], -aberration, axis=1)
    if scan:
        arr[::scan] *= 0.86
    if grain:
        arr += np.random.default_rng(7).normal(0, grain, arr.shape)
    if vignette:
        y, x = np.mgrid[0:h, 0:w].astype(np.float32)
        r = np.sqrt(((x - w / 2) / (w / 2)) ** 2 + ((y - h / 2) / (h / 2)) ** 2)
        arr *= (1 - vignette * np.clip(r - 0.35, 0, 1) ** 1.5)[..., None]
    return Image.fromarray(np.clip(arr, 0, 255).astype(np.uint8))


# ------------------------------------------------------------------ banner
def banner(w=1920, h=1080):
    global VANISH
    VANISH = (0.6, 0.44)
    rng = random.Random(11)
    img = backdrop(w, h, rng)
    data_rain(img, rng, 170, (10, 16), (35, 80), 2)
    grid_floor(img, w, h)
    light(img, w, h)
    network(img, rng, 40, w, h, 760)
    planes(img, rng, w, h, 760, 9)
    data_rain(img, rng, 22, (20, 30), (80, 190), 0)
    finish(img, rng).save(os.path.join(OUT, "banner-1920x1080.png"))


# ------------------------------------------------------------------ icono
def icon(size=1024):
    rng = random.Random(3)
    img = backdrop(size, size, rng)
    data_rain(img, rng, 70, (14, 22), (30, 80), 2)
    c = size / 2
    layer = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    # Broken rings around a core: a network node, an iris.
    for k, r in enumerate([150, 215, 280, 330]):
        start = rng.uniform(0, 360)
        segments = rng.randint(3, 6)
        for s in range(segments):
            a0 = start + s * 360 / segments
            a1 = a0 + 360 / segments * rng.uniform(0.35, 0.8)
            col = CYAN if k % 2 == 0 else DATA
            d.arc((c - r, c - r, c + r, c + r), a0, a1, fill=col + (230 - k * 30,), width=10 - k * 2)
    # Clock-like marks on the outer ring.
    for i in range(48):
        ang = math.radians(i * 7.5)
        r0, r1 = 370, 390 if i % 4 else 410
        d.line((c + r0 * math.cos(ang), c + r0 * math.sin(ang), c + r1 * math.cos(ang), c + r1 * math.sin(ang)), fill=CYAN + (150,), width=3)
    glow(img, layer, 10, 2)
    core = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    cd = ImageDraw.Draw(core)
    cd.rounded_rectangle((c - 46, c - 64, c + 46, c + 64), radius=8, fill=ORANGE + (255,))
    glow(img, core, 30, 3)
    lc = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    l_corners(ImageDraw.Draw(lc), [(110, 110), (size - 110, 110), (size - 110, size - 110), (110, size - 110)], 120, CYAN + (220,), 14)
    glow(img, lc, 8, 1)
    out = finish(img, rng, scan=4, aberration=2, grain=8, vignette=0.4).convert("RGBA")
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size, size), radius=int(size * 0.18), fill=255)
    out.putalpha(mask)
    out.save(os.path.join(OUT, "icon-1024.png"))
    out.resize((512, 512), Image.LANCZOS).save(os.path.join(OUT, "icon-512.png"))


# ------------------------------------------------------------------ title
def title(w=1800, h=420):
    rng = random.Random(5)
    text = "ATLAS CODE"
    spaced = " ".join(text)
    mask = Image.new("L", (w, h), 0)
    md = ImageDraw.Draw(mask)
    # The size that fits with a margin.
    px = 210
    while md.textlength(spaced, font=font(px)) > w * 0.86:
        px -= 4
    f = font(px)
    tw = md.textlength(spaced, font=f)
    md.text(((w - tw) / 2, (h - px * 1.2) / 2), spaced, font=f, fill=255)
    # Shifted horizontal slices: a signal glitch.
    glitched = Image.new("L", (w, h), 0)
    y = 0
    while y < h:
        band = rng.randint(6, 30)
        dx = rng.choice([0, 0, 0, rng.randint(-24, 24)])
        glitched.paste(mask.crop((0, y, w, y + band)), (dx, y))
        y += band
    out = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    for color, dx in [((255, 80, 90), -5), ((80, 200, 255), 5), ((225, 255, 245), 0)]:
        layer = Image.new("RGBA", (w, h), color + (0,))
        layer.putalpha(glitched.transform((w, h), Image.AFFINE, (1, 0, -dx, 0, 1, 0)).point(lambda a: int(a * (0.55 if dx else 1))))
        out.alpha_composite(layer)
    halo_src = Image.new("RGBA", (w, h), DATA + (0,))
    halo_src.putalpha(glitched)
    glow(out, halo_src, 18, 1, keep=False)
    out.alpha_composite(Image.merge("RGBA", [Image.new("L", (w, h), v) for v in (225, 255, 245)] + [glitched]))
    # Scanlines over the letters.
    a = np.asarray(out).copy()
    a[::4, :, 3] = (a[::4, :, 3] * 0.6).astype(np.uint8)
    Image.fromarray(a, "RGBA").save(os.path.join(OUT, "title.png"))


banner()
icon()
title()
print("ok:", sorted(os.listdir(OUT)))
