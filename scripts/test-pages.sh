#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Every page and dialog under qml/pages opened once (tests/pageload): the real window on the real
# Silica, without a window system, in the Sailfish SDK (i486, sb2; no device needed), for both
# flavours, whose C++ side differs. One page per process. Any warning from Qt or the QML engine is a
# failure. The fake C++ side is generated from the bridge's headers (tests/pageload/make-fakes.py).
# A last round, laid out as in a real window, checks that a list nobody touched rests at its top.
# Usage: scripts/test-pages.sh [page under qml/, e.g. pages/ChatPage.qml ...]
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
SFOS=${SFOS_VERSION:-5.1.0.11}
docker run --rm -i -e "PAGES=$*" -v "$ROOT:/src:ro" "harbour-threesea-sdk:$SFOS" sh -euc '
    T=SailfishOS-'"$SFOS"'-i486
    B=$HOME/pageload
    mkdir -p $B/bridge
    cp /src/tests/pageload/main.cpp /src/tests/pageload/pageload-test.pro $B/
    cp /src/bridge/jsonlistmodel.* /src/bridge/contactlistmodel.* /src/bridge/enterkeyfilter.* \
       /src/bridge/emojihighlighter.* $B/bridge/
    cp -r /src/qml $B/
    cd $B
    sb2 -t $T qmake >/dev/null && sb2 -t $T make -s >/dev/null 2>&1 || sb2 -t $T make
    # The texts of the //% comments: without them %1 and %n would not be there to fill in.
    sb2 -t $T lupdate -silent qml -ts texts.ts >/dev/null 2>&1
    sb2 -t $T lrelease -silent -idbased texts.ts -qm texts.qm >/dev/null 2>&1
    if [ -z "$PAGES" ]; then
        PAGES=$(cd qml && find pages -name "*.qml" | sort)
    fi
    failed=0
    for flavour in full lite; do
        A=$B/$flavour
        mkdir -p $A/fakes
        ln -sfn $B/qml $A/qml
        cp texts.qm $A/
        cp /src/tests/pageload/Driver.qml /src/tests/pageload/Pages.js /src/tests/pageload/Results.js $A/fakes/
        python3 -I /src/tests/pageload/make-fakes.py /src/bridge $flavour $A/fakes
        python3 -I /src/tests/pageload/make-fakes.py --check /src/bridge $flavour /src/qml || failed=1
        mkdir -p $A/out
        # Side by side: each takes about two seconds of mostly waiting.
        printf "%s\n" $PAGES | xargs -P 6 -I PAGE sh -c "
            out=$A/out/\$(echo PAGE | tr / _)
            if QT_QPA_PLATFORM=minimal QT_LOGGING_TO_CONSOLE=1 sb2 -t $T ./pageload-test $A $flavour PAGE >\$out 2>&1; then
                rm \$out
            fi" || true
        for page in $PAGES; do
            out=$A/out/$(echo $page | tr / _)
            if [ -f $out ]; then
                echo "FAIL $flavour $page"
                sort $out | uniq -c | sort -rn | cut -c1-300 | head -20
                failed=1
            else
                echo "ok   $flavour $page"
            fi
        done
        # Once more laid out, for where the lists come to rest (one flavour is enough).
        [ $flavour = full ] || continue
        mkdir -p $A/rest
        printf "%s\n" $PAGES | xargs -P 6 -I PAGE sh -c "
            out=$A/rest/\$(echo PAGE | tr / _)
            if QT_QPA_PLATFORM=minimal QT_LOGGING_TO_CONSOLE=1 sb2 -t $T ./pageload-test $A $flavour PAGE layout >\$out 2>&1; then
                rm \$out
            fi" || true
        for page in $PAGES; do
            out=$A/rest/$(echo $page | tr / _)
            if [ -f $out ]; then
                echo "FAIL laid out $page"
                sort $out | uniq -c | sort -rn | cut -c1-300 | head -20
                failed=1
            fi
        done
    done
    exit $failed'
