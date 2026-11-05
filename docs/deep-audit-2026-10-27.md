# Security, rule compliance and performance re-audit — 27 October 2026

> **Status (updated November 2026):** the findings describe the code as audited. Fixed: D01 and D02
> (session events tied to their task; every content command and the UI's media readers behind the
> private-chat check), D03 (decoding budget, bounded event queue), D05, R01 and P01 for the update
> check, R02, P02 (unchanged refreshes, per-chat queries, an index for search), P03, P04, D04
> (notification actions carry a per-chat token), and P01 for the other long operations (Threema
> Safe, linking, export and the rest run beside the core's loop since November, ADR 0028). Open: the
> device check of D04. The probes in `deep-audit-2026-10-27/` were written
> against the audited code (they no longer compile against the changed session types); the regression
> tests that replace them are in `core/src/app_web_security_tests.rs`, `core/src/app_updates.rs`,
> `core/src/web/budget.rs` and `tests/models`.

The previous fixes materially improve the project, but security hardening is not complete. This review reproduced residual Web-session revocation problems, additional private-content access paths, and peer-controlled logging. It also demonstrated a decoded-message memory amplification problem and measured database work that scales poorly with chat history.

The new update-check feature is opt-in and has an explicitly documented exception to the network policy. Its implementation nevertheless conflicts with the promised daily request limit on failures and can delay the core's command/event dispatcher while it waits for the network.

This is a separate AI-run audit, not a remediation patch. Only this report and its evidence directory were added; application changes made meanwhile were left alone.

## Version and method

The source snapshot was taken at **2026-10-27 11:38:53 UTC**, with HEAD a commit from before the public history plus the in-progress working-tree changes. Hashes of 319 first-party project files are in [snapshot-manifest.json](deep-audit-2026-10-27/snapshot-manifest.json). The new update-check files were included even though not yet committed. Vendored code was shared with the snapshot and not modified.

Later changes to `CLAUDE.md`, `README.md`, `docs/ffi.md`, the SDK Dockerfile, ADR 0014, and the newly added CI/release workflows and supporting documents/scripts were read separately. Their hashes are in [late-review-manifest.json](deep-audit-2026-10-27/late-review-manifest.json). In particular, the GitHub exception is treated as authorized by the current written project decision, not incorrectly reported as an unauthorized endpoint. The code paths behind the reproduced findings were unchanged at the review's final comparison. Results describe inspected files, not an immutable release artifact.

Reviewed areas: core commands and events, browser authorization/lifecycle, message storage and queries, media/FFI boundaries, call-frame and DTLS changes, logging policy/enforcement, update checks, QML/models, package/build rules, dependencies, documentation and regression coverage. Tests ran against a disposable source copy. Browser probes model already authenticated sessions at the application-handler boundary; they do not establish a real browser connection.

## Findings to address

| ID | Area | Severity / priority | Result |
|---|---|---|---|
| D01 | Security | Medium; fix before release | Revoked Web events are accepted when a numeric session ID is reused; stale termination deletes the replacement |
| D02 | Security / §7.8 | Medium; fix before release | Private poll/reaction operations still return protected content; FFI media remains ungated |
| D03 | Security / robustness | Medium; fix before release | Web wire-size limits do not bound decoded values or queued application messages |
| D04 | Security | Medium, local-process prerequisite | Notification D-Bus send/read actions remain unauthenticated |
| D05 | Security / §7.3 | Medium; fix before release | Arbitrary browser strings are logged, while the logging check reports success |
| R01 | Rule compliance / privacy | Low; resolve before release | Failed automatic update requests are not counted toward the daily limit |
| R02 | Rule compliance / release | Fix before relying on automated publication | Newly added release workflow does not depend on security/test checks and allows manual dispatch from other refs |
| P01 | Responsiveness | High optimization priority | Network waits inside the core dispatcher delay unrelated commands and events |
| P02 | Database / battery | High optimization priority for large histories | Unchanged Web refreshes and conversation loads repeat history scans and per-chat queries |
| P03 | UI | Medium optimization priority | A common row reorder destroys and recreates nearly the whole list |
| P04 | Development workflow | Low optimization priority | The check script executes the core test suite twice |

“Reproduced” below means the named offline probe reached the behavior. Severity accounts for prerequisites: an authenticated browser is not an arbitrary Internet sender, and an internal core/FFI caller is not a remotely exposed general-purpose API. No new critical issue or arbitrary remote code execution was established.

### D01 — Session revocation still relies on reusable numeric IDs

**Evidence:** [app_web.rs](../core/src/app_web.rs), lines 212, 239, 291–345; [web/mod.rs](../core/src/web/mod.rs), session schema and `add_web_session`.

The earlier fix clears running sessions on identity deletion and rejects message/state events whose ID is absent from `running`. However, the SQL schema uses `INTEGER PRIMARY KEY` without a persistent generation, and deleting the highest/only row allows its ID to be reused. A newly paired session immediately enters `running`, even while still connecting. An old queued `FromWeb::Message` carrying the same ID now passes the check.

`audit_reused_session_id_accepts_revoked_commands` deletes one session, creates its replacement, confirms the database reused the ID, and delivers an old session's queued text command. The command creates a new outgoing message. `audit_stale_end_deletes_replacement_session` confirms an old `Ended { delete: true }` removes the replacement's running entry and persistent session record. `Ended`, `Pinned`, and `CertificateChanged` are not covered by the new membership check; `Pinned` does not even identify a session.

**Prerequisites/impact:** delayed work from a revoked session must overlap replacement pairing. This is a narrower timing-dependent integrity/availability flaw than the original indefinitely surviving-session disclosure. These probes do not show the old browser receiving the replacement's replies or actually sending a message over the service.

**Correction:** attach an identity generation and unique task generation to every event, deferred reply and completion; compare them before dispatch. Cancellation should revoke authority immediately, not merely request transport shutdown. Validate termination and certificate events too. Add ID-reuse tests alongside the existing “ID no longer exists” tests.

### D02 — Private-chat reads remain reachable through mutations

**Evidence:** [app_polls.rs](../core/src/app_polls.rs), `poll` line 35 and `vote` line 74; [app_messages.rs](../core/src/app_messages.rs), `react` line 695 and `own_message` line 735; [ffi.rs](../core/src/ffi.rs), media/voice readers; [gifs.rs](../core/src/gifs.rs), `add_gif_favourite` line 138.

Draft, star, forward and save checks were added correctly for the previously reported cases. They do not cover the entire command surface. `vote` reads a private poll and returns its full updated question/choices. `react` returns the full protected message after changing reactions. Both omit `check_readable`, although the ordinary `message` command rejects that same locked conversation.

`audit_private_vote_and_reaction_return_content` reproduces both disclosures through the JSON command handler, with ordinary message access denied. Further source inspection finds no source-chat read gate in the shared poll helper/close path, the own-message/edit path, or copying an existing message's GIF into favorites. Those additional paths were not individually exercised.

Direct `threesea_media_read` and `threesea_voice_decode` still read decrypted data through the shared media store without a private-chat check. Their same-process callers limit the attacker model, but do not satisfy ADR 0009's stated guarantee that a QML bug cannot leak private content. A known/stale media ID can remain usable independently of a successful private-message query.

**Correction:** centralize authorization before loading protected content, including mutations that return it and exports/copies to other collections. Provide media threads with a revocable authorization view or capability; they need not each open the SQL database. Cover denial before mutation as well as redaction of returned values. Tests should enumerate the command surface and include poll, reaction, edit, GIF, image and voice paths.

### D03 — Resource limits end before decoded data and queues

**Evidence:** [web/chunks.rs](../core/src/web/chunks.rs), lines 16–21; [web/arp.rs](../core/src/web/arp.rs), line 162; [app_web.rs](../core/src/app_web.rs), unbounded event/send channels at lines 83 and 262 and event delivery around line 732.

The previous chunk fixes correctly limit serial numbers, account for empty chunks, and impose a 160 MiB incomplete-message budget. They do not bound the memory used by MessagePack values after reassembly. The decoder constructs a general `rmpv::Value` tree before validating the application message shape. Large arrays of tiny elements amplify memory substantially even when the wire-size limit is respected.

The bounded probe `audit_msgpack_decoded_expansion` decoded **65,541 wire bytes** into an array whose allocation was **2,621,440 bytes** on the audit host: 65,536 values, 40 bytes each. This is approximately 40× amplification, before other objects and copies. A root array is sufficient: rejection by `Incoming::parse` happens after allocation. No large allocation or OOM attack was attempted. The exact multiplier varies by target architecture; the demonstrated allocation was on x86-64.

Completed browser requests and responses also travel through unbounded channels. A sender producing valid requests faster than the core can process them can grow queued decoded data even though each incomplete-message budget is respected. Limits are per connection, not a process-wide budget across paired sessions. Slow core dispatch in P01/P02 compounds this risk.

**Correction:** enforce decoded node/container/string budgets while decoding, validate permitted message shapes early, and use byte-accounted bounded queues with a defined overload/disconnect policy. Add a global budget across sessions and pending sends. A depth limit alone does not constrain wide arrays; simply lowering the encoded file limit is not an adequate schema-level defense.

### D04 — Notification D-Bus commands remain an open local authority boundary

**Evidence:** [dbusinterface.cpp](../bridge/dbusinterface.cpp), `reply` line 68 and `markRead` line 74; [harbour-threesea.qml](../qml/harbour-threesea.qml), handlers at lines 604 and 608.

The slots still forward send/read actions without caller authentication, and the handlers do not apply the app/private-chat lock policy. A process allowed to reach the service can send as the loaded identity or change read state. Sailjail permissions and bus policy constrain reachability, so this remains conditional on local bus access.

**Correction:** authenticate the actual notification-service caller through a device-tested mechanism, and define/enforce lock behavior in the core. No live D-Bus action was invoked. This is the still-open earlier S07, not a newly discovered remote interface.

### D05 — Logging accepts peer text; the checker does not enforce the stated rule

**Evidence:** [app_web.rs](../core/src/app_web.rs), line 465; [web/arp.rs](../core/src/web/arp.rs), `Incoming::parse`; [check-logging.py](../scripts/check-logging.py), lines 21–24 and 76.

The unhandled-message branch logs `kind` and `sub_type` directly. These are arbitrary strings taken from the browser, not values validated against an enum. `audit_peer_controlled_log_fields` captures the subscriber output and confirms both attacker-supplied sentinel strings appear in logs. This occurs with informational logging enabled; the default warnings-only configuration reduces exposure. A compromised paired browser can cause arbitrary supplied text to be persisted in the journal. The probe does not claim that the application independently reads and logs an existing secret.

This contradicts §7.3's explicit ban on verbatim text from other users/devices. Naming a field `kind` does not make its value a safe code. The repository logging script passes the tree despite this path.

Independent negative checks also show that the scanner ignores `body` inside a Rust captured-format string (`tracing::info!("private text: {body}")`) because it strips string literals, and does not recognize printf-style `qWarning("…", value)` at all. These examples demonstrate checker coverage gaps, not additional production logging findings. It also scans tracked files only, so newly introduced untracked source is outside its check until staged.

**Correction:** log fixed classifications for unknown inputs, or bounded non-sensitive metadata such as lengths, never the raw unknown strings. Test the logging checker against captured interpolation, printf-style calls, field aliases and new files. Keep manual data-flow review; a variable-name denylist cannot prove the policy.

### R01 — Failed checks exceed the promised automatic update cadence

**Evidence:** [app_updates.rs](../core/src/app_updates.rs), lines 48–81; [runtime.rs](../core/src/runtime.rs), line 240; current [ADR 0014](decisions/0014-lite-full-github-updates.md).

The daily timestamp is written only after a successful `updates::latest()` result. On network/server failure, the function returns without recording the attempt. The hourly maintenance path can therefore contact GitHub every hour, and startup's immediate interval tick can cause more attempts. This differs from the consent text and §7.6 exception promising daily automatic checks at most. Explicit “Check now” is a documented exception and is not the issue.

**Correction:** persist last automatic attempt before issuing the request, separately from last success; apply the consented interval to attempted requests. If retry behavior is desired, document it precisely and bound it. Add a transport-failure test that counts attempts across ticks/restarts. This finding is source-supported; no GitHub update requests were made by the audit.

### R02 — New release automation is not itself gated on the checks

**Late-review evidence:** [.github/workflows/ci.yml](../.github/workflows/ci.yml), [.github/workflows/release.yml](../.github/workflows/release.yml), and [docs/release.md](release.md). These arrived after the initial snapshot; the initial “CI missing” observation is superseded.

The new CI correctly installs pinned cargo-audit and runs the checks. PRs into `release` also build both flavours for i486, and the release workflow builds both flavours for all three architectures. However, publication depends only on `version` and `build` in its own workflow. That workflow does not run or depend on the separate `checks` job, so a push can start publication independently of failing checks. The documented branch rules could mitigate the ordinary merge path, but their actual installation was not verified.

There is also an unrestricted `workflow_dispatch` trigger with no `github.ref` guard. A permitted operator can select a non-release branch and publish its commit, contrary to the stated release-branch-only process. This is a release-assurance gap, not a claim that an untrusted PR author automatically has publishing permissions. Artifact attestations establish provenance; they do not establish that tests or auditing passed.

**Correction:** require successful checks in the publication dependency graph, preferably using a reusable workflow shared with CI; restrict publish execution to the intended release ref and configure/verify branch or environment protection. Preserve logs and validate store-build gating too. The local `scripts/check.sh` still skips advisories if the tool is missing, although the new CI installation step addresses that particular omission in its normal path. No workflow was dispatched and no release was created during this audit. Historical approvals and working-process compliance remain unverified.

## Performance findings and measurements

### P01 — Async network waits still block the dispatcher

**Evidence:** [runtime.rs](../core/src/runtime.rs), lines 169–240; [app_updates.rs](../core/src/app_updates.rs), line 72; [updates.rs](../core/src/updates.rs), 20-second request timeout at line 81; [app_safe.rs](../core/src/app_safe.rs), maintenance at line 702; [app_multidevice.rs](../core/src/app_multidevice.rs), confirmation around line 231.

The main `select!` loop awaits each selected handler to completion before selecting another command/event. The new update request runs inside that branch. A slow request can delay unrelated core replies, event forwarding, lock commands and notifications for up to its 20-second timeout. The GUI thread and separately spawned network tasks can still run; this is dispatcher starvation, not necessarily a frozen graphics thread or stopped socket keepalive.

Existing maintenance performs several network operations serially. Linking confirmation also waits for a connection response without a local timeout. Moving cryptographic work to `spawn_blocking` helps the Tokio executor, but awaiting its completion inside the same dispatcher still delays other dispatch branches. Synchronous PIN hashing additionally occupies the single runtime thread while it runs.

**Optimization:** run cancellable jobs that return completion events; do not keep the command/event dispatcher borrowed across long waits. Snapshot inputs, use operation/identity generations to reject stale completions, serialize only the final state mutation, and add deadlines/cancel paths. Separate CPU work from the runtime where practical. Regression-test command/event latency while update, backup, link and directory operations are intentionally delayed. The architecture establishes the delay; this audit did not manufacture a live 20-second timeout.

### P02 — History scans and per-chat queries repeat on unchanged data

**Evidence:** [messages.rs](../core/src/messages.rs), `conversations` line 749 and search line 628; [app_messages.rs](../core/src/app_messages.rs), `conversations_json` line 391; [app_web_data.rs](../core/src/app_web_data.rs), `web_refresh` line 684.

`conversations()` groups the message index to find each chat's last ID. The query plan confirms a scan of `messages_by_conversation_id`. Building the final list adds per-conversation group/list/contact, notification, picture and draft queries. `web_refresh()` rebuilds its conversation representation after every core command and relevant event whenever any subscriptions exist, even if nothing affecting that list changed. Search uses leading-wildcard text matching and scans history for a miss.

The following are median milliseconds from five measured iterations after one warm-up, using synthetic SQLCipher stores on the desktop host's debug/test build. Fixture writes occurred in one transaction before timing. One browser stand-in was subscribed for the refresh measurement.

| Chats | Total messages | Core conversation list | Underlying conversation SQL | Search with no match | Unchanged Web refresh |
|---:|---:|---:|---:|---:|---:|
| 10 | 1,000 | 1.12 | 0.26 | 0.29 | 0.57 |
| 100 | 10,000 | 10.04 | 2.06 | 2.16 | 4.36 |
| 100 | 100,000 | 32.89 | 24.57 | 34.77 | 32.79 |
| 1,000 | 100,000 | 114.69 | 34.12 | 33.05 | 60.05 |

These are scaling evidence, not release-build phone latency or battery figures. A second run produced similar scaling. For example, a 60 ms unchanged refresh repeated at 10 relevant events per second would consume about 600 ms of host core time per second; that is an illustrative calculation, not measured device traffic.

**Optimization:** maintain transactional conversation summaries/unread counters or another indexed last-message representation; batch related metadata and reuse prepared statements; refresh only dirty conversation IDs; debounce/coalesce browser list updates. Evaluate an encrypted full-text index for search while preserving current mention/user-visible-field behavior and private-chat filtering. Validate deletions, edits, read receipts and empty-chat semantics before replacing the queries. Preserve bounded pagination for ordinary message reads.

### P03 — Incremental signals still cause excessive row churn

**Evidence:** [jsonlistmodel.cpp](../bridge/jsonlistmodel.cpp), `update` line 75; [contactlistmodel.cpp](../bridge/contactlistmodel.cpp), `update` line 70.

These models avoid full resets, which is useful. However, when an existing row appears later than the desired position, they delete all intervening rows and later reinsert them. Moving the last of 1,000 conversations to the top—an ordinary effect of a new message—removes and reinserts 999 other rows instead of moving one row. A faithful small simulation of the algorithm reproduced those counts; no actual QML frame-time claim is made.

**Optimization:** use stable IDs and `beginMoveRows` for moves, with an index map or a bounded diff strategy. Keep insert/remove operations for actual additions/deletions. Measure delegate creation, layout work, preserved scroll position and frame-time percentiles on the emulator/device. The ordinary unchanged-order case is approximately linear; it would be inaccurate to label every update quadratic.

### P04 — The core test suite runs twice per full check

**Evidence:** [scripts/check.sh](../scripts/check.sh), lines 65–66.

The first invocation filters output and discards failure status, then the second repeats the suite to recover an exit code. That doubles work, including expensive scrypt tests. Capture output and the exit code from one execution, then summarize its log. The snapshot's standalone core test execution took roughly 27 seconds on this host, so this is a useful local-feedback optimization without weakening coverage.

Other performance observations: new message paging, UI refresh coalescing, lazy optional message delegates, requested image sizes, asynchronous images, and the three-download semaphore are useful improvements. Avoid removing those bounds to improve throughput. Further tuning of image copies, codec decoding, frame encryption's shared lock and PIN hashing needs release/device profiling; no speculative speedup is claimed here.

## Status of the earlier security audit

| Earlier finding | Re-audit assessment |
|---|---|
| S01 session authority after deletion | Simple deletion/stale-message regression passes. Still incomplete for ID reuse and late completion events: D01. |
| S02 private-group management | New target authorization and regression tests cover member/name/delete/sync rejection. No recurrence found in inspected paths. |
| S03 stale Web subscriptions/blobs | Privacy rechecks and subscription removal present; both denial regressions pass. |
| S04 core private reads | Previously named draft/star/forward/save paths improved, but D02 finds additional mutation and media gaps. |
| S05 ratchet poisoning/replay | Candidate state is authenticated before commit; replay window present; tests pass. No recurrence found in reviewed paths. |
| S06 resource exhaustion | Streaming HTTP cap, download semaphore, native image/voice limits and chunk accounting are improvements. Decoded values and queues still need budgets: D03. |
| S07 D-Bus actions | Still open: D04. |
| S08 crash recording leftovers | Startup cleanup added. Plaintext staging during capture remains a qualified at-rest exception. |
| V01 pre-verification call media | Atomic verification gates and explicit SFU certificate check are present; negative integration test source exists. This re-audit did not run the Full call engines, so on-device behavior is not newly certified. |
| Additional hardening | PIN backoff, pre-connection host policy and bounded Safe decompression are present. |

## Rule compliance matrix

Current `CLAUDE.md`, relevant ADRs and the UI guidelines are the written requirements. Later explicit decisions take precedence over older blanket language. The matrix assesses observable implementation and evidence, not how the work was done.

| Rule | Assessment |
|---|---|
| §4 Rust core/unsafe boundary | Core denies unsafe code; explicit FFI boundary remains isolated. Audit probes changed only a disposable copy. |
| §7.2 encrypted database / §7.8 hashed PIN | SQLCipher, authenticated encrypted media, Secrets and salted scrypt remain present; tests pass. PIN lockout was added. Private-content access still fails D02. |
| §7.3 no peer text in logs | Fails in D05. The new wording permits fixed codes/counts and server errors; it does not permit arbitrary browser strings. |
| §7.4 hashed matching | Reviewed address-book matching remains hashed in Rust; own-address linking is the explicit existing exception. No new mismatch identified. |
| §7.5 TLS pinning | All 19 pins across four domain sets match the pinned Android XML. This compares the selected upstream revision, not every future upstream change. |
| §7.6 endpoint policy | Custom Web hosts and opt-in GitHub checks are now documented exceptions. Consent/build gates, HTTPS, no redirects and bounded update response are present. Failed-check cadence violates the stated limit: R01. |
| §7.7 / §8 CI and architecture matrix | Added during review: pinned audit installation, CI checks, both-flavour i486 PR builds and six release builds. Publication gating/ref restrictions still need R02. No remote workflow run or branch-rule configuration was verified. |
| §6 native UI/import/version rules | Source imports remain at QtQuick 2.6/QtMultimedia 5.6; no forbidden private Silica/Gallery/com.jolla import was found. Five bridge files passed SDK Qt 5.6 syntax checks. Full UI/Harbour acceptance was not tested. |
| §6 translation | 642 literal translation IDs found; none missing in either catalog; zero unfinished German entries in the snapshot. English unfinished entries are deliberately retained as fallback, not automatically a defect. Dynamic/error strings were not exhaustively classified. |
| §5 incremental models | Insert/remove/data-change signals are present. P03 shows the implementation still defeats much of the intended reuse for reordering. |
| §10 licensing/credits | SPDX markers present in inspected first-party source categories. README/About retain credits and the unofficial/AI/research disclaimers. This is a repository-rule check, not legal certification of every upstream asset. |
| §10 pristine reference submodule | Recorded Android submodule remains at `d44a2b53…`; audio-processing at `846fe90a…`. This audit did not modify either. A complete historical provenance/patch audit was not repeated. |
| Phase 4/5 device, fuzzing and release evidence | Battery, physical-device media behavior, coverage-guided fuzzing, linked Full-engine validation and final store acceptance remain distinct work. Host unit tests do not establish them. |

Remaining documentation discrepancies should be resolved explicitly: the brief says all persistence is behind a Store trait, while the implementation uses a concrete Store and QSettings preferences; its absolute native-key boundary does not describe native DTLS private-key generation/PEM copies and the bridge's database-key handling; `docs/ffi.md` still does not enumerate the Web/group-call surface. Older audit status summaries say categories are fixed while naming only the original probes. Update those summaries to acknowledge D01–D03 after remediation decisions. These are documentation/assurance gaps, not automatic proof that every architectural exception was unauthorized.

## Validation, evidence and limits

| Check | Result |
|---|---|
| Core tests | **154 passed**, 7 ignored, no failures |
| Vendored libthreema library tests | **3,680 passed**, no failures |
| Clippy with `-D warnings` | Passed |
| Core formatting check | Passed |
| New offline probes | Four defect tests, one bounded decoded-memory demonstration and one scaling test passed |
| Refreshed RustSec | **286 packages; zero vulnerabilities/warnings**, database `99670cfc0bc574433034b75876a4b791ccaa2efb`, updated 27 October |
| Qt 5.6 SDK | Five selected Lite-configuration bridge sources passed C++14 syntax checks |
| Late release additions | Both workflow YAML files parsed; dependency/ref paths reviewed; `release-version.sh` returned `0.1.0`. No workflow or publish action executed. |

The installed i486 SDK inventory reports Qt 5.6.3, GStreamer 1.26.11, libnice 0.1.21 and OpenSSL 3.5.6. These are SDK package versions, not a physical device inventory or assurance that every native vulnerability is patched. The required Secrets and WebRTC development metadata were absent from that container; Full native build/integration tests were not run. The Rust advisory result does not cover these native libraries or all local vendored modifications. The late Dockerfile/CI changes add a pinned rustup checksum and SDK base digest, useful supply-chain improvements; the new container build and action-reference resolution were not independently executed/verified.

Reproduce the bounded probes and synthetic measurements (against the audited snapshot; the probes no longer compile against current code) with:

```sh
sh docs/deep-audit-2026-10-27/run-probes.sh
```

The runner copies the core to `/tmp`, uses offline cached dependencies, and reuses the Cargo target directory unless `AUDIT_TARGET_DIR` is supplied. A **passing defect probe demonstrates the defect**, not remediation. After a fix, replace its assertions with rejection/containment expectations. See [probes.rs](deep-audit-2026-10-27/probes.rs), [probe-results.txt](deep-audit-2026-10-27/probe-results.txt), [validation.txt](deep-audit-2026-10-27/validation.txt), [dependencies.json](deep-audit-2026-10-27/dependencies.json), and [sdk-inventory.txt](deep-audit-2026-10-27/sdk-inventory.txt).

No production messages, destructive memory tests, emulator changes or physical-phone measurements were performed. The seven live integration tests stayed ignored. There was no native sanitizer/fuzz campaign, complete history secret scan, formal cryptographic audit, new packet-capture confirmation of DTLS behavior, or guarantee of every UI rule on every screen. The original audit and this re-audit together provide source-based evidence, not release certification.

## Suggested work order

1. Fix D01 with session/identity generations and D02 with a comprehensive authorization boundary. Add the new negative regressions.
2. Address D03 and D05 before relying on resource and logging checks as security controls; validate D04 on the phone.
3. Move long jobs out of the dispatcher, including update checks; enforce the promised attempt cadence. Keep cancellation and generation checks shared with the lifecycle fix.
4. Optimize dirty-chat refresh, batched metadata queries and actual model moves, then measure a release build on the target device.
5. Make release/CI checks mandatory, run the full build/device matrix, and align documentation with demonstrated coverage.
