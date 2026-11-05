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
samples that aren't in the file, a track to drop in a fragmented file) is refused with its own error
(`videoKeepsMetadata`), and nothing of it stays. The chat asks what to do, saying what the file
keeps: send it as it is (`keepMetadata`, then as a plain file, unchanged, under its own name), or
don't send it. Sending it unasked would break what the first part promises at exactly the rare
moment nobody is looking, and silently dropping it would lose the user's video. No second MP4 writer.

Where there is nobody to ask, it isn't sent, and a notice says so: a chat being left (videos still
waiting go to the core at once, without a thumbnail, and captures of ours go with the refusal, as
for any file refused then), and a forward. A video picked "as a file" is the user's file and isn't
touched; every other way a video reaches a chat goes through the core's cleaning, a shared one and
one the thumbnailer has no frame for included.

## Consequences

Subtitle and time code tracks go too; Android's re-encoding drops those as well. WebM, MKV and AVI
never show as media to the recipient: each one asks first, and goes as a file. A received WebM sent
as media can't be forwarded; saving it and sending it as a file can. Forwarding a media video cleans
it as well: running it twice changes nothing, and a list's members get the same bytes. The codecs'
own streams are not looked into, nor is an `mdat` outside any track's samples, nor sample entries'
inner boxes (`avcC` and the like) beyond their vendor and encoder name fields.
