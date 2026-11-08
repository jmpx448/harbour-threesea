# SPDX-License-Identifier: AGPL-3.0-only
"""Turns the test compositor's frame (binary PPM) into a PNG and checks that the map drew:
the middle of the picture must hold more than a handful of colours. Standard library only."""
import struct
import sys
import zlib


def main(src, dst):
    data = open(src, "rb").read()
    parts = data.split(b"\n", 3)
    if len(parts) < 4 or parts[0] != b"P6":
        sys.exit("no frame")
    w, h = map(int, parts[1].split())
    pixels = parts[3]
    rows = b"".join(b"\0" + pixels[y * w * 3:(y + 1) * w * 3] for y in range(h))

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b"")
    open(dst, "wb").write(png)
    middle = {pixels[(y * w + x) * 3:(y * w + x) * 3 + 3]
              for y in range(h // 3, 2 * h // 3, 4) for x in range(0, w, 4)}
    print(f"{dst}: {w}x{h}, {len(middle)} colours in the middle third")
    if len(middle) < 4:
        sys.exit("the map area is blank")


main(sys.argv[1], sys.argv[2])
