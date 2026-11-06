#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Fetch the map renderer's sources at the pinned commits (scripts/maplibre/versions) into
# target/maplibre-src/, once: MapLibre Native's Qt bindings as rinigus packages them for Sailfish
# (with the Sailfish patches; Chum's qmaplibregl) and the QML item mapbox-gl-qml (Chum's
# mapboxgl-qml). Git checks every object against its hash, so a fetched tree is exactly the
# pinned one, submodules included. Runs on the host (needs git and network to github.com only).
#
# Usage: scripts/maplibre/fetch.sh
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
. "$ROOT/scripts/maplibre/versions"
SRC=$ROOT/target/maplibre-src
mkdir -p "$SRC"

fetch() { # dir url commit
  dir=$SRC/$1
  if [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD)" = "$3" ] &&
     [ -f "$dir/.fetched" ]; then
    return 0
  fi
  rm -rf "$dir"
  git init -q "$dir"
  git -C "$dir" fetch -q --depth 1 "$2" "$3"
  git -C "$dir" -c advice.detachedHead=false checkout -q FETCH_HEAD
  [ "$(git -C "$dir" rev-parse HEAD)" = "$3" ] || { echo "$1: wrong commit" >&2; exit 1; }
  # The submodules at the commits the superproject records.
  git -C "$dir" submodule update -q --init --recursive --depth 1 --jobs 4
  touch "$dir/.fetched"
}

fetch pkg-maplibre-gl-native https://github.com/rinigus/pkg-maplibre-gl-native.git \
  "$PKG_MAPLIBRE_COMMIT"
fetch mapbox-gl-qml https://github.com/rinigus/mapbox-gl-qml.git "$MAPBOX_GL_QML_COMMIT"
echo "sources in $SRC"
