#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# The app lock and the page stack (tests/lockstate): the real window on the real Silica, without
# a window system, in the Sailfish SDK (i486, sb2; no device needed). One scenario per process.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
SFOS=${SFOS_VERSION:-5.1.0.11}
docker run --rm -i -v "$ROOT:/src:ro" "harbour-threesea-sdk:$SFOS" sh -euc '
    T=SailfishOS-'"$SFOS"'-i486
    B=$HOME/lockstate
    mkdir -p $B/app/pages $B/app/imports
    # The real window, its components and the pages under test; plain pages for the rest.
    cp /src/qml/harbour-threesea.qml $B/app/
    cp -r /src/qml/components /src/qml/cover $B/app/
    cp /src/tests/lockstate/pages/*.qml $B/app/pages/
    for page in ConfirmSendDialog ChatPickerDialog CallPage GroupCallPage DataBackupImportPage StartupPage PinSetupPage CameraPage ScanQrPage ImageViewerPage EditPhotoPage PhotoTextDialog VideoMetadataDialog; do cp /src/qml/pages/$page.qml $B/app/pages/; done
    # Under their own names, beside the plain ones the window and these two open.
    cp /src/qml/pages/UnlockPage.qml $B/app/pages/RealUnlockPage.qml
    cp /src/qml/pages/SharePage.qml $B/app/pages/RealSharePage.qml
    cp -r /src/tests/lockstate/harbour $B/app/imports/
    cp /src/tests/lockstate/Fakes.qml /src/tests/lockstate/Scenarios.qml $B/app/
    cp /src/tests/lockstate/main.cpp /src/tests/lockstate/lockstate-test.pro $B/
    cd $B
    sb2 -t $T qmake >/dev/null && sb2 -t $T make -s >/dev/null 2>&1 || sb2 -t $T make
    run() {
        QT_QPA_PLATFORM=minimal QT_LOGGING_TO_CONSOLE=1 sb2 -t $T ./lockstate-test $B/app "$1" 2>&1
    }
    failed=0
    for scenario in $(run --list | sed -n "s/.*SCENARIOS //p"); do
        if output=$(run "$scenario"); then
            echo "ok   $scenario"
        else
            echo "FAIL $scenario"
            echo "$output" | grep -v "silica.core" | cut -c1-300
            failed=1
        fi
    done
    exit $failed'
