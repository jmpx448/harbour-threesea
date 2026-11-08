#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Looks inside built RPMs for what must not be in them (CLAUDE.md §10: check the package, don't
# assume), in the SDK image that scripts/sdk-build.sh uses (rpm2cpio, cpio and strings are there):
# - in every RPM: the licence texts (LICENSE, THIRD-PARTY-NOTICES.txt) are installed;
# - a phone build (aarch64, armv7hl; never i486, which carries the demo chats on purpose): no
#   trace of the demo data in `strings` of the binary, in the installed QML or in the .qm files;
# - with --store (STORE=1 builds, ADR 0014): no GitHub update check (its host in any file, its
#   dialog).
# Fails, naming the file and the marker, on anything found.
#
# Usage: scripts/check-package.sh [--store] RPMS/full/harbour-threesea-full-0.2.0-1.aarch64.rpm ...
set -eu
SFOS=${SFOS_VERSION:-5.1.0.11}
IMAGE=harbour-threesea-sdk:$SFOS
ROOT=$(cd "$(dirname "$0")/.." && pwd)
STORE=0
if [ "${1:-}" = --store ]; then
  STORE=1
  shift
fi
[ $# -gt 0 ] || { echo "usage: $0 [--store] <rpm> ..." >&2; exit 2; }
docker image inspect "$IMAGE" >/dev/null 2>&1 ||
  docker build --build-arg SFOS_VERSION="$SFOS" -t "$IMAGE" "$ROOT/scripts/sdk"

failed=0
for RPM in "$@"; do
  case "$RPM" in /*) ABS=$RPM ;; *) ABS=$PWD/$RPM ;; esac
  [ -f "$ABS" ] || { echo "no such file: $RPM" >&2; exit 2; }
  NAME=$(basename "$ABS")
  case "$NAME" in
    *.i486.rpm) PHONE=0 ;;
    *.armv7hl.rpm|*.aarch64.rpm) PHONE=1 ;;
    *) echo "cannot tell the architecture from $NAME" >&2; exit 2 ;;
  esac
  echo "== $NAME"
  docker run --rm -i -e PHONE="$PHONE" -e STORE="$STORE" -v "$ABS:/in/package.rpm:ro" "$IMAGE" sh -euc '
    D=$(mktemp -d)
    cd "$D"
    rpm2cpio /in/package.rpm | cpio -idm --quiet
    S=usr/share/harbour-threesea
    bad=0
    for f in LICENSE THIRD-PARTY-NOTICES.txt; do
      [ -s "$S/licenses/$f" ] || { echo "missing: /$S/licenses/$f"; bad=1; }
    done
    # Markers of the emulator-only demo data (core/src/demo.rs, the D-Bus method in bridge/).
    if [ "$PHONE" = 1 ]; then
      for marker in demoData DemoData DEMO000 THREESEA_DEMO Lindqvist; do
        if strings -a usr/bin/harbour-threesea | grep -q -F "$marker"; then
          echo "demo marker \"$marker\" in /usr/bin/harbour-threesea"; bad=1
        fi
        found=$(grep -r -l -F "$marker" "$S" || true)
        [ -z "$found" ] || { echo "demo marker \"$marker\" in: $found"; bad=1; }
      done
    fi
    if [ "$STORE" = 1 ]; then
      if strings -a usr/bin/harbour-threesea | grep -q -F api.github.com; then
        echo "update check host in /usr/bin/harbour-threesea"; bad=1
      fi
      found=$(grep -r -l -F api.github.com "$S" || true)
      [ -z "$found" ] || { echo "update check host in: $found"; bad=1; }
      [ ! -e "$S/qml/pages/UpdateCheckDialog.qml" ] || { echo "update check dialog installed"; bad=1; }
    fi
    [ "$bad" = 0 ] && echo ok
    exit "$bad"' || failed=1
done
exit $failed
