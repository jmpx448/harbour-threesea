#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Builds tests/notificationtokens with the Sailfish SDK (i486) and runs it there (sb2, no device
# needed).
set -eu
SFOS=${SFOS_VERSION:-5.1.0.11}
IMAGE=harbour-threesea-sdk:$SFOS
ROOT=$(cd "$(dirname "$0")/.." && pwd)
docker run --rm -i -e SFOS="$SFOS" -v "$ROOT:/src:ro" "$IMAGE" sh -euc '
    T=SailfishOS-$SFOS-i486
    B=$HOME/build; mkdir -p $B/tests $B/bridge $B/qml/components
    cp -r /src/tests/notificationtokens $B/tests/; cp /src/bridge/dbusinterface.* $B/bridge/
    cp /src/qml/components/NotificationActions.js $B/qml/components/
    cd $B/tests/notificationtokens && sb2 -t $T qmake >/dev/null && sb2 -t $T make -s >/dev/null 2>&1 || sb2 -t $T make
    sb2 -t $T ./notificationtokens-test'
