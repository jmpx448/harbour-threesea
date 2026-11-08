# 0026 — Pictures are asked for after a restore, once

Status: accepted (2026-11-05). Replaces the triggers recorded with store migration 27.

## Context

To recover pictures after a restore we asked anyone without one: when a chat opened, when a message
came, and in batches on connecting and every hour, with a backoff repeating every 30 days. A
contact who simply has no picture was asked for ever, and each request costs a message; on a device
with forward secrecy it starts a session as well, with an Init and an Accept, and the contact's
Android gets a status line in its chat with us. 500 contacts came to hundreds of messages on the
first day and about 17 a day after that. Android asks once, only for contacts restored from a
backup (`isRestored`, `OutgoingContactRequestProfilePictureTask`), only if there is a chat or a
shared group (`ApplicationSetupSteps`). Contacts send their picture with their messages by
themselves; this is only for what a restore lost.

## Decision

As Android. The Threema Safe and data backup imports mark the contacts they add
(`picture_restored`, migration 38). A marked contact with a chat or a shared group is asked once,
by the restore's setup steps or in the bounded batches on connecting and every hour. A set or
deleted picture from them ends the mark, as does deleting the contact. The chat-open and
incoming-message triggers and the backoff table are gone.

A mark outlives a contact without a chat, where Android's flag does too, but Android only looks at
it during its setup steps and we look again: a restored contact is still asked once if a chat with
them begins later.

## Consequences

Contacts who send no picture stay without one until they send a message with it, as on Android. A
picture whose blob has expired isn't asked for again. Installations from before this have no marks,
so a restore done earlier isn't followed up.
