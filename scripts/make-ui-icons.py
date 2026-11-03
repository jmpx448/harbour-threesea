#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Render the app's own monochrome icons where the theme has none that fits (the emoji panel's
tabs and button, the delivery states, our QR code), as white PNGs in qml/icons/. Drawn like Sailfish's theme icons: thin
outlines with round caps on an 80-unit canvas, the theme's stroke, one element at the theme's
reduced opacity. Silica's Icon tints them with the palette like theme icons. Needs pycairo."""
import math
from pathlib import Path

import cairo

OUT = Path(__file__).resolve().parent.parent / "qml" / "icons"
SIZE = 128
STROKE = 2.4
PI = math.pi


def dim(c):
    """The theme's secondary element: white at alpha 153 (as in icon-m-clock's ring)."""
    c.set_source_rgba(1, 1, 1, 0.6)


def full(c):
    c.set_source_rgba(1, 1, 1, 1)


def rounded_rect(c, x, y, w, h, r):
    c.new_sub_path()
    c.arc(x + w - r, y + r, r, -PI / 2, 0)
    c.arc(x + w - r, y + h - r, r, 0, PI / 2)
    c.arc(x + r, y + h - r, r, PI / 2, PI)
    c.arc(x + r, y + r, r, PI, 1.5 * PI)
    c.close_path()


def smiley(c):
    dim(c)
    c.arc(40, 40, 28, 0, 2 * PI)
    c.stroke()
    full(c)
    for x in (30, 50):
        c.arc(x, 33, 3.2, 0, 2 * PI)
        c.fill()
    c.arc(40, 40, 16, 0.18 * PI, 0.82 * PI)
    c.stroke()


def nature(c):
    # A leaf with its vein and stem.
    dim(c)
    c.move_to(24, 56)
    c.curve_to(14, 30, 36, 12, 64, 14)
    c.curve_to(68, 42, 50, 66, 24, 56)
    c.close_path()
    c.stroke()
    full(c)
    c.move_to(14, 66)
    c.line_to(24, 56)
    c.line_to(50, 30)
    c.stroke()


def food(c):
    # A burger: the buns, the patty between them.
    dim(c)
    c.arc(40, 38, 24, PI, 2 * PI)
    c.close_path()
    c.stroke()
    rounded_rect(c, 16, 54, 48, 11, 4)
    c.stroke()
    full(c)
    c.move_to(14, 47)
    for i in range(6):
        x = 14 + i * 52 / 6
        c.curve_to(x + 2.2, 43, x + 6.5, 43, x + 52 / 6, 47)
    c.stroke()


def objects(c):
    # A light bulb.
    dim(c)
    c.arc(40, 32, 19, 0.72 * PI, 2.28 * PI)
    c.line_to(48, 52)
    c.line_to(32, 52)
    c.close_path()
    c.stroke()
    full(c)
    c.move_to(32, 59)
    c.line_to(48, 59)
    c.move_to(35, 66)
    c.line_to(45, 66)
    c.stroke()


def symbols(c):
    # A heart with a shine.
    full(c)
    c.move_to(40, 66)
    c.curve_to(8, 44, 10, 15, 27, 15)
    c.curve_to(35, 15, 39, 21, 40, 25)
    c.curve_to(41, 21, 45, 15, 53, 15)
    c.curve_to(70, 15, 72, 44, 40, 66)
    c.close_path()
    c.stroke()
    dim(c)
    c.arc(28, 30, 7, 1.05 * PI, 1.5 * PI)
    c.stroke()


def flags(c):
    full(c)
    c.move_to(20, 68)
    c.line_to(20, 12)
    c.stroke()
    dim(c)
    c.move_to(20, 16)
    c.curve_to(32, 9, 46, 23, 62, 15)
    c.line_to(62, 43)
    c.curve_to(46, 51, 32, 37, 20, 44)
    c.stroke()


def gif(c):
    dim(c)
    rounded_rect(c, 9, 20, 62, 40, 6)
    c.stroke()
    full(c)
    # G
    c.arc_negative(28, 40, 9, -0.25 * PI, 0)
    c.line_to(30, 40)
    c.stroke()
    # I
    c.move_to(44, 31)
    c.line_to(44, 49)
    c.stroke()
    # F
    c.move_to(52, 49)
    c.line_to(52, 31)
    c.line_to(62, 31)
    c.move_to(52, 40)
    c.line_to(60, 40)
    c.stroke()


def notifications(c):
    # The theme's icon-m-notifications (a chat bubble with a sparkle) is drawn entirely at the
    # reduced opacity and looks washed out beside the other settings icons; ours gives the
    # sparkle the full-opacity element every theme icon has (user 2026-10-27).
    dim(c)
    x0, y0, x1, y1, r = 5, 14, 68, 61, 22
    c.new_sub_path()
    c.arc(x1 - r, y0 + r, r, -PI / 2, 0)
    c.line_to(x1, 69)  # the tail, down to its tip and back to the bottom edge
    c.line_to(58, y1)
    c.arc(x0 + r, y1 - r, r, PI / 2, PI)
    c.arc(x0 + r, y0 + r, r, PI, 1.5 * PI)
    c.close_path()
    c.stroke()
    full(c)
    sparkle(c, 36, 37, 13, 2.8)
    c.fill()


def sparkle(c, cx, cy, r, w):
    """A four-pointed sparkle: rays of length r pinched to a waist of w."""
    c.move_to(cx, cy - r)
    c.curve_to(cx + w, cy - w, cx + w, cy - w, cx + r, cy)
    c.curve_to(cx + w, cy + w, cx + w, cy + w, cx, cy + r)
    c.curve_to(cx - w, cy + w, cx - w, cy + w, cx - r, cy)
    c.curve_to(cx - w, cy - w, cx - w, cy - w, cx, cy - r)
    c.close_path()


def qr_code(c):
    # A QR code reduced to its three corner squares; a few data modules at the reduced opacity.
    full(c)
    for x, y in ((12, 12), (46, 12), (12, 46)):
        rounded_rect(c, x, y, 22, 22, 3)
        c.stroke()
        rounded_rect(c, x + 7, y + 7, 8, 8, 1.5)
        c.fill()
    dim(c)
    for x, y in ((46, 46), (60, 46), (53, 53), (46, 60), (60, 60)):
        rounded_rect(c, x, y, 8, 8, 1.5)
        c.fill()


def cut(c):
    """Cut what follows out of what is drawn (transparent)."""
    c.set_operator(cairo.OPERATOR_CLEAR)


def paint(c):
    c.set_operator(cairo.OPERATOR_OVER)
    full(c)


# Delivery states: the same meanings as Threema's (clock, envelope, inbox, eye), drawn by us, filled
# so they read at the small size; the details cut out, one part at the theme's reduced opacity.
def pending(c):
    paint(c)
    c.arc(40, 40, 30, 0, 2 * PI)
    c.fill()
    cut(c)
    c.set_line_width(6)
    c.move_to(40, 22)
    c.line_to(40, 41)
    c.line_to(53, 49)
    c.stroke()


def sent(c):
    paint(c)
    rounded_rect(c, 8, 16, 64, 48, 6)
    c.fill()
    cut(c)
    c.set_line_width(6)
    c.move_to(14, 22)
    c.line_to(40, 43)
    c.line_to(66, 22)
    c.stroke()


def delivered(c):
    paint(c)
    rounded_rect(c, 8, 12, 64, 56, 6)
    c.fill()
    cut(c)
    # The open space above the tray, the tray's notch.
    c.move_to(16, 20)
    c.line_to(64, 20)
    c.line_to(64, 46)
    c.line_to(52, 46)
    c.curve_to(50, 53, 30, 53, 28, 46)
    c.line_to(16, 46)
    c.close_path()
    c.fill()
    paint(c)
    dim(c)
    c.move_to(22, 28)
    c.line_to(58, 28)
    c.move_to(22, 37)
    c.line_to(58, 37)
    c.set_line_width(5)
    c.stroke()


def read(c):
    paint(c)
    c.move_to(4, 40)
    c.curve_to(20, 12, 60, 12, 76, 40)
    c.curve_to(60, 68, 20, 68, 4, 40)
    c.close_path()
    c.fill()
    cut(c)
    c.arc(40, 40, 15, 0, 2 * PI)
    c.fill()
    paint(c)
    dim(c)
    c.arc(40, 40, 9, 0, 2 * PI)
    c.fill()


ICONS = {
    "emoji-smileys": smiley,
    "emoji-nature": nature,
    "emoji-food": food,
    "emoji-objects": objects,
    "emoji-symbols": symbols,
    "emoji-flags": flags,
    "emoji-gif": gif,
    "notifications": notifications,
    "qr-code": qr_code,
    "delivery-pending": pending,
    "delivery-sent": sent,
    "delivery-delivered": delivered,
    "delivery-read": read,
}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name, draw in ICONS.items():
        surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, SIZE, SIZE)
        c = cairo.Context(surface)
        c.scale(SIZE / 80, SIZE / 80)
        c.set_line_width(STROKE)
        c.set_line_cap(cairo.LINE_CAP_ROUND)
        c.set_line_join(cairo.LINE_JOIN_ROUND)
        draw(c)
        surface.write_to_png(str(OUT / f"{name}.png"))

if __name__ == "__main__":
    main()
