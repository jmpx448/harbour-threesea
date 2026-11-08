#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Build the map renderer for one architecture, once, into target/maplibre/<sfos>-<arch>/:
# libQMapLibre.so.3 (MapLibre Native's Qt bindings) and the QML module MapboxMap
# (mapbox-gl-qml), both bundled privately under /usr/share/harbour-threesea/lib like Pure Maps'
# Jolla Store build does. A finished build is kept (stamp files with the pins and options), so
# scripts/sdk-build.sh only pays for it the first time; delete the directory to rebuild.
#
# Usage: scripts/maplibre/build.sh <aarch64|armv7hl|i486>   (host; uses the SDK Docker image)
set -eu
ARCH=${1:?usage: $0 <aarch64|armv7hl|i486>}
SFOS=${SFOS_VERSION:-5.1.0.11}
IMAGE=harbour-threesea-sdk:$SFOS
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
. "$ROOT/scripts/maplibre/versions"
OUT=$ROOT/target/maplibre/$SFOS-$ARCH
FLAGS="-O2 -g0 -fstack-protector-strong -D_FORTIFY_SOURCE=2"
CORE_OPTS="-DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr -DMLN_QT_WITH_WIDGETS=OFF \
  -DMLN_QT_WITH_LOCATION=OFF -DMLN_QT_WITH_INTERNAL_ICU=ON -DMLN_WITH_WERROR=OFF -DCMAKE_SKIP_RPATH=ON"
# The plugin finds libQMapLibre beside it: lib/qml/MapboxMap/ -> lib/.
QML_OPTS="-DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr -DUSE_CURL_SSL=ON -DQT_INSTALL_QML=/qml \
  -DCMAKE_INSTALL_RPATH=\$ORIGIN/../.."
# Two parts, each kept while its pins and options stay: the library (several minutes) and the
# QML module (seconds).
CORE_STAMP="$QMAPLIBRE_VERSION $PKG_MAPLIBRE_COMMIT $FLAGS $CORE_OPTS $(cat "$ROOT"/scripts/maplibre/patches/*.patch | sha256sum | cut -c1-16)"
QML_STAMP="$MAPBOX_GL_QML_VERSION $MAPBOX_GL_QML_COMMIT $FLAGS $QML_OPTS $CORE_STAMP $(cat "$ROOT"/scripts/maplibre/patches/mapbox-gl-qml/*.patch | sha256sum | cut -c1-16)"
fresh() { [ -f "$OUT/$1/.stamp" ] && [ "$(cat "$OUT/$1/.stamp")" = "$2" ]; }
if fresh core "$CORE_STAMP" && fresh qml "$QML_STAMP"; then
  echo "map renderer for $ARCH is up to date ($OUT)"
  exit 0
fi
"$ROOT/scripts/maplibre/fetch.sh"
docker image inspect "$IMAGE" >/dev/null 2>&1 ||
  docker build --build-arg SFOS_VERSION="$SFOS" -t "$IMAGE" "$ROOT/scripts/sdk"

# Each part as the SDK user, on a private copy (the sources stay read-only); the result comes back
# as a tar on stdout, like scripts/sdk-build.sh does with the RPMs.
sdk() { # part script
  docker run --rm -i -e ARCH="$ARCH" -e SFOS="$SFOS" -e JOBS="${CARGO_BUILD_JOBS:-4}" \
    -e FLAGS="$FLAGS" -e CORE_OPTS="$CORE_OPTS" -e QML_OPTS="$QML_OPTS" \
    -v "$ROOT/target/maplibre-src:/src:ro" -v "$ROOT/scripts/maplibre/patches:/patches:ro" -v "$OUT/core:/core:ro" "$IMAGE" sh -euc '
    T=SailfishOS-$SFOS-$ARCH
    # What the Chum packages build with (rinigus spec files), into the target only.
    sb2 -t $T -m sdk-install -R zypper --non-interactive in cmake gcc-c++ \
      "pkgconfig(Qt5Core)" "pkgconfig(Qt5Gui)" "pkgconfig(Qt5Sql)" "pkgconfig(Qt5Network)" \
      "pkgconfig(Qt5Qml)" "pkgconfig(Qt5Quick)" "pkgconfig(Qt5Positioning)" "pkgconfig(Qt5Svg)" \
      "pkgconfig(sqlite3)" "pkgconfig(zlib)" "pkgconfig(libcurl)" "pkgconfig(openssl)" \
      "pkgconfig(glesv2)" >&2
    S=$HOME/src B=$HOME/build D=$HOME/stage
    mkdir -p $S $B $D'"
    $2"
}

rm -rf "$OUT/new" && mkdir -p "$OUT/new" "$OUT/core"
if ! fresh core "$CORE_STAMP"; then
  sdk core '
    # Without what only other platforms and the benchmarks use (the test directory stays:
    # CMake reads it).
    (cd /src/pkg-maplibre-gl-native && tar cf - --exclude=.git \
       --exclude=./maplibre-native-qt/vendor/maplibre-native/metrics \
       --exclude=./maplibre-native-qt/vendor/maplibre-native/platform/android \
       --exclude=./maplibre-native-qt/vendor/maplibre-native/platform/ios \
       --exclude=./maplibre-native-qt/vendor/maplibre-native/platform/darwin \
       --exclude=./maplibre-native-qt/vendor/maplibre-native/platform/windows .) | tar xf - -C $S
    Q=$S/maplibre-native-qt
    # The Sailfish patches in the order of rinigus spec file, then ours (scripts/maplibre/patches).
    (cd $Q && patch -s -p1 < ../rpm/0001-Use-CURL-for-downloads.patch &&
              patch -s -p1 < ../rpm/0002-Fixes-for-compilation-on-SFOS.patch)
    (cd $Q/vendor/maplibre-native && patch -s -p1 < ../../../rpm/0003-CURL_POLL_INOUT-action-handle-added-2365.patch &&
              patch -s -p1 < ../../../rpm/0004-fix-double-move-callback.patch &&
              for p in /patches/0*.patch; do patch -s -p1 < $p || exit 1; done)
    cd $B
    sb2 -t $T cmake $Q $CORE_OPTS -DCMAKE_C_FLAGS="$FLAGS" -DCMAKE_CXX_FLAGS="$FLAGS" >&2
    sb2 -t $T make -j$JOBS >&2
    sb2 -t $T make install DESTDIR=$D >&2
    tar cf - -C $D .' | tar xf - -C "$OUT/new"
  rm -rf "$OUT/core" && mv "$OUT/new" "$OUT/core" && echo "$CORE_STAMP" > "$OUT/core/.stamp"
  mkdir -p "$OUT/new"
fi
sdk qml '
    cp -a /src/mapbox-gl-qml $S/ && cp -a /core/usr $D/core-usr
    (cd $S/mapbox-gl-qml && for p in /patches/mapbox-gl-qml/*.patch; do patch -s -p1 < $p || exit 1; done)
    cd $B
    # QMapLibre exports its directories as the prefix'"'"'s, which only /usr has by default.
    L=$(dirname $(find $D/core-usr -name "libQMapLibre.so" | head -1))
    sb2 -t $T cmake $S/mapbox-gl-qml $QML_OPTS -DCMAKE_C_FLAGS="$FLAGS" \
      -DCMAKE_CXX_FLAGS="$FLAGS -I$D/core-usr/include" -DCMAKE_SHARED_LINKER_FLAGS="-L$L" \
      -DQMapLibre_DIR=$(dirname $(find $D/core-usr -name QMapLibreConfig.cmake)) >&2
    sb2 -t $T make -j$JOBS >&2
    sb2 -t $T make install DESTDIR=$D/out >&2
    tar cf - -C $D/out/qml .' | tar xf - -C "$OUT/new"
rm -rf "$OUT/qml" && mv "$OUT/new" "$OUT/qml" && echo "$QML_STAMP" > "$OUT/qml/.stamp"
ls -l "$OUT"/core/usr/lib*/libQMapLibre.so* "$OUT"/qml/MapboxMap
