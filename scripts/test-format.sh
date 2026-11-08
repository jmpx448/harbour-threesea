#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Builds tests/format with the Sailfish SDK (i486) and runs it there (sb2, no device needed).
set -eu
SFOS=${SFOS_VERSION:-5.1.0.11}
IMAGE=harbour-threesea-sdk:$SFOS
ROOT=$(cd "$(dirname "$0")/.." && pwd)
docker run --rm -i -e SFOS="$SFOS" -v "$ROOT:/src:ro" "$IMAGE" sh -euc '
    T=SailfishOS-$SFOS-i486
    B=$HOME/build; mkdir -p $B/tests $B/qml $B/emojis
    cp -r /src/tests/format $B/tests/; cp -r /src/qml/components $B/qml/; cp -r /src/emojis/search $B/emojis/
    cd $B/tests/format && sb2 -t $T qmake >/dev/null && sb2 -t $T make -s >/dev/null 2>&1 || sb2 -t $T make
    QT_QPA_PLATFORM=minimal QT_LOGGING_TO_CONSOLE=1 sb2 -t $T ./format-test'
