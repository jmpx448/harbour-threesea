#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-only
# Refresh translations/*.ts from the QML sources (id-based, Jolla style: qsTrId + //% comments)
# with the SDK's lupdate, so translators work on files in the repo. Run after changing UI texts,
# then scripts/translations-de.py and scripts/translations-fr.py (always last: they fill the
# translations and write the files in the repository's layout).
# Runs as the SDK user on a copy (sb2 targets are set up for it) and streams the .ts files back.
set -eu
SFOS=${SFOS_VERSION:-5.1.0.11}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
docker run --rm -e SFOS="$SFOS" -v "$ROOT:/src:ro" "harbour-threesea-sdk:$SFOS" sh -euc '
    W=$HOME/tr && mkdir -p $W && cp -a /src/qml /src/translations $W/ && cd $W
    sb2 -t SailfishOS-$SFOS-i486 lupdate -noobsolete -locations none qml \
      -ts translations/harbour-threesea.ts translations/harbour-threesea-de.ts \
      translations/harbour-threesea-fr.ts >&2
    tar cf - translations' 2> >(grep -v "bus connection" >&2) | tar xf - -C "$ROOT"
