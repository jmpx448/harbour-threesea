# 0020 — Forward secrecy follows the peer, and a message is rejected rather than lost

Status: accepted (2026-11-04).

## Context

Whether to use forward secrecy rested on a feature mask up to a day old and, when a session existed,
on the session alone. A peer who linked a device kept getting messages it dropped. With our own
forward secrecy off (the setting, or multi-device), messages in an envelope were acknowledged and
dropped without a word, so the sender went on for up to a day.

## Decision

- **The feature mask alone decides, like Android.** This reverses an earlier choice that a session
  counts as support. That choice covered a peer who just unlinked its device, which an Init now covers
  by looking up the mask.
- **A Terminate, Reject, plain message on a session or Init against a "no" mask looks up that contact
  at once**, inside the connection, like the lookup for an unknown sender and at most once a minute
  per contact, an Init every 5 s. Sessions end only on the directory's answer. Android does this synchronously too; going through the App would make a re-send
  overtake the lookup.
- **Deviation from Android, following the spec:** with forward secrecy off, a message in an envelope
  gets a Reject (DISABLED_BY_LOCAL), control messages nothing. Android sends a Terminate, which loses
  the message.
- **Answers go by the key libthreema already has**, for a sender with no contact row yet: an Init comes
  before the message that creates the contact. Not to anyone blocked, nor with "block unknown senders"
  on.

## Consequences

A reply right after an Init can go out plain until the lookup has answered, and stays plain if it
fails: the session is kept, but the mask rules, and sending looks again after a minute. A slow directory holds up incoming messages for up to
10 s, once a minute per contact. An Accept to a stranger shows them that we are online, as on Android.
