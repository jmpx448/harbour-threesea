#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Writes THIRD-PARTY-NOTICES.txt: the licences and copyright notices of everything bundled into
the app (the Rust crates linked into the core for any of the three targets, the vendored C/C++
code, the emoji pictures). The RPM installs it with LICENSE, and About → Licences shows both.

  python3 -I scripts/make-notices.py           rewrite the file
  python3 -I scripts/make-notices.py --check   fail if it is out of date (scripts/check.sh)

The crate list comes from `cargo tree` (normal and build dependencies of threesea-core, offline),
each crate's texts from the licence files in its package. Identical texts are printed once, with
every crate that carries them. System libraries the app links dynamically (Qt, GStreamer,
OpenSSL, abseil…) come with their own packages and are not listed.
"""
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "THIRD-PARTY-NOTICES.txt")
TARGETS = ["aarch64-unknown-linux-gnu", "armv7-unknown-linux-gnueabihf", "i686-unknown-linux-gnu"]
LICENCE_FILE = re.compile(r"(?i)^(licen[cs]e|copying|notice|unlicense|copyright|attribution)")
# Texts a crate keeps below its top directory: the bundled C library's own licence.
EXTRA_FILES = {"libsqlite3-sys": ["sqlcipher/LICENSE"]}
# Ours or Threema's, under the AGPL in LICENSE.
AGPL_OWN = {"threesea-core", "libthreema", "libthreema-macros"}

RUST_STD = """The Rust standard library (std, core, alloc), linked into the core. MIT OR Apache-2.0,
https://github.com/rust-lang/rust. Copyrights are retained by their contributors.

Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE."""

TWEMOJI = """Emoji graphics: Twemoji, Copyright 2019 Twitter, Inc and other contributors,
https://github.com/jdecked/twemoji.
Licensed under the Creative Commons Attribution 4.0 International licence (CC-BY 4.0):
https://creativecommons.org/licenses/by/4.0/
The sprite sheets were put together by Threema (threema-android) from these graphics.

Emoji search terms: from threema-android by Threema AG, AGPL-3.0 (see LICENSE)."""

# The echo canceller of the Full flavour, built from the submodule and linked statically.
WEBRTC_DIR = "third_party/webrtc-audio-processing"
WEBRTC_FILES = ["COPYING", "webrtc/LICENSE", "webrtc/PATENTS",
                "webrtc/modules/third_party/fft/LICENSE", "webrtc/third_party/pffft/LICENSE",
                "webrtc/third_party/rnnoise/COPYING",
                "webrtc/common_audio/third_party/ooura/LICENSE",
                "webrtc/common_audio/third_party/spl_sqrt_floor/LICENSE"]

RULE = "=" * 78


def run(*args):
    return subprocess.run(args, cwd=ROOT, check=True, capture_output=True, text=True).stdout


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        # Line ends and trailing blanks differ between copies of the same text.
        return "\n".join(line.rstrip() for line in f.read().replace("\r\n", "\n").split("\n")).strip()


def crates():
    """(name, version) of every crate linked for any target, without the workspace's own."""
    found = set()
    for target in TARGETS:
        out = run("cargo", "tree", "--offline", "-q", "-e", "normal,build", "-p", "threesea-core",
                  "--target", target, "--prefix", "none", "-f", "{p}")
        for line in out.splitlines():
            parts = line.split()
            if len(parts) >= 2:
                found.add((parts[0], parts[1].lstrip("v")))
    return found


def main():
    check = "--check" in sys.argv[1:]
    meta = json.loads(run("cargo", "metadata", "--offline", "-q", "--format-version", "1"))
    packages = {(p["name"], p["version"]): p for p in meta["packages"]}

    rows = []  # (name, version, licence, where)
    texts = {}  # text -> [crate names]
    for name, version in sorted(crates()):
        if name in AGPL_OWN:
            continue
        p = packages[(name, version)]
        d = os.path.dirname(p["manifest_path"])
        where = p.get("repository") or ""
        if p.get("source") is None:  # a path dependency: vendored here
            where = os.path.relpath(d, ROOT)
        rows.append((name, version, p.get("license") or "see its licence file", where))
        files = sorted(f for f in os.listdir(d) if LICENCE_FILE.match(f)
                       and os.path.isfile(os.path.join(d, f)))
        files += EXTRA_FILES.get(name, [])
        if not files:
            sys.exit(f"make-notices: {name} {version} ships no licence file")
        for f in files:
            texts.setdefault(read(os.path.join(d, f)), []).append(f"{name} {version} ({f})")

    out = []
    out.append("Threesea: third-party notices")
    out.append("")
    out.append("Threesea is free software under the GNU Affero General Public License, version 3")
    out.append("(AGPL-3.0-only), in the file LICENSE next to this one. Its source code:")
    out.append("https://github.com/penttimus/harbour-threesea")
    out.append("It contains libthreema and code adapted from threema-android, both by Threema AG")
    out.append("(https://github.com/threema-ch/threema-android), under the same licence.")
    out.append("")
    out.append("It also bundles the components below. Their licences and copyright notices follow.")
    out.append("This file is generated by scripts/make-notices.py.")
    out.append("")
    out.append(RULE)
    out.append("Rust crates linked into the app's core (crate, version, licence, source)")
    out.append(RULE)
    for name, version, licence, where in rows:
        out.append(f"{name} {version}: {licence}; {where}")
    out.append("")
    out.append(RULE)
    out.append(RUST_STD)
    out.append("")
    out.append(RULE)
    out.append(TWEMOJI)
    out.append("")
    out.append(RULE)
    out.append("webrtc-audio-processing (Full flavour: echo cancellation in calls), from Google's WebRTC,")
    out.append("https://gitlab.freedesktop.org/pulseaudio/webrtc-audio-processing")
    for f in WEBRTC_FILES:
        out.append("")
        out.append(f"--- {f}")
        out.append("")
        out.append(read(os.path.join(ROOT, WEBRTC_DIR, f)))
    out.append("")
    out.append(RULE)
    out.append("Licence texts of the Rust crates")
    for text, users in texts.items():
        out.append("")
        out.append(RULE)
        out.append("Used by: " + ", ".join(users))
        out.append(RULE)
        out.append("")
        out.append(text)
    content = "\n".join(out) + "\n"

    if check:
        try:
            current = open(OUT, encoding="utf-8").read()
        except FileNotFoundError:
            current = ""
        if current != content:
            sys.exit("THIRD-PARTY-NOTICES.txt is out of date: python3 -I scripts/make-notices.py")
        return
    with open(OUT, "w", encoding="utf-8") as f:
        f.write(content)


if __name__ == "__main__":
    main()
