#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Run Jolla's Harbour validator (rpmvalidation, the Jolla Store's own check) on a built RPM,
# inside the SDK image that scripts/sdk-build.sh uses. Fails if the validation fails.
#
# Usage: scripts/validate-rpm.sh RPMS/lite/harbour-threesea-0.1.0-1.i486.rpm
#
# The Lite flavour is the one the Jolla Store takes (Harbour rules: no system services, only
# allowed libraries). Build it with the plain file name the validator insists on:
#   FLAVOUR=lite FLAVOUR_SUFFIX=0 scripts/sdk-build.sh aarch64
set -eu
RPM=${1:?usage: $0 <rpm-file>}
SFOS=${SFOS_VERSION:-5.1.0.11}
IMAGE=harbour-threesea-sdk:$SFOS
ROOT=$(cd "$(dirname "$0")/.." && pwd)
case "$RPM" in
  /*) ABS=$RPM ;;
  *)  ABS=$PWD/$RPM ;;
esac
[ -f "$ABS" ] || { echo "no such file: $RPM" >&2; exit 2; }
NAME=$(basename "$ABS")
case "$NAME" in
  *.i486.rpm)    ARCH=i486 ;;
  *.armv7hl.rpm) ARCH=armv7hl ;;
  *.aarch64.rpm) ARCH=aarch64 ;;
  *) echo "cannot tell the architecture from $NAME" >&2; exit 2 ;;
esac
docker image inspect "$IMAGE" >/dev/null 2>&1 ||
  docker build --build-arg SFOS_VERSION="$SFOS" -t "$IMAGE" "$ROOT/scripts/sdk"
# The validator reads the file from the SDK user's home (the sb2 target cannot see a bind mount).
docker run --rm -i -e NAME="$NAME" -e SFOS="$SFOS" -e ARCH="$ARCH" \
  -v "$ABS:/in/$NAME:ro" "$IMAGE" sh -euc '
    cp "/in/$NAME" "$HOME/" && cd "$HOME" &&
    rpmvalidation -t "SailfishOS-$SFOS-$ARCH" "$NAME"'
