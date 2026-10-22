"""Icona definitiva: globo di rete + prompt '>_', nei colori scelti dall'utente (arancio -> blu),
ispirata al riferimento ma ridisegnata pulita (niente dettagli raster che non reggono il ridimensionamento).
Master a 1024px, da cui `tauri icon` genera tutte le altre dimensioni.
"""

import math
import random

from PIL import Image, ImageDraw, ImageFilter

W = 1024
ORANGE_TOP = (242, 140, 29)
ORANGE_MID = (214, 104, 40)
BLUE_BOT = (16, 50, 110)
CREAM = (255, 238, 214)

random.seed(7)

img = Image.new("RGBA", (W, W), (0, 0, 0, 0))

# Sfondo: piastrella arrotondata con sfumatura verticale arancio -> blu.
grad = Image.new("RGBA", (1, W), (0, 0, 0, 255))
for y in range(W):
    t = y / (W - 1)
    if t < 0.5:
        tt = t / 0.5
        c = tuple(int(ORANGE_TOP[i] + (ORANGE_MID[i] - ORANGE_TOP[i]) * tt) for i in range(3))
    else:
        tt = (t - 0.5) / 0.5
        c = tuple(int(ORANGE_MID[i] + (BLUE_BOT[i] - ORANGE_MID[i]) * tt) for i in range(3))
    grad.putpixel((0, y), c + (255,))
grad = grad.resize((W, W))
mask = Image.new("L", (W, W), 0)
ImageDraw.Draw(mask).rounded_rectangle([0, 0, W - 1, W - 1], radius=200, fill=255)
img.paste(grad, (0, 0), mask)

d = ImageDraw.Draw(img, "RGBA")
cx, cy, r = 512, 512, 300

# Rete di nodi e linee attorno al globo (dietro), sottili.
rng = random.Random(3)
nodes = []
for _ in range(10):
    ang = rng.uniform(0, 2 * math.pi)
    rad = r * rng.uniform(1.05, 1.55)
    nx, ny = cx + rad * math.cos(ang), cy + rad * math.sin(ang) * 0.85
    if 60 < nx < W - 60 and 60 < ny < W - 60:
        nodes.append((nx, ny, rng.uniform(5, 11)))
for nx, ny, nr in nodes:
    ex = cx + (r * 0.92) * (nx - cx) / math.hypot(nx - cx, ny - cy)
    ey = cy + (r * 0.92) * (ny - cy) / math.hypot(nx - cx, ny - cy)
    d.line([(ex, ey), (nx, ny)], fill=(*CREAM, 70), width=2)
    d.ellipse([nx - nr, ny - nr, nx + nr, ny + nr], fill=(*CREAM, 110))

# Il globo: disco "vetro" con bordo chiaro e un filo di luce interna.
glass = Image.new("RGBA", (W, W), (0, 0, 0, 0))
dg = ImageDraw.Draw(glass)
dg.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(255, 255, 255, 30), outline=(*CREAM, 230), width=6)
glow = Image.new("RGBA", (W, W), (0, 0, 0, 0))
ImageDraw.Draw(glow).ellipse([cx - r + 40, cy - r + 30, cx + r - 60, cy + r - 90], fill=(255, 255, 255, 55))
glow = glow.filter(ImageFilter.GaussianBlur(50))
glass = Image.alpha_composite(glass, glow)
img = Image.alpha_composite(img, glass)
d = ImageDraw.Draw(img, "RGBA")

# Meridiani/paralleli del globo.
d.line([(cx, cy - r), (cx, cy + r)], fill=(*CREAM, 150), width=4)
d.line([(cx - r, cy), (cx + r, cy)], fill=(*CREAM, 150), width=4)
for frac in (0.45, 0.8):
    rw = r * frac
    d.ellipse([cx - rw, cy - r, cx + rw, cy + r], outline=(*CREAM, 110), width=3)
d.arc([cx - r, cy - r * 0.55, cx + r, cy + r * 0.55], 0, 360, fill=(*CREAM, 110), width=3)

# Nodi piccoli sulle intersezioni del reticolo (decorativi, pochi, grandi abbastanza da reggere il
# ridimensionamento).
for ang in (40, 140, 220, 320):
    rad = math.radians(ang)
    px, py = cx + r * 0.6 * math.cos(rad), cy + r * 0.42 * math.sin(rad)
    d.ellipse([px - 7, py - 7, px + 7, py + 7], fill=(*CREAM, 200))

# Il prompt '>_' al centro: la forma che deve restare leggibile anche piccolissima.
glow2 = Image.new("RGBA", (W, W), (0, 0, 0, 0))
dg2 = ImageDraw.Draw(glow2)
dg2.line([(cx - 95, cy - 90), (cx + 15, cy), (cx - 95, cy + 90)], fill=CREAM, width=70, joint="curve")
glow2 = glow2.filter(ImageFilter.GaussianBlur(28))
img = Image.alpha_composite(img, glow2)
d = ImageDraw.Draw(img, "RGBA")
d.line([(cx - 95, cy - 90), (cx + 15, cy), (cx - 95, cy + 90)], fill=(255, 255, 255, 255), width=46, joint="curve")
d.rounded_rectangle([cx + 40, cy + 62, cx + 190, cy + 108], radius=22, fill=(255, 255, 255, 255))

img.save("icon_master_1024.png")
img.resize((256, 256), Image.LANCZOS).save("icon_master_preview.png")
print("done")
