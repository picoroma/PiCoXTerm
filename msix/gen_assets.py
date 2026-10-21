"""Genera le icone MSIX da src-tauri/icons/icon.png (512x512), nelle dimensioni che winapp si aspetta.
Placeholder finché arriva l'artwork originale dello Store: usa la stessa icona dell'app, senza padding
extra per le dimensioni piccole (AppList) e con un margine per i tile grandi, come da linee guida MSIX.
"""

from PIL import Image

SRC = "../src-tauri/icons/icon.png"
base = Image.open(SRC).convert("RGBA")


def square(size: int, margin_pct: float = 0.0) -> Image.Image:
    inner = int(size * (1 - 2 * margin_pct))
    icon = base.resize((inner, inner), Image.LANCZOS)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    off = (size - inner) // 2
    canvas.paste(icon, (off, off), icon)
    return canvas


def wide(w: int, h: int, margin_pct: float = 0.18) -> Image.Image:
    inner = int(h * (1 - 2 * margin_pct))
    icon = base.resize((inner, inner), Image.LANCZOS)
    canvas = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    canvas.paste(icon, ((w - inner) // 2, (h - inner) // 2), icon)
    return canvas


targets = {
    "AppList.png": (44, 44, 0.12),
    "AppList.scale-200.png": (88, 88, 0.12),
    "AppList.targetsize-24_altform-unplated.png": (24, 24, 0.0),
    "MedTile.png": (150, 150, 0.12),
    "MedTile.scale-200.png": (300, 300, 0.12),
    "StoreLogo.png": (50, 50, 0.08),
}
for name, (w, h, m) in targets.items():
    square(w, m).resize((w, h)).save(f"Assets/{name}") if w == h else None
    if w == h:
        square(w, m).save(f"Assets/{name}")

wide(310, 150).save("Assets/WideTile.png")
wide(620, 300).save("Assets/WideTile.scale-200.png")

print("done")
