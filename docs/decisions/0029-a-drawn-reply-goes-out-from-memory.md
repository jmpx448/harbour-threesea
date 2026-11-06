# 0029 — A picture drawn on leaves from memory, a received one is never copied to disk

Status: accepted (2026-11-05).

## Context

Android's "Draw reply" and the chat's editor write a copy into a plain file and attach it. Here,
attachments are files in the capture folder until they are stored encrypted, and §7.1 only lets
decrypted media pass through the UI's readers in memory.

## Decision

"Draw on it" on a message reads the picture from the core when the dialog is accepted, on the GUI
thread, as a private chat's can't be read once the chat has gone: the bytes stay in memory. The
result goes straight to the core: crop, turn and marks on the decoded image (at most 4096 px on a side)
on a worker, the encoded result to the core (`ThreeseBackend::holdPicture`, `sendHeldPicture`). The
dialog's accept says "Send". A caption field under its header goes with the picture (user decision
2026-11-06): the core checks it like any file's caption, and if the message wouldn't fit a frame
with it, the notice says the text is too long and the editor opens again as it was, caption
included, to shorten it. No plain copy of a received picture exists at any point: the bytes,
the decoded and edited images and the encoded picture are wiped once done with, as far as the bridge
holds them alone. Only what the user brought in (a picked or just taken photo, already a plain file)
goes through the capture folder as before, and is removed once sent.

A dialog is accepted with a swipe that is quickly done by accident, and left with one. So nothing
goes until a remorse on the chat has run ("Sending drawing"), a picture nothing was done to can't be
sent, and leaving with something drawn runs a remorse too ("Discarding drawing"); taking either back
returns to the editor as it was. The chat leaving during the remorse sends at once, before a private
chat is hidden: that is what the remorse was about to do.

Every picture Qt encodes for sending is written through `PhotoEdit::withoutMetadata`: Qt's writers
otherwise write out again the text its readers put on the QImage (ADR 0025). GIFs and the browser's
pictures don't pass there: the core cleans those (`gif.rs`, `web/images.rs`).

## Consequences

No second look in the chat's input for a picture drawn on, and no mentions in its caption; if the
core refuses it for anything but the caption's length, a notice says so and the drawing is gone. The bytes of a picture being sent stay in memory for the
remorse's four seconds. The encoders' own buffers (`QBuffer` growing) aren't wiped.
The marks are burnt in by `QPainter`, from fractions of the picture: the preview's Canvas and Labels
are only that, a preview.
