# 0013 — Threema Web over relayed data, both flavours

> Flavour names since 2026-10-27 (ADR 0014): **Lite** (was "Harbour", the Jolla Store rules) and
> **Full** (was "Chum", background service and calls). Below, "Harbour" and "Chum" mostly name
> the flavours by their old names.

Status: accepted (2026-10-26). Amends ADR 0012, which planned Threema Web on webrtcbin (Chum).

## Context

Threema Web (web.threema.ch) runs in a browser and gets everything from the phone over SaltyRTC
(docs/research/threema-web.md). The browser is the SaltyRTC initiator, the phone the responder.
Android answers with the WebRTC task (a data channel, ADR 0012's plan). The browser also always
offers the relayed-data task, which iOS uses: all messages then go end-to-end encrypted through
the WebSocket to Threema's signalling server (`saltyrtc-XX.threema.ch`). That needs no WebRTC
at all, only a WebSocket, NaCl boxes and msgpack.

The QR code of a self-hosted Threema Web can name any signalling server.

## Decision (user, 2026-10-26)

- **Relayed data only**, implemented in the Rust core: a sans-I/O SaltyRTC responder, chunking,
  Threema's app remote protocol. `clientInfo.os` is `"android"` as a protocol setting: it
  selects the browser's parser for our `threema-gateway` push token, which Threema Web reads only for
  that value (`appVersion` names Threesea). A combination to be tested first.
- **Both flavours**: Harbour while the app runs, Chum also in the background (and woken by
  `web-session-resume` from `*3MAPUSH` over the chat server connection).
- **Servers**: hosts under `threema.ch` connect without asking and only with Threema's built-in
  certificate pins; a mismatch fails, and there is no way to override it. Any other host (a
  self-hosted Threema Web) is shown to the user once; if allowed, it is added to an allowlist and
  its certificate is pinned on first use. A later connection with a different certificate asks
  again: accept the new certificate (and pin it) or abort. A session the browser never stored is
  gone by then, so accepting pins the certificate and asks for the code again.
  The sessions page lists the allowed servers; removing one takes its permission, its pin and its
  sessions, and the next code naming it asks again.
- New dependency: `rmpv` (msgpack values), pinned; SaltyRTC and its crypto are our own code on
  libthreema's X25519/HSalsa20 and the `crypto_secretbox` crate already used.

## Consequences

- Everything, files included, goes through Threema's relay instead of peer to peer (as on iOS);
  still end-to-end encrypted by SaltyRTC.
- Sessions (our permanent key, the browser's key, the server) are stored in the encrypted
  store; hidden private chats (ADR 0009) stay hidden from the browser; private groups and lists aren't in its receiver lists at all,
  and marking one takes it out of a connected browser.
- ADR 0012's step "Threema Web on webrtcbin" is dropped.
