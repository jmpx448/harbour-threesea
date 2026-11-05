# 0025 — Videos sent as media leave without their metadata

Status: accepted (2026-11-05).

## Context

Photos lose EXIF when re-encoded, but a video sent as media went out byte for byte unless scaled,
with whatever the camera wrote: position, time of recording, device. Scaling exists only in Full
and needs an H.264 encoder; Lite and "Original size" must be covered too. `mp4mux` also takes tags
over from `qtdemux`, so a scaled video isn't clean by itself.

## Decision

The core cleans every outgoing `rendering: media` video where all outgoing files pass, in memory,
before it is stored (`core/src/mp4.rs`, details in protocol-notes.md). In place, nothing moves: no
offsets to rewrite, so a player can't get a file that differs in anything but what was blanked. A
list of what stays, not of what goes.

A file that can't be vouched for (another container, a damaged one, sample tables that don't add up,
a track to drop in a fragmented file) goes out with rendering `file`, unchanged, not as media. A
plain file is what the user can recognise as "the file as it is", and files are never changed. No
second MP4 writer, and no refusing to send.

## Consequences

Subtitle and time code tracks go too; Android's re-encoding drops those as well. WebM, MKV and AVI
never show as media to the recipient. Forwarding a media video cleans it as well: running it twice
changes nothing, and a list's members get the same bytes. The codecs' own streams are not looked
into.
