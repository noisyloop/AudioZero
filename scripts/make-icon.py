#!/usr/bin/env python3
"""Generates resources/icon.png, the placeholder AudioZero app icon (requires Pillow)."""
import math
import pathlib
from PIL import Image, ImageDraw, ImageFilter

S = 2048  # supersampled, downscaled to 1024 at the end
root = pathlib.Path(__file__).resolve().parent.parent


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(len(a)))


# Background: rounded square with a vertical gradient
bg = Image.new("RGBA", (S, S))
top, bottom = (36, 40, 49, 255), (13, 15, 19, 255)
px = bg.load()
for y in range(S):
    c = lerp(top, bottom, y / S)
    for x in range(S):
        px[x, y] = c
mask = Image.new("L", (S, S), 0)
margin = int(S * 0.06)
ImageDraw.Draw(mask).rounded_rectangle([margin, margin, S - margin, S - margin], radius=int(S * 0.2), fill=255)
icon = Image.new("RGBA", (S, S), (0, 0, 0, 0))
icon.paste(bg, (0, 0), mask)

# Gradient ring ("zero"), orange -> pink -> teal around the circle
ring = Image.new("RGBA", (S, S), (0, 0, 0, 0))
rp = ring.load()
cx, cy = S / 2, S * 0.45
r_out, r_in = S * 0.29, S * 0.215
stops = [(255, 138, 61), (255, 93, 143), (140, 123, 255), (56, 217, 198), (255, 138, 61)]
for y in range(int(cy - r_out) - 2, int(cy + r_out) + 2):
    for x in range(int(cx - r_out) - 2, int(cx + r_out) + 2):
        d = math.hypot(x - cx, y - cy)
        if r_in <= d <= r_out:
            a = (math.atan2(y - cy, x - cx) + math.pi) / (2 * math.pi)
            seg = a * (len(stops) - 1)
            i = min(int(seg), len(stops) - 2)
            c = lerp(stops[i], stops[i + 1], seg - i)
            edge = min(1.0, (d - r_in) / 3.0, (r_out - d) / 3.0)
            rp[x, y] = (*c, int(255 * edge))
glow = ring.filter(ImageFilter.GaussianBlur(S * 0.03))
icon = Image.alpha_composite(icon, Image.composite(glow, Image.new("RGBA", (S, S)), mask))
icon = Image.alpha_composite(icon, ring)

# Waveform through the middle, like a signal passing the zero line
draw = ImageDraw.Draw(icon)
points = []
x0, x1 = S * 0.24, S * 0.76
for i in range(400):
    t = i / 399
    x = x0 + (x1 - x0) * t
    env = math.sin(math.pi * t) ** 1.5
    y = cy - math.sin(t * math.pi * 7) * S * 0.12 * env
    points.append((x, y))
# Stamp discs along the curve for a smooth, round-capped stroke.
r = S * 0.014
for (ax, ay), (bx, by) in zip(points, points[1:]):
    steps = max(1, int(math.hypot(bx - ax, by - ay) / 2))
    for k in range(steps):
        x = ax + (bx - ax) * k / steps
        y = ay + (by - ay) * k / steps
        draw.ellipse([x - r, y - r, x + r, y + r], fill=(240, 243, 248, 255))

# Arrangement clips along the bottom
clips = [(0.22, 0.42, (255, 138, 61)), (0.45, 0.6, (56, 217, 198)), (0.63, 0.78, (140, 123, 255))]
for a, b, c in clips:
    draw.rounded_rectangle([S * a, S * 0.815, S * b, S * 0.86], radius=int(S * 0.012), fill=(*c, 255))

out = root / "resources" / "icon.png"
out.parent.mkdir(exist_ok=True)
icon.resize((1024, 1024), Image.LANCZOS).save(out)
print("wrote", out)
