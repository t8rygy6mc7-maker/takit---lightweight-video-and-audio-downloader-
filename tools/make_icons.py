#!/usr/bin/env python3
"""Generate Takit's icon set (PNG/ICO/ICNS) using only the standard library.

Run from the repo root:  python3 tools/make_icons.py

Produces a 1024px source (app-icon.png) plus the platform icons Tauri bundles
in src-tauri/icons/. CI can regenerate the set with `cargo tauri icon` from the
source PNG, but committing them lets `cargo build` work out of the box.
"""

import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
ICON_DIR = os.path.join(ROOT, "src-tauri", "icons")

# Colours
TOP = (45, 212, 191)     # teal  (accent-2)
BOTTOM = (79, 107, 237)  # indigo (accent)
GLYPH = (250, 250, 252)


def lerp(a, b, t):
    return a + (b - a) * t


def render(size):
    """Render the icon at `size` px, returning raw RGBA bytes."""
    ss = 4 if size <= 64 else (3 if size <= 256 else 2)
    inv = 1.0 / size
    sub = [(i + 0.5) / ss for i in range(ss)]
    n = ss * ss

    # Background rounded-rect (signed-distance) parameters, normalised 0..1.
    m = 0.085
    r = 0.20
    left, right, top, bottom = m, 1 - m, m, 1 - m
    cx, cy = 0.5, 0.5
    hw, hh = (right - left) / 2, (bottom - top) / 2

    out = bytearray(size * size * 4)
    pos = 0
    for py in range(size):
        for px in range(size):
            pr = pg = pb = pa = 0.0
            for sy in sub:
                y = (py + sy) * inv
                for sx in sub:
                    x = (px + sx) * inv

                    # Rounded-box SDF membership.
                    qx = abs(x - cx) - (hw - r)
                    qy = abs(y - cy) - (hh - r)
                    d = math.hypot(max(qx, 0.0), max(qy, 0.0)) + min(max(qx, qy), 0.0) - r
                    if d > 0:
                        continue  # transparent outside the badge

                    # Download glyph (white): stem + arrowhead + tray.
                    in_stem = 0.45 <= x <= 0.55 and 0.30 <= y <= 0.52
                    in_arrow = False
                    if 0.50 <= y <= 0.71:
                        half = 0.17 * (1.0 - (y - 0.50) / 0.21)
                        in_arrow = abs(x - 0.50) <= half
                    in_tray = 0.29 <= x <= 0.71 and 0.745 <= y <= 0.80

                    if in_stem or in_arrow or in_tray:
                        pr += GLYPH[0]; pg += GLYPH[1]; pb += GLYPH[2]; pa += 1.0
                    else:
                        t = min(max((y - m) / (1 - 2 * m), 0.0), 1.0)
                        pr += lerp(TOP[0], BOTTOM[0], t)
                        pg += lerp(TOP[1], BOTTOM[1], t)
                        pb += lerp(TOP[2], BOTTOM[2], t)
                        pa += 1.0

            cov = pa / n
            a = int(round(cov * 255))
            if cov > 0:
                out[pos] = max(0, min(255, int(round((pr / n) / cov))))
                out[pos + 1] = max(0, min(255, int(round((pg / n) / cov))))
                out[pos + 2] = max(0, min(255, int(round((pb / n) / cov))))
                out[pos + 3] = a
            pos += 4
    return bytes(out)


def png_bytes(size, rgba):
    stride = size * 4
    raw = bytearray()
    for y in range(size):
        raw.append(0)  # filter: none
        raw.extend(rgba[y * stride:(y + 1) * stride])
    comp = zlib.compress(bytes(raw), 9)

    def chunk(typ, data):
        return (struct.pack(">I", len(data)) + typ + data +
                struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF))

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)  # 8-bit RGBA
    return sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", comp) + chunk(b"IEND", b"")


def ico_bytes(pngs):
    """pngs: dict size -> png bytes. Build a PNG-compressed .ico."""
    sizes = [16, 32, 48, 64, 128, 256]
    n = len(sizes)
    header = struct.pack("<HHH", 0, 1, n)
    entries = b""
    data = b""
    offset = 6 + 16 * n
    for s in sizes:
        png = pngs[s]
        b = s if s < 256 else 0
        entries += struct.pack("<BBBBHHII", b & 0xFF, b & 0xFF, 0, 0, 1, 32, len(png), offset)
        data += png
        offset += len(png)
    return header + entries + data


def icns_bytes(pngs):
    """pngs: dict size -> png bytes. Build a PNG-based .icns."""
    mapping = [
        (b"ic07", 128), (b"ic08", 256), (b"ic09", 512),
        (b"ic11", 32), (b"ic12", 64), (b"ic13", 256), (b"ic14", 512),
    ]
    body = b""
    for typ, s in mapping:
        png = pngs[s]
        body += typ + struct.pack(">I", 8 + len(png)) + png
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def main():
    os.makedirs(ICON_DIR, exist_ok=True)
    needed = [16, 32, 48, 64, 128, 256, 512, 1024]
    print("Rendering icon at sizes:", needed)
    rendered = {}
    pngs = {}
    for s in needed:
        rendered[s] = render(s)
        pngs[s] = png_bytes(s, rendered[s])
        print(f"  {s}px ok")

    # Source for CI (`cargo tauri icon`).
    with open(os.path.join(ROOT, "app-icon.png"), "wb") as f:
        f.write(pngs[1024])

    # Platform icons referenced by tauri.conf.json.
    out = {
        "32x32.png": pngs[32],
        "128x128.png": pngs[128],
        "128x128@2x.png": pngs[256],
        "icon.png": pngs[512],
        "icon.ico": ico_bytes(pngs),
        "icon.icns": icns_bytes(pngs),
    }
    for name, data in out.items():
        with open(os.path.join(ICON_DIR, name), "wb") as f:
            f.write(data)
        print(f"  wrote icons/{name} ({len(data)} bytes)")

    print("Done.")


if __name__ == "__main__":
    main()
