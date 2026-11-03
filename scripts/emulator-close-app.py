#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Close the first app on the emulator's home screen like a user: home, long-press its cover,
tap the close button. Dev helper for testing "keep running in the background"."""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import emulator as e  # noqa: E402

e.swipe(2, 640, 600, 640)  # to home
time.sleep(2)
mgr, sess = e.session()
try:
    x, y = e.to_screen(sess, 200, 350)
    e.mouse(sess, x, y, 0)
    e.mouse(sess, x, y, 1)
    time.sleep(1.5)
    e.mouse(sess, x, y, 0)
finally:
    sess.unlockMachine()
time.sleep(2)
e.tap(204, 593)  # close button of the first cover
time.sleep(3)
e.tap(600, 1000)  # leave the close mode
