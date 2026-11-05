# Threesea — an unofficial Threema client for Sailfish OS

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/ai-generated-white.svg">
  <img alt="AI generated" src="docs/assets/ai-generated-black.svg" height="40">
</picture>

A native Sailfish OS app for [Threema](https://threema.ch), built on Threema's own Rust protocol
library `libthreema`, with a Silica UI that follows Jolla's apps. You need a **Threema Libre
license key from the Threema Shop**. No Google services, no telemetry.

## Disclaimer — please read

- **Made with AI.** This app was developed with [Claude Code](https://claude.com/claude-code)
  (Anthropic) and audited with [OpenAI Codex](https://openai.com/codex/), directed and tested by
  a human maintainer. It is a **research and experimentation** project.
- **AI makes mistakes.** The code, including its security-relevant parts, can contain errors
  that neither the AI nor the maintainer noticed. The developers give **no warranty** of any
  kind (see sections 15 and 16 of the [license](LICENSE)); **you use the app at your own risk.**
- **Not connected to Threema.** Threesea is based on Threema Libre's open-source code and tries
  to play fair with Threema's APIs and licensing: it needs a license bought in the Threema Shop,
  checks it like Threema's own apps, uses Threema's protocol library, names itself to Threema's
  servers, and tries to follow the protocol as Threema's apps do. We asked Threema about third-party
  clients, and they told us (November 2026) that they don't actively block them as long as they
  cause no stability issues or high load on the servers, are clearly marked as unofficial without
  using Threema's branding, and don't circumvent the licensing. Threesea aims to meet that. This is
  not a permission, Threema can change its position, and Threesea is **not affiliated with, endorsed
  or supported by Threema AG**: support questions belong here, not with them. As with any Threema app, keep a backup of your ID.

## Screenshots

<table>
<tr>
<td><img src="docs/screenshots/01-chats.png" width="200" alt="Chat list with unread counts"></td>
<td><img src="docs/screenshots/02-chat.png" width="200" alt="A chat with a quote and reactions"></td>
<td><img src="docs/screenshots/03-chat-media.png" width="200" alt="A chat with a GIF, a voice message and a file"></td>
<td><img src="docs/screenshots/04-group.png" width="200" alt="A group chat with a mention"></td>
</tr>
<tr>
<td><img src="docs/screenshots/05-contact.png" width="200" alt="A contact's page"></td>
<td><img src="docs/screenshots/07-emoji.png" width="200" alt="The emoji panel"></td>
<td><img src="docs/screenshots/08-cover.png" width="200" alt="The cover on the home screen"></td>
<td><img src="docs/screenshots/10-settings.png" width="200" alt="Settings"></td>
</tr>
</table>

Made-up demo data, taken in the emulator. [Which version each picture is from](docs/screenshots/README.md).

## What works

| Feature | State |
|---|---|
| Builds and installs on SFOS 5.1+ (aarch64, armv7hl, i486) | ✅ |
| Libre license check, ID creation, ID backup export/import, Threema Safe backup/restore | ✅ |
| 1:1 chats with forward secrecy (off while a device is linked, as Threema's multi-device requires, and if you switch it off; no warning yet on a message sent without it), delivery/read receipts, typing indicators | ✅ |
| Report spam, 1:1 and groups (the complete report is not tested end to end yet) | ✅ |
| Groups (create, rename, members, leave, pictures), mentions, distribution lists | ✅ |
| Photos, videos, files, voice messages, GIFs, polls, locations, quotes, reactions, edit, delete | ✅ |
| Profile pictures, contact verification by QR code, address book matching (opt-in, hashed) | ✅ |
| App lock and private chats, encrypted storage (SQLCipher) | ✅ |
| Notifications with quick reply, cover, background connection | ✅ |
| Share from other apps, Threema links (`threema://`, `threema.id`) | ✅ |
| Multi-device (Threema for Desktop, with names for the linked devices), Threema Web | ✅ |
| Archive, pinned chats, mark as unread, chat search, starred messages, auto-delete | ✅ |
| 1:1 calls and group calls, audio and video (Full flavour) | ✅ (phone tests pending) |
| German translation | ✅ |

Two flavours are built from this tree, both as RPMs on the GitHub releases page (not in the
Jolla Store or Chum yet): **Full**, with a background service that connects at login and calls,
and **Lite**, which follows the Jolla Store's rules (it stays connected only once opened, no
calls). Install one of them; see [ADR 0014](docs/decisions/0014-lite-full-github-updates.md).

## Install

Download an RPM from the [releases](https://github.com/penttimus/harbour-threesea/releases) and
install it in developer mode; see [docs/release.md](docs/release.md#installing-a-release-on-a-phone).
Each release is built by GitHub Actions from the tagged source, with checksums and build
provenance attestations ([SECURITY.md](SECURITY.md#verifying-a-release)).

### If you forget the app lock's PIN or passphrase

This takes the chats and media on this phone with it; what comes back is what your backups hold.

1. Delete the app's data folder, `.local/share/org.threesea/harbour-threesea` in your home folder:
   with a file manager that shows hidden folders, or in a terminal with
   `rm -rf ~/.local/share/org.threesea/harbour-threesea`.
2. Restart the phone. Closing the window is not enough: with "Keep running in the background" on,
   which is the default, Threesea goes on running and keeps the deleted data open.
3. Open Threesea. It starts as on a new phone, without the lock: enter the license key and restore
   your ID from Threema Safe or an ID backup. A data backup brings the chats back after that, from
   its page in the settings.

With a passphrase for the stored data (Settings → Security), Threesea asks for that passphrase
before step 3: it is kept in Sailfish Secrets, not in the folder. Only that passphrase gets past
this page, so keep it safe.

## How it was made, and how to continue

Everything the AI was told and everything learned on the way is in the repository, so anyone
can pick up the work:

- [CLAUDE.md](CLAUDE.md) is the standing brief given to Claude Code: goal, stack, security
  rules, the phased plan, UI conventions and working rules. Notes like "(user)" or "user
  decision" in it and in the docs mark decisions the human maintainer made.
- [docs/](docs) holds the decisions (`decisions/`, one per choice), protocol notes, the mapping
  to the Android app, research, the build and emulator guides, and the status log
  ([docs/STATUS.md](docs/STATUS.md)).
- Security reviews: [docs/security-audit.md](docs/security-audit.md) and the audits of
  2026-10-27 ([security](docs/security-audit-2026-10-27.md), [rules](docs/rules-audit-2026-10-27.md)),
  each with what has been fixed since.
- `scripts/check.sh` runs all checks; `git config core.hooksPath scripts/hooks` runs the quick
  ones before every commit.

## Build

See [docs/build.md](docs/build.md). In short, with Docker installed:

```sh
git clone --recurse-submodules --shallow-submodules <repo> && cd harbour-threesea
scripts/sdk-build.sh aarch64
```

## Credits

Threesea stands on the work of others; thank you:

- **[Threema AG](https://threema.ch)**: [threema-android](https://github.com/threema-ch/threema-android)
  (AGPL-3.0), the reference for nearly everything here, and its Rust library `libthreema`
  (AGPL-3.0, vendored with our changes in `third_party/libthreema`); the protocols of Threema,
  Threema for Desktop and Threema Web; the emoji panel's search terms, from threema-android's
  assets.
- **[Twemoji](https://github.com/jdecked/twemoji)**: emoji graphics (CC-BY 4.0), as packaged
  in Threema Libre.
- **[opus-pure](https://crates.io/crates/opus-pure)** by Stephen Berry, from rusty-opus and
  opus-rs, Rust ports of [libopus](https://opus-codec.org/) by the Xiph.Org Foundation and
  others (BSD-3-Clause): voice messages where the phone has no Opus encoder.
- **[webrtc-audio-processing](https://gitlab.freedesktop.org/pulseaudio/webrtc-audio-processing)**,
  the PulseAudio project's package of Google's WebRTC audio processing (BSD-3-Clause): echo
  cancellation in calls.
- **[Jolla](https://jolla.com) and Sailfish OS**: the Silica components and theme icons, and
  Jolla's Messages, People, Settings and Gallery apps as the model for the UI.
- **[Whisperfish](https://gitlab.com/whisperfish/whisperfish)**: the Rust-based Signal client for
  Sailfish OS, a reference for how such an app can be built.
- **[SQLCipher](https://www.zetetic.net/sqlcipher/)** (BSD-3-Clause), **[GStreamer](https://gstreamer.freedesktop.org/)**
  and its `webrtcbin` (LGPL), **[Qt](https://www.qt.io/)** (LGPL), and the Rust crates in
  [Cargo.lock](Cargo.lock), among them tokio, rustls, reqwest, rusqlite, RustCrypto, prost and
  image.
- The **"AI generated" label** is the European Commission's icon for AI-generated content, free
  to use.

## License

AGPL-3.0-only, like upstream threema-android. See [LICENSE](LICENSE).
