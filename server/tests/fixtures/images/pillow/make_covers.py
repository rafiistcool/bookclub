"""Random covers and the colour the legacy spine_color.dominant_color (Pillow) picks for each.

Run with Pillow: python -I make_covers.py <dir>. Writes <n>.png and expected.txt; the third
column of expected.txt ("exact" or "differs") was added by hand after comparing the Rust port.
"""
import colorsys, random, sys
from io import BytesIO
from pathlib import Path
from PIL import Image, ImageDraw

out = Path(sys.argv[1])
def dominant_color(data):
    image = Image.open(BytesIO(data)); width, height = image.size
    image = image.convert("RGB")
    ix, iy = int(width * 0.08), int(height * 0.08)
    image = image.crop((ix, iy, width - ix, height - iy))
    image.thumbnail((48, 48))
    q = image.quantize(colors=6, method=Image.Quantize.MEDIANCUT)
    pal = q.getpalette() or []
    total = image.width * image.height
    best = None
    for count, index in q.getcolors() or []:
        r, g, b = (c / 255 for c in pal[index * 3: index * 3 + 3])
        _, s, _ = colorsys.rgb_to_hsv(r, g, b)
        score = (count / total) * (0.35 + s)
        if best is None or score > best[0]:
            best = (score, (r, g, b))
    h, l, s = colorsys.rgb_to_hls(*best[1])
    l = min(0.86, max(0.17, l))
    r, g, b = colorsys.hls_to_rgb(h, l, s)
    return "#{:02x}{:02x}{:02x}".format(round(r * 255), round(g * 255), round(b * 255))

random.seed(7)
lines = []
for i in range(12):
    w, h = random.choice([(120, 180), (300, 450), (200, 300), (64, 96)])
    im = Image.new("RGB", (w, h), tuple(random.randrange(256) for _ in range(3)))
    d = ImageDraw.Draw(im)
    for _ in range(random.randrange(1, 6)):
        x0, y0 = random.randrange(w), random.randrange(h)
        d.rectangle((x0, y0, x0 + random.randrange(w), y0 + random.randrange(h)), fill=tuple(random.randrange(256) for _ in range(3)))
    for _ in range(random.randrange(0, 4)):
        x0, y0 = random.randrange(w), random.randrange(h); d.ellipse((x0, y0, x0 + random.randrange(1, w), y0 + random.randrange(1, h)), fill=tuple(random.randrange(256) for _ in range(3)))
    buf = BytesIO(); im.save(buf, format="PNG"); data = buf.getvalue()
    (out / f"{i}.png").write_bytes(data)
    lines.append(f"{i}.png {dominant_color(data)}")
(out / "expected.txt").write_text("\n".join(lines) + "\n")
print("\n".join(lines))
