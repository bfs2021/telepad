"""Draw the Telepad app icon and write every format the build uses.

    python scripts/make-icon.py

Writes crates/telepad-gui/assets/icon/: telepad.png (256 px, window icon and
README), telepad-1024.png (master), telepad.ico (Windows exe and installer)
and telepad.icns (macOS app). Needs Pillow. The design is plain shapes, so no
fonts or third-party artwork are involved: a rounded square with a blue
gradient, a white "T", and a green "online" dot.
"""

import os
import sys

from PIL import Image, ImageDraw

SIZE = 1024
SUPER = 4  # draw larger, then shrink: smooth edges without anti-aliasing support
TOP = (56, 189, 248)  # sky
BOTTOM = (30, 64, 175)  # deep blue
WHITE = (255, 255, 255, 255)
GREEN = (34, 197, 94, 255)

OUT = os.path.join(os.path.dirname(__file__), '..', 'crates', 'telepad-gui', 'assets', 'icon')


def master() -> Image.Image:
    s = SIZE * SUPER
    img = Image.new('RGBA', (s, s), (0, 0, 0, 0))

    # Rounded square with a vertical gradient (8 % margin, like platform icons).
    m = int(s * 0.08)
    radius = int(s * 0.21)
    grad = Image.new('RGBA', (s, s))
    gd = ImageDraw.Draw(grad)
    for y in range(s):
        t = y / (s - 1)
        c = tuple(round(TOP[i] + (BOTTOM[i] - TOP[i]) * t) for i in range(3))
        gd.line([(0, y), (s, y)], fill=c + (255,))
    mask = Image.new('L', (s, s), 0)
    ImageDraw.Draw(mask).rounded_rectangle([m, m, s - m, s - m], radius=radius, fill=255)
    img.paste(grad, (0, 0), mask)

    d = ImageDraw.Draw(img)
    # The "T": a top bar and a stem, both with rounded ends.
    r = int(s * 0.045)
    d.rounded_rectangle([int(s * 0.25), int(s * 0.25), int(s * 0.75), int(s * 0.38)], radius=r, fill=WHITE)
    d.rounded_rectangle([int(s * 0.435), int(s * 0.25), int(s * 0.565), int(s * 0.76)], radius=r, fill=WHITE)

    # Green "online" dot with a ring in the gradient's bottom colour, bottom right.
    cx, cy, rad = int(s * 0.735), int(s * 0.705), int(s * 0.105)
    ring = int(s * 0.035)
    d.ellipse([cx - rad - ring, cy - rad - ring, cx + rad + ring, cy + rad + ring], fill=BOTTOM + (255,))
    d.ellipse([cx - rad, cy - rad, cx + rad, cy + rad], fill=GREEN)

    return img.resize((SIZE, SIZE), Image.LANCZOS)


def main() -> int:
    os.makedirs(OUT, exist_ok=True)
    big = master()
    big.save(os.path.join(OUT, 'telepad-1024.png'), optimize=True)
    big.resize((256, 256), Image.LANCZOS).save(os.path.join(OUT, 'telepad.png'), optimize=True)
    big.save(
        os.path.join(OUT, 'telepad.ico'),
        sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    big.save(os.path.join(OUT, 'telepad.icns'))
    for name in sorted(os.listdir(OUT)):
        print(f'{name:18} {os.path.getsize(os.path.join(OUT, name)):>8} bytes')
    return 0


if __name__ == '__main__':
    sys.exit(main())
