# 0030: Places come from Threema's server, and no map is drawn yet

## Context

Sending a location could only send the device's own fix. Android offers a map, nearby places and a
search by name. Places (`poi.threema.ch`) and the map style (`map.threema.ch`) are Threema's hosts.
The style file names the hosts of the tiles and fonts, and it isn't in the Android source. Its tiles
need a vector renderer (MapLibre on Android), which Qt 5.6 and the Harbour-allowed imports don't
offer.

## Decision

- Choosing a place uses `poi.threema.ch` only, through the pinned client, like Android.
- The position leaves the device only after the user tapped the row that opens the place list, and
  that row says so beforehand. Opening the dialog to send the device's own position sends nothing to
  that server.
- Search goes around the last position known, even an old one: it only orders the results. Sending
  one's own position still takes a recent fix.
- No map. Whether its tiles come from Threema alone hasn't been checked, and §7.6 allows no other
  host, so it waits for that answer and a choice of renderer.

## Consequences

- A place has to be found by name or be near the device, and the search needs a position to start
  from. A point can't be picked by hand, and a received location still opens in a maps app.
- There is no address for a chosen place, like Android's place list: only its name and coordinates.
