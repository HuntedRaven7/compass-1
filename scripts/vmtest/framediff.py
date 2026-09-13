#!/usr/bin/env python3
"""Compare two screenshots and say where they differ.

Written because corral's luminance standard deviation cannot answer the one
question this tier most needs answered — "did a window appear" — and pretending
otherwise cost three runs and a retracted conclusion. Five frames across two
jobs all reported deviation 0.1564 to four decimal places while showing
visibly different things, including one with a 640x480 launcher in it.

Pixels can answer it. This decodes PNG without Pillow, which is not available
on the runner and is not worth a dependency for two hundred lines of zlib and
an unfilter loop.

Usage:
    framediff.py BEFORE AFTER [--min-percent P] [--expect-box X0 Y0 X1 Y1]

Exit status is 0 when every requested assertion holds, 1 when one does not, and
2 when the files cannot be read. With no assertions it only reports, which is
how it should be used until there is a measurement to set them from.
"""

from __future__ import annotations

import argparse
import struct
import sys
import zlib


def load(path: str) -> tuple[int, int, bytes]:
    """Decode a PNG to 8-bit RGB rows. Handles the filter types corral emits."""
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG")
    pos, idat = 8, b""
    width = height = depth = color = 0
    while pos < len(data):
        length = struct.unpack(">I", data[pos : pos + 4])[0]
        kind = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            width, height, depth, color = struct.unpack(">IIBB", body[:10])
        elif kind == b"IDAT":
            idat += body
        pos += 12 + length
    if depth != 8 or color not in (2, 6):
        raise ValueError(f"{path}: unsupported depth {depth} / colour type {color}")

    channels = 3 if color == 2 else 4
    raw = zlib.decompress(idat)
    stride = width * channels
    out = bytearray()
    prev = bytearray(stride)
    i = 0
    for _ in range(height):
        filter_type = raw[i]
        i += 1
        line = bytearray(raw[i : i + stride])
        i += stride
        for x in range(stride):
            a = line[x - channels] if x >= channels else 0
            b = prev[x]
            c = prev[x - channels] if x >= channels else 0
            if filter_type == 1:
                line[x] = (line[x] + a) & 0xFF
            elif filter_type == 2:
                line[x] = (line[x] + b) & 0xFF
            elif filter_type == 3:
                line[x] = (line[x] + (a + b) // 2) & 0xFF
            elif filter_type == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[x] = (line[x] + pred) & 0xFF
        out += line
        prev = line
    return width, height, bytes(out), channels


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("before")
    ap.add_argument("after")
    ap.add_argument("--min-percent", type=float, default=None,
                    help="fail unless at least this percent of pixels changed")
    ap.add_argument("--expect-box", nargs=4, type=int, metavar=("X0", "Y0", "X1", "Y1"),
                    default=None,
                    help="fail unless the changed region lies within this box")
    args = ap.parse_args()

    try:
        w, h, a, ch = load(args.before)
        w2, h2, b, ch2 = load(args.after)
    except (OSError, ValueError) as err:
        print(f"framediff: {err}", file=sys.stderr)
        return 2
    if (w, h) != (w2, h2):
        print(f"framediff: sizes differ: {w}x{h} vs {w2}x{h2}", file=sys.stderr)
        return 2

    minx, maxx, miny, maxy, n = w, -1, h, -1, 0
    for y in range(h):
        row = y * w
        for x in range(w):
            k = (row + x) * ch
            if a[k : k + 3] != b[k : k + 3]:
                n += 1
                if x < minx: minx = x
                if x > maxx: maxx = x
                if y < miny: miny = y
                if y > maxy: maxy = y

    total = w * h
    pct = 100.0 * n / total
    if n == 0:
        print(f"IDENTICAL  {args.before} and {args.after} ({w}x{h})")
    else:
        print(f"{n} of {total} pixels differ ({pct:.2f}%)  "
              f"box x {minx}..{maxx} ({maxx - minx + 1}w) y {miny}..{maxy} ({maxy - miny + 1}h)")

    ok = True
    if args.min_percent is not None:
        if pct < args.min_percent:
            print(f"FAIL: only {pct:.2f}% of pixels changed, expected at least "
                  f"{args.min_percent:.2f}%", file=sys.stderr)
            ok = False
        else:
            print(f"ok: {pct:.2f}% >= {args.min_percent:.2f}%")
    if args.expect_box is not None:
        x0, y0, x1, y1 = args.expect_box
        if n == 0:
            print(f"FAIL: nothing changed, so nothing is inside {x0},{y0}..{x1},{y1}",
                  file=sys.stderr)
            ok = False
        elif not (minx >= x0 and maxx <= x1 and miny >= y0 and maxy <= y1):
            print(f"FAIL: changed region x {minx}..{maxx} y {miny}..{maxy} is not "
                  f"inside {x0},{y0}..{x1},{y1}", file=sys.stderr)
            ok = False
        else:
            print(f"ok: changed region is inside {x0},{y0}..{x1},{y1}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
