// SPDX-License-Identifier: AGPL-3.0-only
//! Profile pictures (csp.struct.yml "Profile Picture Distribution Steps", `set-` / `delete-` /
//! `contact-request-profile-picture`):
//!
//! - Contacts' pictures (_contact-defined_) are stored encrypted in the media store under
//!   `contact:<ID>`; `contact_pictures.version` changes with them (the UI reloads by it).
//! - Our own picture lives under `me`; its upload (blob id, key, size, time) is reused for seven
//!   days. `picture_distribution` remembers per contact what we last sent (a blob id or a
//!   removal), so each contact gets the picture once with the next message that distributes the
//!   user profile.

use libthreema::common::BlobId;
use rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::{
    messages::now_ms,
    store::{Store, StoreError},
};

/// Schema (store migration 11).
pub(crate) const MIGRATION: &str = "
    CREATE TABLE contact_pictures (
        identity  TEXT PRIMARY KEY NOT NULL,
        version   INTEGER NOT NULL
    ) WITHOUT ROWID;
    CREATE TABLE picture_distribution (
        identity    TEXT PRIMARY KEY NOT NULL,
        blob_id     BLOB,
        removed_at  INTEGER
    ) WITHOUT ROWID;";

/// Who gets our picture when sharing with selected contacts (store migration 18; Android's
/// `ProfilePictureRecipientsService`).
pub(crate) const MIGRATION_RECIPIENTS: &str = "
    CREATE TABLE picture_recipients (
        identity  TEXT PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;";

/// Picture recovery state (store migration 27). `picture_requests` replaces the permanent
/// `pictureRequested:<ID>` settings markers, which suppressed every later request forever (a
/// restore left all pictures missing for good): bounded backoff instead; old markers migrate
/// with one attempt and a short pause, so existing installations recover. `picture_downloads`
/// keeps a received picture descriptor until the download succeeded — before, any failure
/// dropped it and nothing retried.
pub(crate) const MIGRATION_RECOVERY: &str = "
    CREATE TABLE picture_requests (
        identity      TEXT PRIMARY KEY NOT NULL,
        requested_at  INTEGER NOT NULL,
        attempts      INTEGER NOT NULL,
        next_at       INTEGER NOT NULL
    ) WITHOUT ROWID;
    INSERT OR IGNORE INTO picture_requests (identity, requested_at, attempts, next_at)
        SELECT substr(key, 18), CAST(value AS INTEGER), 1, CAST(value AS INTEGER) + 21600000
        FROM settings WHERE key LIKE 'pictureRequested:%';
    DELETE FROM settings WHERE key LIKE 'pictureRequested:%';
    CREATE TABLE picture_downloads (
        identity    TEXT PRIMARY KEY NOT NULL,
        blob        BLOB NOT NULL,
        key         BLOB NOT NULL,
        size        INTEGER NOT NULL,
        generation  INTEGER NOT NULL,
        attempts    INTEGER NOT NULL,
        next_at     INTEGER NOT NULL
    ) WITHOUT ROWID;";

/// Settings keys.
pub const SETTING_UPLOAD: &str = "profilePictureUpload";
pub const SETTING_SHARE: &str = "profilePictureShare";
/// "Show profile pictures" ("0" off; default on), Android's `getProfilePicReceive`: pictures
/// contacts send, or other devices sync, are downloaded and stored all the same, and everything
/// that shows one comes by [`Store::shown_contact_picture_version`] and
/// [`crate::media::MediaStore::read_picture_for_ui`]; with it off contacts show the usual stand-in.
/// Android asks for pictures only in its setup steps; we also ask when a chat opens or a message
/// comes, which stops with this off ([`Store::request_picture`]). (The key is from when it was
/// called "Accept".)
pub const SETTING_SHOW: &str = "profilePictureReceive";
/// Media store names.
pub const OWN_PICTURE: &str = "me";
/// Our picture in Threema Safe's format (400 px, q=60), made by the bridge with `me`.
pub const OWN_PICTURE_SAFE: &str = "me-safe";
pub fn contact_picture_name(identity: &str) -> String {
    format!("contact:{identity}")
}
pub fn is_contact_picture(name: &str) -> bool {
    name.starts_with("contact:")
}

/// Outbox kind: ask a contact for their picture (`body`: `{}`).
pub const OUTBOX_REQUEST_PICTURE: &str = "requestProfilePicture";

const HOUR_MS: i64 = 60 * 60 * 1000;
const DAY_MS: i64 = 24 * HOUR_MS;
const WEEK_MS: i64 = 7 * DAY_MS;

/// How long each picture request holds off the next one (bounded backoff; the last pause
/// repeats). Requests are real protocol messages, and no answer may simply mean the contact
/// has no picture or keeps it from us — that is not an error, so asking often is useless.
const REQUEST_PAUSES_MS: [i64; 4] = [6 * HOUR_MS, 3 * DAY_MS, 10 * DAY_MS, 30 * DAY_MS];

/// After they deleted their picture: nothing to fetch until they set a new one, which they
/// distribute by themselves.
pub const REQUEST_PAUSE_DELETED_MS: i64 = 30 * DAY_MS;

/// After a download found the blob expired (or undecryptable): only a freshly requested
/// picture helps; one request is allowed again after this.
pub const REQUEST_PAUSE_EXPIRED_MS: i64 = HOUR_MS;

/// Pauses between download retries of a kept descriptor (transient failures; the last one
/// repeats until the attempts are used up).
const DOWNLOAD_PAUSES_MS: [i64; 5] = [60_000, 10 * 60_000, HOUR_MS, 6 * HOUR_MS, DAY_MS];
const DOWNLOAD_MAX_ATTEMPTS: i64 = 10;

/// A new version for a picture that just changed: later than now _and_ than the last one, so
/// two changes in the same millisecond never reuse a cached image URL.
pub fn next_version(previous: i64) -> i64 {
    now_ms().max(previous + 1)
}

/// Who gets our profile picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Share {
    Everyone,
    Nobody,
    /// The contacts in `picture_recipients` (Android's allow list).
    Selected,
}

impl Share {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Everyone => "everyone",
            Self::Nobody => "nobody",
            Self::Selected => "selected",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "everyone" => Some(Self::Everyone),
            "nobody" => Some(Self::Nobody),
            "selected" => Some(Self::Selected),
            _ => None,
        }
    }
}

/// Threema Safe's `profilePicRelease` (Android `ThreemaSafeServiceImpl`): `["*"]` everyone,
/// `[null]` nobody, otherwise the allow list.
pub fn safe_release(share: Share, recipients: &[String]) -> Vec<Option<String>> {
    match share {
        Share::Everyone => vec![Some("*".to_owned())],
        Share::Nobody => vec![None],
        Share::Selected => recipients.iter().cloned().map(Some).collect(),
    }
}

/// The setting from a Safe backup's `profilePicRelease`, read like Android: the first `null` or
/// `"*"` decides, otherwise IDs (8 characters) form the allow list.
pub fn from_safe_release(release: &[Option<String>]) -> (Share, Vec<String>) {
    let mut recipients = Vec::new();
    for entry in release {
        match entry.as_deref() {
            None | Some("null") => return (Share::Nobody, Vec::new()),
            Some("*") => return (Share::Everyone, Vec::new()),
            Some(id) if id.len() == 8 => recipients.push(id.to_owned()),
            Some(_) => {}
        }
    }
    (Share::Selected, recipients)
}

/// Our uploaded picture (persisted blob).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub blob_id: BlobId,
    pub key: [u8; 32],
    pub size: u32,
    pub uploaded_at: i64,
}

/// What a contact should get before our next message to them.
#[derive(Debug, PartialEq, Eq)]
pub enum Distribution {
    /// Nothing (up to date, or not applicable).
    None,
    /// Our picture: upload it first if `upload` is `None` (or too old).
    Set(Option<Upload>),
    /// A `delete-profile-picture`.
    Delete,
}

impl Store {
    pub fn contact_picture_version(&self, identity: &str) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT version FROM contact_pictures WHERE identity = ?1",
                [identity],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    /// The contact's picture changed (`version`, raised to stay monotonic) or was removed
    /// (`None`). A stored picture clears the request backoff: a later deletion starts fresh.
    pub fn set_contact_picture(
        &self,
        identity: &str,
        version: Option<i64>,
    ) -> Result<(), StoreError> {
        match version {
            Some(version) => {
                let version = version.max(self.contact_picture_version(identity)? + 1);
                self.conn.execute(
                    "INSERT OR REPLACE INTO contact_pictures (identity, version) VALUES (?1, ?2)",
                    params![identity, version],
                )?;
                self.conn.execute(
                    "DELETE FROM picture_requests WHERE identity = ?1",
                    [identity],
                )?;
            }
            None => {
                self.conn.execute(
                    "DELETE FROM contact_pictures WHERE identity = ?1",
                    [identity],
                )?;
            }
        };
        Ok(())
    }

    /// A new version for our own picture (`PROFILE_PICTURE_VERSION`), monotonic like the
    /// contacts' versions.
    pub fn bump_profile_picture_version(&self) -> Result<(), StoreError> {
        let old: i64 = self
            .setting(crate::store::keys::PROFILE_PICTURE_VERSION)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        self.set_setting(
            crate::store::keys::PROFILE_PICTURE_VERSION,
            &next_version(old).to_string(),
        )
    }

    pub fn picture_share(&self) -> Result<Share, StoreError> {
        Ok(self
            .setting(SETTING_SHARE)?
            .as_deref()
            .and_then(Share::parse)
            .unwrap_or(Share::Everyone))
    }

    pub fn set_picture_share(&self, share: Share) -> Result<(), StoreError> {
        self.set_setting(SETTING_SHARE, share.as_str())
    }

    /// The allow list for [`Share::Selected`], sorted.
    pub fn picture_recipients(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT identity FROM picture_recipients ORDER BY identity")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn is_picture_recipient(&self, identity: &str) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM picture_recipients WHERE identity = ?1",
                [identity],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Add `identity` to the allow list or remove it. Its cached distribution state is dropped, so
    /// the next message to it sends the picture or its removal.
    pub fn set_picture_recipient(&self, identity: &str, share: bool) -> Result<(), StoreError> {
        if share {
            self.conn.execute(
                "INSERT OR IGNORE INTO picture_recipients (identity) VALUES (?1)",
                [identity],
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM picture_recipients WHERE identity = ?1",
                [identity],
            )?;
        }
        self.purge_picture_distribution(Some(identity))
    }

    /// Replace the allow list.
    pub fn set_picture_recipients(&self, identities: &[String]) -> Result<(), StoreError> {
        let old = self.picture_recipients()?;
        for identity in old.iter().filter(|id| !identities.contains(id)) {
            self.set_picture_recipient(identity, false)?;
        }
        for identity in identities.iter().filter(|id| !old.contains(id)) {
            self.set_picture_recipient(identity, true)?;
        }
        Ok(())
    }

    pub fn picture_upload(&self) -> Result<Option<Upload>, StoreError> {
        let Some(json) = self.setting(SETTING_UPLOAD)? else {
            return Ok(None);
        };
        let value: Value = serde_json::from_str(&json).unwrap_or_default();
        let hex = |field: &str| {
            data_encoding::HEXLOWER
                .decode(value[field].as_str().unwrap_or_default().as_bytes())
                .ok()
        };
        Ok((|| {
            Some(Upload {
                blob_id: BlobId(hex("blob")?.try_into().ok()?),
                key: hex("key")?.try_into().ok()?,
                size: u32::try_from(value["size"].as_u64()?).ok()?,
                uploaded_at: value["uploadedAt"].as_i64()?,
            })
        })())
    }

    pub fn set_picture_upload(&self, upload: Option<&Upload>) -> Result<(), StoreError> {
        match upload {
            Some(upload) => self.set_setting(
                SETTING_UPLOAD,
                &json!({
                    "blob": data_encoding::HEXLOWER.encode(&upload.blob_id.0),
                    "key": data_encoding::HEXLOWER.encode(&upload.key),
                    "size": upload.size,
                    "uploadedAt": upload.uploaded_at,
                })
                .to_string(),
            ),
            None => {
                self.conn
                    .execute("DELETE FROM settings WHERE key = ?1", [SETTING_UPLOAD])?;
                Ok(())
            }
        }
    }

    /// The _Profile Picture Distribution Steps_ (without the upload) for `receiver`.
    pub fn picture_distribution(
        &self,
        receiver: &str,
        have_picture: bool,
    ) -> Result<Distribution, StoreError> {
        if receiver == "ECHOECHO" || receiver.starts_with('*') {
            return Ok(Distribution::None);
        }
        let cache: Option<(Option<Vec<u8>>, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT blob_id, removed_at FROM picture_distribution WHERE identity = ?1",
                [receiver],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let now = now_ms();
        let shared = match self.picture_share()? {
            Share::Everyone => true,
            Share::Nobody => false,
            Share::Selected => self.is_picture_recipient(receiver)?,
        };
        if !have_picture || !shared {
            if let Some((None, Some(removed_at))) = cache
                && now - removed_at < WEEK_MS
            {
                return Ok(Distribution::None);
            }
            return Ok(Distribution::Delete);
        }
        let upload = self
            .picture_upload()?
            .filter(|upload| now - upload.uploaded_at <= WEEK_MS);
        if let (Some(upload), Some((Some(sent), _))) = (&upload, &cache)
            && sent.as_slice() == upload.blob_id.0
        {
            return Ok(Distribution::None);
        }
        Ok(Distribution::Set(upload))
    }

    /// Remember what `receiver` got (a blob id, or a removal).
    pub fn picture_sent(&self, receiver: &str, blob_id: Option<&BlobId>) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO picture_distribution (identity, blob_id, removed_at)
             VALUES (?1, ?2, ?3)",
            params![
                receiver,
                blob_id.map(|id| id.0.to_vec()),
                blob_id.is_none().then(now_ms)
            ],
        )?;
        Ok(())
    }

    /// Every contact's picture version in one query (the chat and contact lists).
    pub fn contact_picture_versions(
        &self,
    ) -> Result<std::collections::HashMap<String, i64>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT identity, version FROM contact_pictures")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Whether contacts' profile pictures are shown ([`SETTING_SHOW`]).
    pub fn pictures_shown(&self) -> Result<bool, StoreError> {
        Ok(self.setting(SETTING_SHOW)?.as_deref() != Some("0"))
    }

    /// The version of a contact's picture to show: 0, as for none, while they aren't shown.
    pub fn shown_contact_picture_version(&self, identity: &str) -> Result<i64, StoreError> {
        if self.pictures_shown()? {
            self.contact_picture_version(identity)
        } else {
            Ok(0)
        }
    }

    /// [`Self::shown_contact_picture_version`] of every contact with one, in one query.
    pub fn shown_contact_picture_versions(
        &self,
    ) -> Result<std::collections::HashMap<String, i64>, StoreError> {
        if self.pictures_shown()? {
            self.contact_picture_versions()
        } else {
            Ok(std::collections::HashMap::new())
        }
    }

    /// Ask `identity` for their picture (we have none, e.g. it was never received or the
    /// backup carried none): queued in the outbox; `true` if queued now. At most once per
    /// backoff step ([`REQUEST_PAUSES_MS`]); the marking and the queueing are one transaction,
    /// which also keeps concurrent triggers from queueing twice.
    pub fn request_picture(&self, identity: &str) -> Result<bool, StoreError> {
        // Not shown, so nothing to ask them for: what they send anyway still comes in.
        if !self.pictures_shown()? {
            return Ok(false);
        }
        if self.contact_picture_version(identity)? != 0
            || identity == "ECHOECHO"
            || identity.starts_with('*')
        {
            return Ok(false);
        }
        // Nobody to ask: a revoked ID, or someone we blocked.
        if self.is_explicitly_blocked(identity)?
            || self.contact(identity)?.is_some_and(|c| {
                c.activity_state == libthreema::protobuf::d2d_sync::contact::ActivityState::Invalid
            })
        {
            return Ok(false);
        }
        // The emulator's demo contacts don't exist.
        #[cfg(feature = "demo")]
        if identity.starts_with("DEMO") {
            return Ok(false);
        }
        let now = now_ms();
        let state: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT attempts, next_at FROM picture_requests WHERE identity = ?1",
                [identity],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let attempts = match state {
            Some((_, next_at)) if now < next_at => return Ok(false),
            Some((attempts, _)) => attempts.max(0),
            None => 0,
        };
        let pause = REQUEST_PAUSES_MS[usize::try_from(attempts)
            .unwrap_or(usize::MAX)
            .min(REQUEST_PAUSES_MS.len() - 1)];
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let queued = (|| -> Result<(), StoreError> {
            self.conn.execute(
                "INSERT OR REPLACE INTO picture_requests (identity, requested_at, attempts, next_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![identity, now, attempts + 1, now + pause],
            )?;
            self.outbox_add(
                identity,
                libthreema::common::MessageId::random().0,
                OUTBOX_REQUEST_PICTURE,
                &json!({}),
            )
        })();
        match queued {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(true)
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    /// Hold picture requests to `identity` for `pause_ms` without sending one (their picture
    /// was deleted, or its blob is gone).
    pub fn pause_picture_requests(&self, identity: &str, pause_ms: i64) -> Result<(), StoreError> {
        let now = now_ms();
        self.conn.execute(
            "INSERT OR REPLACE INTO picture_requests (identity, requested_at, attempts, next_at)
             VALUES (?1, ?2,
                     coalesce((SELECT attempts FROM picture_requests WHERE identity = ?1), 0),
                     ?3)",
            params![identity, now, now + pause_ms],
        )?;
        Ok(())
    }

    /// A contact's new picture descriptor arrived: kept until the download succeeded (it
    /// survives restarts and failures), under a new generation — whatever an older download
    /// still delivers is obsolete. Returns the generation.
    pub fn set_picture_download(
        &self,
        identity: &str,
        picture: &crate::groups::GroupPicture,
    ) -> Result<i64, StoreError> {
        let old: i64 = self
            .conn
            .query_row(
                "SELECT generation FROM picture_downloads WHERE identity = ?1",
                [identity],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let generation = next_version(old);
        self.conn.execute(
            "INSERT OR REPLACE INTO picture_downloads
                 (identity, blob, key, size, generation, attempts, next_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 0)",
            params![
                identity,
                picture.blob_id.0.to_vec(),
                picture.key.to_vec(),
                picture.size,
                generation
            ],
        )?;
        Ok(generation)
    }

    /// The pending download for `identity`: its descriptor and generation.
    pub fn picture_download(
        &self,
        identity: &str,
    ) -> Result<Option<(crate::groups::GroupPicture, i64)>, StoreError> {
        let row: Option<(Vec<u8>, Vec<u8>, i64, i64)> = self
            .conn
            .query_row(
                "SELECT blob, key, size, generation FROM picture_downloads WHERE identity = ?1",
                [identity],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        Ok(row.and_then(|(blob, key, size, generation)| {
            Some((
                crate::groups::GroupPicture {
                    blob_id: BlobId(blob.try_into().ok()?),
                    key: key.try_into().ok()?,
                    size: u32::try_from(size).ok()?,
                },
                generation,
            ))
        }))
    }

    /// Whose pending downloads are due now (startup, reconnect, maintenance).
    pub fn due_picture_downloads(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT identity FROM picture_downloads WHERE next_at <= ?1")?;
        let rows = statement.query_map([now_ms()], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Done (published) — drops the row if it still is this generation.
    pub fn clear_picture_download(
        &self,
        identity: &str,
        generation: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM picture_downloads WHERE identity = ?1 AND generation = ?2",
            params![identity, generation],
        )?;
        Ok(())
    }

    /// The picture was deleted (or the blob is gone for good): any pending download is
    /// obsolete, whatever its generation.
    pub fn cancel_picture_download(&self, identity: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM picture_downloads WHERE identity = ?1",
            [identity],
        )?;
        Ok(())
    }

    /// A download attempt failed for a passing reason: try again later with bounded backoff.
    /// `false` once the attempts are used up (the descriptor is dropped; a fresh request can
    /// recover the picture).
    pub fn picture_download_failed(
        &self,
        identity: &str,
        generation: i64,
    ) -> Result<bool, StoreError> {
        let attempts: Option<i64> = self
            .conn
            .query_row(
                "SELECT attempts FROM picture_downloads WHERE identity = ?1 AND generation = ?2",
                params![identity, generation],
                |row| row.get(0),
            )
            .optional()?;
        let Some(attempts) = attempts else {
            return Ok(false);
        };
        if attempts + 1 >= DOWNLOAD_MAX_ATTEMPTS {
            self.clear_picture_download(identity, generation)?;
            return Ok(false);
        }
        let pause = DOWNLOAD_PAUSES_MS[usize::try_from(attempts)
            .unwrap_or(usize::MAX)
            .min(DOWNLOAD_PAUSES_MS.len() - 1)];
        self.conn.execute(
            "UPDATE picture_downloads SET attempts = ?3, next_at = ?4
             WHERE identity = ?1 AND generation = ?2",
            params![identity, generation, attempts + 1, now_ms() + pause],
        )?;
        Ok(true)
    }

    /// _Common Request Profile Picture Receive Steps_ (or our picture changed: everyone).
    pub fn purge_picture_distribution(&self, receiver: Option<&str>) -> Result<(), StoreError> {
        match receiver {
            Some(receiver) => self.conn.execute(
                "DELETE FROM picture_distribution WHERE identity = ?1",
                [receiver],
            )?,
            None => self.conn.execute("DELETE FROM picture_distribution", [])?,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_release_round_trip() {
        let ids = vec!["00000BOB".to_owned(), "00000ANN".to_owned()];
        for (share, recipients) in [
            (Share::Everyone, vec![]),
            (Share::Nobody, vec![]),
            (Share::Selected, ids.clone()),
            (Share::Selected, vec![]),
        ] {
            assert_eq!(
                from_safe_release(&safe_release(share, &recipients)),
                (share, recipients)
            );
        }
        assert_eq!(safe_release(Share::Nobody, &ids), vec![None]);
        assert_eq!(
            from_safe_release(&[Some("00000BOB".into()), Some("bad".into())]),
            (Share::Selected, vec!["00000BOB".to_owned()])
        );
    }

    #[test]
    fn no_picture_requests_to_revoked_or_blocked_ids() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let mut revoked = crate::contacts::test_contact("00000BOB");
        revoked.activity_state = libthreema::protobuf::d2d_sync::contact::ActivityState::Invalid;
        store.upsert_contact(&revoked).unwrap();
        assert!(!store.request_picture("00000BOB").unwrap());

        store
            .set_setting(
                crate::store::keys::APP_SETTINGS,
                &json!({ "blockedContacts": ["0000CARL"] }).to_string(),
            )
            .unwrap();
        assert!(!store.request_picture("0000CARL").unwrap());

        assert!(store.outbox().unwrap().is_empty());
    }

    #[test]
    fn picture_request_backoff() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        // Queued once, then held back by the backoff.
        assert!(store.request_picture("00000BOB").unwrap());
        assert!(!store.request_picture("00000BOB").unwrap());
        // Gateway and no-picture IDs are never asked.
        assert!(!store.request_picture("*THREEMA").unwrap());
        assert!(!store.request_picture("ECHOECHO").unwrap());
        // The pause passed: asked again, counting up.
        store
            .conn
            .execute(
                "UPDATE picture_requests SET next_at = 0 WHERE identity = '00000BOB'",
                [],
            )
            .unwrap();
        assert!(store.request_picture("00000BOB").unwrap());
        let attempts: i64 = store
            .conn
            .query_row(
                "SELECT attempts FROM picture_requests WHERE identity = '00000BOB'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(attempts, 2);
        // A stored picture clears the state (a later deletion starts fresh) and ends requests.
        store.set_contact_picture("00000BOB", Some(1)).unwrap();
        assert!(!store.request_picture("00000BOB").unwrap());
        let rows: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM picture_requests WHERE identity = '00000BOB'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0);
        // Deleted and paused: no request until the pause is over.
        store.set_contact_picture("00000BOB", None).unwrap();
        store
            .pause_picture_requests("00000BOB", REQUEST_PAUSE_DELETED_MS)
            .unwrap();
        assert!(!store.request_picture("00000BOB").unwrap());
    }

    #[test]
    fn picture_download_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let picture = |b: u8| crate::groups::GroupPicture {
            blob_id: BlobId([b; 16]),
            key: [7; 32],
            size: 9,
        };
        let first = store.set_picture_download("00000BOB", &picture(1)).unwrap();
        let (stored, generation) = store.picture_download("00000BOB").unwrap().unwrap();
        assert_eq!(stored, picture(1));
        assert_eq!(generation, first);
        assert_eq!(
            store.due_picture_downloads().unwrap(),
            vec!["00000BOB".to_owned()]
        );
        // A newer descriptor gets a newer generation; the old download must not publish or
        // clear it.
        let second = store.set_picture_download("00000BOB", &picture(2)).unwrap();
        assert!(second > first);
        store.clear_picture_download("00000BOB", first).unwrap();
        assert!(
            store.picture_download("00000BOB").unwrap().is_some(),
            "an old generation's clear leaves the new row"
        );
        // Transient failures back off, and eventually the attempts are used up.
        assert!(store.picture_download_failed("00000BOB", second).unwrap());
        assert!(
            store.due_picture_downloads().unwrap().is_empty(),
            "waiting for the retry pause"
        );
        for _ in 0..DOWNLOAD_MAX_ATTEMPTS {
            if !store.picture_download_failed("00000BOB", second).unwrap() {
                break;
            }
        }
        assert!(
            store.picture_download("00000BOB").unwrap().is_none(),
            "attempts used up"
        );
        // A deletion cancels whatever is pending, whatever its generation.
        store.set_picture_download("00000BOB", &picture(3)).unwrap();
        store.cancel_picture_download("00000BOB").unwrap();
        assert!(store.picture_download("00000BOB").unwrap().is_none());
    }

    #[test]
    fn legacy_request_markers_migrate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        {
            let store = Store::open(&path, &[1; 32]).unwrap();
            // Pretend the store predates migration 27, with a once-ever marker (later
            // migrations' artifacts have to go too, so the reopen can replay them).
            store
                .conn
                .execute_batch(
                    "ALTER TABLE messages DROP COLUMN written_out;
                     ALTER TABLE groups DROP COLUMN kicked;
                     DROP TABLE group_member_lookups; DROP TABLE group_picture_syncs;
                     DROP TABLE chat_tones;
                     DROP TRIGGER messages_search_insert; DROP TRIGGER messages_search_update;
                     DROP TRIGGER messages_search_delete; DROP INDEX messages_mentions;
                     DROP TABLE messages_search; DROP TABLE marked_unread_chats;
                     DROP TRIGGER chat_summaries_insert; DROP TRIGGER chat_summaries_sort;
                     DROP TRIGGER chat_summaries_state; DROP TRIGGER chat_summaries_delete;
                     DROP TABLE chat_summaries;
                     DROP TABLE picture_requests; DROP TABLE picture_downloads;
                     DROP INDEX messages_position; DROP TRIGGER messages_sort_key;
                     ALTER TABLE messages DROP COLUMN sort_key;
                     DROP TABLE android_import_ids;
                     INSERT INTO settings (key, value) VALUES ('pictureRequested:00000BOB', '1000');
                     PRAGMA user_version = 26;",
                )
                .unwrap();
        }
        let store = Store::open(&path, &[1; 32]).unwrap();
        assert!(
            store
                .setting("pictureRequested:00000BOB")
                .unwrap()
                .is_none()
        );
        let (attempts, next_at): (i64, i64) = store
            .conn
            .query_row(
                "SELECT attempts, next_at FROM picture_requests WHERE identity = '00000BOB'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((attempts, next_at), (1, 1000 + 6 * HOUR_MS));
        // The old marker no longer suppresses recovery forever: long overdue, so it queues.
        assert!(store.request_picture("00000BOB").unwrap());
    }

    #[test]
    fn versions_stay_monotonic() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        // Two changes in the same millisecond never reuse a version (cached image URLs).
        store
            .set_contact_picture("00000BOB", Some(now_ms()))
            .unwrap();
        let first = store.contact_picture_version("00000BOB").unwrap();
        store
            .set_contact_picture("00000BOB", Some(now_ms()))
            .unwrap();
        assert!(store.contact_picture_version("00000BOB").unwrap() > first);
        store.bump_profile_picture_version().unwrap();
        let own = |store: &Store| -> i64 {
            store
                .setting(crate::store::keys::PROFILE_PICTURE_VERSION)
                .unwrap()
                .unwrap()
                .parse()
                .unwrap()
        };
        let first = own(&store);
        store.bump_profile_picture_version().unwrap();
        assert!(own(&store) > first);
        assert!(next_version(i64::MAX - 1) >= i64::MAX - 1);
    }

    #[test]
    fn distribution_steps() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        assert_eq!(
            store.picture_distribution("ECHOECHO", true).unwrap(),
            Distribution::None
        );
        // No picture: a removal, once a week.
        assert_eq!(
            store.picture_distribution("00000BOB", false).unwrap(),
            Distribution::Delete
        );
        store.picture_sent("00000BOB", None).unwrap();
        assert_eq!(
            store.picture_distribution("00000BOB", false).unwrap(),
            Distribution::None
        );
        // A picture: upload first, then once per upload.
        assert_eq!(
            store.picture_distribution("00000BOB", true).unwrap(),
            Distribution::Set(None)
        );
        let upload = Upload {
            blob_id: BlobId([3; 16]),
            key: [4; 32],
            size: 10,
            uploaded_at: now_ms(),
        };
        store.set_picture_upload(Some(&upload)).unwrap();
        assert_eq!(store.picture_upload().unwrap(), Some(upload.clone()));
        assert_eq!(
            store.picture_distribution("00000BOB", true).unwrap(),
            Distribution::Set(Some(upload.clone()))
        );
        store
            .picture_sent("00000BOB", Some(&upload.blob_id))
            .unwrap();
        assert_eq!(
            store.picture_distribution("00000BOB", true).unwrap(),
            Distribution::None
        );
        // Requested again.
        store.purge_picture_distribution(Some("00000BOB")).unwrap();
        assert!(matches!(
            store.picture_distribution("00000BOB", true).unwrap(),
            Distribution::Set(Some(_))
        ));
        // Sharing with nobody.
        store.set_picture_share(Share::Nobody).unwrap();
        assert_eq!(
            store.picture_distribution("00000BOB", true).unwrap(),
            Distribution::Delete
        );
        // Selected contacts: only those on the list.
        store.set_picture_share(Share::Selected).unwrap();
        assert_eq!(
            store.picture_distribution("00000BOB", true).unwrap(),
            Distribution::Delete
        );
        store.picture_sent("00000BOB", None).unwrap();
        store
            .set_picture_recipients(&["00000BOB".to_owned()])
            .unwrap();
        assert!(matches!(
            store.picture_distribution("00000BOB", true).unwrap(),
            Distribution::Set(Some(_))
        ));
        assert_eq!(
            store.picture_distribution("00000ANN", true).unwrap(),
            Distribution::Delete
        );
        store
            .picture_sent("00000BOB", Some(&upload.blob_id))
            .unwrap();
        // Taken off the list: the removal goes out with the next message.
        store.set_picture_recipients(&[]).unwrap();
        assert_eq!(
            store.picture_distribution("00000BOB", true).unwrap(),
            Distribution::Delete
        );
        assert!(store.picture_recipients().unwrap().is_empty());
        // Old uploads are not reused.
        store.set_picture_share(Share::Everyone).unwrap();
        store
            .set_picture_upload(Some(&Upload {
                uploaded_at: now_ms() - 8 * 24 * 3600 * 1000,
                ..upload
            }))
            .unwrap();
        assert_eq!(
            store.picture_distribution("00000BOB", true).unwrap(),
            Distribution::Set(None)
        );
        assert_eq!(store.contact_picture_version("00000BOB").unwrap(), 0);
        store.set_contact_picture("00000BOB", Some(5)).unwrap();
        assert_eq!(store.contact_picture_version("00000BOB").unwrap(), 5);
    }
}
