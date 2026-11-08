#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Builds tests/secretstore with the Sailfish SDK (i486) and runs it there (sb2, no device or
# secrets daemon needed: the test brings its own).
set -eu
SFOS=${SFOS_VERSION:-5.1.0.11}
IMAGE=harbour-threesea-sdk:$SFOS
ROOT=$(cd "$(dirname "$0")/.." && pwd)
docker run --rm -i -e SFOS="$SFOS" -v "$ROOT:/src:ro" "$IMAGE" sh -euc '
    T=SailfishOS-$SFOS-i486
    B=$HOME/build; mkdir -p $B/tests $B/bridge $B/core
    cp -r /src/tests/secretstore $B/tests/; cp /src/bridge/secretstore.* $B/bridge/
    cp -r /src/core/include $B/core/
    cd $B/tests/secretstore && sb2 -t $T qmake >/dev/null && sb2 -t $T make -s >/dev/null 2>&1 || sb2 -t $T make
    QT_QPA_PLATFORM=minimal sb2 -t $T ./secretstore-test'
