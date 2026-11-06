# 0032: Threema's map, drawn by a bundled MapLibre with the core's pins

Supersedes the "no map" of ADR 0030; its rules for places stay.

## Context

A point can't be picked by hand, nearby places can't be seen around it, and a received location
only opens in another app. Android draws Threema's map (`map.threema.ch`, OpenMapTiles' "Bright"
style) with MapLibre. Checked once (2026-11-06): the style, its TileJSON, the vector tiles
(zoom 0-14), glyphs and sprite are all on `map.threema.ch`; there are no raster tiles. Qt Location
5.4 on the device has no geoservice plugin, and a bundled one needs private Qt API (Harbour
refuses it).

## Decision

- MapLibre Native's Qt bindings and rinigus' QML module `MapboxMap` (the versions Chum builds for
  Pure Maps), built in our SDK image and bundled privately under `/usr/share/harbour-threesea/lib`,
  in Lite and Full (`scripts/maplibre`, docs/build.md). Jolla's validator accepts it.
- MapLibre downloads with curl, not through the core. Our patch to its downloader refuses every
  URL outside one origin before any lookup, pins that origin's keys, follows no redirect and
  speaks HTTPS only; with nothing set it fetches nothing. The bridge sets origin
  (`https://map.threema.ch/`) and pins from the core (`threesea_map_policy`), which takes them from
  the same table as every other Threema host. `scripts/test-mapview.sh` shows a wrong key, another
  host, a redirect and a missing policy refused against a local server.
- Nothing is fetched before the user asks for a map: opening the app, a chat or the send dialog
  loads no tiles; a received location's bubble has no map, and its map loads when the bubble is
  tapped (`LocationPage`); the picker loads when its row is tapped (`MapPickerDialog`), whose text
  says what the servers get.
- The attribution OpenStreetMap's licence asks for ("© OpenMapTiles © OpenStreetMap
  contributors") shows on every map, small, at its bottom right. Android leaves it out.
- Tiles, glyphs and the style are cached in the cache directory (`map/tiles.db`, at most 20 MB),
  which Delete ID and data empties. A private chat's maps (and a chat's before it is known)
  keep them in memory only, gone with the page.

## Consequences

- Threema's map server learns which area is looked at, and the places server the middle of the
  picker's map, as with Android. Both are Threema's; nothing else is contacted (§7.6).
- The package grows by 5.6-9.4 MB installed per architecture (armv7hl to i486). The renderer is ours to update:
  `scripts/maplibre/versions`, then the notices.
- MapLibre's cache file isn't encrypted; it holds map data only, not which point was sent or
  received, but the areas it covers say where maps were looked at, so it stays small and goes with
  Delete ID and data.
