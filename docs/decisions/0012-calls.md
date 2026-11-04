# 0012 — Calls (and Threema Web) on GStreamer, Chum only

> Flavour names since 2026-10-27 (ADR 0014): **Lite** (was "Harbour", the Jolla Store rules) and
> **Full** (was "Chum", background service and calls). Below, "Harbour" and "Chum" mostly name
> the flavours by their old names.

Status: accepted (2026-10-23); its Threema Web step is replaced by ADR 0013; the Jolla phone
app integration is dropped (user, 2026-10-26: Android has none; phone calls just ring, no
reject setting)

## Context

Threema calls (upstream Android 6.5.3, `app/.../voip`): 1:1 signalling as CSP E2E messages with
JSON bodies (0x60 offer, 0x61 answer, 0x62 ICE candidates, 0x63 hangup, 0x64 ringing; call
features), WebRTC with Unified Plan, max-bundle, DTLS-SRTP, Opus (CBR, patched SDP,
`SdpPatcher`), VP8/H.264 video, TURN credentials from `identity/turn_cred` (type `voip`), and an
in-call data channel (`3MACALLdc0`, `o2o-call.proto`) even for audio. Group calls: SFU
(`identity/sfu_cred`, HTTP peek/join), a data channel to the SFU, `group-call.proto`, every
media frame end-to-end encrypted (AES-256-GCM, per participant). libthreema has no call code.

Sailfish OS 5.1 ships GStreamer 1.26 with `webrtcbin`, libnice, DTLS, SRTP, SCTP data channels,
Opus, VP8 and the droid (hardware) camera/codecs, but no echo cancellation (`webrtcdsp`).
The Jolla Store (Harbour) allows none of these libraries, no ngf (ringtones) and no call UI
over the lock screen. The alternative, Google's libwebrtc linked statically, is a very large
build for three architectures.

## Decision (user, 2026-10-23)

- **Calls are Chum only**, built on the system's GStreamer `webrtcbin`, driven by the bridge;
  signalling, JSON message types, TURN/SFU credentials, SDP rules, the group-call protocol and
  its frame encryption in the Rust core (CLAUDE.md §5: no business logic in C++). The Harbour
  build has no call button (not "not available" texts).
- **Echo cancellation is bundled from the start**: `webrtc-audio-processing` (the PulseAudio
  project's extraction of WebRTC's audio processing, BSD), pinned, as our own processing step.
- Phone tests stay in the last phase (earpiece/speaker routing, echo, ringing while locked,
  proximity): built from documentation and emulator checks, fixed then.
- Order: 1:1 audio → 1:1 video → group calls → Threema Web (the same webrtcbin, it only needs
  data channels; SaltyRTC in the core) → calls in Jolla's own phone app (Telepathy plugin,
  Chum).

## Consequences

- Chum RPM depends on the GStreamer WebRTC plugins (`gstreamer1.0-plugins-bad` etc.) and
  bundles webrtc-audio-processing; the Harbour RPM stays as it is.
- Interop with Android's patched SDP (Opus CBR, header-extension encryption) must be checked
  against a real Threema app; the user's phone tests come last, so a fallback (libwebrtc) is
  only considered if webrtcbin cannot interoperate.
- Group calls need an encoded-frame transform inside the GStreamer pipeline (our element
  between encoder and payloader).

## Implementation notes (2026-10-23)

- Echo cancellation: webrtc-audio-processing v2.1 (submodule, built in the Chum `%build` with
  meson against the system's abseil-cpp) runs AEC3, noise suppression, high-pass filter and AGC2
  from two pad probes in our own pipeline (`bridge/aec/echocanceller.*`, the only C++17 file):
  10 ms buffers from `audiobuffersplit` before the call's PulseAudio sink (reference) and after
  its source (cleaned in place). GStreamer's own `webrtcdsp` element (LGPL, not shipped by Jolla)
  is not used, so no LGPL code is vendored.
- The core lowercases webrtcbin's `OPUS` codec name before the SdpPatcher, as Android's would
  reject the SDP otherwise.
- DTLS: Android's libwebrtc uses ECDSA certificates and, as DTLS client, offers only ECDSA
  cipher suites; GStreamer's default certificate is a process-wide RSA-2048 one, so our outgoing
  calls (we are the DTLS server) failed with "no shared cipher". The engine makes an ECDSA P-256
  certificate per call and puts it on webrtcbin's DTLS decoders when the transports are added
  (deep-element-added). webrtcbin computes the SDP fingerprint earlier, from its default
  certificate, so the engine writes our certificate's fingerprint into every SDP it hands out.
  The engine wipes its copy of the private key when the call ends; the copies GStreamer took of
  the PEM are out of our reach and not cleared. The key is good for that one call.
- webrtcbin does not check the peer's certificate against the SDP fingerprint. The engine does,
  once the peer connection is up, and ends the call on a mismatch (otherwise anyone on the media
  path could take a call over; libwebrtc checks it). DTLS, and so SRTP, is up before that check
  can run (it is queued to the engine's thread), so until it passed pad probes drop all media
  into and out of webrtcbin and data channel messages wait in both directions (security audit
  2026-10-27, V01: without the gate a delayed check let 220 ms of audio in and 780 ms out).
  Group calls check the SFU's certificate against the fingerprint from the join response the
  same way, with the same gate. tests/callengine covers both outcomes, the wrong fingerprint also
  with a delayed check (no audio may pass either way).

