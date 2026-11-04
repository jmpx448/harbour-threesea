// SPDX-License-Identifier: AGPL-3.0-only
//! Encrypted local store (SQLCipher). All persistence goes through [`Store`].
//!
//! The 32-byte database key is supplied by the bridge (`SecretStore`, backed by Sailfish
//! Secrets); it never touches disk unencrypted and is zeroized after use.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension as _, params};
use zeroize::{Zeroize as _, Zeroizing};

/// Store errors.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The key does not open the database (wrong key or corrupted file).
    #[error("database key rejected")]
    WrongKey,
    /// The database has a newer schema than this app understands.
    #[error("database schema {0} is newer than supported")]
    TooNew(i64),
    /// Any other SQLite error.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Persisted own identity.
pub struct StoredIdentity {
    /// Threema ID, e.g. `ECHOECHO`.
    pub threema_id: String,
    /// Client key (secret). Zeroized on drop.
    pub client_key: Zeroizing<[u8; 32]>,
    /// Chat server group (from identity creation or `identity/fetch_priv`).
    pub server_group: String,
}

/// Versioned schema migrations; index + 1 = `user_version` after applying.
const MIGRATIONS: &[&str] = &[
    // 1: settings and own identity
    "CREATE TABLE settings (
         key   TEXT PRIMARY KEY NOT NULL,
         value TEXT NOT NULL
     );
     CREATE TABLE identity (
         id           INTEGER PRIMARY KEY CHECK (id = 1),
         threema_id   TEXT NOT NULL,
         client_key   BLOB NOT NULL,
         server_group TEXT NOT NULL,
         created_at   INTEGER NOT NULL
     );",
    // 2: contacts
    crate::contacts::MIGRATION,
    // 3: messages, nonces
    crate::messages::MIGRATION,
    // 4: forward security sessions
    crate::fs_store::MIGRATION,
    // 5: reactions, outbox
    crate::reactions::MIGRATION,
    // 6: edited/deleted marker
    crate::messages::MIGRATION_EDITS,
    // 7: blob keys of file messages
    crate::media::MIGRATION,
    // 8: groups, message senders, per-receiver sending
    crate::groups::MIGRATION,
    // 9: group pictures
    crate::groups::MIGRATION_PICTURES,
    // 10: polls
    crate::polls::MIGRATION,
    // 11: profile pictures
    crate::profile_pictures::MIGRATION,
    // 12: per-chat notification settings
    crate::chat_notifications::MIGRATION,
    // 13: pinned chats
    crate::pins::MIGRATION,
    // 14: distribution lists
    crate::distribution_lists::MIGRATION,
    // 15: starred messages
    crate::messages::MIGRATION_STARRED,
    // 16: archived chats
    crate::archive::MIGRATION,
    // 17: sent/delivered/read/received times (message details)
    crate::messages::MIGRATION_TIMES,
    // 18: profile picture allow list
    crate::profile_pictures::MIGRATION_RECIPIENTS,
    // 19: private chats
    crate::applock::MIGRATION,
    // 20: emptied chats
    crate::messages::MIGRATION_EMPTIED,
    // 21: favourite GIFs
    crate::gifs::MIGRATION,
    // 22: D2D nonces (multi-device)
    crate::multidevice::MIGRATION,
    // 23: queued sync envelopes (multi-device)
    crate::multidevice::MIGRATION_SYNC_QUEUE,
    // 24: running group calls
    crate::group_calls::MIGRATION,
    // 25: Threema Web sessions and allowed self-hosted servers
    crate::web::MIGRATION,
    // 26: indexes for the chat list
    crate::messages::MIGRATION_INDEXES,
    // 27: picture request backoff (replaces the once-ever markers) and kept downloads
    crate::profile_pictures::MIGRATION_RECOVERY,
    // 28: history position (sort_key) for imported history next to live arrival order
    crate::messages::MIGRATION_POSITIONS,
    // 29: stable synthetic message ids for backup rows without an apiid
    crate::android_backup::import::MIGRATION_IMPORT_IDS,
    // 30: voice messages from before the listened flag existed count as heard once read,
    // so old chats do not light up as new.
    "UPDATE messages SET body = json_set(body, '$.listened', json('true'))
     WHERE kind = 'file' AND outgoing = 0 AND state != 0
       AND json_extract(body, '$.mediaType') LIKE 'audio/%'
       AND json_extract(body, '$.rendering') != 'file';",
    // 31: search index (trigram full text) instead of scanning every message
    crate::messages::MIGRATION_SEARCH,
    // 32: chats marked unread by hand
    crate::messages::MIGRATION_MARKED_UNREAD,
    // 33: per-chat summary (newest message, unread count), kept by triggers
    crate::messages::MIGRATION_CHAT_SUMMARIES,
    // 34: per-chat notification tones (until now in the plain settings file)
    crate::chat_notifications::MIGRATION_TONES,
    // 35: the creator back where a group sync dropped them
    crate::groups::MIGRATION_SYNC,
];

/// Setting keys.
pub mod keys {
    /// Video in calls ("0" off; default on). Android's synced video policy, not in Threema Safe.
    pub const VIDEO_CALLS: &str = "videoCalls";
    /// Video quality in calls: `auto`, `low` or `max` (Android's local video profile setting).
    pub const VIDEO_QUALITY: &str = "videoQuality";
    /// Group calls ("0" off; default on). Android's synced group call policy, not in Safe.
    pub const GROUP_CALLS: &str = "groupCalls";
    /// Directory match token (address book matching).
    pub const MATCH_TOKEN: &str = "matchToken";
    /// Our profile picture's version (changes with it; absent: none).
    pub const PROFILE_PICTURE_VERSION: &str = "profilePictureVersion";
    /// Threema Shop license key.
    pub const LICENSE_KEY: &str = "license_key";
    /// Set when Threema's directory refused the stored license (its message): no connection until
    /// a key is accepted again.
    pub const LICENSE_REFUSED: &str = "license_refused";
    /// When the directory last accepted the stored license (ms since the epoch).
    pub const LICENSE_CHECKED_AT: &str = "license_checked_at";
    /// Random persistent device id sent with license checks (like Android's `DeviceIdProvider`).
    pub const DEVICE_ID: &str = "device_id";
    /// App settings as the Threema Safe `settings` object (JSON). One source of truth that
    /// round-trips every field, including ones we don't use yet (calls, emojis, …).
    pub const APP_SETTINGS: &str = "app_settings";
    /// CSP device cookie (16 bytes hex), random once per installation.
    pub const DEVICE_COOKIE: &str = "device_cookie";
    /// Set when the device cookie was just generated: the next change indication is ours
    /// (Android's `skipNextIndication`), not another device's.
    pub const DEVICE_COOKIE_FRESH: &str = "device_cookie_fresh";
    /// Feature mask last published to the directory (decimal).
    pub const PUBLISHED_FEATURE_MASK: &str = "published_feature_mask";
    /// Unix time of the last daily contact refresh (feature masks, revocations).
    pub const CONTACTS_REFRESHED_AT: &str = "contacts_refreshed_at";
    /// Public nickname.
    pub const NICKNAME: &str = "nickname";
    /// App lock: wrong PINs since the last pause, when the current pause ends (ms), and which
    /// pause is next (see `App::check_lock`).
    pub const PIN_FAILED_ATTEMPTS: &str = "pin_failed_attempts";
    pub const PIN_LOCKOUT_UNTIL: &str = "pin_lockout_until";
    pub const PIN_LOCKOUT_STEP: &str = "pin_lockout_step";
    /// Update check (ADR 0014): "1" on, "0" off, absent: not asked yet; when it last asked
    /// GitHub (ms); the newest release found (`{version, url}` JSON); the version last notified.
    pub const UPDATE_CHECK: &str = "update_check";
    pub const UPDATE_CHECKED_AT: &str = "update_checked_at";
    /// When it last tried (successful or not): the daily limit counts attempts.
    pub const UPDATE_ATTEMPTED_AT: &str = "update_attempted_at";
    pub const UPDATE_LATEST: &str = "update_latest";
    pub const UPDATE_NOTIFIED: &str = "update_notified";
    /// The emoji picker's recently used emoji: a JSON array, newest first.
    pub const RECENT_EMOJI: &str = "recentEmoji";
    /// Linked e-mail / mobile number (from `identity/fetch_priv`).
    pub const LINKED_EMAIL: &str = "linked_email";
    pub const LINKED_MOBILE: &str = "linked_mobile";
    /// "1" while a linked e-mail waits for its confirmation link.
    pub const LINKED_EMAIL_PENDING: &str = "linked_email_pending";
    /// A phone number waiting for its SMS code: the number (E.164 without `+`) and the
    /// verification id, `<number> <id>`.
    pub const LINKED_MOBILE_PENDING: &str = "linked_mobile_pending";
    /// Auto-download kinds (comma-separated `image,audio,video,file`) on Wi-Fi / mobile data;
    /// absent: Android's defaults.
    pub const AUTO_DOWNLOAD_WIFI: &str = "auto_download_wifi";
    pub const AUTO_DOWNLOAD_MOBILE: &str = "auto_download_mobile";
    /// "1": received photos and videos are also saved to Pictures/Threesea.
    /// Largest file that downloads by itself, in MiB; "0" (or unset): any size.
    pub const AUTO_DOWNLOAD_LIMIT: &str = "auto_download_limit";
    pub const AUTO_SAVE_MEDIA: &str = "auto_save_media";
    /// Delete messages older than this many days by itself (Android's auto-delete; 0/absent:
    /// keep everything).
    pub const AUTO_DELETE_DAYS: &str = "auto_delete_days";
    /// When the auto-delete last ran (ms).
    pub const AUTO_DELETE_LAST: &str = "auto_delete_last";
    /// Settings → Appearance (Android's contact list options), "1"/"0": names as "Last First",
    /// sorted by last name, inactive IDs hidden.
    pub const LAST_NAME_FIRST: &str = "last_name_first";
    pub const SORT_BY_LAST_NAME: &str = "sort_by_last_name";
    pub const HIDE_INACTIVE: &str = "hide_inactive";
    /// "1": Android's "IPv6 for messages" off, Threema's IPv4-only hosts (`https::IPV4_ONLY`).
    pub const IPV4_ONLY: &str = "ipv4_only";
    /// Forward security switched off by the user ("0"; Settings → Advanced, user decision
    /// 2026-10-28). Messages stay end-to-end encrypted; the feature mask drops the FS bit
    /// and sessions are terminated, exactly like while multi-device is active. Device-local,
    /// never in Threema Safe.
    pub const FORWARD_SECURITY: &str = "forwardSecurity";
    /// The media gallery's filter: comma-separated kinds (absent: all, like Android).
    pub const MEDIA_KINDS: &str = "media_kinds";
    /// When a revocation password was set (ms; "0": set, date unknown), as last seen on the
    /// server. Absent: none, or never checked.
    pub const REVOCATION_SET_AT: &str = "revocation_set_at";
    /// Threema Safe: 64-byte master key (hex), never the password.
    pub const SAFE_MASTER_KEY: &str = "safe_master_key";
    /// Threema Safe: unix time of the last successful upload.
    pub const SAFE_LAST_UPLOAD: &str = "safe_last_upload";
    /// Threema Safe: SHA-256 (hex) of the last uploaded backup JSON.
    pub const SAFE_HASH: &str = "safe_hash";
    /// Threema Safe: last error kind, absent if the last attempt worked.
    pub const SAFE_LAST_ERROR: &str = "safe_last_error";
    /// Threema Safe: server limits from `GET config`.
    pub const SAFE_MAX_BYTES: &str = "safe_max_bytes";
    pub const SAFE_RETENTION_DAYS: &str = "safe_retention_days";
    /// Threema Safe: size of the last uploaded blob.
    pub const SAFE_SIZE: &str = "safe_size";
    /// Restored Safe data not yet imported (contacts/groups/lists/settings JSON) — kept until
    /// contact and group storage exist, and carried into new backups so nothing is lost.
    pub const SAFE_PENDING_IMPORT: &str = "safe_pending_import";
    /// Why the last pending Safe import attempt failed (an error kind; absent after success).
    pub const SAFE_IMPORT_ERROR: &str = "safe_import_error";
    /// Android-format backups written by us: target folder, schedule (`off`/`daily`/`weekly`),
    /// how many to keep, the last run and its error. The password lives in this encrypted
    /// store like every other secret-bearing setting; scheduled runs need it unattended.
    pub const ANDROID_EXPORT_DIR: &str = "android_export_dir";
    pub const ANDROID_EXPORT_PASSWORD: &str = "android_export_password";
    pub const ANDROID_EXPORT_INTERVAL: &str = "android_export_interval";
    pub const ANDROID_EXPORT_KEEP: &str = "android_export_keep";
    pub const ANDROID_EXPORT_LAST: &str = "android_export_last";
    pub const ANDROID_EXPORT_ERROR: &str = "android_export_error";
    /// What the pending Safe import did (JSON counts: imported, skipped and why), for the
    /// status page.
    pub const SAFE_IMPORT_SUMMARY: &str = "safe_import_summary";
}

/// The encrypted store.
pub struct Store {
    pub(crate) conn: Connection,
    /// Messages another device forwarded whose notification waits (multi-device): if they
    /// are read there meanwhile, the phone stays silent. In memory only.
    pub(crate) held_notifications: std::cell::RefCell<std::collections::HashSet<i64>>,
}

impl Store {
    /// Open (or create) the database at `path` with the given raw 32-byte key.
    ///
    /// # Errors
    /// [`StoreError::WrongKey`] if the key does not match an existing database.
    pub fn open(path: &Path, key: &[u8; 32]) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        let mut pragma = Zeroizing::new(format!(
            "PRAGMA key = \"x'{}'\";",
            data_encoding::HEXLOWER.encode(key)
        ));
        conn.execute_batch(&pragma)?;
        pragma.zeroize();
        // Fails with "file is not a database" if the key is wrong.
        if conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
            .is_err()
        {
            return Err(StoreError::WrongKey);
        }
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        let mut store = Self {
            conn,
            held_notifications: std::cell::RefCell::default(),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let target = i64::try_from(MIGRATIONS.len()).unwrap_or(i64::MAX);
        if version > target {
            return Err(StoreError::TooNew(version));
        }
        let tx = self.conn.transaction()?;
        for (index, sql) in MIGRATIONS
            .iter()
            .enumerate()
            .skip(usize::try_from(version).unwrap_or(0))
        {
            tx.execute_batch(sql)?;
            tx.execute_batch(&format!("PRAGMA user_version = {}", index + 1))?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Read a setting.
    pub fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Every setting whose key starts with `prefix`, by the rest of the key (the chat list
    /// reads all the chats' drafts at once this way).
    pub fn settings_with_prefix(
        &self,
        prefix: &str,
    ) -> Result<std::collections::HashMap<String, String>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT substr(key, ?2), value FROM settings WHERE key LIKE ?1 || '%'")?;
        let rows = statement.query_map(
            rusqlite::params![prefix, i64::try_from(prefix.len()).unwrap_or(0) + 1],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Forward security switched off by the user (Settings → Advanced): treated exactly
    /// like the multi-device state — no sessions, feature mask without the FS bit.
    pub fn forward_security_disabled(&self) -> Result<bool, StoreError> {
        Ok(self.setting(keys::FORWARD_SECURITY)?.as_deref() == Some("0"))
    }

    /// Rows this store has changed since it was opened (inserts, updates, deletes). All writes
    /// go through this one connection, so an unchanged count means unchanged data.
    #[must_use]
    pub fn changes(&self) -> u64 {
        self.conn.total_changes()
    }

    /// Write a setting.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Bytes the database uses (pages × page size).
    pub fn size(&self) -> Result<u64, StoreError> {
        // Not `PRAGMA page_size`: SQLCipher answers that with its own cipher settings row.
        let bytes: i64 = self.conn.query_row(
            "SELECT c.page_count * s.page_size FROM pragma_page_count() c, pragma_page_size() s",
            [],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(bytes).unwrap_or(0))
    }

    /// Delete everything (identity, contacts, messages, settings, …) but keep the schema: every
    /// table is emptied, so tables added by later migrations can never be left out here.
    ///
    /// All or nothing: on an error the transaction rolls back and the ID is still there. Once it
    /// has committed the deletion counts as done; shrinking the file afterwards can fail for
    /// lack of space, just when someone removes everything because the phone is full, and is
    /// only logged. The bridge replaces the file and its key next (`threesea_data_remove`).
    pub fn wipe(&self) -> Result<(), StoreError> {
        let tables: Vec<String> = {
            // Not the search index's own tables (its virtual table and its shadow tables):
            // they are emptied through the index below, touching them directly corrupts it.
            let mut statement = self.conn.prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                     AND name NOT LIKE 'messages_search%'",
            )?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        let transaction = self.conn.unchecked_transaction()?;
        // Deferred foreign keys: the deletion order doesn't matter inside the transaction.
        let mut sql = String::from("PRAGMA defer_foreign_keys = ON;");
        for table in &tables {
            sql.push_str(&format!(" DELETE FROM \"{table}\";"));
        }
        sql.push_str(" INSERT INTO messages_search (messages_search) VALUES ('delete-all');");
        transaction.execute_batch(&sql)?;
        transaction.commit()?;
        self.held_notifications.borrow_mut().clear();
        // The old rows still lie in the free pages and the write-ahead log until these two ran.
        for (step, sql) in [
            ("vacuum", "VACUUM;"),
            ("checkpoint", "PRAGMA wal_checkpoint(TRUNCATE);"),
        ] {
            if let Err(error) = self.conn.execute_batch(sql) {
                tracing::warn!(step, %error, "the emptied database could not be shrunk");
            }
        }
        Ok(())
    }

    /// Remove a setting.
    pub fn delete_setting(&self, key: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }

    /// The own identity, if one exists.
    pub fn identity(&self) -> Result<Option<StoredIdentity>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT threema_id, client_key, server_group FROM identity WHERE id = 1",
                [],
                |row| {
                    let mut key: Vec<u8> = row.get(1)?;
                    let mut client_key = Zeroizing::new([0_u8; 32]);
                    let ok = key.len() == 32;
                    if ok {
                        client_key.copy_from_slice(&key);
                    }
                    key.zeroize();
                    if !ok {
                        return Err(rusqlite::Error::InvalidColumnType(
                            1,
                            "client_key".into(),
                            rusqlite::types::Type::Blob,
                        ));
                    }
                    Ok(StoredIdentity {
                        threema_id: row.get(0)?,
                        client_key,
                        server_group: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// Store the own identity, replacing any previous one.
    pub fn set_identity(&self, identity: &StoredIdentity) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO identity (id, threema_id, client_key, server_group, created_at)
             VALUES (1, ?1, ?2, ?3, strftime('%s','now'))",
            params![
                identity.threema_id,
                &identity.client_key[..],
                identity.server_group
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_security_setting() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        assert!(!store.forward_security_disabled().unwrap(), "on by default");
        store.set_setting(keys::FORWARD_SECURITY, "0").unwrap();
        assert!(store.forward_security_disabled().unwrap());
        store.delete_setting(keys::FORWARD_SECURITY).unwrap();
        assert!(!store.forward_security_disabled().unwrap());
    }

    #[test]
    fn roundtrip_and_wrong_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        {
            let store = Store::open(&path, &[1; 32]).unwrap();
            store.set_setting(keys::LICENSE_KEY, "ABCDE-FGHIJ").unwrap();
            store
                .set_identity(&StoredIdentity {
                    threema_id: "ECHOECHO".into(),
                    client_key: Zeroizing::new([7; 32]),
                    server_group: "a1".into(),
                })
                .unwrap();
        }
        // Not readable as plain SQLite.
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.starts_with(b"SQLite format 3"));
        assert!(!raw.windows(8).any(|w| w == b"ECHOECHO"));

        assert!(matches!(
            Store::open(&path, &[2; 32]),
            Err(StoreError::WrongKey)
        ));

        let store = Store::open(&path, &[1; 32]).unwrap();
        assert_eq!(
            store.setting(keys::LICENSE_KEY).unwrap().as_deref(),
            Some("ABCDE-FGHIJ")
        );
        let identity = store.identity().unwrap().unwrap();
        assert_eq!(identity.threema_id, "ECHOECHO");
        assert_eq!(*identity.client_key, [7; 32]);
        assert_eq!(identity.server_group, "a1");
    }

    #[test]
    fn wipe_empties_every_table() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        // Rows across the schema's generations, including tables the old hard-coded wipe missed.
        store.set_setting(keys::LICENSE_KEY, "ABCDE-FGHIJ").unwrap();
        store
            .set_identity(&StoredIdentity {
                threema_id: "ECHOECHO".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        store
            .upsert_contact(&crate::contacts::test_contact("ECHOECHO"))
            .unwrap();
        store
            .insert_incoming("ECHOECHO", 1, "text", "secret words", 1)
            .unwrap();
        store
            .outbox_add(
                "ECHOECHO",
                2,
                crate::reactions::OUTBOX_EDIT,
                &serde_json::json!({"messageId": 1, "text": "edited secret"}),
            )
            .unwrap();
        // Three chats: marking one private unpins and unarchives that same chat.
        store.set_pinned("AAAAAAAA", true).unwrap();
        store.set_archived("BBBBBBBB", true).unwrap();
        store.set_private("ECHOECHO", true).unwrap();

        store.wipe().unwrap();

        let mut statement = store
            .conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                     AND name NOT LIKE 'messages_search_%'",
            )
            .unwrap();
        let tables: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(tables.len() >= 20, "schema unexpectedly small: {tables:?}");
        for table in tables {
            let rows: i64 = store
                .conn
                .query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 0, "table {table} not wiped");
        }
        // The schema survives: the store works again afterwards.
        assert_eq!(store.identity().unwrap().map(|i| i.threema_id), None);
        store.set_setting(keys::LICENSE_KEY, "NEWKE-YHERE").unwrap();
    }

    fn store_with_identity(path: &Path) -> Store {
        let store = Store::open(path, &[1; 32]).unwrap();
        store
            .set_identity(&StoredIdentity {
                threema_id: "ECHOECHO".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        store
            .insert_incoming("ECHOECHO", 1, "text", "secret words", 1)
            .unwrap();
        store
    }

    /// The rows must not stay readable with the old key in the main file: not in the free pages
    /// and not behind a write-ahead log that a later checkpoint would have folded in. The closed
    /// earlier session matters: without it the rows never reached the main file and any wipe
    /// looks clean.
    #[test]
    fn wipe_leaves_nothing_in_the_main_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        drop(store_with_identity(&path));

        let store = Store::open(&path, &[1; 32]).unwrap();
        assert!(store.identity().unwrap().is_some());
        store.wipe().unwrap();

        // The main file alone, as a copy would be taken while the app still runs.
        let copy = dir.path().join("copy.db");
        std::fs::copy(&path, &copy).unwrap();
        let copied = Store::open(&copy, &[1; 32]).unwrap();
        assert!(copied.identity().unwrap().is_none());
        assert!(
            copied
                .conversation_messages("ECHOECHO", 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            std::fs::metadata(dir.path().join("t.db-wal")).map_or(0, |wal| wal.len()),
            0,
            "the write-ahead log is emptied too"
        );
    }

    /// A deletion that fails changes nothing and leaves no transaction open.
    #[test]
    fn failed_wipe_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_identity(&dir.path().join("t.db"));
        store.set_setting(keys::LICENSE_KEY, "ABCDE-FGHIJ").unwrap();
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER refuse BEFORE DELETE ON settings
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();

        assert!(store.wipe().is_err());

        assert!(store.conn.is_autocommit(), "no transaction left open");
        assert!(store.identity().unwrap().is_some());
        assert_eq!(
            store.setting(keys::LICENSE_KEY).unwrap().as_deref(),
            Some("ABCDE-FGHIJ")
        );
    }

    /// VACUUM can't run (here: a statement in progress; on the phone: no space left) and the
    /// deletion stands all the same.
    #[test]
    fn wipe_survives_a_failing_vacuum() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_identity(&dir.path().join("t.db"));
        let mut reading = store.conn.prepare("SELECT 1 FROM sqlite_master").unwrap();
        let mut rows = reading.query([]).unwrap();
        assert!(rows.next().unwrap().is_some());

        store.wipe().unwrap();

        drop(rows);
        assert!(store.identity().unwrap().is_none());
    }
}
