# 0022 — What remains of a deleted contact or chat

Status: accepted (2026-11-05).

## Context

Deleting a contact removed its row and nothing else. A chat with them stayed in the list under the
bare ID, could not be opened, emptied or deleted, and kept its unread count. Deleting a chat left
its pin and draft behind, which came back with the next message. Each path decided on its own what
goes.

## Decision

As Android's `MarkContactAsDeletedBackgroundTask`:

- **The contact row stays**, with the level group-or-deleted: out of the contact list, with its key,
  its name and its verification level, and back in it by adding it or by writing to it. Other
  devices hear of the new level. A distribution list keeps reaching them.
- **Deleting a contact deletes the chat with them**, also for someone a group still names; the
  remorse says so. Their messages in groups stay. A hidden private chat stays as it is, is not
  given away by an error, and can be opened and deleted after unlocking (ADR 0009).
- **A deleted chat takes what is the chat's**: pin, archive and unread marks, draft. The notification
  choice and the tone are the contact's or group's and stay; emptying a chat keeps all of it.
- **A deleted contact takes what was chosen for them and queued for them**: notification choice,
  tone and what waits in the outbox (reactions, edits, picture requests). Its picture goes too unless
  a group names them, since group pages show it; a pause in asking for pictures stays. The block
  list, the private mark and the picture allow list are choices about an ID that outlast the contact
  (Android keeps them too): what is hidden or blocked stays so if the person writes again. Forward
  security sessions stay, as on Android, whose task for this doesn't touch them: the row stays, and
  a session gone would only get the person's next message rejected.
- **A deleted contact goes on the address book's exclusion list**, or the next match would bring
  them back. Android asks, for contacts it knows to come from the address book; here deleting
  needs no extra question, the list is in Settings, and adding the ID by hand takes it off again.
- **A deleted group or list takes all of it**: also the notification choice and tone, a group's
  picture, its running calls' keys and its resync marks. Deleting a group we left also goes to the
  other devices; a group deleted on another device loses its picture here too.
- **A contact page of someone only a group of ours needs** has no Delete, and has Add to contacts,
  like every contact out of the list.

Chats left by earlier versions without a contact row can be read, marked read, emptied and deleted;
writing needs the ID added back. `check_readable` holds for them like for any chat.

## Consequences

Rows are not removed any more, so the table is bounded by everyone ever known, as on Android.
Deleting a contact loses the chat without a question beyond the remorse timer, which Android asks
with a dialog; the remorse is how this app confirms what it deletes.
