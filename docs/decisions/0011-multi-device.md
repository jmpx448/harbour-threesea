# 0011 — Multi-device (Threema Desktop) and Threema Web

> Flavour names since 2026-10-27 (ADR 0014): **Lite** (was "Harbour", the Jolla Store rules) and
> **Full** (was "Chum", background service and calls). Below, "Harbour" and "Chum" mostly name
> the flavours by their old names.

Status: accepted (2026-10-23)

## Context

The user wants Threema Desktop and Threema Web (roadmap of 2026-10-23: after GIFs, before
calls). Upstream Android 6.5.3:

- **Desktop 2.0** uses multi-device: the phone and the desktop app form a *device group* on the
  mediator server (D2M, `wss://mediator-….threema.ch`). CSP is proxied through the mediator, and
  every device reflects what it sends and receives to the others (D2D envelopes encrypted with
  keys derived from the device group key, DGK). A new device joins by showing a QR code
  (`threema://device-group/join#…`); the phone scans it, both meet on a rendezvous WebSocket
  relay, the user compares three emojis, and the phone sends the *essential data* (identity,
  profile, contacts, groups, settings). Android does all of this in Kotlin; only the rendezvous
  crypto comes from libthreema. Our vendored libthreema has the D2M state machine, the
  rendezvous protocol, transaction and reflect subtasks and DGK derivations, but no join
  protocol, no outgoing reflection and no handling of reflected envelopes.
- Multi-device and forward security (PFS) don't combine yet (`IS_FS_SUPPORTED_WITH_MD = false`,
  SE-199): Android turns PFS off when it links the first device and back on when the last one
  is gone.
- **Threema Web** is the older browser client: SaltyRTC signalling plus a WebRTC data channel
  (WebRTC is mandatory, no relayed fallback), about 40 web-protocol handlers. Upstream calls it
  "the old Threema web sessions screen" and steers users to Desktop; linking a multi-device
  device stops Web sessions.

## Decision

1. **Desktop now**, ported behaviour by behaviour from Android, in our core:
   - WebSockets via `tokio-tungstenite` over our own rustls configuration
     (`https::tls_config`: Threema hosts only, pinned) — mediator and rendezvous.
   - Linking (we are always the existing device): QR parse, rendezvous as RRD and nominator,
     emoji confirmation in the UI (Android's three choices, one of them right), then
     activation: random DGK, mediator and CSP device ids, PFS off (feature mask without FS,
     every session terminated), reconnect through the mediator; then a `NEW_DEVICE_SYNC`
     transaction carrying the essential data.
   - The connection runs CSP over D2M when multi-device is active; leader handling and the
     41xx close codes as upstream.
   - Reflection both ways (outgoing/incoming messages, updates, read receipts; contact, group,
     profile and settings sync), dropping devices, a "Linked devices" page.
   - PFS off while linked, like Android (user decision); the linking dialog shows
     Android's warning first.
   - Everything multi-device lives in the encrypted store; the DGK never leaves the core.
2. **Threema Web: decided in the Calls phase.** Calls need WebRTC media (codecs, echo
   cancellation, audio routing); Web needs WebRTC data channels. They share only ICE/TURN/DTLS.
   If the WebRTC stack chosen for calls gives data channels without a second stack, Web is built
   on it (SaltyRTC responder + the web protocol); otherwise Web is dropped (user decision).
3. Harbour and Chum both get multi-device (nothing in it needs more than the network).

## Consequences

- A second transport in `connection.rs`; the direct CSP path stays as it is for everyone who
  never links a device.
- Reflection touches every send path (messages, receipts, groups, contacts, profile); it is
  added step by step, each with tests, and the feature stays hidden until the whole flow works
  with Threema Desktop.
- New dependency: `tokio-tungstenite` (=0.29.0, no TLS of its own: `__rustls-tls` only wires in
  our `ClientConfig`; the version libthreema's examples use) and `futures-util` (sink/stream
  traits it needs).
- Linking can only be tested with the user's help (Threema Desktop running, its QR code), so
  that test is planned explicitly; the mediator connection itself can be tested alone with the
  test ID.
