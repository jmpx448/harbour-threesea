#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Builds tests/gifdecoder with the Sailfish SDK (i486) and runs it there (sb2, no device needed).
# The target has no GIF plugin (phones and the emulator do): it goes into this throwaway container,
# which takes network access to Jolla's repositories.
set -eu
SFOS=${SFOS_VERSION:-5.1.0.11}
IMAGE=harbour-threesea-sdk:$SFOS
ROOT=$(cd "$(dirname "$0")/.." && pwd)
docker run --rm -i -e SFOS="$SFOS" -v "$ROOT:/src:ro" "$IMAGE" sh -euc '
    T=SailfishOS-$SFOS-i486
    sb2 -t $T -m sdk-install -R zypper --non-interactive in qt5-plugin-imageformat-gif >/dev/null
    B=$HOME/build; mkdir -p $B/tests $B/bridge
    cp -r /src/tests/gifdecoder $B/tests/; cp /src/bridge/gifdecoder.* /src/bridge/peerimage.h $B/bridge/
    cd $B/tests/gifdecoder && sb2 -t $T qmake >/dev/null && sb2 -t $T make -s >/dev/null 2>&1 || sb2 -t $T make
    sb2 -t $T ./gifdecoder-test'
