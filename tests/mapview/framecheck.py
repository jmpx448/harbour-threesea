# SPDX-License-Identifier: AGPL-3.0-only
"""Checks a frame of the test compositor (binary PPM) for scripts/test-mapview.sh.

  python3 -I framecheck.py <frame.ppm> drawn         the map area holds more than a few colours
  python3 -I framecheck.py <frame.ppm> has RRGGBB    that colour fills a good part of the map area
  python3 -I framecheck.py <frame.ppm> lacks RRGGBB  that colour is nowhere
"""
import sys


def main(path, check, colour=None):
    data = open(path, "rb").read()
    parts = data.split(b"\n", 3)
    if len(parts) < 4 or parts[0] != b"P6":
        sys.exit(f"{path}: no frame")
    w, h = map(int, parts[1].split())
    pixels = parts[3]
    # The map area: below the page header, above the attribution.
    sample = [pixels[(y * w + x) * 3:(y * w + x) * 3 + 3]
              for y in range(h // 4, 3 * h // 4, 4) for x in range(0, w, 4)]
    if check == "drawn":
        ok = len(set(sample)) >= 4
    else:
        wanted = bytes.fromhex(colour)
        share = sum(1 for p in sample if p == wanted) / len(sample)
        ok = share > 0.5 if check == "has" else share == 0
    print(f"{path}: {check} {colour or ''}: {'ok' if ok else 'FAILED'}")
    return ok


sys.exit(0 if main(*sys.argv[1:]) else 1)
