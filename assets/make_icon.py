"""Draws the OpenClaw Bot icon: a lobster-red robot head with two claw antennas.

Run: python assets/make_icon.py   (needs Pillow). Writes icon.png and icon.ico.
"""
from pathlib import Path

from PIL import Image, ImageDraw

S = 2048  # drawn large, scaled down for smooth edges
OUT = Path(__file__).resolve().parent

RED_TOP = (255, 98, 78)
RED_BOTTOM = (205, 48, 40)
RED_DARK = (150, 30, 28)
FACE = (24, 28, 38)
EYE = (96, 232, 255)
EYE_GLOW = (96, 232, 255, 70)
WHITE = (255, 255, 255)


def s(v):
    return int(v * S / 1024)


def box(cx, cy, w, h):
    return [s(cx - w / 2), s(cy - h / 2), s(cx + w / 2), s(cy + h / 2)]


def gradient_mask_fill(img, mask, top, bottom):
    grad = Image.new("RGBA", img.size)
    gd = ImageDraw.Draw(grad)
    for y in range(img.size[1]):
        t = y / img.size[1]
        c = tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)) + (255,)
        gd.line([(0, y), (img.size[0], y)], fill=c)
    img.paste(grad, (0, 0), mask)


def claw(d, cx, cy, r, gap_center):
    """A pincer: a disc with a wedge cut out, opening toward gap_center (degrees, PIL clockwise from 3 o'clock)."""
    half = 24
    d.pieslice(box(cx, cy, 2 * r, 2 * r), gap_center + half, gap_center - half + 360, fill=255)


def main():
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # Red parts (head, stalks, claws, ear bolts) share one gradient.
    mask = Image.new("L", (S, S), 0)
    m = ImageDraw.Draw(mask)
    # stalks
    m.line([(s(372), s(420)), (s(318), s(250))], fill=255, width=s(46))
    m.line([(s(652), s(420)), (s(706), s(250))], fill=255, width=s(46))
    # claws, opening up and outward
    claw(m, 300, 205, 112, 252)
    claw(m, 724, 205, 112, 288)
    # ear bolts
    m.rounded_rectangle(box(170, 620, 90, 170), radius=s(40), fill=255)
    m.rounded_rectangle(box(854, 620, 90, 170), radius=s(40), fill=255)
    # head
    m.rounded_rectangle(box(512, 630, 700, 560), radius=s(170), fill=255)
    gradient_mask_fill(img, mask, RED_TOP, RED_BOTTOM)

    d = ImageDraw.Draw(img)
    # subtle shading line under the head
    d.rounded_rectangle(box(512, 630, 700, 560), radius=s(170), outline=RED_DARK + (255,), width=s(10))
    # face plate
    d.rounded_rectangle(box(512, 640, 540, 360), radius=s(120), fill=FACE + (255,))

    # eye glow
    glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    g = ImageDraw.Draw(glow)
    for cx in (412, 612):
        g.ellipse(box(cx, 605, 140, 172), fill=EYE_GLOW)
    img.alpha_composite(glow)
    d = ImageDraw.Draw(img)
    # eyes
    for cx in (412, 612):
        d.rounded_rectangle(box(cx, 605, 92, 132), radius=s(46), fill=EYE + (255,))
        d.ellipse(box(cx + 18, 578, 30, 30), fill=WHITE + (255,))
    # smile
    d.arc(box(512, 712, 140, 80), 20, 160, fill=EYE + (255,), width=s(18))

    big = img.resize((1024, 1024), Image.LANCZOS)
    big.save(OUT / "icon.png")
    big.resize((256, 256), Image.LANCZOS).save(OUT / "icon-256.png")
    big.save(OUT / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])


if __name__ == "__main__":
    main()
