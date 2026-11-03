#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Build installable RPMs on a Linux host that only has Docker (no Sailfish SDK installer needed).
# 1. builds the SDK+Rust image (scripts/sdk/Dockerfile) if missing,
# 2. cross-builds the Rust core per arch (scripts/build-core.sh, as the host user),
# 3. packages with mb2 inside the SDK (as the SDK user, on a private copy of the tree),
# and drops the RPMs into ./RPMS/<flavour>, named harbour-threesea-<flavour>-<version>…rpm (the
# package itself is harbour-threesea in both: one or the other is installed).
#
# Usage: scripts/sdk-build.sh [aarch64|armv7hl|i486 ...]   (default: all three)
# FLAVOUR_SUFFIX=0 keeps the plain RPM name (for a Jolla Store submission).
# FLAVOUR=full (default): background service and calls; FLAVOUR=lite: Jolla Store rules
# (ADR 0004).
set -eu
SFOS=${SFOS_VERSION:-5.1.0.11}
IMAGE=harbour-threesea-sdk:$SFOS
ROOT=$(cd "$(dirname "$0")/.." && pwd)
ARCHES=${*:-aarch64 armv7hl i486}
FLAVOUR=${FLAVOUR:-full}
case $FLAVOUR in lite|full) OUT=$ROOT/RPMS/$FLAVOUR ;; *) echo "FLAVOUR: lite|full" >&2; exit 1 ;; esac

docker image inspect "$IMAGE" >/dev/null 2>&1 ||
  docker build --build-arg SFOS_VERSION="$SFOS" -t "$IMAGE" "$ROOT/scripts/sdk"

mkdir -p "$OUT"
for ARCH in $ARCHES; do
  docker run --rm -u "$(id -u):$(id -g)" -e SFOS_VERSION="$SFOS" \
    -v "$ROOT:$ROOT" -w "$ROOT" "$IMAGE" scripts/build-core.sh "$ARCH"

  # Runs as the SDK user (sb2 targets are set up for it); the RPMs come back as a tar on stdout
  # so nothing in the source tree is written with the container's uid.
  docker run --rm -i -e ARCH="$ARCH" -e SFOS="$SFOS" -e FLAVOUR="$FLAVOUR" -v "$ROOT:/src:ro" "$IMAGE" sh -euc '
      B=$HOME/build
      mkdir -p $B/target
      # third_party: only webrtc-audio-processing is built here (Full); the rest is Rust.
      (cd /src && tar cf - --exclude=./target --exclude=./RPMS --exclude=./tools --exclude=.git \
         --exclude=./third_party/threema-android --exclude=./third_party/libthreema \
         --exclude=./third_party/opus-pure --exclude=./.idea . ) | tar xf - -C $B
      for a in /src/target/*/release/libthreesea_core.a; do
        d=$B/target/${a#/src/target/}; mkdir -p ${d%/*}; cp $a $d
      done
      if [ "$FLAVOUR" = full ]; then
        sed -i "s/^%bcond_with full$/%bcond_without full/" $B/rpm/harbour-threesea.spec
      fi
      # The demo chats come with the emulator build only (i486), like the core feature.
      if [ "$ARCH" = i486 ]; then
        sed -i "s/^%bcond_with demo$/%bcond_without demo/" $B/rpm/harbour-threesea.spec
      fi
      cd $B && mb2 -t SailfishOS-$SFOS-$ARCH build >&2
      tar cf - -C $B/RPMS .' | tar xf - -C "$OUT"
done
# The flavour into the file names (harbour-threesea-0.1.0-1.aarch64.rpm →
# harbour-threesea-full-0.1.0-1.aarch64.rpm), so both can sit side by side in a release.
# FLAVOUR_SUFFIX=0 keeps the plain name: the Jolla Store requires the file to be called
# <package>-<version>-<release>.<arch>.rpm (scripts/validate-rpm.sh says so too).
if [ "${FLAVOUR_SUFFIX:-1}" = 0 ]; then
  ls -1 "$OUT"/*.rpm
  exit 0
fi
for f in "$OUT"/harbour-threesea-[0-9]*.rpm "$OUT"/harbour-threesea-debug*-[0-9]*.rpm; do
  [ -e "$f" ] || continue
  base=${f##*/}
  mv -f "$f" "$OUT/harbour-threesea-$FLAVOUR-${base#harbour-threesea-}"
done
ls -1 "$OUT"/*.rpm
