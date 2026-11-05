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
    /// SQLite says this isn't a database: the key doesn't fit, or the file's first page is damaged.
    /// With SQLCipher nobody can tell the two apart.
    #[error("database key rejected")]
    WrongKey,
    /// The database has a newer schema than this app understands.
    #[error("database schema {0} is newer than supported")]
    TooNew(i64),
    /// The schema upgrade ran out of room and was rolled back, so nothing has changed.
    #[error("no room for the database upgrade: {0}")]
    UpgradeNoRoom(rusqlite::Error),
    /// Any other SQLite error.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// SQLite calls it that when it writes. Opening a store on a full disk fails earlier, on growing the
/// WAL's shared memory file (measured on a full tmpfs).
fn is_disk_full(error: &rusqlite::Error) -> bool {
    error.sqlite_error_code() == Some(rusqlite::ErrorCode::DiskFull)
        || matches!(error, rusqlite::Error::SqliteFailure(failure, _)
            if failure.extended_code == rusqlite::ffi::SQLITE_IOERR_SHMSIZE)
}

/// Whether a failure with this SQLite code ([`StoreError::sqlite_code`]) comes from what was being
/// stored, so that trying again won't help (a broken rule, a bad type, something too big; `0`: what
/// isn't SQLite's as such). Anything else may be the device's state of the moment (a full disk, an
/// I/O error, damage someone can repair): that is never a reason to give up on anything.
#[must_use]
pub fn fails_every_time(code: i32) -> bool {
    use rusqlite::ffi::{SQLITE_CONSTRAINT, SQLITE_ERROR, SQLITE_MISMATCH, SQLITE_TOOBIG};
    let primary = code & 0xff;
    primary == 0
        || primary == SQLITE_ERROR
        || primary == SQLITE_TOOBIG
        || primary == SQLITE_CONSTRAINT
        || primary == SQLITE_MISMATCH
}

impl StoreError {
    /// What went wrong as the UI tells it apart (`Errors.js`); `store` when there is nothing more
    /// to say. SQLCipher reports damage on any page but the first as corruption.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        use rusqlite::ErrorCode as Code;
        match self {
            Self::WrongKey => "storeKey",
            Self::TooNew(_) => "storeTooNew",
            Self::UpgradeNoRoom(_) => "upgradeNoRoom",
            Self::Sqlite(error) if is_disk_full(error) => "diskFull",
            Self::Sqlite(error) => match error.sqlite_error_code() {
                Some(Code::NotADatabase) => "storeKey",
                Some(Code::DatabaseCorrupt) => "storeDamaged",
                Some(
                    Code::SystemIoFailure
                    | Code::CannotOpen
                    | Code::PermissionDenied
                    | Code::ReadOnly
                    | Code::DatabaseBusy
                    | Code::DatabaseLocked
                    | Code::OutOfMemory,
                ) => "storeUnreadable",
                _ => "store",
            },
        }
    }

    /// SQLite's extended result code; `0` for what didn't come from SQLite as such.
    #[must_use]
    pub fn sqlite_code(&self) -> i32 {
        match self {
            Self::UpgradeNoRoom(rusqlite::Error::SqliteFailure(error, _))
            | Self::Sqlite(rusqlite::Error::SqliteFailure(error, _)) => error.extended_code,
            Self::WrongKey | Self::TooNew(_) | Self::UpgradeNoRoom(_) | Self::Sqlite(_) => 0,
        }
    }

    /// For the log and for support: numbers only, SQLite's own texts can name columns.
    #[must_use]
    pub fn code(&self) -> String {
        let sqlite = |error: &rusqlite::Error| match error {
            rusqlite::Error::SqliteFailure(failure, _) => {
                format!("sqlite {}", failure.extended_code)
            }
            _ => String::new(),
        };
        match self {
            Self::WrongKey => String::new(),
            Self::TooNew(version) => format!("schema {version}"),
            Self::UpgradeNoRoom(error) | Self::Sqlite(error) => sqlite(error),
        }
    }
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
    // 35: groups say whether we were removed; the creator back where a group sync dropped them
    crate::groups::MIGRATION_SYNC,
    // 36: outgoing messages say whether a send has begun
    crate::messages::MIGRATION_WRITTEN_OUT,
    // 37: received messages the other devices of the group haven't been told of yet
    crate::messages::MIGRATION_AWAITING_REFLECTION,
    // 38: whom a restore left without a picture, in place of asking everyone without one
    crate::profile_pictures::MIGRATION_RESTORED,
    // 39: pictures another device of the group changed, until they are stored here
    crate::profile_pictures::MIGRATION_SYNCED_PICTURES,
];

/// Setting keys.
pub mod keys {
    /// Video in calls ("0" off; default on). Android's synced video policy, not in Threema Safe.
    pub const VIDEO_CALLS: &str = "videoCalls";
    /// Video quality in calls: `auto`, `low` or `max` (Android's local video profile setting).
    pub const VIDEO_QUALITY: &str = "videoQuality";
    /// Group calls ("0" off; default on). Android's synced group call policy, not in Safe.
    pub const GROUP_CALLS: &str = "groupCalls";
    /// Threema Web ("0" off: no pairing, connecting or wake-ups; default on).
    pub const WEB_ENABLED: &str = "webEnabled";
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
    /// Until when automatic directory calls rest after a 429 (ms since the epoch).
    pub const DIRECTORY_RETRY_AFTER: &str = "directory_retry_after";
    /// Group members still to be looked up (JSON list of IDs), see `owe_member_lookups`.
    pub const MEMBER_LOOKUPS_OWED: &str = "member_lookups_owed";
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
    /// Seconds between contact refreshes, when the directory has a say in it (`checkInterval`).
    pub const CONTACTS_CHECK_INTERVAL: &str = "contacts_check_interval";
    /// Public nickname.
    pub const NICKNAME: &str = "nickname";
    /// A new or ID-backup-restored ID whose user hasn't said what to do about a nickname: messages
    /// leave contacts' stored name alone. `ask`: to be asked; `asked`: declined for now.
    pub const NICKNAME_UNANSWERED: &str = "nickname_unanswered";
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
    /// The region of address book numbers without a country code, if the user chose one.
    pub const ADDRESS_BOOK_COUNTRY: &str = "address_book_country";
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
    /// Forward secrecy answers and statuses a connection ended on, as JSON
    /// (`connection::carried_to_json`): until the next one has handled them.
    pub const FS_EVENTS_CARRIED: &str = "fs_events_carried";
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
    /// How many chats the last backup had to leave out (`export::CHATS_WITHOUT_CONTACT`).
    pub const ANDROID_EXPORT_LEFT_OUT: &str = "android_export_left_out";
    /// The same for private chats, which `androidBackupExportStatus` adds only while they are open.
    pub const ANDROID_EXPORT_LEFT_OUT_PRIVATE: &str = "android_export_left_out_private";
    /// How many media files the last backup couldn't read (`export::FILES_UNREADABLE`).
    pub const ANDROID_EXPORT_FILES_LEFT_OUT: &str = "android_export_files_left_out";
    /// The same for files of private chats.
    pub const ANDROID_EXPORT_FILES_LEFT_OUT_PRIVATE: &str = "android_export_files_left_out_private";
    /// When the schedule last started a backup, whatever became of it.
    pub const ANDROID_EXPORT_ATTEMPT: &str = "android_export_attempt";
    pub const ANDROID_EXPORT_ERROR: &str = "android_export_error";
    /// What the pending Safe import did (JSON counts: imported, skipped and why), for the
    /// status page.
    pub const SAFE_IMPORT_SUMMARY: &str = "safe_import_summary";
    /// A restore's setup steps still to run (`App::restore_setup_step`): `start`, or the JSON list
    /// of contacts still to go.
    pub const SAFE_RESTORE_SETUP: &str = "safe_restore_setup";
    /// Set by store migration 35: see `sync::reflect_group_contacts_once`.
    pub const GROUP_CONTACTS_TO_REFLECT: &str = "group_contacts_to_reflect";
}

/// The encrypted store.
pub struct Store {
    pub(crate) conn: Connection,
    /// Messages another device forwarded whose notification waits (multi-device): if they
    /// are read there meanwhile, the phone stays silent. In memory only.
    pub(crate) held_notifications: std::cell::RefCell<std::collections::HashSet<i64>>,
    /// IDs the directory just said don't exist, with when (`groups::recently_invalid`). In memory
    /// only: a day's worth, bounded by what may be looked up.
    pub(crate) invalid_ids: std::cell::RefCell<std::collections::HashMap<String, i64>>,
    /// The current hour's group member lookups: when it began, and how many so far.
    pub(crate) member_lookups: std::cell::Cell<(i64, usize)>,
}

/// What the WAL may keep of its file once its contents are in the database: SQLite would otherwise
/// keep its largest size so far until the last connection closes, and on the Full flavour that is
/// never. Several times the 4 MiB at which SQLite checkpoints by itself.
const WAL_SIZE_LIMIT: i64 = 16 * 1024 * 1024;

/// Writes that stand or fall together: what isn't [`Step::commit`]ted is undone when this goes out
/// of scope. A step inside a step is a savepoint, so a function that needs one can be called from
/// a larger one; there is one connection and a step never spans an `await`, so nothing else writes
/// meanwhile.
pub(crate) struct Step<'a> {
    conn: &'a Connection,
    nested: bool,
    open: bool,
}

impl Step<'_> {
    pub(crate) fn commit(mut self) -> Result<(), StoreError> {
        self.conn.execute_batch(if self.nested {
            "RELEASE store_step"
        } else {
            "COMMIT"
        })?;
        self.open = false;
        Ok(())
    }
}

impl Drop for Step<'_> {
    fn drop(&mut self) {
        if self.open {
            // Fails only if SQLite has rolled back by itself (a full disk, say): then it's undone.
            let _ = self.conn.execute_batch(if self.nested {
                "ROLLBACK TO store_step; RELEASE store_step"
            } else {
                "ROLLBACK"
            });
        }
    }
}

/// Below this share of free pages (in percent) a rewrite gives back too little to wait for.
const SHRINK_MIN_FREE_PERCENT: i64 = 25;
/// VACUUM's copy of the live pages lies in memory: SQLCipher builds keep temporary data out of
/// plain files (`SQLITE_TEMP_STORE=2`). More than this could run a phone out of memory, 32-bit ones
/// above all.
const SHRINK_LIVE_BYTES_MAX: i64 = 128 * 1024 * 1024;

fn worth_shrinking(total_pages: i64, free_pages: i64, page_size: i64) -> bool {
    free_pages * 100 >= total_pages * SHRINK_MIN_FREE_PERCENT
        && free_pages > 0
        && (total_pages - free_pages).saturating_mul(page_size) <= SHRINK_LIVE_BYTES_MAX
}

/// How long automatic directory calls rest after a 429. Maintenance runs hourly, so together
/// that's about one request an hour or two while the directory is limiting us.
const DIRECTORY_REST_MS: i64 = 3_600_000;

/// A date further off than a rest lasts comes from a clock that was wrong then, and must not stop
/// the calls for however long that was.
fn directory_resting_at(until: Option<i64>, now: i64) -> bool {
    until.is_some_and(|until| now < until && until - now <= DIRECTORY_REST_MS)
}

impl Store {
    /// Open (or create) the database at `path` with the given raw 32-byte key.
    ///
    /// # Errors
    /// [`StoreError::WrongKey`] if the key does not match an existing database.
    pub fn open(path: &Path, key: &[u8; 32]) -> Result<Self, StoreError> {
        Self::open_with(path, key, &|| {})
    }

    /// Like [`Self::open`]; `upgrading` is called before an existing database's schema is brought
    /// up to date, which can take a while.
    ///
    /// # Errors
    /// See [`Self::open`].
    pub fn open_with(
        path: &Path,
        key: &[u8; 32],
        upgrading: &dyn Fn(),
    ) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        let mut pragma = Zeroizing::new(format!(
            "PRAGMA key = \"x'{}'\";",
            data_encoding::HEXLOWER.encode(key)
        ));
        conn.execute_batch(&pragma)?;
        pragma.zeroize();
        // Fails with "file is not a database" if the key is wrong. Any other failure says nothing
        // about the key: a full disk or an I/O error must not read as a wrong one.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
            .map_err(|error| {
                if error.sqlite_error_code() == Some(rusqlite::ErrorCode::NotADatabase) {
                    StoreError::WrongKey
                } else {
                    StoreError::Sqlite(error)
                }
            })?;
        conn.execute_batch(&format!(
            "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;
             PRAGMA journal_size_limit = {WAL_SIZE_LIMIT};"
        ))?;
        let mut store = Self {
            conn,
            held_notifications: std::cell::RefCell::default(),
            invalid_ids: std::cell::RefCell::default(),
            member_lookups: std::cell::Cell::default(),
        };
        store.migrate(MIGRATIONS, upgrading)?;
        Ok(store)
    }

    /// Begin writes that stand or fall together ([`Step`]).
    pub(crate) fn step(&self) -> Result<Step<'_>, StoreError> {
        let nested = !self.conn.is_autocommit();
        self.conn.execute_batch(if nested {
            "SAVEPOINT store_step"
        } else {
            "BEGIN"
        })?;
        Ok(Step {
            conn: &self.conn,
            nested,
            open: true,
        })
    }

    fn migrate(&mut self, migrations: &[&str], upgrading: &dyn Fn()) -> Result<(), StoreError> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let target = i64::try_from(migrations.len()).unwrap_or(i64::MAX);
        if version > target {
            return Err(StoreError::TooNew(version));
        }
        if version == target {
            return Ok(());
        }
        if version > 0 {
            upgrading();
        }
        self.apply_migrations(migrations, version)
            .map_err(|error| match error {
                StoreError::Sqlite(error) if is_disk_full(&error) => {
                    StoreError::UpgradeNoRoom(error)
                }
                other => other,
            })?;
        // A row-rewriting migration leaves a WAL about as large as the data it touched, and the
        // file would otherwise stay that large for as long as the app runs.
        if let Err(error) = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                row.get::<_, i64>(0)
            })
        {
            // The upgrade itself is committed; the next checkpoint gets another go at it.
            let code = StoreError::Sqlite(error).code();
            tracing::warn!(code, "checkpoint after the store upgrade failed");
        }
        Ok(())
    }

    fn apply_migrations(&mut self, migrations: &[&str], version: i64) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        for (index, sql) in migrations
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
    /// The directory said too many requests: what runs by itself leaves it alone for a while.
    /// What the user asks for still goes, and is told what the directory says. Best effort. A rest
    /// under way keeps its end: otherwise whatever keeps asking would keep it going for good, and
    /// the license recheck is among what waits.
    pub fn rest_directory_calls(&self) {
        if self.directory_resting() {
            return;
        }
        tracing::warn!("the directory says too many requests, automatic calls rest");
        let until = crate::messages::now_ms() + DIRECTORY_REST_MS;
        if let Err(error) = self.set_setting(keys::DIRECTORY_RETRY_AFTER, &until.to_string()) {
            tracing::warn!(%error, "noting the directory's rate limit failed");
        }
    }

    /// Group members who have no contact yet and still need their lookup: a rest or a failed
    /// request doesn't lose them. Noted before asking, and kept until the directory has answered
    /// for them. Without another device too; `group_member_lookups` is what the devices are owed.
    pub fn owe_member_lookups(&self, identities: &[String]) -> Result<(), StoreError> {
        let mut owed: std::collections::BTreeSet<String> =
            self.owed_member_lookups()?.into_iter().collect();
        owed.extend(identities.iter().cloned());
        self.set_owed_member_lookups(&owed)
    }

    pub fn owed_member_lookups(&self) -> Result<Vec<String>, StoreError> {
        Ok(self
            .setting(keys::MEMBER_LOOKUPS_OWED)?
            .and_then(|owed| serde_json::from_str(&owed).ok())
            .unwrap_or_default())
    }

    /// The directory has answered for these, whatever it said.
    pub fn member_lookups_answered(&self, identities: &[&str]) -> Result<(), StoreError> {
        let mut owed: std::collections::BTreeSet<String> =
            self.owed_member_lookups()?.into_iter().collect();
        for identity in identities {
            owed.remove(*identity);
        }
        self.set_owed_member_lookups(&owed)
    }

    fn set_owed_member_lookups(
        &self,
        owed: &std::collections::BTreeSet<String>,
    ) -> Result<(), StoreError> {
        if owed.is_empty() {
            return self.delete_setting(keys::MEMBER_LOOKUPS_OWED);
        }
        let owed = serde_json::to_string(owed).expect("a list of strings");
        self.set_setting(keys::MEMBER_LOOKUPS_OWED, &owed)
    }

    /// Whether automatic directory calls are resting.
    pub fn directory_resting(&self) -> bool {
        let until = self
            .setting(keys::DIRECTORY_RETRY_AFTER)
            .ok()
            .flatten()
            .and_then(|until| until.parse().ok());
        directory_resting_at(until, crate::messages::now_ms())
    }

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
        self.invalid_ids.borrow_mut().clear();
        self.shrink();
        Ok(())
    }

    /// [`Self::shrink`] if enough of the file is free for it to matter and the copy fits in memory
    /// (`worth_shrinking`). Otherwise later writes reuse the free pages.
    pub fn shrink_if_worthwhile(&self) {
        let pages = self.conn.query_row(
            "SELECT c.page_count, f.freelist_count, CAST(s.page_size AS INTEGER)
             FROM pragma_page_count() c, pragma_freelist_count() f, pragma_page_size() s",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        );
        match pages {
            Ok((total, free, page_size)) if worth_shrinking(total, free, page_size) => {
                self.shrink();
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "reading the free pages failed"),
        }
    }

    /// Give the space of deleted rows back to the system: they still lie in the free pages and the
    /// write-ahead log until these two ran. Rewrites the whole file, and the core waits for it, so
    /// for what the user just asked to be gone, not for routine deletions. Needs free space for
    /// the copy, which can run out just when someone clears up a full phone: then the file stays
    /// as big as it was and later writes reuse its free pages, so this is only logged.
    fn shrink(&self) {
        for (step, sql) in [
            ("vacuum", "VACUUM;"),
            ("checkpoint", "PRAGMA wal_checkpoint(TRUNCATE);"),
        ] {
            if let Err(error) = self.conn.execute_batch(sql) {
                tracing::warn!(step, %error, "the database could not be shrunk");
            }
        }
    }

    /// Remove a setting.
    pub fn delete_setting(&self, key: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }

    /// The own e-mail address and number as far as they are confirmed: what backups and linked
    /// devices may hold (Android: only `LinkingState_LINKED`). One still waiting for its
    /// confirmation may be a typo, and a restore would link it again, with a mail or an SMS.
    pub fn confirmed_links(&self) -> Result<(Option<String>, Option<String>), StoreError> {
        let email = match self.setting(keys::LINKED_EMAIL_PENDING)? {
            Some(_) => None,
            None => self.setting(keys::LINKED_EMAIL)?,
        };
        let mobile = match self.setting(keys::LINKED_MOBILE_PENDING)? {
            Some(_) => None,
            None => self.setting(keys::LINKED_MOBILE)?,
        };
        Ok((email, mobile))
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
    fn a_directory_rest_ends_also_when_the_clock_was_wrong() {
        let now = 1_700_000_000_000;
        assert!(!directory_resting_at(None, now));
        assert!(directory_resting_at(Some(now + 1), now));
        assert!(directory_resting_at(Some(now + DIRECTORY_REST_MS), now));
        assert!(!directory_resting_at(Some(now), now), "over");
        assert!(
            !directory_resting_at(Some(now + DIRECTORY_REST_MS + 1), now),
            "set by a clock ahead of this one"
        );
    }

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

    /// A group sync from another device can delete a group, inside the step that applies it.
    #[test]
    fn deleting_chats_works_inside_a_step() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_identity(&dir.path().join("t.db"));
        let step = store.step().unwrap();
        store.delete_group("g:ABCD1234:0000000000000007").unwrap();
        store.delete_conversation("ABCD1234").unwrap();
        store.delete_distribution_list("d:1").unwrap();
        step.commit().unwrap();
        assert!(store.conn.is_autocommit());
    }

    #[test]
    fn only_failures_of_the_data_itself_count_as_failing_every_time() {
        use rusqlite::ffi;
        for code in [
            0,
            ffi::SQLITE_ERROR,
            ffi::SQLITE_TOOBIG,
            ffi::SQLITE_CONSTRAINT,
            ffi::SQLITE_CONSTRAINT_TRIGGER,
            ffi::SQLITE_MISMATCH,
        ] {
            assert!(fails_every_time(code), "{code}");
        }
        for code in [
            ffi::SQLITE_FULL,
            ffi::SQLITE_IOERR,
            ffi::SQLITE_IOERR_SHMSIZE,
            ffi::SQLITE_READONLY,
            ffi::SQLITE_BUSY,
            ffi::SQLITE_LOCKED,
            ffi::SQLITE_NOMEM,
            ffi::SQLITE_CANTOPEN,
            ffi::SQLITE_CORRUPT,
            ffi::SQLITE_NOTADB,
        ] {
            assert!(!fails_every_time(code), "{code}");
        }
    }

    /// A step undoes what it didn't commit, and one inside another only its own part.
    #[test]
    fn steps_nest() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_identity(&dir.path().join("t.db"));
        let get = |key| store.setting(key).unwrap();

        let outer = store.step().unwrap();
        store.set_setting("kept", "1").unwrap();
        let inner = store.step().unwrap();
        store.set_setting("undone", "1").unwrap();
        drop(inner);
        let inner = store.step().unwrap();
        store.set_setting("kept too", "1").unwrap();
        inner.commit().unwrap();
        outer.commit().unwrap();
        assert!(store.conn.is_autocommit());
        assert!(get("kept").is_some() && get("kept too").is_some());
        assert!(get("undone").is_none());

        // What an inner step committed goes with the outer one.
        let outer = store.step().unwrap();
        let inner = store.step().unwrap();
        store.set_setting("gone", "1").unwrap();
        inner.commit().unwrap();
        drop(outer);
        assert!(store.conn.is_autocommit());
        assert!(get("gone").is_none());
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

    #[test]
    fn shrinking_is_for_a_large_share_of_a_database_that_fits_in_memory() {
        let page = 4096;
        let megabyte = 1024 * 1024 / page;
        assert!(!worth_shrinking(1000, 0, page), "nothing free");
        assert!(!worth_shrinking(1000, 249, page), "too little free");
        assert!(worth_shrinking(1000, 250, page));
        assert!(worth_shrinking(1000, 1000, page), "everything free");
        assert!(worth_shrinking(128 * megabyte * 2, 128 * megabyte, page));
        assert!(
            !worth_shrinking(129 * megabyte * 2, 129 * megabyte, page),
            "the copy would be too large for memory"
        );
    }

    /// A store on disk, closed again with everything in the main file.
    fn closed_store(path: &Path) {
        drop(store_with_identity(path));
    }

    fn open_error(path: &Path, key: &[u8; 32]) -> StoreError {
        match Store::open(path, key) {
            Ok(_) => panic!("the store opened"),
            Err(error) => error,
        }
    }

    #[test]
    fn a_key_that_does_not_fit_is_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        closed_store(&path);
        let error = open_error(&path, &[2; 32]);
        assert!(matches!(error, StoreError::WrongKey), "{error}");
        assert_eq!(error.kind(), "storeKey");
        assert_eq!(error.code(), "");
    }

    #[test]
    fn a_damaged_first_page_reads_like_a_wrong_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        closed_store(&path);
        let mut raw = std::fs::read(&path).unwrap();
        raw[100..200].fill(0xA5);
        std::fs::write(&path, raw).unwrap();
        assert_eq!(open_error(&path, &[1; 32]).kind(), "storeKey");
    }

    #[test]
    fn damage_behind_the_first_page_is_damage_and_not_a_wrong_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        closed_store(&path);
        let mut raw = std::fs::read(&path).unwrap();
        for page in raw[4096..].chunks_mut(4096) {
            page[100..200].fill(0xA5);
        }
        std::fs::write(&path, raw).unwrap();
        let error = open_error(&path, &[1; 32]);
        assert_eq!(error.kind(), "storeDamaged", "{error}");
        assert_eq!(error.code(), "sqlite 11");
    }

    #[test]
    fn a_path_that_cannot_be_opened_is_no_key_problem() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be.
        let error = open_error(dir.path(), &[1; 32]);
        assert_eq!(error.kind(), "storeUnreadable", "{error}");
    }

    #[test]
    fn a_full_disk_is_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        // How SQLite's limit looks to the caller is what a full disk looks like: SQLITE_FULL.
        store
            .conn
            .execute_batch("PRAGMA max_page_count = 1")
            .unwrap();
        let error = store.set_setting("key", &"x".repeat(100_000)).unwrap_err();
        assert_eq!(error.kind(), "diskFull", "{error}");

        // A store reopened on a full disk, as measured on a full tmpfs.
        let error = StoreError::Sqlite(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR_SHMSIZE),
            None,
        ));
        assert_eq!(error.kind(), "diskFull");
        assert_eq!(error.code(), "sqlite 4874");
    }

    #[test]
    fn a_newer_schema_is_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let newer = MIGRATIONS.len() + 1;
        Store::open(&path, &[1; 32])
            .unwrap()
            .conn
            .execute_batch(&format!("PRAGMA user_version = {newer}"))
            .unwrap();
        let error = open_error(&path, &[1; 32]);
        assert_eq!(error.kind(), "storeTooNew");
        assert_eq!(error.code(), format!("schema {newer}"));
    }

    #[test]
    fn an_upgrade_says_so_first_and_a_new_database_or_a_current_one_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let told = std::cell::Cell::new(0);
        let tell = || told.set(told.get() + 1);
        let mut store = Store::open_with(&path, &[1; 32], &tell).unwrap();
        assert_eq!(told.get(), 0, "a new database has nothing to upgrade");
        drop(store);
        store = Store::open_with(&path, &[1; 32], &tell).unwrap();
        assert_eq!(told.get(), 0, "up to date");

        let mut next = MIGRATIONS.to_vec();
        next.push("CREATE TABLE later (a INTEGER)");
        store.migrate(&next, &tell).unwrap();
        assert_eq!(told.get(), 1);
        store.migrate(&next, &tell).unwrap();
        assert_eq!(told.get(), 1, "done");
    }

    #[test]
    fn an_upgrade_without_room_says_so_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let pages: i64 = store
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        store
            .conn
            .execute_batch(&format!("PRAGMA max_page_count = {pages}"))
            .unwrap();
        let mut next = MIGRATIONS.to_vec();
        next.push(
            "CREATE TABLE later (a TEXT);
             INSERT INTO later VALUES (zeroblob(1000000));",
        );

        let error = store.migrate(&next, &|| {}).unwrap_err();

        assert_eq!(error.kind(), "upgradeNoRoom", "{error}");
        let version: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, i64::try_from(MIGRATIONS.len()).unwrap());
        assert!(
            store.conn.prepare("SELECT a FROM later").is_err(),
            "rolled back"
        );
    }

    #[test]
    fn an_upgrade_leaves_no_large_wal_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let mut store = Store::open(&path, &[1; 32]).unwrap();
        let limit: i64 = store
            .conn
            .query_row("PRAGMA journal_size_limit", [], |row| row.get(0))
            .unwrap();
        assert_eq!(limit, WAL_SIZE_LIMIT);

        let mut next = MIGRATIONS.to_vec();
        next.push(
            "CREATE TABLE later (a BLOB);
             INSERT INTO later VALUES (zeroblob(30000000));",
        );
        store.migrate(&next, &|| {}).unwrap();

        let wal = std::fs::metadata(dir.path().join("t.db-wal"))
            .unwrap()
            .len();
        assert_eq!(wal, 0);
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
