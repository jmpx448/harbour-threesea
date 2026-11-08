# Android data backup: format and import design

How Threema for Android's full data backup is read and written here, and the rules for merging it
into a running ID. `ANDROID_BACKUP` and `ANDROID_APP` below stand for
`third_party/threema-android/app/src/main/java/ch/threema/app/backuprestore/csv` and
`third_party/threema-android/app/src/main/java/ch/threema`.
What is implemented: `core/src/android_backup/`, `core/src/app_android_backup.rs`,
`docs/STATUS.md`. Manual tests: `docs/android-backup-device-checklist.md`.

## Android full data backup format

### Authoritative references

| Source under the Android tree | Purpose |
| --- | --- |
| `ANDROID_BACKUP/BackupService.java` | Exact filenames, CSV headers, avatar/media selection and write order. |
| `ANDROID_BACKUP/RestoreService.java` | Version gates, foreign-key mappings, body conversion, identity and media restoration. |
| `ANDROID_BACKUP/Tags.java` | Exact field names, including compatibility spelling mistakes. |
| `ANDROID_BACKUP/RestoreSettings.java` | Format history and current version 27. |
| `ANDROID_BACKUP/BackupMessageTypes.kt` | Message-type names and deprecated types. |
| `ANDROID_APP/app/utils/CSVReader.java`, `CSVWriter.java`, `CSVRow.java` | OpenCSV behavior, extra escaping, boolean/time/list conventions. |
| `ANDROID_APP/app/utils/FileHandlingZipOutputStream.kt` | Zip4j AES-256 ZIP entries and compression. |
| `ANDROID_APP/app/utils/BackupUtils.java` | Legacy group UID fallback `<group-id>-<creator>`. |
| `ANDROID_APP/storage/models/data/media/FileDataModelSerializer.kt` | Android FILE body JSON array. |
| `ANDROID_APP/storage/models/data/LocationDataModel.kt` | Location body JSON array. |
| `ANDROID_APP/app/backuprestore/LegacyMessageBodyTransformer.kt` and its tests | Deprecated image/video/voice body conversion. |

### Container and records

- Zip4j writes AES-256 encrypted entries. `compress: false` still selects DEFLATE with `NO_COMPRESSION`; it does not necessarily mean ZIP method STORED. Support actual entry methods, AES authentication and ZIP64 for large backups. Reject unsupported split/multidisk archives explicitly.
- `settings` is a two-column CSV containing `version`; it is not JSON and not a regular data table with a header row. Android defaults a missing version to 1. Support only documented/tested versions; reject a version newer than supported before changes. Older unsupported variants must produce a clear compatibility report rather than a false success.
- `identity` contains an additional encrypted ID-backup string, made using the same backup password. Reuse libthreema's identity-backup decryption. Check both ID and public key derived from the private key against the current account. Never replace the current account's key while merging history.
- Data CSV files are header-based. Parse by names, not positions. Preserve `messagestae` exactly: it is the upstream message-state column spelling. Booleans are `1`/`0`; timestamps are epoch milliseconds; member lists are semicolon-separated inside a CSV cell.
- CSV has quoted multiline text, commas, quotes, Unicode, empty cells and backslashes. `CSVRow.escape(String)` additionally doubles backslashes before OpenCSV writes them. A default Rust CSV parser is not automatically equivalent to the Java parser's escape rules. Build interoperability fixtures for literal backslashes, quotes, newlines and serialized JSON before fixing parser configuration. Do not blindly unescape twice or trim message text.

| Entry | Relationship and destination |
| --- | --- |
| `contacts.csv` | `identity` is the Threema ID; version 19+ `identity_id` is an archive-local alias. Build alias-to-ID map before resolving message and picture filenames. |
| `groups.csv` | Group identity is creator plus 8-byte API ID; `group_uid` is an archive-local alias. New backups use generated UIDs; older backups use the legacy fallback. Retain both maps as required by version. |
| `distribution_list.csv` | Local list identities and recipient lists; these are not ordinary protocol group identities. |
| `message_<contact-alias>.csv` | Contact message history; do not assume the filename suffix is an 8-character Threema ID. |
| `group_message_<group-alias>.csv` | Group messages; `identity` identifies sender. Group alias is resolved through `groups.csv`. |
| `distribution_list_message_<list-alias>.csv` | Distribution list history; preserve the separate list identity model. |
| `contact_avatar_me` | Own picture. |
| `contact_avatar_<contact-alias>` | Locally chosen/contact-address-book picture. Distinct from contact-defined profile picture. |
| `contact_profile_pic_<contact-alias>` | Picture supplied by the contact. |
| `group_avatar_<group-alias>` | Group picture; imported image bytes do not by themselves provide a reusable remote blob descriptor. |
| `message_media_<uid>`, `group_message_media_<uid>`, `distribution_list_message_media_<uid>` | Media linked by Android message `uid`, not protocol `apiid`. Build `(entry family, uid)` to local-message mapping. |
| `message_thumbnail_<uid>`, `group_message_thumbnail_<uid>`, `distribution_list_thumbnail_<uid>` | Optional thumbnails; full media and thumbnail can be independently absent. |
| `ballot.csv`, `ballot_choice.csv`, `ballot_vote.csv` | Poll definitions, choices and votes with archive-local IDs and protocol IDs; resolve in passes. |
| `contact_reactions.csv`, `group_reactions.csv`, `reaction_counts.csv` | Reactions keyed by conversation, protocol message ID and reactor; restore after message mapping. |
| `nonces.csv`, `nonces_d2d.csv`, `nonce_counts.csv` | Replay-protection data. Inspect exact encoding in upstream restore before import. CSP and D2D nonce domains must remain separate. |

### Message conversion

Convert backup records into the core's canonical message bodies, not directly into QML objects or unchanged Android database JSON.

- `TEXT`: preserve Unicode and exact text. Reconcile the separate `quoted_message_apiid` with `messages.rs::split_quote` and its stored quote representation, so a quote is neither lost nor prefixed twice.
- `LOCATION`: Android array is `[latitude, longitude, accuracy, address, name]` with optional tails; convert to current location JSON after finite/range validation.
- `FILE`: Android array is `[blobId, encryptionKey, mimeType, fileSize, fileName, renderingType, isDownloaded, caption, thumbnailMimeType, metadata]`, with older missing optional tails. See `FileDataModelSerializer.kt`. Convert to the shape in `app_media.rs::file_body`, and keep any usable keys only in the encrypted files store/table. Use actual archived entry presence and authentication, not Android's stale `isDownloaded` flag, for local availability.
- Deprecated `IMAGE`, `VIDEO`, `VOICEMESSAGE`: port the legacy transformations against upstream fixtures before claiming support. `CONTACT` can follow Android's text fallback with a compatibility count.
- `BALLOT`: reconstruct polls using archive-ID-to-protocol-ID mappings. Never issue a fresh poll setup/vote message to recreate history.
- `STATUS`, `VOIP_STATUS`, `GROUP_CALL_STATUS`, `GROUP_STATUS`: map into the existing status/call display model. An imported call record must never start/join a call. Types without a faithful UI representation need a preserved local compatibility record and an explicit count, not silent omission or raw markup in a Label.
- `deleted_at` must retain deletion and not resurrect text from an older backup over a newer local tombstone. Preserve edited/starred/read/delivery timestamps according to explicit merge rules. Map Android `USERACK`/`USERDEC` to reactions using upstream behavior.

## Import architecture and merge rules

### Suggested modules and UI

Add a Rust `android_backup` module for ZIP/CSV decoding, versioned records and validation; a store import layer for transactional writes and mapping; and `app_android_backup` for jobs, preview/apply/status/cancel commands. Exact module names may follow current conventions. Extend `app.rs`, `lib.rs`, runtime event handling, FFI docs and translations. Reuse `MediaStore` encryption and picture naming; avoid a second plaintext media directory.

Use Rust ZIP and CSV libraries instead of shelling out to `unzip`, Java or an Android runtime. The [zip crate](https://docs.rs/zip/latest/zip/) documents `aes-crypto` for AE-1/AE-2 plus deflate support; [csv ReaderBuilder](https://docs.rs/csv/latest/csv/struct.ReaderBuilder.html) supports configurable escapes and headers. These are dependency candidates, not a tested dependency selection. Pin compatible versions, minimize features, review licenses, run the repository dependency checks, and cross-build with the pinned Rust/Sailfish targets. Explicitly prove Zip4j interoperability and non-ASCII password behavior with a fixture.

Expose “Import Android data backup” in Settings → Backup, so that a restored ID can be merged with its Android history. Use the existing Sailfish picker pattern to choose a local ZIP, enter the password locally, inspect, then review counts/warnings and apply. The import requires an existing unlocked identity matching the archive; a device with no ID gets that identity first from the same archive (`androidBackupRestore`, "Restore from Data backup" on the first page), then the same import. Keep the ID-backup text page separate.

Proposed command lifecycle: start inspection → poll/receive progress → receive preview plus opaque job token → apply token → progress → completion summary; cancel/retry explicitly. A token binds to the same validated archive bytes and chosen options, not merely a mutable path. Passwords must not enter logs, command-line arguments, persisted settings or reports. Keep secrets out of generic Debug output and clear QML password fields on completion/cancel. Clear password-owning buffers where supported.

`harbour-threesea.desktop` already requests Documents and Downloads. Still test real Sailjail picker access: normalize local file URLs in the bridge and pass a stable readable handle or protected encrypted source copy into the worker. Do not load an entire multi-gigabyte ZIP into `Command` JSON or existing attachment buffers.

### Worker and persistence design

The core runs a current-thread Tokio loop and owns `Rc<Store>`. `runtime.rs::run_loop` awaits commands sequentially. A large synchronous parse/apply inside a command would stall that loop. Put ZIP I/O/decryption/decompression/CSV conversion on a bounded worker; send batches through a bounded channel, apply bounded transactions on the store-owning thread, and yield between batches. Do not move `Rc<Store>` to `spawn_blocking` or let workers write the live DB unsafely. Support progress and cancellation without unbounded buffered message bodies.

For crash consistency, use a durable import job/checkpoint and explicit mappings. Authenticate an entire entry before publishing its records: ZIP AES authentication may fail only at end-of-entry. For large CSV files, stage validated records into an encrypted temporary database, or another encrypted staging format, then commit them. A late authentication failure must not leave earlier rows from that entry visible as trusted history.

Stage media encrypted in the app-owned directory with unique temporary names. Ensure media files exist and are authenticated before marking rows local; use a recovery journal/checkpoint for the gap between filesystem publication and DB commit. Existing `MediaStore::write_to` buffers a whole plaintext and ciphertext and uses a fixed `.tmp` sibling, so it needs either bounded per-entry use or a reviewed streaming/staging extension. Do not invent a chunked encryption format without a migration/design decision. Report oversized entries clearly; a whole backup is not subject to the current 100 MiB per-blob limit. Account for encrypted staging space and recover cleanly from ENOSPC.

Keep the import source immutable for a job or revalidate its fingerprint before apply. On restart, remove orphaned staging files or resume only from a verified checkpoint. Cancellation before apply makes no live changes. Cancellation during apply leaves only consistent, documented batches and can resume idempotently; do not promise whole-job rollback unless it is implemented across both DB and files.

### Historical records must have no sending side effects

Use dedicated historical inserts. Do not call `sendText`, `sendFile`, `createGroup`, receive-message providers or `setGroupPicture` to recreate old data. Those paths can queue sends, receipts, uploads, notifications, photo requests, resyncs or D2D reflections. The one exception is deliberate: with other devices linked, the metadata import reflects the contacts and groups it added and the own and group pictures it filled in (`App::reflect_imported`), or later messages would name a group those devices don't know; old messages stay here.

Imported outgoing `PENDING`/`SENDING` rows must not enter `Store::pending_outgoing` (currently selects outgoing state 0). Preserve them as failed/unsent historical items, with a separate explicit user action if resend is offered. Preserve incoming read/unread state without issuing delivery/read receipts or notification banners. Recompute unread counts after import and refresh models through aggregate completion events, not thousands of `messageAdded` notifications.

Do not copy Android device sessions, D2D keys or forward-security ratchets into this app's runtime. CSP nonce union can be imported through a dedicated path once encoding is verified. D2D nonces require compatibility with the current device-group key/domain; do not indiscriminately union them or discard existing nonce protection. Archive content containing a historical blob ID is not authorization to redownload/upload everything during import.

### Merge policy

| Entity | Default behavior |
| --- | --- |
| Own identity | Must match both current ID and derived public key; mismatch aborts before writes. Do not recreate identity, reset license or erase Safe credentials. |
| Contacts | Add missing valid records from archive keys offline; reconcile with current public keys. Never overwrite a key mismatch or downgrade verification. Preserve newer local names/policies/blocks; count conflicts, and leave out the chat, group messages, reactions and picture the archive holds in that contact's name. A group the import creates still keeps such a contact as creator or member, with the group's name and picture from the archive, and their votes stay in another person's poll. Import revoked historical contacts without making them sendable. |
| Groups | Key by normalized creator and the existing little-endian 8-byte ID convention, not `group_uid`. Add missing metadata offline. Fill missing data, preserve newer known local membership/left status, and report ambiguous conflicts. An old backup must not rejoin a left group. |
| Pictures | Fill absent images by default; preserve an existing newer local picture. Distinguish contact-defined pictures from local overrides. Current core has one contact picture slot: add a separate override/source policy or report that only one layer was imported. Do not silently replace contact-defined images with local address-book pictures. |
| Own picture | Import absent own image into `me`, regenerate Safe-size copy using existing policy when needed, advance version, and invalidate upload/distribution cache only when image actually changes. No automatic distribution during import. |
| Group picture | Import bytes locally and version them without fabricating a blob descriptor or broadcasting a new group picture. Verify own-group picture redistribution handles locally present bytes if a descriptor is absent. |
| Messages | Deduplicate by stable protocol identity, conversation and direction/sender where applicable. Preserve live edits/deletions and newer state; never downgrade read/delivered status. Merge missing local media separately from message duplication. |
| Missing media | Preserve the historical message and explicit unavailable state. Missing media in an otherwise valid metadata-only backup is not corruption. Generate supported thumbnails from present media or use fallback icons. |
| Archive/private/pin/notification state | Preserve existing local decisions by default. Apply archive flags for newly created conversations where represented. Never turn a private chat public or unlock it through import. Preserve the media gate. |
| Polls/reactions/lists | Use stable protocol IDs where available and explicit archive-local mappings elsewhere; repeated import adds no duplicates or outgoing work. |

Persist a mapping for rows without a valid `apiid`, keyed by source/archive fingerprint plus record family, conversation and Android `uid`. Synthetic IDs must be stable across resume/repeated import; never generate a fresh random ID on every attempt. Cross-archive deduplication of such records is inherently weaker and must be reported rather than guessed from text alone.

The current schema uniquely constrains `(conversation, outgoing, message_id)`. Incoming group rows also have `sender`, but it is not in that constraint. Do not pretend an importer can fix cross-sender collisions with `INSERT OR IGNORE`: define whether to migrate the uniqueness model and audit lookup, reaction, edit/delete and quote operations accordingly, or reject/report collisions explicitly without losing either record silently. Scope a wider protocol migration separately if required.

### Message chronology is a prerequisite

Keep local row IDs stable because media filenames and UI references use them. The existing `conversation_messages` comment explicitly chooses arrival/sending order because the sender's clock can be wrong. Preserve that intentional behavior for live traffic: do not globally switch to sender `created_at` without resolving the clock-skew regression. Prefer a separate stable display-order representation that can place imported history before/between existing records while retaining live arrival order. Define how Android posted/created/modified timestamps contribute, use trustworthy local timestamps where available, and make ties and missing-time fallback deterministic. Any alternative based on `(created_at, id)` needs an explicit accepted change to those semantics and clock-skew tests. Convert all relevant queries, not just the chat list:

- latest message and conversation sorting (`max(id)` is wrong after merging old history);
- chat initial page and older-page cursor;
- incremental “since” updates and focus-message queries;
- search, starred and media lists;
- Threema Web pagination/subscription refresh, which currently assumes row IDs sort messages;
- bottom-to-top QML model ordering and scroll-position preservation.

Separate change detection from position in history: a higher row ID can still be a useful insertion cursor, but cannot mean “later in conversation history.” Changing only the last-message query leaves chat pages wrong. Inserting old messages with negative IDs or renumbering live IDs is not a safe shortcut. Add appropriate indexes and test large histories, delayed delivery and sender clocks far in the future/past.

## Validation and archive handling

Treat the ZIP as untrusted local input: passwords alone do not establish record validity. Bound entry count, expanded bytes, per-record/field size and image dimensions. Validate actual byte counts while reading, rather than trusting central-directory sizes or compression ratios alone. Reject duplicate logical entries, path traversal, absolute paths, symlinks and unsupported entry methods. Never extract archive filenames directly to arbitrary filesystem paths. Read each encrypted entry to EOF and verify authentication/integrity before accepting it.

### What the password covers

The format is WinZip AE-2 per entry: a correct password proves each entry's *content* is authentic, and nothing else. Entry names, the set of entries, their order and the central directory are not covered, and every backup with the same password has entries that verify. So someone who can write to the file (shared storage, a cloud folder) but has no password can, without being noticed by any check:

- rename entries: `message_<a>.csv` and `message_<b>.csv` swap, and one contact's messages land in another's chat under the other's name; renaming `message_media_<uid>` shows one message's picture in another;
- remove entries: a chat, its media, or `nonces.csv`, which drops the replay protection for what Android already received;
- put in entries from an older backup with the same password, under their own names or others.

This is the format's limit, shared with Android's own restore, and Threesea can't close it while reading Android's files. What the reader does:

- it refuses duplicate names and entries that share bytes, so an entry can't be shadowed by a second record and every reader of the file sees the same content;
- the result page says when `nonces.csv` is missing, since that takes something away from the user;
- two cross-checks give hints on the result page: rows whose `uid` another chat of the same kind already holds, and group chats whose incoming rows mostly come from people outside the group. They notice crude copies and swaps only. No hint is not evidence of an untouched file, and a hint can have an innocent cause (a group whose writers all left).

Keep the backup file where others can't write to it, as you would with the messages inside.

### Memory

Decrypted entries, CSV tables, message rows and blob keys are wiped when dropped, and their buffers don't grow by leaving freed copies behind. What is not wiped: the zip crate's internal buffers (the inflate window), and the converted text on its way into the store, as for any received message.

### Bounds

The CSV reader refuses a table beyond fixed limits for field, row, row count and cell count (`android_backup/csv.rs`), in time linear in the text; tables are read and parsed on a blocking thread.

Preview must distinguish wrong password/corruption, unsupported format, identity mismatch, malformed metadata, absent optional media, and insufficient free space. Report compatibility issues by file category/record number and reason without disclosing private contents. Preview should state verified counts versus estimates and whether all selected media have been authenticated. A “successful” full restore must not silently omit unsupported message families.

Use synthetic public fixtures in tests. A user backup stays local/ignored and never becomes a repository fixture, CI upload, log attachment or third-party upload. The password is entered in the app and goes nowhere else.
