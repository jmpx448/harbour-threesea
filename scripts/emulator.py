#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Drive the Sailfish OS emulator VM (VirtualBox) for UI testing without the SDK IDE.

Start the VM with scripts/emulator-start.sh so the screen is portrait-native (768x1280); then
coordinates are simply screen pixels. If the VM came up landscape-native (1024x768, phone UI
rotated), this tool converts portrait coordinates automatically; `raw` disables that.

  emulator.py shot out.png          screenshot, rotated to portrait
  emulator.py tap X Y               tap at portrait coordinates
  emulator.py hold X Y              long press (context menus)
  emulator.py swipe X1 Y1 X2 Y2     drag (pulley menus, page back, …)
  emulator.py flick X1 Y1 X2 Y2     quick swipe without hold: opens a pulley without selecting
  emulator.py type "text"           type text via the keyboard
  emulator.py key enter|shift-enter|backspace   press a key
  emulator.py raw shot|tap|swipe …  same, in raw landscape framebuffer coordinates (use for
                                    apps that follow the emulator's landscape-native screen)

Needs VirtualBox's Python API (vboxapi). VM name: $THREESEA_VM or the default below.
"""
import os
import struct
import subprocess
import sys
import time
import zlib

VM = os.environ.get("THREESEA_VM", "Sailfish OS Emulator 5.1.0.11")


_MANAGER = None


def session():
    import vboxapi  # noqa: PLC0415 (only needed for input)

    global _MANAGER
    # vboxapi allows only one manager per process.
    if _MANAGER is None:
        _MANAGER = vboxapi.VirtualBoxManager(None, None)
    mgr = _MANAGER
    vbox = mgr.getVirtualBox()
    machine = vbox.findMachine(VM)
    sess = mgr.getSessionObject()
    machine.lockMachine(sess, mgr.constants.LockType_Shared)
    return mgr, sess


def screen_size(sess):
    w, h, _bpp, _x, _y, _status = sess.console.display.getScreenResolution(0)
    return w, h


RAW = False


def to_screen(sess, px, py):
    w, h = screen_size(sess)
    if RAW or h > w:
        return px, py
    # portrait (px, py) -> landscape framebuffer (x, y); portrait size is (h, w)
    return py, h - 1 - px


def mouse(sess, x, y, buttons):
    sess.console.mouse.putMouseEventAbsolute(int(x) + 1, int(y) + 1, 0, 0, buttons)


def tap(px, py):
    mgr, sess = session()
    try:
        x, y = to_screen(sess, px, py)
        mouse(sess, x, y, 0)
        mouse(sess, x, y, 1)
        time.sleep(0.08)
        mouse(sess, x, y, 0)
    finally:
        sess.unlockMachine()


def hold(px, py, seconds=1.2):
    mgr, sess = session()
    try:
        x, y = to_screen(sess, px, py)
        mouse(sess, x, y, 0)
        mouse(sess, x, y, 1)
        time.sleep(seconds)
        mouse(sess, x, y, 0)
    finally:
        sess.unlockMachine()


def swipe(px1, py1, px2, py2, steps=20, duration=0.4):
    mgr, sess = session()
    try:
        x1, y1 = to_screen(sess, px1, py1)
        x2, y2 = to_screen(sess, px2, py2)
        mouse(sess, x1, y1, 0)
        mouse(sess, x1, y1, 1)
        for i in range(1, steps + 1):
            mouse(sess, x1 + (x2 - x1) * i / steps, y1 + (y2 - y1) * i / steps, 1)
            time.sleep(duration / steps)
        time.sleep(0.3)  # hold, so pulley menus select the item under the finger
        mouse(sess, x2, y2, 0)
    finally:
        sess.unlockMachine()


def flick(px1, py1, px2, py2):
    """A quick swipe without the final hold: opens a pulley menu without selecting an item
    (then tap the item), unlike swipe(), which selects what is under the finger."""
    mgr, sess = session()
    try:
        x1, y1 = to_screen(sess, px1, py1)
        x2, y2 = to_screen(sess, px2, py2)
        mouse(sess, x1, y1, 0)
        mouse(sess, x1, y1, 1)
        for i in range(1, 6):
            mouse(sess, x1 + (x2 - x1) * i / 5, y1 + (y2 - y1) * i / 5, 1)
            time.sleep(0.015)
        mouse(sess, x2, y2, 0)
    finally:
        sess.unlockMachine()


def vboxmanage(*args):
    subprocess.run(["VBoxManage", "controlvm", VM, *args], check=True)


def read_png(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    pos, idat, width = 8, b"", 0
    while pos < len(data):
        length, kind = struct.unpack(">I4s", data[pos:pos + 8])
        chunk = data[pos + 8:pos + 8 + length]
        if kind == b"IHDR":
            width, height, depth, ctype = struct.unpack(">IIBB", chunk[:10])
            assert depth == 8 and ctype in (2, 6), "only 8-bit RGB/RGBA"
            bpp = 3 if ctype == 2 else 4
        elif kind == b"IDAT":
            idat += chunk
        pos += 12 + length
    raw = zlib.decompress(idat)
    stride = width * bpp
    rows, prev = [], bytearray(stride)
    for r in range(height):
        f = raw[r * (stride + 1)]
        line = bytearray(raw[r * (stride + 1) + 1:(r + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            if f == 1:
                line[i] = (line[i] + a) & 0xFF
            elif f == 2:
                line[i] = (line[i] + b) & 0xFF
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 0xFF
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 0xFF
        rows.append(line)
        prev = line
    return width, height, bpp, rows


def write_png(path, width, height, bpp, rows):
    raw = b"".join(b"\x00" + bytes(r) for r in rows)
    ctype = 2 if bpp == 3 else 6

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, ctype, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def shot(path):
    tmp = path + ".landscape.png"
    vboxmanage("screenshotpng", os.path.abspath(tmp))
    w, h, bpp, rows = read_png(tmp)
    if RAW or h > w:
        os.replace(tmp, path)
        return
    os.remove(tmp)
    # rotate 90° clockwise: portrait (px, py) = landscape (py, h-1-px)
    out = []
    for px_row in range(w):  # portrait height = w
        line = bytearray()
        for px in range(h):  # portrait width = h
            src = rows[h - 1 - px]
            line += src[px_row * bpp:(px_row + 1) * bpp]
        out.append(line)
    write_png(path, h, w, bpp, out)


KEYS = {"enter": ["1c", "9c"], "shift-enter": ["2a", "1c", "9c", "aa"], "backspace": ["0e", "8e"], "escape": ["01", "81"]}


def main(argv):
    global RAW
    if len(argv) > 1 and argv[1] == "raw":
        RAW = True
        argv = argv[:1] + argv[2:]
    cmd = argv[1] if len(argv) > 1 else "help"
    if cmd == "shot":
        shot(argv[2])
    elif cmd == "tap":
        tap(float(argv[2]), float(argv[3]))
    elif cmd == "hold":
        hold(float(argv[2]), float(argv[3]))
    elif cmd == "swipe":
        swipe(*map(float, argv[2:6]))
    elif cmd == "flick":
        flick(*map(float, argv[2:6]))
    elif cmd == "type":
        vboxmanage("keyboardputstring", argv[2])
    elif cmd == "key":
        vboxmanage("keyboardputscancode", *KEYS[argv[2]])
    else:
        print(__doc__)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
