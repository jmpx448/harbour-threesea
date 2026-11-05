# 0017 — Linking and removing devices report what the mediator did

Status: accepted (2026-11-04).

## Context

Linking and unlinking inferred their result from a timer or the local step, not from the device
group's state. Removing the last device left the phone behind the mediator with forward secrecy
off. "Remove all" offline said "No linked devices" while the other device kept its slot and the
ID, and the phone had forgotten the group needed to remove it. A join's reply limit was shorter
than the join, and a failure after the identity had gone over was rolled back locally, leaving a
working device in a group the phone no longer knew.

An earlier user report stands: a phone must never be trapped behind an unreachable mediator.

## Decision

1. Removal is decided inside the `DROP_DEVICE` transaction from the mediator's list, as in Android's
   `runDropDevicesSteps`. When no other device would remain, this one leaves too. The same check
   with nothing to remove runs about hourly and when the page finds only this device.
2. A failed "Remove all" changes nothing. **Deviation from what we did before:** leaving locally is
   no longer automatic but a choice in a dialog that says the others stay linked. That is the way
   out of an unreachable mediator, and it keeps a retry possible for the stolen laptop. A failed
   removal is also an event, so the user hears of it after leaving the page during the wait.
3. A join tells whether the identity has left this device (from the first byte of the essential
   data). Before that, a group made just for the join is left locally. After that, a fresh group is
   removed through the mediator; when that fails, or when the group existed before, multi-device
   stays on and the UI says the computer may be linked. Only a member can remove it.
4. A `Registered` new device counts as joined, whatever the commit or the list afterwards does.
5. Every step of a join has a limit and the core waits longer than their sum. If no answer comes
   even so, nothing is rolled back: the connection may still be at it.

## Consequences

- In the worst case the core waits about 11 minutes for a join's reply (was: 200 s, too short for
  the join). That wait is beside the core loop (ADR 0028); the user is on the linking page meanwhile.
- A new device registering after the group was removed forms a new group of its own with the keys
  it got. The phone cannot prevent that, and the error text says the computer received the ID.
- Android retries a failed drop as a persistent task; we leave the retry to the user and to the
  hourly check.
