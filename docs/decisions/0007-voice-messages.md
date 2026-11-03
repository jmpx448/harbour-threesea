# 0007 — Voice messages: platform AAC, Opus fallback

## Context
Threema voice messages are AAC in MP4 (Android) or M4A (iOS). Sailfish OS has AAC only through
gst-droid (phone hardware codecs); Jolla's ffmpeg has no AAC, the SDK emulator has no AAC and no
Qt mediaservice. Shipping an own AAC codec raises patent/licensing questions.

## Decision
Record with the platform's AAC (`QAudioRecorder`) where available. Otherwise fall back
automatically to Opus in Ogg, encoded/decoded in the core with `opus-pure` (pure Rust,
BSD-3-Clause, no dependencies), vendored in `third_party/opus-pure` with a one-line-kind fix for
32-bit x86. No setting.

## Consequences
- Same format as Threema Android on phones with AAC; Opus messages play on Android and in
  Threema Desktop; iOS may only offer them as a file.
- Received AAC can't be played without a platform player (no AAC decoder shipped).
- The emulator exercises the whole voice path via the fallback.
