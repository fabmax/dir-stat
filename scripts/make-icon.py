#!/usr/bin/env python3
"""Draws the placeholder app icon (a rounded square with three size bars) as a 1024 px PNG.

No image library needed: pixels are written straight into a PNG with zlib.
Usage: make-icon.py OUT.png
"""
import struct
import sys
import zlib

SIZE = 1024
ACCENT = (0x2A, 0x78, 0xD6)
TRACK = (0x5A, 0x98, 0xE2)
WHITE = (0xFF, 0xFF, 0xFF)

# macOS icon grid: the shape is 824 px with a ~185 px corner radius, centered.
MARGIN, RADIUS = 100, 185
BARS = [(0.80, 330), (0.55, 470), (0.32, 610)]  # (length, top y)
BAR_H, BAR_X, BAR_W = 90, 230, 564


def inside_rounded(x, y):
    lo, hi = MARGIN, SIZE - MARGIN
    if not (lo <= x < hi and lo <= y < hi):
        return 0.0
    cx = min(max(x, lo + RADIUS), hi - RADIUS)
    cy = min(max(y, lo + RADIUS), hi - RADIUS)
    d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
    return max(0.0, min(1.0, RADIUS - d + 0.5))


def pixel(x, y):
    a = inside_rounded(x + 0.5, y + 0.5)
    if a == 0.0:
        return (0, 0, 0, 0)
    color = ACCENT
    for length, top in BARS:
        if top <= y < top + BAR_H and BAR_X <= x < BAR_X + BAR_W:
            color = WHITE if x < BAR_X + BAR_W * length else TRACK
    return (*color, round(a * 255))


def png(path):
    raw = bytearray()
    for y in range(SIZE):
        raw.append(0)
        for x in range(SIZE):
            raw.extend(pixel(x, y))

    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF))

    header = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
                + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))


if __name__ == "__main__":
    png(sys.argv[1])
