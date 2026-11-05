# CLAUDE.md — harbour-threesea (unofficial Sailfish OS client, works with Threema Libre licenses)

This file is the standing brief for Claude Code working in this repository. Read it fully before
doing anything. It describes the goal, the chosen stack, the constraints, the phased plan and the
working rules. You are expected to make your own tactical decisions within these boundaries,
keep this file and `docs/` up to date as you learn things, and never silently deviate from the
security rules.

---

## 1. Goal

An unofficial native client for Sailfish OS that works with **Threema Libre** licenses: it uses
licenses bought in the Threema Shop, uses no Google services, runs in the background, and feels at
home among Jolla's apps. No custom or flashy UI elements; when Silica or a Jolla app has a pattern, use it
(see `docs/ui-guidelines.md`). Long-term every Threema feature is in scope.

The app is called **harbour-threesea** ("Threesea" in the launcher) — we do not use Threema's
name or logo as our own, and we state that we are unofficial.

Build a **native** Sailfish OS client that is compatible with Threema:

- Native UI built with QML + Sailfish Silica (cover page, pulley menus, remorse timers, Ambience-aware).
- Full Threema protocol compatibility by **reusing `libthreema`** (Rust) from the upstream
  Android repository (`domain/libthreema`), plus a Rust core for everything Android still keeps in
  Kotlin/Java.
- Security posture at least as strong as the Android app: same crypto, same key handling
  discipline, encrypted local store, no secrets on disk unencrypted, no telemetry.
- As much Sailfish platform integration as is feasible: notifications, contacts, sharing,
  background connection, media.

Upstream reference: https://github.com/threema-ch/threema-android (AGPL-3.0).
This port is also AGPL-3.0. Keep every source file licensed accordingly.

**Not now (deferred, not dropped):** nothing at the moment. Multi-device (Threema for Desktop,
ADR 0011), calls (Full flavour only, GStreamer webrtcbin; 1:1 and group calls, ADR 0012) and Threema Web
(relayed data, both flavours, ADR 0013) are in. **Non-goals:** Threema Work/OnPrem variants, calls in Jolla's own phone app (Android has no such
integration either; user decision 2026-10-26),
Android-specific features (Play Services, Android Auto, Wear, widgets).

---

## 2. Stack decision (do not re-litigate without a written reason in `docs/decisions/`)

| Layer | Technology | Why |
|---|---|---|
| Protocol / crypto core | **Rust** — `libthreema` (upstream, vendored or submodule) + our own `core` crate | Reuse Threema's own protocol implementation; memory safety for everything that touches keys and network bytes. |
| Bridge | **C++14 / Qt 5.6** (what SFOS ships; see ADR 0001) | Thin `QObject` wrappers, `QAbstractListModel`s, D-Bus, keepalive, threads. |
| UI | **QML + Sailfish Silica** | Native look and behaviour. |
| Local store | **SQLCipher** (bundled) | Same approach as Android; encrypted at rest. |
| FFI | C ABI via `cbindgen` (preferred) or the `cxx` crate | Stable, debuggable boundary; avoid passing complex ownership across it. |
| Packaging | RPM via the Sailfish SDK build engine | Required for install; Two flavours from one tree: Lite (Jolla Store rules) and Full (background service, calls); released on GitHub first, Jolla Store/Chum later (ADR 0014). No OpenRepos. ADR 0004. |

Python / PyOtherSide was considered and rejected: it would put the protocol loop and key
handling behind a dynamic boundary, complicates the persistent connection, and gains nothing
over Rust here.

---

## 3. Repository layout (create and keep to this)

```
harbour-threesea/
  CLAUDE.md                 this file
  README.md                 user-facing: what works, how to build, how to install
  LICENSE                   AGPL-3.0
  docs/
    decisions/              one short markdown file per architectural decision (ADR style)
    android-mapping.md      which upstream Kotlin/Java package maps to which module here
    protocol-notes.md       anything learned about the protocol while porting
    build.md                exact, reproducible build steps
    ui-guidelines.md        how to make every screen feel at home among Jolla's apps (read before UI work)
    ffi.md                  core ⇄ bridge JSON protocol
    emulator.md             running/driving the official emulator without the SDK IDE
    screenshots/            pictures for the README; its README.md says which version each is from
                            and how to retake them without showing real data
    STATUS.md               phase status
  third_party/
    threema-android/        git submodule of upstream (read-only reference)
    libthreema/             vendored upstream domain/libthreema + our commits (ADR 0006;
                            pristine imports on branch vendor/libthreema)
    opus-pure/              vendored Opus encoder/decoder (voice fallback), one patch
    webrtc-audio-processing/  submodule: echo cancellation for calls (Full)
  core/                     Rust crate `threesea-core`: our application core (see §4); SQLCipher
                            comes bundled through rusqlite (pinned in Cargo.lock)
    include/                C header for the FFI (hand-written, mirrors core/src/ffi.rs)
    fuzz/                   cargo-fuzz targets for the decoders fed from outside (own
                            workspace, nightly; docs/build.md)
  bridge/                   C++: Qt side of the FFI, models, platform integration
  qml/                      Silica UI
    cover/
    pages/
    components/
  icons/                    launcher icons (generated by scripts/make-icons.py)
  translations/             id-based .ts files
  tests/                    tests run in the SDK/emulator: callengine (calls), models, gifdecoder,
                            notificationtokens, lifecycle, voicerecorder, pageload (every page opened; any
                            QML warning fails), lockstate (the window's QML on
                            real Silica, no window system)
  rpm/
    harbour-threesea.spec   %bcond_with full selects the Full flavour (ADR 0010, 0014),
                            %bcond_with demo the emulator's demo data
  systemd/                  Full only: user service starting the app at login
  notifications/            Full only: lipstick's category of the incoming call's notification
  scripts/                  sdk-build.sh = full Docker build (sdk/Dockerfile), build-core.sh,
                            update-libthreema.sh, emulator*.py/sh (UI testing), check.sh (all
                            checks; hooks/pre-commit runs its quick part)
  .github/                  CI (checks; SDK builds for pull requests into `release`), releases
                            from the `release` branch (docs/release.md), issue templates
  harbour-threesea.desktop
  harbour-threesea.pro
  Cargo.toml                Rust workspace (excludes third_party), rust-toolchain.toml
  README.md, CHANGELOG.md, SECURITY.md
```

`libthreema` is **vendored** (`third_party/libthreema`, vendor branch `vendor/libthreema`, ADR 0006);
the upstream repository is a pristine submodule pinned to the same tag for reference. Update
both together with `scripts/update-libthreema.sh <tag>` and record the version in `docs/build.md`.

---

## 4. Rust core (`core/`)

Responsibilities, roughly mirroring what Android keeps outside `libthreema`:

- **Identity:** ID creation, ID backup export/import, Threema Safe backup/restore, license check
  against the directory server.
- **Directory client:** public key lookup, phone/e-mail hash lookup (hashing done here, nothing
  unhashed ever leaves the device), feature masks.
- **Chat server connection:** connect, handshake, keepalive/echo, reconnect with backoff, queue
  outgoing messages while offline, acks.
- **Message handling:** encode/decode all message types used in 1:1 and group chats, delivery
  receipts, typing indicators, group management (create/rename/members/leave/sync), file/blob
  upload and download with the blob server, thumbnails.
- **Storage:** all persistence in one SQLCipher `Store` (`core/src/store.rs`, a concrete type,
  not a trait: there is one backend), via `rusqlite`'s bundled-sqlcipher; schema migrations are
  versioned. Media files are encrypted separately (`media.rs`). Device-local preferences that
  aren't secret (log level, the notification sounds of all chats, background running) live in the
  bridge's QSettings in the sandboxed config directory; never message content, keys or who one
  chats with: per-chat tones are in the store, notification action tokens under a salted hash
  (against casual reading only: all IDs can be tried offline) and only while their notification
  exists and a short grace period after it, which is what limits a copy of the file.
- **FFI surface (`core/src/ffi.rs`):** a small, explicit C API: open/close app, event callbacks
  (new message, state change, contact update), commands (send text, send file, create group…).
  Everything crossing the boundary is plain data (ids, strings, byte buffers); no Rust objects
  leak to C++.

Rules:
- Pin the Rust toolchain to upstream's `domain/libthreema/rust-toolchain.toml` (currently 1.95;
  SFOS only ships 1.75, so we bring our own — ADR 0002).
- Use `libthreema`'s types wherever they exist. Do not reimplement crypto that `libthreema`
  already exposes. If something is missing, implement it in `core` with the same primitives
  (`libsodium`-compatible NaCl boxes, Blake2b, etc.) and document in `docs/protocol-notes.md`.
- `#![deny(unsafe_code)]` at the crate root, `allow` only in `ffi.rs`.
- Zeroize key material on drop (`zeroize` crate).
- Sans-I/O style where practical: protocol state machines separated from sockets, so they are
  unit-testable without a network.

---

## 5. C++ bridge (`bridge/`)

- One `ThreeseBackend` QObject owning the Rust handle (the core runs on its own thread);
  callbacks reach the GUI thread through queued signals.
- Models: the generic `JsonListModel` (chats, messages, members…; keyed rows, applied with
  insert/remove/move/dataChanged, never a reset per event) and `ContactListModel`.
- Platform integration where it is simplest: notifications (Nemo.Notifications, actions over
  `DBusInterface` with a per-chat token), keepalive (`BackgroundJob`) and sharing
  (Sailfish.Share) in QML; address book matching through `AddressBookReader.qml` (opt-in,
  hashed in Rust); voice recording, players, camera and calls in their own C++ classes. The
  Full flavour adds a systemd user unit (ADR 0010).
- `SecretStore`: the database key lives in Sailfish Secrets (required by the RPM; without it
  the app fails closed, no plaintext fallback). Optionally it is wrapped with a passphrase
  ("Encrypt stored data", ADR 0009). The key is never written unencrypted. Removing the ID
  replaces the key and the database file (ADR 0018).
- No business logic in C++. If you find yourself writing protocol code here, move it to Rust.

---

## 6. QML UI (`qml/`)

- **Read `docs/ui-guidelines.md` first.** Only Harbour-allowed imports (Jolla Store rules), in both flavours.
- Follow Silica conventions: `ApplicationWindow` + `pageStack`, `PullDownMenu` for primary
  actions, `RemorseItem` for destructive ones, `SilicaListView`, `Theme.*` for all spacing,
  colours and fonts (no hard-coded sizes), cover page showing the unread count and who wrote (not the text; it can be hidden too).
- Pages (initial set): Onboarding/License, ID setup/restore, Conversation list, Chat,
  Contact details, Group details, New chat/group, Settings, Backup (ID + Safe), About/Licenses.
- Keep QML dumb: it binds to models and calls backend slots. No crypto, no persistence, no
  JavaScript protocol logic in QML.
- Everything the user sees must be translatable — id-based like Jolla's apps:
  `qsTrId("threesea-<kind>-<name>")` with a `//%` engineering-English comment; `.ts` files in
  `translations/`, refreshed with `scripts/update-translations.sh` after UI text changes; German
  texts live in `scripts/translations-de.py` and the French in `scripts/translations-fr.py`,
  which are always the **last** step (they fill the texts and write the files in the
  repository's layout; running lupdate afterwards reformats every line). New
  texts get their German at the very end, after all features and UI polish (user decision
  2026-10-22); until then they stay unfinished. Plurals: `qsTrId(id, n)` with `%n`, forms in
  both files.

---

## 7. Security rules (hard constraints)

1. Only `libthreema`/`core` touch the identity's private key, message keys and plaintext message
   bodies at rest. Bounded exceptions (ADR 0009, 0012): the bridge holds the 32-byte database
   key in memory to start the core and to (un)wrap it; the Full call engines generate a per-call
   DTLS key in C++ for WebRTC, wiped when the call ends; decrypted media pass through the UI's
   readers in memory only (wiped buffers).
2. Database encrypted with SQLCipher; key derived/stored as described in §5 `SecretStore`.
3. Never log key material, PINs, message contents, phone numbers or e-mail addresses. Logs
   carry fixed texts plus kinds, codes, counts and internal IDs; text from other users or
   devices never verbatim (Threema's servers' own error texts are fine). Enforced by
   `scripts/check-logging.py` (part of `scripts/check.sh`); a deliberate exception is marked
   `log-ok: <why>`. How much is logged is a setting (Settings → Advanced: problems only by
   default), the same rules at every level. (User decision 2026-10-27, replacing "one function
   with a redaction layer": pattern redaction can't recognise message text.)
4. Directory lookups use the hashed-identifier endpoints only, same as Android. One exception
   (user decision 2026-10-22): linking the user's *own* e-mail address or phone number to the
   ID sends it in plain to Threema's directory, as Android does, only when the user asks for it
   in Settings, and it is never logged.
5. TLS to Threema servers with certificate pinning matching upstream's pins (copy from the
   Android source and record the source file in `docs/protocol-notes.md`).
6. No third-party network endpoints. No analytics, no crash reporting that leaves the device.
   Exceptions, both only on the user's explicit choice: self-hosted Threema Web servers the user
   allows (ADR 0013), and the update check against GitHub in builds for GitHub (opt-in, asked
   once, daily at most, nothing but the app version sent; ADR 0014; user decision 2026-10-27).
7. Dependencies: pin versions; run `cargo audit` in CI; prefer well-known crates; justify every
   new dependency in the commit message.
8. The app lock PIN/passphrase is stored only as a slow hash (scrypt) inside the encrypted
   store; private chats are hidden by the core, not only the UI (ADR 0009).
9. Treat the Android source as a reference, not a template: port behaviour and invariants, not
   Activity/Service structure.

---

## 8. Platform constraints to keep in mind

- Target architectures: `aarch64` (primary, current devices) and `armv7hl` (older devices);
  `i486` for the emulator. Build all three in CI once the toolchain works.
- No FCM/APNs. Push does not exist; the app owns its connection. Design the reconnect/keepalive
  logic for a phone that sleeps aggressively.
- Harbour (Jolla Store) restricts allowed shared libraries and forbids daemons/system services.
  Plan for two targets: a Harbour-compliant build (everything statically linked, no system
  service), the Lite flavour, and the Full flavour with the background service and calls. Document
  the delta. No OpenRepos. Distribution starts with RPMs on GitHub (ADR 0014).
- Sailfish ships **Qt 5.6.3** and QtQuick ≤ 2.6 (Harbour limit); check before using an API.
- **Minimum Sailfish OS: 5.1** (user decision 2026-10-21, lowered from 5.2 so the official
  emulator — newest image is 5.1.0.11 — can run it). Builds against 5.1 run on 5.2. No
  compatibility work for 5.0 or older.
- Build target is SFOS 5.1.0.11 (`coderus/sailfishos-platform-sdk`); the SDK userland is
  32-bit x86. 5.1 and 5.2 both ship glibc 2.41 and Qt 5.6.3.
- Sandboxing (Sailjail) applies to apps; declare the needed permissions in the `.desktop` file
  (`Contacts`, `Notifications`, `Internet`, `Microphone`, `Camera`, `Pictures`, `Documents`…).

---

## 9. Phased plan

Work phase by phase. Each phase ends with: it builds in the SDK, tests pass, `README.md` and
`docs/` updated, a short status note in `docs/STATUS.md`. Do not start the next phase while the
current one has known-broken core functionality.

Status 2026-10-29: Phases 0–4.5 done (phone tests collected for the end), Phase 5 in progress;
details in `docs/STATUS.md`.

### Phase 0 — Toolchain and skeleton (highest risk, do first)
- Add upstream as submodule; pin `libthreema`.
- Make `libthreema` + an empty `core` crate build for `aarch64-unknown-linux-gnu` and
  `armv7-unknown-linux-gnueabihf` inside the Sailfish SDK build engine. Record exact steps in
  `docs/build.md`.
- Expose one trivial function over the C ABI, link it into a minimal Silica app, show the result
  on a page. Produce an installable RPM. Run it in the emulator.
- Set up `cargo test`, `cargo clippy`, `cargo audit`, and a C++ unit test target.

### Phase 1 — Identity
- ID creation with license key; ID backup export/import; Threema Safe backup and restore.
- Directory lookups; contact storage; verification levels.
- UI: onboarding, ID setup/restore, settings skeleton.

### Phase 2 — Messaging core
- Chat server connection, handshake, keepalive, reconnect, offline queue.
- 1:1 text messages, delivery receipts, typing indicators.
- SQLCipher store with migrations; conversation and message models; chat UI; cover page.

### Phase 3 — Groups and media
- Group lifecycle and sync.
- Image/file/voice messages via blob server; thumbnails; media viewer; voice recording.
- Quotes, reactions (as far as the protocol version supports), message deletion/editing.

### Phase 4 — Platform integration
- Notifications with actions; background connection strategy; contacts sync (opt-in);
  Transfer Engine share plugin; URL/ID link handling.
- Battery and reconnect behaviour measured on a real device and documented.

### Phase 4.5 — UI polish (requested by the user)
- Go screen by screen against Jolla's own apps (Messages/SMS, People, Settings, Gallery) on a
  device or the emulator and remove every deviation in layout, navigation and wording.
- Known items (keep this list current; add whatever the user points out):
  - Chat page like Jolla Messages: name on top, avatar to the right of it, contact details as an
    **attached page** (swipe left, forward indicator top right), not a pulley item. *(done early
    in Phase 2)*
  - Avatars everywhere (chat list, contacts, notifications) once profile pictures exist (Phase 3);
    until then the theme's head-and-shoulders icon `icon-m-contact` without background (user).
  - Chat header: Threema ID only under non-contacts; contacts show just the name. Header stays
    fixed; the input scrolls with the messages like Jolla Messages (user). *(done)*
  - Delivery-state icons: the same meanings as Threema's states, drawn by us and filled so they read this small:
    clock (pending), envelope (sent), inbox (delivered), eye (read); ours
    (`scripts/make-ui-icons.py`), the theme's outlines were unreadable (user 2026-10-26). *(done)*
  - Chat and chat list like Jolla Messages (user's screenshots): centred day headers, bubbles with
    a small edge gap, corners: inner bottom always rounded, top corners only on the first bubble
    of a run, outer bottom square; runs packed tightly; text `fontSizeMedium` (same as day
    headers); time grey on every message (user 2026-10-26; was: only if the next is > 1 min later), "Tuesday 20:42" format; keyboard stays
    open after sending; bubble overlay white 12 % (incoming) / 6 % (outgoing); sticky day under the
    name while scrolling; time and delivery state after a short one-line text, else on their own row under the content (user 2026-10-26), name in primary
    colour, no channel label under the input; list with right-aligned day sections, clock time,
    3-line previews, unread: name stays white with the unread count after it in the highlight
    colour, preview tinted; the delivery icon in the preview's colour (user 2026-10-26). *(done)*
  - Editing a message pulls it back into the chat's input box ("Editing message" + cancel above
    it), no separate edit page (user). *(done)*
  - Contacts and chat lists: tapping a row opens the chat, tapping its picture opens the
    contact's (or group's) details (user). *(done)*
  - Pictures are square with no rounded corners, except message pictures, whose three-sided
    rounding follows the bubble and is intended (user confirmation 2026-10-27), and the call
    previews/tiles specified below. Contact and group pages start with a
    Jolla People card (square picture at the left edge, name on the highlight background; tap
    the picture for full screen) (user). *(done)*
  - Single choices use Silica's ComboBox like Jolla Settings (Display → Orientation / Blank
    display after): applies at once, Silica shows few options inline and many as a page (user
    2026-10-23, replaces the earlier "Dialog accepted by swipe" rule). *(done: ChoiceBox)*
  - Search in a chat is in a pull-down menu of the chat (on the page, since the bottom-to-top
    message list cannot host one: pull on the header), not on the contact page (user). *(done)*
  - Single-choice dialogs mark the choice with Silica's glowing switch (TextSwitch) (user). *(done)*
  - Multi-select pickers (group members, list recipients): like Jolla's top menu pickers
    (IconTextSwitch): glowing switch, name, picture at the right edge; never check marks (user
    2026-10-23). *(done)*
  - Pinning: the star top right on contact/group pages (People's favourite star; the theme has no
    pushpin) and the chat list's context menu (user). *(done)*
  - "Last backup" shows 12 h time on a 24 h device. *(done)*
  - Message long-press menu: reaction row (Threema's quick reactions, then "+" for any emoji
    from the emoji panel), then Forward, Star, Quote as columns (icon and small label; the one
    used most at the right, nearest the thumb), then the other actions (Copy, Edit, Save,
    Close poll, Details…) as ordinary menu items — a third column row looked convoluted — and
    last Delete: "Delete for me" and "Delete for
    everyone" as two columns while deleting for everyone is possible, else an ordinary full-
    width "Delete" item without icon (a static menu, nothing changes while it is open — user
    found an in-place sub-choice not Sailfish-like) (user 2026-10-23, reworked 2026-10-28). *(done)*
  - Group members and list recipients like Jolla Settings → Lock screen → Select shortcuts:
    "Add …" row with a round plus, a round × beside each entry (remorse), no pulley (user). *(done)*
  - Chat list: pulley busy (blinking) while connecting, header description "Offline" /
    "Connecting…", a "Connect now" pulley item while offline, like Jolla Mail (user). *(done)*
  - Photos and videos taken inside the app (camera page from the attachment menu) (user).
    *(done; capture to be tested on the phone)*
  - Call page (user 2026-10-23): the other side's video in a frame filling down to name and
    time (radius like the bubbles, bottom left square, like an incoming bubble), our preview at
    its top right (bottom right square, like an outgoing bubble), buttons mute / camera / hang up
    right under name and time; pull-down menu: hide own picture, switch camera. *(done)*
  - Group call page (user 2026-10-26): everyone as square tiles, two columns, us first; corners
    like the bubbles (others: bottom left square, us: bottom right square); a running call as a
    banner under the group chat's header (who, how long, Join/Return). *(done)*
  - Chat: the sticky day steps aside (fades) while a day header of the list passes the top, so
    "Yesterday" never overlaps "Today". *(done)*
  - Media gallery: delete from the grid (long press, remorse), like Android. *(done)*
  - Settings → Notifications icon: ours (`scripts/make-ui-icons.py`); the theme's
    icon-m-notifications has no full-opacity element and looked washed out beside the other
    settings icons (user 2026-10-27). *(done)*
  - Twemoji also in the chat input while typing (highlighter hides the system glyphs, pictures
    drawn over them; the text stays plain) and in reactions. Notifications keep the system's
    emoji: their text is drawn by Lipstick, plain text only. *(done)*
  - Own emoji picker like Android (user 2026-10-26): a smiley left of the input swaps
    the keyboard for a panel: recently used, Threema's categories, skin tones on long press,
    backspace, and a GIF tab with the favourite GIFs (add from the device, long press to
    remove; replaces the chat pulley's "Send GIF" page). Styled like the keyboard (tint, top
    line, tabs as keys); tab icons from the theme where one fits (history, contact, game
    controller, car, the keyboard's backspace), else ours in the theme's style with its dimmed
    element (`scripts/make-ui-icons.py`); skin tones in a context menu like a bubble's (user); swipe down closes it like the
    keyboard, the chat's pulleys are off while it is open (user). *(done)*
  - Emoji in the chat input with some room on both sides, every emoji equally wide (user). *(done)*
  - Reactions under a bubble: small, nearly round chips with the emoji's picture, always in the
    highlight colour; the top corner under the bubble square (incoming left, outgoing right),
    the others round (user). *(done)*
  - Polls: closing is a message of its own on both sides (Android's "Poll closed. Tap to view
    results"); a closed poll opens a read-only results page with Silica's ProgressBar per choice
    (no own bar components, user), who voted, not the disabled voting dialog (user). *(done)*
  - German translation (Sailfish style: "du", ß; Threema's terms). *(done; all texts translated
    2026-10-27; new texts go into `scripts/translations-de.py`, ideally with the change)*
  - Phone feedback 2026-10-28 (user): keyboard stays open when a message arrives; chat scrolls
    back through the whole history (paged) with Silica's quick scroll; an unread-start line in
    the highlight colour, the chat opens there; voice bubbles use Silica's Slider; unplayed
    voice messages glow; voice queue with a soft chime between consecutive voice messages;
    at-the-ear playback over the earpiece (proximity, screen black, pause when lowered);
    failed uploads show the warning icon and retry on reconnect; the backup feature is called
    "Data backup" with a results page after the import. *(done; voice/proximity need the phone)*
  - 2026-10-28, second round: an in-app log with a viewer and "Save support package"
    (Settings → Advanced; §7.3 holds at every level), and a forward-secrecy switch under Privacy → Encryption
    (user decision: off = exactly the multi-device posture, as an escape while FS session
    churn shows contacts repeated security notices; Threema Web shows quotes). *(done)*
  - 2026-10-29, third round: at-the-ear playback goes through the calls' audio route (Qt's
    voice-communication role plays scrambled silence on the phone), the chime between two voice
    messages is a soft bell at a quarter of the level, voice playback speed (1× / 1.5× / 2×) on
    the bubble, and the settings Android has and we lacked: play at the ear, in-app sound and
    vibration, group calls on/off, message text size, chat background, accept profile pictures,
    keep messages (auto-delete), video size (Full; GStreamer), names for linked devices and
    "Mark as unread" in the chat list. *(done; voice, video and the background need the phone)*
  - 2026-11-02 (user): videos were black (VideoOutput needs `mediaObject` on its source); voice
    stops on closing the window, drops when paused and the chat is left, never restarts by itself;
    voice and video on the system's media controls over MPRIS (`MediaSession.qml`): Sailfish shows
    those on the lock screen only, without a position slider. *(done; the phone must confirm video)*
- Navigation rule: pulley menus are for actions; details of the current object are attached
  pages (`pageStack.pushAttached`).

### Phase 5 — Hardening and release (in progress, 2026-10-27)
- Done: security audit and two separate AI-run audits with their fixes (docs/*audit*), regression
  tests for each finding, enforced logging rule, CI (`scripts/check.sh`), release workflow,
  Lite/Full for GitHub releases with an opt-in update check (ADR 0014), German translation,
  README/CHANGELOG/SECURITY, a clean public history.
- Open: phone tests (calls, camera, battery, video, and the 2026-10-29 round: at-the-ear
  playback, the chime, in-app sound, playback speed, video scaling, chat background, renaming
  this device, and the 2026-11-02 round: video picture, media controls —
  `docs/voice-video-device-checklist.md`), P01 for Threema Safe and linking (long
  network waits in the core's dispatcher), Jolla Store and Chum packaging. Done 2026-10-28:
  fuzzing setup and first runs (`core/fuzz/`), indexed search. Done 2026-10-29 (user asked for
  it after all): the per-chat summary (store migration 33) — SQL triggers keep it in step, so
  no write path has to, which was the risk; with the chat list's per-chat lookups batched into
  one query each, the list the UI waits for went from 34 ms to 7 ms for 1000 chats.

### Later / optional
- Threema Work. (Calls are in, ADR 0012; Threema Web is in, over SaltyRTC's relayed data in both
  flavours, ADR 0013.)

---

## 10. Working rules for Claude Code

- **Explore before writing.** When porting a feature, first read the corresponding upstream code
  (`third_party/threema-android/app/src/main/java/ch/threema/...` and `domain/libthreema`),
  then write a short note in `docs/android-mapping.md` saying where it goes here, then implement.
- **Nothing for testing ships.** Demo data, probes and developer tools exist only in builds for
  the emulator, and they have no user interface at all: the demo chats are asked for over
  D-Bus (`scripts/emulator-demo.sh`), their core command is behind the `demo` Cargo feature
  (`scripts/build-core.sh` sets it for i486 alone) and their D-Bus method behind
  `qmake CONFIG+=demo` (`scripts/sdk-build.sh`, i486 alone). No QML, no translated text, no
  property in the binary: a build for a phone must not contain a trace — check it (`strings`
  over the binary, `grep` over the installed QML and the `.qm` files), don't assume. Hiding
  something in the UI is not enough; it must not be in the package (user decision 2026-10-30).
- **Small, buildable commits.** Every commit must build in the SDK. Commit messages explain why,
  not just what.
- **Tests first for protocol code.** Message encoders/decoders and state machines get unit tests
  with fixtures; where upstream has test vectors, reuse them.
- **Record decisions.** Any non-obvious choice (dependency, API, deviation from upstream) gets a
  file in `docs/decisions/NNN-title.md`: context, decision, consequences.
- **Ask, don't guess, on:** anything that changes the security rules in §7, anything that would
  send data to a non-Threema server, licensing questions, and whether a given feature goes into
  the Lite flavour or only Full. Otherwise decide and move on.
- **Verify platform APIs** against the installed SDK headers/QML modules rather than memory;
  Sailfish APIs have changed across releases. When unsure, write a tiny probe and run it in the
  emulator.
- **Test UI changes in the emulator** (docs/emulator.md): build i486, deploy, drive it with
  `scripts/emulator.py`, look at the screenshots.
- **Never modify the submodule** (`third_party/threema-android` is a pristine reference). Change
  libthreema in `third_party/libthreema` with commits prefixed `libthreema:`, in libthreema's
  style and with its tests/lints passing. Format only with `cargo fmt -p libthreema` *inside*
  `third_party/libthreema` (its `.rustfmt.toml` reproduces upstream's style); never run the
  workspace's `cargo fmt` over it. Check `git diff --stat vendor/libthreema -- third_party/libthreema`
  touches only the files you meant to change. No pull requests to Threema (user decision).
- **Git workflow:** work happens on `main`; a release is a version bump plus a CHANGELOG
  section merged into `release` by pull request, which CI builds and publishes
  (docs/release.md). Commits are authored as `penttimus <348773418+penttimus@users.noreply.github.com>`
  (set in the repository's git config). `scripts/check.sh` before pushing; the pre-commit hook
  runs its quick part.
- **Public repository:** no personal data in files, tests or commit messages: no real Threema
  IDs, names, phone numbers, e-mail addresses, PINs, device names or test credentials (made-up
  ones in tests, e.g. `ABCD1234`). The maintainer's test details live only in the local,
  ignored `handover.md` and `.local/`; `scripts/check.sh` searches for the local
  `.local/private-patterns`.
- **Credits and disclaimer:** anything taken or adapted from someone else (code, assets, ideas
  of substance) is credited in the README's Credits and on the About page; both carry the EU
  "AI generated" label and the disclaimer (made with Claude Code, audited with OpenAI Codex,
  research project, no warranty, not affiliated with Threema).
- **Keep this file current.** If the layout, stack, or phases change, update `CLAUDE.md` in the
  same commit.

---

## 11. Known facts and open questions (update as you go)

Facts:
- Upstream is AGPL-3.0; Threema is a paid app and an ID must be registered with a license. A
  third-party client works with a valid license but has no official support.
- Pinned upstream: `6.5.3-1216` (`d44a2b53`). libthreema is pure Rust (no libsodium/C deps),
  sans-I/O, not `Send`. It covers CSP transport, part of CSP-E2E, D2M, D2D rendezvous, ID backup,
  identity creation, parts of the directory API. It does **not** cover groups, PFS, blobs, Threema
  Safe, most message types — Android does these in Kotlin, we do them in `core`
  (`docs/android-mapping.md`). Android's uniffi bindings don't expose CSP at all.
- Libre licensing and push details: `docs/protocol-notes.md`.
- Threema told us (November 2026) it doesn't actively block third-party clients that cause no
  stability issues or high server load, are clearly unofficial, use no "Threema" branding without
  authorization and don't circumvent the licensing. We send our own `client-info`, as it suggested
  (ADR 0015), and the launcher icon is our own (green bubble over three waves). Threema is fine with
  our platform code `S` and our user agent, "Threema compatible" included (November 2026).
- SFOS 5.1/5.2: Qt 5.6.3, glibc 2.41, Rust 1.75, OpenSSL 3.5, SQLCipher 4.5.0 in the repos.
- Jolla's Messages/People/Email/Phone UIs are closed source; Notes, Gallery, Camera, Weather are
  open (BSD). Whisperfish (AGPL, Rust) is the closest reference app.
- Phase 0 RPMs pass `mb2 check` (Harbour validator) on all three architectures.

- Decided: minimum SFOS 5.1; ship Lite **and** Full flavours, no OpenRepos (ADR 0004, 0014).

Open questions:
- How Chum builds us: OBS builds from source in `%build`, but the SFOS 5.1/5.2 repos only have
  Rust 1.75 — need Chum's newer Rust packages or a Rust ≥ 1.95 from sailfishos/rust. Until then
  releases come from GitHub Actions (ADR 0014).
- OrganizationName for Sailjail (placeholder `org.threesea`).
