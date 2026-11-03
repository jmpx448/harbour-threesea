# Audit against CLAUDE.md and a private working note — 27 October 2026

> **Status (updated November 2026):** R01–R03 fixed (see the status note in
> `security-audit-2026-10-27.md`), R06 (crash leftovers removed at start; recording while capturing
> is still a plain file in the app's cache), R07 (CI: `.github/workflows/`) and R09 fixed, R08
> withdrawn, R05 decided (the logging rule in CLAUDE.md §7.3, enforced by
> `scripts/check-logging.py`). Open: R04 (needs a phone) and the documentation discrepancies below,
> as far as later documents have not resolved them. The commits are from before the public history.

The app does **not fully meet its standing brief**. The main blockers are Web-session
revocation, private-chat authorization, and group-call receive-state handling. There are also
missing engineering controls, UI deviations, and substantial documentation drift.

This is an audit, not a remediation change. No application source, credentials, settings, or
test identities were changed. Existing security-audit artifacts and concurrent edits to
`ChatPage.qml` and `MessageDelegate.qml` were preserved.

## Scope and interpretation

- Read `CLAUDE.md` and a private working note, then checked their requirements against the
  Rust core, bridge, QML, translations, packaging, build scripts, and relevant ADRs. Nothing
  from that note is reproduced here.
- The code moved during the review, including root-window and conversation QML edits. UI
  observations refer to the inspected working files, not an immutable release; file line numbers
  may shift.
- Later explicit user decisions recorded in CLAUDE.md and ADRs take precedence over older
  statements in the note. Approved exceptions are called out as documentation discrepancies,
  rather than automatically treated as unauthorized features.
- The existing [security audit](security-audit-2026-10-27.md) was an input, not proof by itself.
  Its seven defect probes were rerun successfully against a temporary copy of the current core.
  Each passing defect probe demonstrates vulnerable behavior, **not** a security check passing.
- This is source review plus offline validation. It is not a new screen-by-screen emulator
  review, device certification, exhaustive cryptographic audit, or historical commit audit.

## Findings requiring code changes

### R01 — High: Web authority survives identity deletion and session revocation

**Rules:** CLAUDE.md §1 security parity, §7; ADR 0013's authenticated browser-session boundary.

`delete_all_data()` wipes the database and media but leaves running Web tasks alive.
`on_web_event()` also dispatches messages without checking that the sending session still
exists. An already paired browser can remain authorized across delete/recreate in the same
process, and a queued command can execute after explicit revocation.

**Evidence:** [app_contacts.rs](../core/src/app_contacts.rs), `delete_all_data` at line 155;
[app_web.rs](../core/src/app_web.rs), lines 200 and 276. Reproduced by
`audit_browser_survives_identity_deletion` and
`audit_queued_browser_command_runs_after_session_deletion` (security audit S01).

**Correction:** revoke per-identity/session generations before wiping or replacing identity
state; stop tasks and reject stale events and completions. Numeric session IDs alone are
insufficient if they can be reused.

### R02 — High: private chats are not consistently protected by the core

**Rules:** CLAUDE.md §7.8; ADR 0009; ADR 0013 says private chats stay hidden from browsers.

Three independently reproduced gaps violate that boundary:

- A paired browser can add members to a private group owned by the local identity through
  management handlers that bypass the browser's private-target check.
- A browser subscribed before a chat becomes private still receives later message edits;
  an outstanding media request can likewise deliver its bytes after the privacy change.
- While ordinary `message` rejects a locked private chat, `setStarred` returns its message,
  `draft` returns its draft, and forwarding reads its content. Direct media/voice FFI reads
  also lack the conversation authorization check, as established by source inspection.

**Evidence:** [app_web_manage.rs](../core/src/app_web_manage.rs):215,
[app_web_data.rs](../core/src/app_web_data.rs):596,
[app_web_media.rs](../core/src/app_web_media.rs):189,
[app.rs](../core/src/app.rs):1265 and 1286,
[app_media.rs](../core/src/app_media.rs):806,
[ffi.rs](../core/src/ffi.rs):128 and 196. Four offline probes reproduced these paths
(security audit S02–S04). The high priority reflects the private-group membership change;
the original audit rates the other two categories medium.

**Correction:** centralize authorization for reads, mutations, forwarding, exports, FFI
media reads, and asynchronous emissions. Browser access to private targets must stay denied
even while the local UI is unlocked. Privacy changes must revoke subscriptions and pending
replies. Test all entry points, not just the normal message-list command.

### R03 — Medium: unauthenticated group-call frames change receive-key state

**Rules:** CLAUDE.md §1 security parity and §10 protocol invariants/tests.

The group-call decoder discards earlier keys and advances its ratchet before verifying the
frame tag. A forged footer can therefore make later valid frames undecryptable. An identical
authenticated frame is also accepted again; no receive replay window rejects it.

**Evidence:** [group_call_frames.rs](../core/src/group_call_frames.rs):183–220;
`audit_invalid_frame_poisoning_and_replay` reproduced both behaviors (S05).
This requires access to frames carried by the media transport, for example a malicious SFU;
it is not evidence of an arbitrary unauthenticated Internet attacker reading call audio.

**Correction:** authenticate candidate state before committing it and enforce a bounded
replay window that permits legitimate reordering.

### R04 — Medium: notification actions lack caller and lock authorization

**Rules:** CLAUDE.md §7.8 and ADR 0009's lock boundary.

The exported D-Bus `reply` and `markRead` slots forward requests without authenticating the
caller. The QML handlers invoke core commands without consulting the app/private-chat lock.
A local process allowed to reach the bus service can submit these actions as the user.

**Evidence:** [dbusinterface.cpp](../bridge/dbusinterface.cpp):29, 61, 67;
[harbour-threesea.qml](../qml/harbour-threesea.qml):558–570 (S07).
Actual reachability depends on Sailjail and device bus policy; no D-Bus message was sent.

**Correction:** implement and device-test legitimate notification caller authorization and
an explicit core lock policy. The device Reply check was deferred to later; that
deferral does not make the current boundary compliant.

### R05 — Medium: logging lacks the required redaction boundary

**Rule:** CLAUDE.md §7.3 explicitly requires one logging function with a redaction layer.

`init_logging()` installs a normal tracing formatter and filter, with no redacting layer.
Call sites log arbitrary error displays and the server's close reason; C++ uses separate
`qWarning`/`qInfo` paths. For example, the mediator's supplied `reason` is formatted directly.
This depends on each caller and dependency never including sensitive data, rather than
enforcing the stated rule.

**Evidence:** [runtime.rs](../core/src/runtime.rs):109,
[connection.rs](../core/src/connection.rs):319,
[secretstore.cpp](../bridge/secretstore.cpp):128,
[callengine.cpp](../bridge/callengine.cpp):698.

**Correction:** use structured allowlisted diagnostics and a shared redaction policy for
Rust and Qt, including external error strings. Add representative sensitive-data checks.
This finding establishes the missing control; it does not claim an observed production
private-key or message-body leak.

### R06 — Low security severity, explicit rule failure: plaintext recording staging

**Rules:** CLAUDE.md §1 and §7.1–2: encrypted local storage and no unencrypted secrets at rest.

The AAC recorder writes `voice-recording.m4a` to the application cache. Normal cleanup removes
it, but a crash leaves it behind, and the constructor does not sweep an old recording.
The cache file remains plaintext while capture is active even with normal cleanup.

**Evidence:** [voicerecorder.cpp](../bridge/voicerecorder.cpp):47, 142, 159, 199 (S08).
Camera capture also uses plaintext staging, but its dedicated directory is swept at core
startup. Explicit user exports to Downloads/Gallery are a different, intentional behavior.

**Correction:** prefer memory/encrypted staging; at minimum sweep crash leftovers and restrict
permissions. If platform capture requires plaintext, document and obtain the required policy
exception rather than calling it fully compliant. Startup deletion alone does not satisfy
the literal no-plaintext-at-rest rule.

### R07 — Medium engineering priority: required CI and release checks are absent

**Rules:** CLAUDE.md §7.7, §8, Phase 0 and Phase 5.

There is no repository CI configuration enforcing tests, Clippy, `cargo audit`, the C++ test
target, or the three Sailfish architecture builds. Local build scripts and the call-engine
test target exist, but do not implement the required CI. No coverage-guided fuzz target was
found; the existing malformed-input smoke probe is not a fuzz campaign.

**Evidence:** repository tracked-file inventory;
[build-core.sh](../scripts/build-core.sh),
[test-callengine.sh](../scripts/test-callengine.sh),
[callengine-test.pro](../tests/callengine/callengine-test.pro).

**Correction:** add CI enforcing the documented checks and a release build matrix for
aarch64/armv7hl/i486. Add decoder fuzz targets as the already planned Phase 5 work.
Cargo.lock is committed and release builds use `--locked`; semver ranges in Cargo.toml alone
are therefore not evidence that release dependencies are unpinned.

### R08 — Withdrawn: rounded message pictures are intended

The user confirmed on 27 October 2026 that the three-sided rounding of message pictures is
intended. It is an approved exception to the general square-picture rule, not a UI defect.
The standing brief now records this exception. No rendering change is needed.

### R09 — Low: newer Web screens miss declared UI conventions

**Rules:** `docs/ui-guidelines.md`: all content orientations and a vertical decorator on
scrollable views.

`WebSessionsPage` does not explicitly allow all orientations. `WebHostDialog` also omits
that declaration and contains a `SilicaFlickable` without a `VerticalScrollDecorator`, even
though its long certificate/trust text can need scrolling on a small or landscape screen.

**Evidence:** [WebSessionsPage.qml](../qml/pages/settings/WebSessionsPage.qml):9;
[WebHostDialog.qml](../qml/pages/settings/WebHostDialog.qml):7–51.

**Correction:** use the same orientation and scroll-decorator conventions as the other
settings pages, then check the long trust text in both orientations in the emulator.
An omitted declaration is established here; actual default orientation behavior was not
measured on Sailfish.

## Additional security validation still required

The existing security audit describes two more concerns that prevent a broad security sign-off:

- **Call authentication timing (V01):** the 1:1 media pipeline enters `PLAYING` before the
  queued connected callback checks the peer certificate. ADR 0012 explicitly documents this
  ordering. It needs a wrong-certificate test on the shipped GStreamer stack to establish
  whether any media flows first. This audit does not promote the possible leak to a reproduced
  exploit. See [callengine.cpp](../bridge/callengine.cpp):194, 420, 791.
- **Resource limits (S06):** complete HTTP responses are buffered before the blob size check;
  Web reassembly permits large aggregate payloads and does not budget header-only chunks
  adequately. See [https.rs](../core/src/https.rs):350 and
  [web/chunks.rs](../core/src/web/chunks.rs):14–99. No destructive OOM test was run.

Destination policy also needs tightening: rendezvous accepts a `wss://` URL and relies on the
TLS verifier to reject non-Threema hosts. DNS/TCP setup happens before certificate verification.
This does not meet the strongest reading of §7.6's ban on contacting third-party endpoints.
Check the allowlist before connecting, preserving ADR 0013's explicit self-hosted Web exception.
See [rendezvous.rs](../core/src/rendezvous.rs):86, 125 and
[websocket.rs](../core/src/websocket.rs):52–67. No successful unauthorized HTTP exchange was
demonstrated.

## Documentation and architecture discrepancies

These need reconciliation, but should not be fixed by blindly changing functioning behavior
to match an older document.

| Requirement or claim | Current evidence | Recommended resolution |
|---|---|---|
| §7.6: no third-party endpoints | ADR 0013 explicitly approves user-authorized self-hosted Web servers with pinning. | Add this named exception to §7.6. The approved feature itself is not an unauthorized deviation. |
| §7.1: only Rust touches private/session keys; §5: no protocol code in C++ | ADR 0012 documents per-call ECDSA certificates and fingerprint handling in the engine; `dtlscertificate.cpp:43` generates a private key and `dtlscertificate.h:18` stores its PEM in QByteArray. SecretStore also necessarily handles the database key. | Precisely document the approved platform-key boundary and review erasure of native key copies. The blanket wording does not describe the implementation. |
| §4: all persistence behind a Store trait | `store.rs:174` defines a concrete SQLCipher `Store`, not a trait. `threesebackend.cpp:397` and `:685–719` persist local preferences through QSettings, including per-chat tone keys. | Document explicit local-preference exceptions and the concrete Store design, or implement the requested abstraction. QSettings was not observed storing message bodies or identity private keys. |
| §5: separate bridge classes for notifications, contacts, background service and sharing | Much of this integration is in root QML (`harbour-threesea.qml:253,297,378,484`) and `AddressBookReader.qml`; the named C++ classes do not exist. Incremental generic JsonListModel replaces several named model classes. | Update the architecture description or refactor where justified. Generic incremental models satisfy the behavioral update requirement; class-name differences are not runtime failures. |
| §5: passphrase fallback when Sailfish Secrets is unavailable | `secretstore.cpp:60–77` emits failure when collection creation fails. RPM explicitly requires Secrets. The optional wrapped-key passphrase still depends on Secrets. | Either implement the promised independent fallback or explicitly make Secrets mandatory in the brief. Failure is closed, not a plaintext fallback. |
| Working rule: every new JSON command is documented in docs/ffi.md | `docs/ffi.md` omits Web commands (`webPair`, `webDelete`, etc.) and group-call command families present in `app.rs:575–619`, plus several older commands. Its opening still says four C functions; ffi.rs exports 17. | Complete the command/event/C-ABI reference and add a command-coverage check. Missing Web revocation semantics are particularly relevant to R01. |
| CLAUDE.md: German translation marked done | All 609 literal QML/JS translation IDs are present in both catalogs, but 348 used German entries are unfinished, including 335 empty translations. | Mark German as incomplete. Translations were explicitly deferred until final polish, so this is not a demand to translate per feature. English source strings exist for all entries; blank unfinished English translations are intentional. |
| §6: cover shows last message | Cover shows unread count and latest sender, matching the alternative wording in ui-guidelines.md, but not message text. | Align the brief with the intended cover privacy/content behavior. |
| ui-guidelines.md screen patterns | Its one-line chat previews, docked composer and generic bubble styling predate the detailed Phase 4.5 rules for three-line previews, scrolling composer and specific bubbles. | Make Phase 4.5's later decisions canonical and update the older pattern table. |

The repository layout example also places `libthreema/` and `sqlcipher/` differently from the
actual vendored libthreema and Cargo-bundled SQLCipher. The prose already partially corrects
this; update the example rather than moving working dependencies merely to match it.

## Rule coverage and controls observed

| Area | Assessment |
|---|---|
| Native stack, branding, licensing markers | Rust + C++14/Qt + Silica; unofficial branding in README/desktop/About. All scanned application-owned `.rs/.cpp/.h/.qml/.js/.py/.sh` source files carry AGPL markers. This is not a third-party license-compliance certification. |
| libthreema and toolchain | Reused vendored library; root and vendor toolchain files both specify 1.95. Several protocol state machines have offline tests. Local upstream patches were not exhaustively audited here. |
| Unsafe boundary | Core denies unsafe code at the root; explicit C ABI is isolated in ffi.rs. Native-library safety still needs separate validation. |
| Encrypted store and PIN | SQLCipher and encrypted media paths present; salted scrypt for lock credentials. Storage/wrong-key tests pass. Plaintext capture staging and incomplete read authorization remain exceptions/failures above. |
| Hashed directory matching | Address-book hashing remains in Rust; the documented own-address linking exception is separate. No new exception was inferred. |
| TLS pins | All 19 pins and their four domain sets match the pinned Android `network_security_config.xml`. This checks the repository's chosen upstream version, not whether upstream has changed since that tag. |
| UI text/network safety | Plain-text default in root QML and offline QML network manager remain present; rich-text formatting is explicitly opted into. No blanket rich-text pass was inferred from the default alone. |
| Incremental models | JsonListModel and ContactListModel use row insert/remove and data-change notifications; reset calls are initialization/clear paths, not resets on every ordinary event. |
| Packaging split | Source gates service installation and GStreamer call engines on Chum; Harbour does not install the systemd unit. Current RPM validation was not rerun. C++17 AEC wrapper is an explicit ADR 0012 exception. |
| UI API versions | Imports use QtQuick 2.6 and QtMultimedia 5.6; no forbidden `Sailfish.Silica.private`, `Sailfish.Gallery`, or `com.jolla.*` import was found. This is not a current Harbour API certification. |
| Translatability | 609 literal IDs found, none missing from either catalog; English sources and English plural forms present. Runtime-generated strings/raw server errors were not exhaustively classified. |
| Phase completion | Phone/battery/camera/audio checks, fuzzing, translations, store submission and Chum source-build resolution remain release work. Do not report them complete on the strength of unit tests. |

## Validation performed in this audit

| Check | Result |
|---|---|
| `PROTOC="$PWD/tools/protoc/bin/protoc" cargo test --workspace --locked --offline -q` | 142 passed, 7 ignored; no failures. SQLCipher's wrong-key test emits expected decryption diagnostics. |
| Same environment, `cargo clippy --workspace --all-targets --locked --offline -q` | Passed without warnings. |
| `sh docs/security-audit-2026-10-27/run-probes.sh` | Seven defect reproductions and one parser smoke probe passed. Temporary core copy only; no live-service messaging. |
| `cargo audit --db /tmp/threesea-audit-advisory-db --no-fetch --no-yanked --json` | 286 dependencies, zero reported vulnerabilities/warnings against the available cached database. No refresh or yanked-crate check in this run; not a native-library audit. |
| Pin comparison | Four domain sets / 19 pins match the pinned upstream XML. |
| Translation/source scans | Results above; scans were read-only. |
| SDK / emulator / device | Not run. Host has no qmake; the SDK container was not available to this review in this environment. No new UI screenshots, RPM/Harbour results, native C++ tests or device observations are claimed. |

Recommended order: fix R01–R03 and the missing private-media gates first; validate call
authentication; address notification authorization, resource limits, redaction and recording
staging; establish CI/fuzzing; reconcile the brief and working notes; finish the visual and device
release checks against the corrected requirements.
