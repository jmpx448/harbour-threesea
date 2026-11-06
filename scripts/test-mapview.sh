#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# The bundled map renderer (scripts/maplibre/build.sh) on a real Silica page in the SDK (i486, sb2;
# no device, no network): tests/mapview draws an offline fixture style with Mesa's llvmpipe into a
# tiny Wayland compositor that writes the frame to target/mapview.png. Fails on any QML warning,
# if nothing was drawn or the map area stayed one colour.
# Usage: scripts/test-mapview.sh   (the frame comes back on the container's stdout)
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
SFOS=${SFOS_VERSION:-5.1.0.11}
LIB=$ROOT/target/maplibre/$SFOS-i486
[ -f "$LIB/.stamp" ] || "$ROOT/scripts/maplibre/build.sh" i486
docker run --rm -i -v "$ROOT/tests/mapview:/src:ro" -v "$LIB:/maplibre:ro" "harbour-threesea-sdk:$SFOS" sh -euc '
    T=SailfishOS-'"$SFOS"'-i486
    # Software GL for the test only (the device has its own driver).
    sb2 -t $T -m sdk-install -R zypper --non-interactive in mesa-llvmpipe-dri-swrast-driver \
        "pkgconfig(wayland-server)" >&2
    B=$HOME/mapview && mkdir -p $B && cp /src/* $B/ && cd $B
    # Laid out as in the package (the module finds the library through its RUNPATH), inside the
    # home directory, where sb2 shows it to the target'"'"'s programs.
    mkdir -p $B/lib && cp -a /maplibre/core/usr/lib/libQMapLibre.so.3* $B/lib/ && cp -a /maplibre/qml $B/lib/qml
    sb2 -t $T gcc -O1 -o shotcompositor shotcompositor.c $(sb2 -t $T pkg-config --cflags --libs wayland-server) >&2
    sb2 -t $T qmake >&2 && sb2 -t $T make -s >&2
    export XDG_RUNTIME_DIR=$B/run && mkdir -p -m 700 $XDG_RUNTIME_DIR
    sb2 -t $T ./shotcompositor $B/shot.ppm 25 > socket 2>compositor.log &
    while [ ! -s socket ]; do sleep 0.2; done
    status=0
    WAYLAND_DISPLAY=$(cat socket) QT_QPA_PLATFORM=wayland-egl EGL_PLATFORM=wayland LIBGL_ALWAYS_SOFTWARE=1 \
      QT_WAYLAND_DISABLE_WINDOWDECORATION=1 \
      sb2 -t $T ./mapview-test $B $B/lib/qml 20 >&2 || status=$?
    wait || true
    cat compositor.log >&2
    [ -f shot.ppm ] && cat shot.ppm || true
    exit $status' > "$ROOT/target/mapview.ppm"
python3 -I "$ROOT/tests/mapview/ppm2png.py" "$ROOT/target/mapview.ppm" "$ROOT/target/mapview.png"
