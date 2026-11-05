# 0029 — A picture drawn on leaves from memory, a received one is never copied to disk

Status: accepted (2026-11-05).

## Context

Android's "Draw reply" and the chat's editor write a copy into a plain file and attach it. Here,
attachments are files in the capture folder until they are stored encrypted, and §7.1 only lets
decrypted media pass through the UI's readers in memory.

## Decision

"Draw on it" on a message reads the picture from the core on a worker, applies crop, turn and marks
to the decoded image and hands the encoded result straight to the core
(`ThreeseBackend::sendEditedPicture`). The dialog's accept says "Send". No plain copy of a received
picture exists at any point. Only what the user brought in (a picked or just taken photo, already a
plain file) goes through the capture folder as before, and is removed once sent.

Every picture is written through `PhotoEdit::withoutMetadata`: Qt's writers otherwise write out again
the text its readers put on the QImage (ADR 0025).

## Consequences

No caption and no second look in the chat's input for a picture drawn on; if the core refuses it, a
notice says so and the drawing is gone. The marks are burnt in at full resolution by `QPainter`, from
fractions of the picture: the preview's Canvas and Labels are only that, a preview, so a font missing
glyphs at one size may differ by a pixel, not in place.
