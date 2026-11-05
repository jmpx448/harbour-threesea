# 0028: Long work goes beside the core loop

## Context

All commands, connection events and replies go through one loop on one thread. Whatever a handler
waited for, everything waited for: on a stalled mobile link the first reply at start took a minute,
and during a Safe upload or a linking attempt no notification, ringing or hang-up got through.
`App` was held mutably across each wait, so a task couldn't simply take the work over.

## Decision

- `Shared` (behind an `Rc`, `App` dereferences to it) holds what such work needs: the store, the
  clients, the connection's channel, a few marks in cells. No cell's borrow is held across an
  `.await`; clippy's `await_holding_refcell_ref` watches that.
- `core/src/detached.rs` runs a command's long part or a round of work as a local task on `Shared`,
  and hands the result back to the loop, which finishes it on `App` and replies.
- `Lane::InTurn`: one at a time, in the order asked (account, Safe, linking, device group, export,
  maintenance, what follows connecting). `Lane::Free`: someone is waiting and nothing depends on it
  (TURN servers, joining a group call, a contact lookup, the device list).
- Only the UI's requests in the loop go this way. Threema Web and what is queued behind a quit need
  the answer in place.
- Removing the ID drops all of it, including results the loop hasn't looked at yet: nothing may be
  written or sent for that ID afterwards. The removal itself stays in the loop.
- A process that ends waits for replies still due, within the quit's limits, not for maintenance.

## Consequences

- A long command's reply can come after those of later commands; every caller goes by the reply's
  id already. A command depending on a long one waits for its reply, or is in turn itself, as
  `cancelLinking` is.
- A task can see changes made between its steps, as it could by the connection task already. Work
  that outlives a lock checks again what it hands out (`exportChat`, `setGroupPicture`), and a group
  call left during its join is not entered.
- Threema Web's commands, forwarding and favourite GIFs still run in place, with their waits.
