#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Render the launcher icon as PNGs for the sizes Sailfish needs (icons/<size>/harbour-threesea.png).

The shape of Jolla's launcher icons (a disc with one nearly square corner, here the bottom left),
filling the canvas like theirs: a flat pale green speech bubble with three dots over three waves
(the "sea"), the sky and the sea in gradients of green like Jolla's backgrounds. Our own mark, on purpose not Threema's (a dark
disc with a white bubble and bright green dots), which we may not use (ADR 0015). Kept simple so it
reads in a large app grid. Needs pycairo."""
import math
import sys
from pathlib import Path

import cairo

SIZES = (86, 108, 128, 172)
# Drawn on a 100-unit canvas.


def rgb(colour):
    return tuple(int(colour[i : i + 2], 16) / 255 for i in (1, 3, 5))


def gradient(x0, y0, x1, y1, *colours):
    result = cairo.LinearGradient(x0, y0, x1, y1)
    for i, colour in enumerate(colours):
        result.add_color_stop_rgb(i / (len(colours) - 1), *rgb(colour))
    return result


def shape(c, x, y, w, h, r, corner):
    """A rectangle with corners of radius r, the bottom left one nearly square (radius `corner`)."""
    c.new_sub_path()
    c.arc(x + w - r, y + r, r, -math.pi / 2, 0)
    c.arc(x + w - r, y + h - r, r, 0, math.pi / 2)
    c.arc(x + corner, y + h - corner, corner, math.pi / 2, math.pi)
    c.arc(x + r, y + r, r, math.pi, 1.5 * math.pi)
    c.close_path()


def wave(c, y, amplitude, phase, crest, deep):
    """Everything under a gentle wave (cut by the shape): bright at the crest on the left, deeper
    towards the bottom right."""
    c.move_to(-5, y + amplitude * math.sin(phase - 5 / 100 * 1.25 * 2 * math.pi))
    for x in range(-5, 106):
        c.line_to(x, y + amplitude * math.sin(x / 100 * 1.25 * 2 * math.pi + phase))
    c.line_to(105, 105)
    c.line_to(-5, 105)
    c.close_path()
    c.set_source(gradient(0, y - amplitude, 100, y + amplitude + 13, crest, deep))
    c.fill()


def draw(c):
    shape(c, 0, 0, 100, 100, 50, 2.5)
    c.save()
    c.clip()
    # Lime to teal like the gradients on Jolla's icons, with a soft light from the top left.
    c.set_source(gradient(5, 0, 95, 75, "#C6F27A", "#6AD670", "#14A87E"))
    c.paint()
    light = cairo.RadialGradient(16, 6, 0, 16, 6, 66)
    light.add_color_stop_rgba(0, 1, 1, 0.82, 0.38)
    light.add_color_stop_rgba(1, 1, 1, 0.82, 0)
    c.set_source(light)
    c.paint()

    # The bubble, its nearly square corner on the same side as the shape's, over a translucent
    # copy of itself (Jolla's icons layer shapes, they have no blurred shadows).
    x, y, w, h = 23, 11, 54, 44
    shape(c, x + 1.2, y + 2.2, w, h, 18, 1.6)
    c.set_source_rgba(0.05, 0.35, 0.2, 0.20)
    c.fill()
    shape(c, x, y, w, h, 18, 1.6)
    c.set_source_rgb(*rgb("#D8F5C6"))
    c.fill()
    c.set_source_rgb(*rgb("#176B45"))
    for dot in (37.5, 50, 62.5):
        c.arc(dot, 33, 4.3, 0, 2 * math.pi)
        c.fill()

    wave(c, 63, 4.2, 0.6, "#52DA86", "#0F9C74")
    wave(c, 75, 4.2, 2.5, "#2CBC72", "#097F68")
    wave(c, 88, 3.8, 4.4, "#159A66", "#045C50")
    c.restore()


def render(size):
    surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, size, size)
    c = cairo.Context(surface)
    c.scale(size / 100, size / 100)
    draw(c)
    return surface


out = Path(sys.argv[1] if len(sys.argv) > 1 else "icons")
for s in SIZES:
    d = out / f"{s}x{s}"
    d.mkdir(parents=True, exist_ok=True)
    render(s).write_to_png(str(d / "harbour-threesea.png"))
