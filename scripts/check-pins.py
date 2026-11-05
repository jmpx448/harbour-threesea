#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""CLAUDE.md §7.5: the TLS pins in core/src/https.rs are upstream's, domain by domain, in the
Android source at the pinned tag. A new upstream tag with other pins fails here, until they are
copied. Exit status 1 on a difference. Without the submodule checked out there is nothing to
compare with: skipped, except on CI, where it must be there. Another copy of upstream's file can
be named as the argument."""
import os
import re
import sys
import xml.etree.ElementTree as ElementTree

UPSTREAM = "third_party/threema-android/app/src/main/res/xml/network_security_config.xml"
OURS = "core/src/https.rs"


def upstream_pins(path):
    pins = {}
    for config in ElementTree.parse(path).getroot().findall("domain-config"):
        found = {pin.text.strip() for pin in config.iterfind("pin-set/pin")}
        for domain in config.findall("domain"):
            if domain.get("includeSubdomains") != "true":
                sys.exit(f"{path}: {domain.text} without its subdomains: we have no such rule")
            pins[domain.text.strip()] = found
    return pins


def our_pins(path):
    with open(path, encoding="utf-8") as file:
        source = file.read()
    table = re.search(r"const PINS: [^=]*= &\[(.*?)\n\];", source, re.DOTALL)
    if not table:
        sys.exit(f"{path}: no PINS table")
    entries = re.findall(r'\(\s*"([^"]+)",\s*&\[(.*?)\],\s*\)', table.group(1), re.DOTALL)
    if not entries:
        sys.exit(f"{path}: an empty PINS table")
    return {domain: set(re.findall(r'"([^"]+)"', pins)) for domain, pins in entries}


def main():
    source = sys.argv[1] if len(sys.argv) > 1 else UPSTREAM
    if not os.path.exists(source):
        print(f"{source} not there: skipped")
        return 1 if os.environ.get("CI") else 0
    upstream, ours = upstream_pins(source), our_pins(OURS)
    differences = []
    for domain in sorted(upstream.keys() | ours.keys()):
        theirs, mine = upstream.get(domain), ours.get(domain)
        if theirs is None:
            differences.append(f"{domain}: not upstream")
        elif mine is None:
            differences.append(f"{domain}: only upstream")
        elif theirs != mine:
            differences.append(
                f"{domain}: only upstream {sorted(theirs - mine)}, only here {sorted(mine - theirs)}"
            )
    for difference in differences:
        print(difference)
    if differences:
        print(f"The pins in {OURS} differ from {source}")
        return 1
    print(f"{sum(len(pins) for pins in ours.values())} pins of {len(ours)} domains as upstream")
    return 0


if __name__ == "__main__":
    sys.exit(main())
