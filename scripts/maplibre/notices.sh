#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Writes scripts/maplibre/NOTICES.txt from the fetched sources (scripts/maplibre/fetch.sh): the
# licences of the map renderer and of everything compiled into it. scripts/make-notices.py puts
# the file into THIRD-PARTY-NOTICES.txt (it is kept in the repository so that works without the
# sources). Rerun after changing scripts/maplibre/versions.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
. "$ROOT/scripts/maplibre/versions"
"$ROOT/scripts/maplibre/fetch.sh" >/dev/null
S=$ROOT/target/maplibre-src
Q=$S/pkg-maplibre-gl-native/maplibre-native-qt
N=$Q/vendor/maplibre-native
section() { printf '\n----------------------------------------------------------------------\n%s\n\n' "$1"; cat "$2"; }
{
  echo "The map renderer, bundled under /usr/share/harbour-threesea/lib (scripts/maplibre)."
  section "MapLibre Native Qt bindings $QMAPLIBRE_VERSION (libQMapLibre), https://github.com/maplibre/maplibre-native-qt,
packaged for Sailfish OS by rinigus, https://github.com/rinigus/pkg-maplibre-gl-native (patches: MIT)." "$Q/LICENSES/BSD-2-Clause.txt"
  section "The Sailfish OS packaging and patches of MapLibre Native Qt by rinigus" "$S/pkg-maplibre-gl-native/LICENSE"
  section "MapLibre Native, https://github.com/maplibre/maplibre-native (compiled into libQMapLibre)." "$N/LICENSE.md"
  section "Code compiled into MapLibre Native's core, as MapLibre lists it (LICENSE.mbgl-core.md):" "$N/LICENSE.mbgl-core.md"
  section "ICU (bundled in libQMapLibre for text shaping), https://icu.unicode.org" "$N/vendor/icu/LICENSE"
  section "nunicode (bundled in libQMapLibre), https://bitbucket.org/alekseyt/nunicode" "$N/vendor/nunicode/LICENSE"
  section "mapbox-gl-qml $MAPBOX_GL_QML_VERSION (the QML module MapboxMap), https://github.com/rinigus/mapbox-gl-qml,
by rinigus and contributors, GNU Lesser General Public License 3.0. Its source is at that address;
the version and how it is built are in this application's source, scripts/maplibre. The LGPL 3.0
adds to the GNU General Public License 3.0, which follows it." "$S/mapbox-gl-qml/LICENSE"
  section "GNU General Public License 3.0 (the terms the LGPL 3.0 above refers to)" "$Q/LICENSES/GPL-3.0-only.txt"
} > "$ROOT/scripts/maplibre/NOTICES.txt"
echo "wrote scripts/maplibre/NOTICES.txt"
