"""Generate the skeuomorphic textures for the GPUI front end.

birch.jpg  - pale birch desk (cathedral grain, low contrast, like Smartisan Notes)
walnut.jpg - walnut strip for the title/status bars
paper.jpg  - cream writing paper with faint fibres
"""
import sys
import numpy as np
from PIL import Image, ImageFilter

OUT = sys.argv[1]
rng = np.random.default_rng(7)


def value_noise(h, w, cell_y, cell_x, seed):
    """Smooth value noise with cells of (cell_y, cell_x) pixels."""
    r = np.random.default_rng(seed)
    gh, gw = int(h / cell_y) + 3, int(w / cell_x) + 3
    grid = r.random((gh, gw))
    y = np.arange(h) / cell_y
    x = np.arange(w) / cell_x
    y0 = np.floor(y).astype(int); x0 = np.floor(x).astype(int)
    fy = y - y0; fx = x - x0
    sy = fy * fy * (3 - 2 * fy); sx = fx * fx * (3 - 2 * fx)
    a = grid[y0][:, x0]; b = grid[y0][:, x0 + 1]
    c = grid[y0 + 1][:, x0]; d = grid[y0 + 1][:, x0 + 1]
    top = a + (b - a) * sx[None, :]
    bot = c + (d - c) * sx[None, :]
    return top + (bot - top) * sy[:, None]


def fbm(h, w, cy, cx, octaves, seed, gain=0.5):
    total = np.zeros((h, w)); amp = 1.0; norm = 0
    for o in range(octaves):
        total += amp * value_noise(h, w, cy / 2**o, cx / 2**o, seed + o)
        norm += amp; amp *= gain
    return total / norm


def lerp_color(c0, c1, t):
    c0 = np.array(c0, float); c1 = np.array(c1, float)
    return c0 + (c1 - c0) * t[..., None]


def hexc(s):
    return tuple(int(s[i:i + 2], 16) for i in (1, 3, 5))


def birch(w=1600, h=1200):
    """Flat-sawn birch: growth rings are cylinders around a pith running along
    y; the board plane is slightly tilted, so ring contours close into the
    arched "cathedral" figures Smartisan Notes' desk shows."""
    y, x = np.mgrid[0:h, 0:w].astype(float)
    warp = (fbm(h, w, 500, 220, 4, 11) - 0.5) * 90
    wobble = (fbm(h, w, 60, 25, 3, 21) - 0.5) * 5
    def cathedral(cx, depth, tilt, phase):
        dist = depth + tilt * (y - h / 2)
        return np.sqrt((x - cx + warp) ** 2 + dist ** 2) + phase
    # Two boards glued side by side, blended softly at the seam.
    # Narrow, tall flames: small tilt so arches stretch along the grain.
    r1 = cathedral(330, 150, -0.07, 0)
    r2 = cathedral(900, 120, 0.06, 9)
    r3 = cathedral(1380, 170, -0.05, 4)
    s1 = 1 / (1 + np.exp(-(x - 620 - warp * 0.5) / 30))
    s2 = 1 / (1 + np.exp(-(x - 1150 - warp * 0.5) / 30))
    r = r1 * (1 - s1) + (r2 * (1 - s2) + r3 * s2) * s1 + wobble
    rings = (r / 21.0) % 1.0
    late = np.exp(-((1 - rings) / 0.07) ** 2) + 0.6 * np.exp(-(rings / 0.035) ** 2)
    early = 0.5 + 0.5 * np.cos(rings * 2 * np.pi)          # broad soft bands
    fine = fbm(h, w, 300, 2.6, 3, 31)
    blotch = fbm(h, w, 420, 420, 3, 51)
    t = 0.20 * late + 0.08 * early + 0.30 * fine + 0.30 * (blotch - 0.5) + 0.20
    t = t.clip(0, 1)
    light, dark = hexc('#f6e5c8'), hexc('#e3c697')
    img = lerp_color(light, dark, t)
    return Image.fromarray(img.clip(0, 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(1.1))

def walnut(w=1600, h=160):
    y, x = np.mgrid[0:h, 0:w].astype(float)
    warp = (fbm(h, w, 60, 500, 3, 61) - 0.5) * 30
    rings = ((y * 3.0 + warp) / 7.0) % 1.0
    late = np.exp(-((1 - rings) / 0.18) ** 2)
    fine = fbm(h, w, 1.4, 180, 3, 71)                        # brushed horizontal fibres
    t = (0.25 * late + 0.45 * fine + 0.15 * fbm(h, w, 80, 400, 2, 81)).clip(0, 1)
    img = lerp_color(hexc('#7c6a59'), hexc('#6a584a'), t)
    return Image.fromarray(img.clip(0, 255).astype(np.uint8))


def paper(w=1024, h=1024):
    base = np.array(hexc('#fbf7ec'), float)
    n = fbm(h, w, 6, 6, 3, 91) - 0.5
    fibres = fbm(h, w, 1.5, 40, 2, 101) - 0.5
    blot = fbm(h, w, 300, 300, 3, 111) - 0.5
    shade = 5.0 * n + 3.0 * fibres + 4.0 * blot
    img = base[None, None, :] + shade[..., None] * np.array([1.0, 0.95, 0.85])
    return Image.fromarray(img.clip(0, 255).astype(np.uint8))


birch().save(f'{OUT}/birch.jpg', quality=88, optimize=True)
walnut().save(f'{OUT}/walnut.jpg', quality=90, optimize=True)
paper().save(f'{OUT}/paper.jpg', quality=90, optimize=True)
print('ok')
