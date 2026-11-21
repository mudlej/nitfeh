#!/usr/bin/env python3
"""Renders the app icon (the scalloped badge with the waveform) into assets/: PNG, ICO and ICNS.

Needs Pillow. Run through `just icons`; the results are committed, so a normal build does not need this.
"""
import math
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
PRIMARY = (103, 80, 164, 255)
ON_PRIMARY = (255, 255, 255, 255)
LOBES = 9
# The waveform of ui/icons/wave.svg: (x, top, bottom) on a 24 unit grid, drawn with round ends.
BARS = [(4, 10, 14), (8, 6.5, 17.5), (12, 3.5, 20.5), (16, 7.5, 16.5), (20, 10.5, 13.5)]


def render(size: int) -> Image.Image:
    scale = 4
    full = size * scale
    image = Image.new("RGBA", (full, full), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    centre = full / 2
    points = []
    for i in range(720):
        angle = math.tau * i / 720
        radius = centre * 0.98 * (0.93 + 0.07 * math.cos(LOBES * angle))
        points.append((centre + radius * math.cos(angle), centre + radius * math.sin(angle)))
    draw.polygon(points, fill=PRIMARY)

    glyph = full * 0.5
    unit = glyph / 24
    origin = (full - glyph) / 2
    half = unit * 1.1
    for x, top, bottom in BARS:
        cx = origin + x * unit
        box = (cx - half, origin + top * unit - half, cx + half, origin + bottom * unit + half)
        draw.rounded_rectangle(box, radius=half, fill=ON_PRIMARY)

    return image.resize((size, size), Image.LANCZOS)


def main() -> int:
    assets = ROOT / "assets"
    (assets / "windows").mkdir(parents=True, exist_ok=True)
    (assets / "macos").mkdir(parents=True, exist_ok=True)

    render(256).save(assets / "icon.png")
    render(256).save(assets / "windows" / "nitfeh.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
    render(1024).save(assets / "macos" / "nitfeh.icns")
    print("wrote assets/icon.png, assets/windows/nitfeh.ico, assets/macos/nitfeh.icns")
    return 0


if __name__ == "__main__":
    sys.exit(main())
