# Security audit (Phase 5, first pass)

> First pass; some verdicts were later found incomplete (see the audits of 2026-10-27).

Against CLAUDE.md §7, on 2026-10-22, at a commit before the public history plus the fixes below. Read-only review
of the code, plus checks run in the emulator. Fuzzing the message decoders is a separate step,
still to do.

## Results per rule

| § | Rule | Result |
|---|------|--------|
| 7.1 | Only the core touches keys and plaintext at rest | OK. The bridge only holds the 32-byte database key, in memory, to start the core and to encrypt it for "Encrypt stored data" (ADR 0009), wiped on exit. Media are decrypted into memory buffers for display; nothing is decrypted to disk except "Save to Downloads" (user action). The voice recording is a temporary file in the app's cache until it is sent or discarded (see open points). |
| 7.2 | SQLCipher, key in Secrets | OK. Key from `/dev/urandom`, kept in an owner-only Secrets collection; replacing it uses a staging copy so a crash can't lose it. The app lock PIN is stored only as a salted scrypt hash. |
| 7.3 | No secrets in logs | OK. Every `tracing` call was checked: only counts, kinds, flags and error texts (never message text, numbers, addresses or keys). `eprintln!` only in tests. The bridge logs no content; QML logs only an error kind. One hold-over: `debug` logs a contact's feature mask (not sensitive). |
| 7.4 | Hashed directory lookups only | OK, with the user's exception (linking the user's own e-mail/number, only on request). Address book matching sends HMAC hashes only. |
| 7.5 | TLS pinning as upstream | OK. The pin list is identical to upstream's `network_security_config.xml` (19 SPKI pins, checked by diff). Hosts outside threema.ch/threema.com are refused before any connection, so the HTTPS client cannot reach third parties. CSP uses libthreema's server-key pinning. |
| 7.6 | No third-party endpoints | OK. All URLs in core, bridge and QML are Threema's (directory, blob, Safe); the QML engine has no network (ADR 0008). Links in messages open in other apps only when tapped. |
| 7.7 | Dependencies pinned, `cargo audit` | Fixed: `cargo audit` was not installed and so not run. It found **RUSTSEC-2026-0384** (race in `tokio::time::Interval`, tokio 1.54.0, used for keepalive and maintenance): updated to 1.54.1. Now clean (256 crates). |
| 7.8 | Android as reference | n/a |

Other checks:
- `#![deny(unsafe_code)]` holds; `unsafe` only in `ffi.rs`, each block with its safety comment.
- SQL: values are always bound parameters; `format!` only inserts fixed column lists.
- Rich text in QML: the two `StyledText` labels (search hits, mentions) escape all user text
  before adding markup (`Format.escapeHtml`); everything else is plain text (ADR 0008).

## Open points

1. **Notification actions over D-Bus can be called by any process on the session bus.**
   `reply(identity, text)` sends a message as the user and `markRead` marks a chat read. Lipstick
   calls them for the notification buttons; another process of the same user could too. Under
   Sailjail, other sandboxed apps only reach session-bus names their permissions allow, so the
   exposure is mainly unsandboxed processes, which as the same user can already do more harm.
   A check that only lipstick may call them was tried and **reverted**: lipstick's caller couldn't
   be confirmed in the emulator, and a wrong check would silently break the Reply button.
   **Fixed 2026-10-27:** the caller turned out to be a short-lived helper (`mliteremoteaction`)
   whose process the sandbox can't read, so the actions now carry a per-chat random token the
   notification holds; calls without it are ignored (tested in the emulator both ways).
2. **Voice recording temp file**: the platform recorder writes an unencrypted
   `voice-recording.m4a` into the app's private cache until it is sent or discarded (then deleted;
   a crash can leave it). Opus fallback records in memory only. Severity: low.
3. **"Save to Downloads"** writes the decrypted file where other apps can read it; that is the
   point of the action, but worth saying so in the UI. Severity: informational.
4. **Fuzzing** the message decoders (§9 Phase 5) is still to do.
5. `cargo audit` should run in CI; there is no CI yet.
