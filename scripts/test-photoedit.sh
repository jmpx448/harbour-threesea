#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Builds tests/photoedit with the Sailfish SDK (i486) and runs it there (sb2, no device needed).
set -eu
SFOS=${SFOS_VERSION:-5.1.0.11}
IMAGE=harbour-threesea-sdk:$SFOS
ROOT=$(cd "$(dirname "$0")/.." && pwd)
docker run --rm -i -e SFOS="$SFOS" -v "$ROOT:/src:ro" "$IMAGE" sh -euc '
    T=SailfishOS-$SFOS-i486
    B=$HOME/build; mkdir -p $B/tests $B/bridge
    cp -r /src/tests/photoedit $B/tests/
    cp /src/bridge/photoedit.* /src/bridge/filepreparer.* /src/bridge/filetype.h $B/bridge/
    cd $B/tests/photoedit && sb2 -t $T qmake >/dev/null && sb2 -t $T make -s >/dev/null 2>&1 || sb2 -t $T make
    sb2 -t $T ./photoedit-test -platform minimal:enable_fonts'
