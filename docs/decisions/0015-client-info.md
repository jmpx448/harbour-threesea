# 0015 — Our own client-info towards Threema's servers

Status: accepted (2026-11-03).

## Context

Every client sends a `client-info` string in the chat server handshake, comparable to a user agent:
`<app-version>;<platform>;<lang>/<country>;<rest>`. Until now we sent libthreema's default for its
stand-alone use (`<libthreema-version>;L;en/CH;linux;<arch>`), which says nothing about the app and
has a fixed language and country. Asked whether third-party clients are acceptable, Threema replied
(November 2026) that they don't actively block them on some conditions (`docs/protocol-notes.md`) and
suggested that this string identify our project.

The platform codes Threema documents are Android, iOS, Desktop/Web, Windows Phone and Bot.

## Decision

`Threesea/<version>;S;<lang>/<country>;<device-model>;<os-version>`, through a new
`ClientInfo::Threesea` in our libthreema. The rest has the shape of the mobile clients, which is what
we are. `S` is the code we already sent with license checks; using a documented code would claim to
be a client we are not. The name is in the version field, which Threema defines as arbitrary, so
the string explains itself to someone reading it without knowing `S`.

The license check and the handshake send the same string, from one place
(`ClientVersion::client_info`). The D2M device info says `Threesea (<device-model>)`.

The one place where we send another platform's value is Threema Web's `clientInfo.os`, which is
`"android"` (ADR 0013): it only selects the browser's parser for our push token, `appVersion` names
Threesea, and it goes only to the user's own browser inside the end-to-end encrypted session, never
to Threema's servers.

## Consequences

- The chat server and the mediator now see the device model, the OS version and the system's
  language and country, as they do from Threema's own apps. The directory already saw them in the
  license check. Nothing identifying the person beyond that; the string holds no user input.
- Threema is fine with the code `S` and with our HTTPS user agent (asked, November 2026). If it ever
  assigns or prefers another code, it changes in one place in libthreema.
- A libthreema update must keep the variant (one more commit in the vendored tree, ADR 0006).
