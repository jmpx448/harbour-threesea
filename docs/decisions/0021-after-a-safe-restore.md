# 0021 — What a device does after a Threema Safe restore

Status: accepted (2026-11-04).

## Context

A restored device's groups are as old as the backup, it has no forward secrecy sessions, and Safe
holds no messages. Android runs its _Application Setup Steps_ after a restore
(`ApplicationSetupSteps.kt`); we ran nothing, and our notes wrongly said Android doesn't either.

## Decision

We run those steps once, after the import and with the connection up: feature mask, group resync
or sync request, an FS `Init` and a picture request per solicited contact. The flag is stored with
the restore. Contacts go 20 per round (connect, hourly), since a late session only delays a message
by a re-send and Threema asks for no high server load. Android does them all at once.

**Deviations from Android:**

- A creator's restore doesn't send `delete-profile-picture`. Safe doesn't carry group pictures, so
  ours is unknown, not removed; Android deletes it for every member.
- A contact we already have a session with gets no empty message: that session was made after the
  restore, so nothing about it is stale.
- Blocked contacts get nothing, not even a sync request (ADR 0019).
- A session's "established" line only appears in a chat with something in it. Android's status
  messages don't list a chat, ours do, and every Accept would start a chat or move one to the top.

A chat that a backup gives a `lastUpdate` is listed without messages, like an emptied chat. That is
what keeps the time for our next upload, and it is Android's list after a restore too.

## Consequences

The list after a restore shows empty chats for the contacts the backup says had one. A creator's
group picture stays with the members until the creator sets a new one, but a member's own sync
request to that creator still gets a removal, as `send_state` has no way to tell. A contact outside
the first rounds may reject one message, which goes out again.
