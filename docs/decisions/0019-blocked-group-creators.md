# 0019 — A blocked creator can't bring us into a group we aren't in

Status: accepted (2026-11-04).

## Context

group-setup is exempt from the general block check. Android's `IncomingGroupSetupTask` has no check
of its own, so a contact the user blocked can create a new group with them in it, or add them back
to one they left. A spam group reported with block and delete comes back that way. Android does stop
the group-sync-request to a blocked creator on group-leave, but not in its common receive steps.

## Decision

**Deviation from Android**, for the block list only: for a group we are not in (none, or left) a
group-setup from a creator on the block list is discarded, and for a group we don't have nobody
asks a creator on the block list for it. Their setup for a group we are in still counts, so that
blocking one person can't take a group out of step.

"Block unknown senders" plays no part in this and keeps Android's behaviour: such a creator can
add the user to a new group, which is also why they keep a forward secrecy session, and why their
group-setup isn't lost in it.

## Consequences

A blocked creator's invitation disappears without a trace, as with anything else they send. After
unblocking, the group comes back with their next group-setup or the next message in it.

libthreema runs this check before the setup can create a contact (a group-setup makes its sender a
direct contact), so a stranger on the block list doesn't become one by trying.

Under "Block unknown senders", a stranger's group-setup makes them a direct contact, even one that
doesn't name the user and so brings no group. From then on the setting no longer stops them. Android
does the same.
