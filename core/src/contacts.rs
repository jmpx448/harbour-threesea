// SPDX-License-Identifier: AGPL-3.0-only
//! Contacts: storage of libthreema's [`Contact`] model, directory lookups, and the conversion
//! from/to Threema Safe backups.
//!
//! We store libthreema's own model (the multi-device sync model) so that Phase 2 can implement
//! libthreema's `ContactProvider` on top of this table. Lookups use `identity/fetch_bulk` like
//! libthreema's (crate-private) lookup subtask; upstream Android: `APIConnector.fetchIdentities`.

use data_encoding::BASE64;
use libthreema::{
    common::{FeatureMask, ThreemaId, config::Config, keys::PublicKey},
    https::HttpsResult,
    model::contact::{Contact, ContactInit},
    protobuf::d2d_sync::{
        ConversationCategory, ConversationVisibility, ReadReceiptPolicy, TypingIndicatorPolicy,
        contact as pc,
    },
};
use rusqlite::{OptionalExtension as _, Row, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    directory::{DIRECTORY_URL, DirectoryError},
    https::HttpsClient,
    safe::backup,
    store::{Store, StoreError},
};

/// Schema for contacts (store migration 2).
pub(crate) const MIGRATION: &str = "
    CREATE TABLE contacts (
        identity                 TEXT PRIMARY KEY NOT NULL,
        public_key               TEXT NOT NULL,
        created_at               INTEGER NOT NULL,
        first_name               TEXT,
        last_name                TEXT,
        nickname                 TEXT,
        verification_level       INTEGER NOT NULL,
        work_verification_level  INTEGER NOT NULL,
        identity_type            INTEGER NOT NULL,
        acquaintance_level       INTEGER NOT NULL,
        activity_state           INTEGER NOT NULL,
        feature_mask             INTEGER NOT NULL,
        sync_state               INTEGER NOT NULL,
        read_receipt_override    INTEGER,
        typing_indicator_override INTEGER,
        conversation_category    INTEGER NOT NULL,
        conversation_visibility  INTEGER NOT NULL
    );";

const COLUMNS: &str = "identity, public_key, created_at, first_name, last_name, nickname,
    verification_level, work_verification_level, identity_type, acquaintance_level,
    activity_state, feature_mask, sync_state, read_receipt_override, typing_indicator_override,
    conversation_category, conversation_visibility";

fn bad(column: usize) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnType(column, "contact".into(), rusqlite::types::Type::Integer)
}

fn enum_at<T: TryFrom<i32>>(row: &Row<'_>, column: usize) -> rusqlite::Result<T> {
    T::try_from(row.get::<_, i32>(column)?).map_err(|_| bad(column))
}

fn opt_enum_at<T: TryFrom<i32>>(row: &Row<'_>, column: usize) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<i32>>(column)?
        .map(|v| T::try_from(v).map_err(|_| bad(column)))
        .transpose()
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<Contact> {
    let identity: String = row.get(0)?;
    let public_key: String = row.get(1)?;
    Ok(Contact {
        identity: identity.parse().map_err(|_| bad(0))?,
        public_key: PublicKey::from(
            <[u8; PublicKey::LENGTH]>::try_from(hex_bytes(&public_key).as_slice())
                .map_err(|_| bad(1))?,
        ),
        created_at: u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
        first_name: row.get(3)?,
        last_name: row.get(4)?,
        nickname: row.get(5)?,
        verification_level: enum_at(row, 6)?,
        work_verification_level: enum_at(row, 7)?,
        identity_type: enum_at(row, 8)?,
        acquaintance_level: enum_at(row, 9)?,
        activity_state: enum_at(row, 10)?,
        feature_mask: FeatureMask(row.get::<_, i64>(11)?.cast_unsigned()),
        sync_state: enum_at(row, 12)?,
        work_last_full_sync_at: None,
        work_availability_status: None,
        read_receipt_policy_override: opt_enum_at(row, 13)?,
        typing_indicator_policy_override: opt_enum_at(row, 14)?,
        notification_trigger_policy_override: None,
        conversation_category: enum_at(row, 15)?,
        conversation_visibility: enum_at(row, 16)?,
    })
}

impl Store {
    /// Insert or replace a contact.
    pub fn upsert_contact(&self, c: &Contact) -> Result<(), StoreError> {
        self.conn.execute(
            &format!(
                "INSERT OR REPLACE INTO contacts ({COLUMNS}) VALUES
                      (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)"
            ),
            params![
                c.identity.to_string(),
                c.public_key.to_string(),
                i64::try_from(c.created_at).unwrap_or(i64::MAX),
                c.first_name,
                c.last_name,
                c.nickname,
                c.verification_level as i32,
                c.work_verification_level as i32,
                c.identity_type as i32,
                c.acquaintance_level as i32,
                c.activity_state as i32,
                c.feature_mask.0.cast_signed(),
                c.sync_state as i32,
                c.read_receipt_policy_override.map(|p| p as i32),
                c.typing_indicator_policy_override.map(|p| p as i32),
                c.conversation_category as i32,
                c.conversation_visibility as i32,
            ],
        )?;
        Ok(())
    }

    /// Whether the user put `identity` on the block list.
    pub fn is_explicitly_blocked(&self, identity: &str) -> Result<bool, StoreError> {
        let settings = self.setting(crate::store::keys::APP_SETTINGS)?;
        Ok(settings
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["blockedContacts"].as_array().cloned())
            .is_some_and(|blocked| blocked.iter().any(|id| id.as_str() == Some(identity))))
    }

    /// One contact.
    pub fn contact(&self, identity: &str) -> Result<Option<Contact>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM contacts WHERE identity = ?1"),
                [identity],
                from_row,
            )
            .optional()?)
    }

    /// All contacts, ordered by identity.
    pub fn contacts(&self) -> Result<Vec<Contact>, StoreError> {
        let mut statement = self
            .conn
            .prepare(&format!("SELECT {COLUMNS} FROM contacts ORDER BY identity"))?;
        let rows = statement.query_map([], from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Delete a contact.
    pub fn delete_contact(&self, identity: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM contacts WHERE identity = ?1", [identity])?;
        Ok(())
    }
}

/// Result of a directory lookup for one ID.
pub enum Lookup {
    /// Valid ID: a fresh contact (unverified, group-or-deleted acquaintance) to adjust and store.
    Found(Box<ContactInit>),
    /// Revoked or never existed.
    Invalid,
    /// A predefined contact with a key other than its known one: a wrong answer, not to be used.
    Refused,
}

/// Threema's own contacts have a known key ([`Config::predefined_contacts`]): another one is refused
/// (`false`), the right one counts as fully verified and brings the known nickname, like
/// libthreema's lookup and Android's `requireValidContact`. Anyone else stays as they are.
#[must_use]
pub fn pin_predefined(config: &Config, c: &mut ContactInit) -> bool {
    let Some(predefined) = config.predefined_contacts.get(&c.identity) else {
        return true;
    };
    if predefined.public_key != c.public_key {
        tracing::warn!("predefined contact with another key refused");
        return false;
    }
    c.verification_level = pc::VerificationLevel::FullyVerified;
    c.nickname = Some(predefined.nickname.clone());
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchedIdentity {
    identity: String,
    #[serde(rename = "type", default)]
    identity_type: u8,
    public_key: String,
    #[serde(default)]
    state: u8,
    #[serde(default)]
    feature_mask: u64,
}

fn parse_fetch_bulk(result: HttpsResult) -> Result<Vec<FetchedIdentity>, DirectoryError> {
    #[derive(Deserialize)]
    struct Response {
        identities: Vec<FetchedIdentity>,
    }
    let response = result?;
    match response.status {
        200 => Ok(serde_json::from_slice::<Response>(&response.body)?.identities),
        429 => Err(DirectoryError::Server(
            "Too many requests, please try again later.".into(),
        )),
        status => Err(DirectoryError::Status(status)),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// A new contact from a directory entry, defaults as in libthreema's `ValidIdentity` conversion.
fn contact_init(entry: &FetchedIdentity) -> Option<ContactInit> {
    let public_key: [u8; PublicKey::LENGTH] = BASE64
        .decode(entry.public_key.as_bytes())
        .ok()?
        .try_into()
        .ok()?;
    Some(ContactInit {
        identity: entry.identity.parse().ok()?,
        public_key: PublicKey::from(public_key),
        created_at: now_ms(),
        first_name: None,
        last_name: None,
        nickname: None,
        verification_level: pc::VerificationLevel::Unverified,
        work_verification_level: pc::WorkVerificationLevel::None,
        identity_type: if entry.identity_type == 1 {
            pc::IdentityType::Work
        } else {
            pc::IdentityType::Regular
        },
        acquaintance_level: pc::AcquaintanceLevel::GroupOrDeleted,
        activity_state: if entry.state == 1 {
            pc::ActivityState::Inactive
        } else {
            pc::ActivityState::Active
        },
        feature_mask: FeatureMask(entry.feature_mask),
        sync_state: pc::SyncState::Initial,
        work_last_full_sync_at: None,
        work_availability_status: None,
        read_receipt_policy_override: None,
        typing_indicator_policy_override: None,
        notification_trigger_policy_override: None,
        conversation_category: ConversationCategory::Default,
        conversation_visibility: ConversationVisibility::Normal,
    })
}

/// What the directory's answer says about `id`.
fn lookup_result(config: &Config, fetched: &[FetchedIdentity], id: &str) -> Lookup {
    let found = fetched
        .iter()
        .find(|f| f.identity == id)
        .and_then(contact_init);
    let Some(mut contact) = found else {
        return Lookup::Invalid;
    };
    if pin_predefined(config, &mut contact) {
        Lookup::Found(Box::new(contact))
    } else {
        Lookup::Refused
    }
}

/// `POST identity/fetch_bulk` for up to a few hundred IDs. IDs missing from the answer are
/// invalid (revoked or never existed).
pub async fn lookup(
    client: &HttpsClient,
    config: &Config,
    identities: &[String],
) -> Result<Vec<(String, Lookup)>, DirectoryError> {
    let mut results = Vec::with_capacity(identities.len());
    for chunk in identities.chunks(100) {
        let fetched = parse_fetch_bulk(
            client
                .post_json(
                    &format!("{DIRECTORY_URL}identity/fetch_bulk"),
                    &json!({ "identities": chunk }),
                )
                .await,
        )?;
        for id in chunk {
            results.push((id.clone(), lookup_result(config, &fetched, id)));
        }
    }
    Ok(results)
}

/// Display name like Threema: "first last", else nickname (shown with `~`), else the ID.
/// Settings → Appearance: names as "Last First" (Android's "View contact names"). Process-wide,
/// set when the setting is loaded or changed.
pub static LAST_NAME_FIRST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn display_name(c: &Contact) -> String {
    let (first, second) = if LAST_NAME_FIRST.load(std::sync::atomic::Ordering::Relaxed) {
        (c.last_name.as_deref(), c.first_name.as_deref())
    } else {
        (c.first_name.as_deref(), c.last_name.as_deref())
    };
    let full = [first, second]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !full.is_empty() {
        full
    } else if let Some(nick) = c.nickname.as_deref().filter(|n| !n.is_empty()) {
        format!("~{nick}")
    } else {
        c.identity.to_string()
    }
}

/// What the contact list sorts by (and takes its section letter from): first or last name first
/// (Android's "Sorting"), else the nickname or ID; lower case.
pub fn sort_key(c: &Contact, by_last_name: bool) -> String {
    let (first, second) = if by_last_name {
        (c.last_name.as_deref(), c.first_name.as_deref())
    } else {
        (c.first_name.as_deref(), c.last_name.as_deref())
    };
    let name = [first, second]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if name.is_empty() {
        display_name(c).trim_start_matches('~').to_lowercase()
    } else {
        name.to_lowercase()
    }
}

/// JSON for the UI.
pub fn to_json(c: &Contact) -> Value {
    json!({
        "identity": c.identity.to_string(),
        "displayName": display_name(c),
        "firstName": c.first_name,
        "lastName": c.last_name,
        "nickname": c.nickname,
        "verification": c.verification_level as i32,
        "work": c.identity_type == pc::IdentityType::Work,
        "workVerified": c.work_verification_level == pc::WorkVerificationLevel::WorkSubscriptionVerified,
        "inactive": c.activity_state == pc::ActivityState::Inactive,
        "invalid": c.activity_state == pc::ActivityState::Invalid,
        "hidden": c.acquaintance_level == pc::AcquaintanceLevel::GroupOrDeleted,
        // A real contact entry (a name set by the user, or from a backup / the address book), not
        // just someone known by their nickname.
        "saved": c.first_name.as_deref().is_some_and(|n| !n.is_empty())
            || c.last_name.as_deref().is_some_and(|n| !n.is_empty()),
        // Any emoji as a reaction; otherwise only thumbs up/down (legacy reactions).
        "reactions": c.feature_mask.0 & libthreema::common::FeatureMask::REACTION_SUPPORT != 0,
        "canEdit": c.feature_mask.0 & libthreema::common::FeatureMask::EDIT_MESSAGE_SUPPORT != 0,
        "canDelete": c.feature_mask.0 & libthreema::common::FeatureMask::DELETE_MESSAGE_SUPPORT != 0,
        "canPoll": c.feature_mask.0 & libthreema::common::FeatureMask::POLL_SUPPORT != 0,
        "canCall": c.feature_mask.0 & libthreema::common::FeatureMask::O2O_AUDIO_CALL_SUPPORT != 0
            && c.activity_state != pc::ActivityState::Invalid,
        "canVideoCall": c.feature_mask.0 & libthreema::common::FeatureMask::O2O_VIDEO_CALL_SUPPORT != 0
            && c.activity_state != pc::ActivityState::Invalid,
        // 32-byte key, shown as fingerprint-like groups in the UI.
        "publicKey": c.public_key.to_string(),
        // This contact's own choice (Android): 0 like the app setting, 1 send, 2 don't send.
        "readReceipts": match c.read_receipt_policy_override {
            None => 0,
            Some(ReadReceiptPolicy::SendReadReceipt) => 1,
            Some(ReadReceiptPolicy::DontSendReadReceipt) => 2,
        },
        "typing": match c.typing_indicator_policy_override {
            None => 0,
            Some(TypingIndicatorPolicy::SendTypingIndicator) => 1,
            Some(TypingIndicatorPolicy::DontSendTypingIndicator) => 2,
        },
    })
}

/// A contact's receipt/typing choice from the UI (0 default, 1 send, 2 don't send).
pub fn read_receipt_policy(choice: u8) -> Option<ReadReceiptPolicy> {
    match choice {
        1 => Some(ReadReceiptPolicy::SendReadReceipt),
        2 => Some(ReadReceiptPolicy::DontSendReadReceipt),
        _ => None,
    }
}

pub fn typing_policy(choice: u8) -> Option<TypingIndicatorPolicy> {
    match choice {
        1 => Some(TypingIndicatorPolicy::SendTypingIndicator),
        2 => Some(TypingIndicatorPolicy::DontSendTypingIndicator),
        _ => None,
    }
}

/// Apply a Threema Safe contact entry to a freshly looked-up contact (`ThreemaSafeServiceImpl`
/// restore): names, verification, hidden flag, creation time and receipt/typing overrides.
///
/// A contact verified in person keeps the key from the backup and its level, also if the directory
/// answers with another one: that is exactly what the verification is for. Anyone else starts out
/// unverified with the directory's key, since a backup carries no key to compare for them;
/// address book matching raises them again.
pub fn apply_backup(mut c: ContactInit, b: &backup::Contact) -> ContactInit {
    c.first_name = b.firstname.clone().filter(|s| !s.is_empty());
    c.last_name = b.lastname.clone().filter(|s| !s.is_empty());
    c.nickname = b.nickname.clone().filter(|s| !s.is_empty());
    let verified_key = (b.verification == 2)
        .then_some(b.publickey.as_deref())
        .flatten()
        .and_then(|k| BASE64.decode(k.as_bytes()).ok())
        .and_then(|k| <[u8; PublicKey::LENGTH]>::try_from(k).ok())
        .map(PublicKey::from);
    match verified_key {
        Some(key) => {
            if key != c.public_key {
                tracing::warn!("restored verified contact differs from the directory, key kept");
            }
            c.public_key = key;
            c.verification_level = pc::VerificationLevel::FullyVerified;
        }
        None => c.verification_level = pc::VerificationLevel::Unverified,
    }
    if b.work_verified {
        c.work_verification_level = pc::WorkVerificationLevel::WorkSubscriptionVerified;
    }
    c.acquaintance_level = if b.hidden {
        pc::AcquaintanceLevel::GroupOrDeleted
    } else {
        pc::AcquaintanceLevel::Direct
    };
    if let Some(created) = b.created_at.and_then(|t| u64::try_from(t).ok()) {
        c.created_at = created;
    }
    c.read_receipt_policy_override = match b.read_receipts {
        1 => Some(ReadReceiptPolicy::SendReadReceipt),
        2 => Some(ReadReceiptPolicy::DontSendReadReceipt),
        _ => None,
    };
    c.typing_indicator_policy_override = match b.typing_indicators {
        1 => Some(TypingIndicatorPolicy::SendTypingIndicator),
        2 => Some(TypingIndicatorPolicy::DontSendTypingIndicator),
        _ => None,
    };
    // "private" chats on Android = protected conversation category.
    if b.is_private {
        c.conversation_category = ConversationCategory::Protected;
    }
    c
}

pub(crate) fn hex_bytes(hex: &str) -> Vec<u8> {
    data_encoding::HEXLOWER_PERMISSIVE
        .decode(hex.as_bytes())
        .unwrap_or_default()
}

/// A stored contact as a Threema Safe entry (`ThreemaSafeServiceImpl` backup writer).
pub fn to_backup(c: &Contact) -> backup::Contact {
    let verification = match c.verification_level {
        pc::VerificationLevel::Unverified => 0,
        pc::VerificationLevel::ServerVerified => 1,
        pc::VerificationLevel::FullyVerified => 2,
    };
    backup::Contact {
        identity: c.identity.to_string(),
        // Android writes the key only for fully verified (or revoked) contacts.
        publickey: (verification == 2 || c.activity_state == pc::ActivityState::Invalid)
            .then(|| BASE64.encode(&hex_bytes(&c.public_key.to_string()))),
        created_at: i64::try_from(c.created_at).ok(),
        verification,
        work_verified: c.work_verification_level
            == pc::WorkVerificationLevel::WorkSubscriptionVerified,
        hidden: c.acquaintance_level == pc::AcquaintanceLevel::GroupOrDeleted,
        firstname: c.first_name.clone(),
        lastname: c.last_name.clone(),
        nickname: c.nickname.clone(),
        last_update: None,
        is_private: c.conversation_category == ConversationCategory::Protected,
        read_receipts: match c.read_receipt_policy_override {
            None => 0,
            Some(ReadReceiptPolicy::SendReadReceipt) => 1,
            Some(ReadReceiptPolicy::DontSendReadReceipt) => 2,
        },
        typing_indicators: match c.typing_indicator_policy_override {
            None => 0,
            Some(TypingIndicatorPolicy::SendTypingIndicator) => 1,
            Some(TypingIndicatorPolicy::DontSendTypingIndicator) => 2,
        },
    }
}

/// Is this a syntactically valid Threema ID? (Normalizes to upper case.)
pub fn parse_identity(input: &str) -> Option<ThreemaId> {
    crate::app_safe::normalize_identity(input)?.parse().ok()
}

/// A contact as the directory would return it (tests elsewhere in the crate, and the demo data
/// of the emulator build).
#[cfg(any(test, feature = "demo"))]
pub(crate) fn test_contact(id: &str) -> ContactInit {
    contact_init(&FetchedIdentity {
        identity: id.into(),
        identity_type: 0,
        public_key: BASE64.encode(&[9; 32]),
        state: 0,
        feature_mask: 255,
    })
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> FetchedIdentity {
        FetchedIdentity {
            identity: id.into(),
            identity_type: 0,
            public_key: BASE64.encode(&[9; 32]),
            state: 0,
            feature_mask: 255,
        }
    }

    #[test]
    fn sort_keys() {
        let mut c = contact_init(&entry("ECHOECHO")).unwrap();
        c.first_name = Some("Anna".into());
        c.last_name = Some("Zeller".into());
        assert_eq!(sort_key(&c, false), "anna zeller");
        assert_eq!(sort_key(&c, true), "zeller anna");
        c.first_name = None;
        c.last_name = None;
        c.nickname = Some("Echo".into());
        assert_eq!(sort_key(&c, true), "echo");
    }

    #[test]
    fn store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let mut c = contact_init(&entry("ECHOECHO")).unwrap();
        c.first_name = Some("Echo".into());
        c.read_receipt_policy_override = Some(ReadReceiptPolicy::DontSendReadReceipt);
        store.upsert_contact(&c).unwrap();
        let back = store.contact("ECHOECHO").unwrap().unwrap();
        assert_eq!(back.first_name.as_deref(), Some("Echo"));
        assert_eq!(back.public_key.to_string(), c.public_key.to_string());
        assert_eq!(back.feature_mask.0, 255);
        assert_eq!(
            back.read_receipt_policy_override,
            Some(ReadReceiptPolicy::DontSendReadReceipt)
        );
        assert_eq!(store.contacts().unwrap().len(), 1);
        store.delete_contact("ECHOECHO").unwrap();
        assert!(store.contact("ECHOECHO").unwrap().is_none());
    }

    #[test]
    fn safe_mapping_roundtrip_and_key_check() {
        let c = contact_init(&entry("ECHOECHO")).unwrap();
        let b = backup::Contact {
            identity: "ECHOECHO".into(),
            publickey: Some(BASE64.encode(&[9; 32])),
            created_at: Some(1000),
            verification: 2,
            work_verified: false,
            hidden: false,
            firstname: Some("Echo".into()),
            lastname: None,
            nickname: Some("".into()),
            last_update: None,
            is_private: false,
            read_receipts: 2,
            typing_indicators: 0,
        };
        let restored = apply_backup(c.clone(), &b);
        assert_eq!(
            restored.verification_level,
            pc::VerificationLevel::FullyVerified
        );
        assert_eq!(restored.acquaintance_level, pc::AcquaintanceLevel::Direct);
        assert_eq!(restored.nickname, None);
        assert_eq!(restored.created_at, 1000);
        let again = to_backup(&restored);
        assert_eq!(again.verification, 2);
        assert_eq!(again.publickey, b.publickey);
        assert_eq!(again.read_receipts, 2);

        // Level 2 with a different key: the backup's key, still verified, not the directory's.
        let mut other = b.clone();
        other.publickey = Some(BASE64.encode(&[8; 32]));
        let restored = apply_backup(c.clone(), &other);
        assert_eq!(
            restored.verification_level,
            pc::VerificationLevel::FullyVerified
        );
        assert_eq!(restored.public_key, PublicKey::from([8; 32]));

        // Level 2 without a usable key, and level 1: the directory's key, unverified.
        for (verification, key) in [
            (2, None),
            (2, Some(BASE64.encode(&[8; 31]))),
            (1, Some(BASE64.encode(&[8; 32]))),
            (1, None),
        ] {
            let mut other = b.clone();
            other.verification = verification;
            other.publickey = key;
            let restored = apply_backup(c.clone(), &other);
            assert_eq!(
                restored.verification_level,
                pc::VerificationLevel::Unverified
            );
            assert_eq!(restored.public_key, c.public_key);
        }
    }

    #[test]
    fn predefined_contacts_are_pinned() {
        let config = Config::from(libthreema::common::config::ConfigEnvironment::Production);
        let id = ThreemaId::try_from("*3MATOKN").unwrap();
        let known = config.predefined_contacts.get(&id).unwrap();

        // What the directory says about it decides nothing but whether it is used.
        let answer = |key: &[u8]| FetchedIdentity {
            public_key: BASE64.encode(key),
            ..entry("*3MATOKN")
        };
        let right = hex_bytes(&known.public_key.to_string());
        let Lookup::Found(contact) = lookup_result(&config, &[answer(&right)], "*3MATOKN") else {
            panic!("the right key is taken");
        };
        assert_eq!(
            contact.verification_level,
            pc::VerificationLevel::FullyVerified
        );
        assert_eq!(contact.nickname.as_deref(), Some(known.nickname.as_str()));
        assert!(matches!(
            lookup_result(&config, &[answer(&[9; 32])], "*3MATOKN"),
            Lookup::Refused
        ));
        assert!(matches!(
            lookup_result(&config, &[], "*3MATOKN"),
            Lookup::Invalid
        ));

        // Restored from a backup, as unverified or with another key: the same.
        let mut restored = *contact;
        restored.verification_level = pc::VerificationLevel::Unverified;
        assert!(pin_predefined(&config, &mut restored));
        assert_eq!(
            restored.verification_level,
            pc::VerificationLevel::FullyVerified
        );
        restored.public_key = PublicKey::from([9; 32]);
        assert!(!pin_predefined(&config, &mut restored));

        // Everyone else stays as they are.
        let mut other = contact_init(&entry("ABCD1234")).unwrap();
        assert!(pin_predefined(&config, &mut other));
        assert_eq!(other.verification_level, pc::VerificationLevel::Unverified);
    }

    #[test]
    fn display_names() {
        let mut c = contact_init(&entry("ECHOECHO")).unwrap();
        assert_eq!(display_name(&c), "ECHOECHO");
        c.nickname = Some("echo".into());
        assert_eq!(display_name(&c), "~echo");
        c.first_name = Some("Eve".into());
        c.last_name = Some("Echo".into());
        assert_eq!(display_name(&c), "Eve Echo");
    }

    #[test]
    fn fetch_bulk_parsing() {
        let body = br#"{"identities":[{"identity":"ECHOECHO","type":1,"publicKey":"CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk=","state":1,"featureMask":3}]}"#;
        let parsed = parse_fetch_bulk(Ok(libthreema::https::HttpsResponse {
            status: 200,
            body: body.to_vec(),
        }))
        .unwrap();
        let c = contact_init(&parsed[0]).unwrap();
        assert_eq!(c.identity_type, pc::IdentityType::Work);
        assert_eq!(c.activity_state, pc::ActivityState::Inactive);
    }
}

/// Live test against the real directory; run with `cargo test -- --ignored`.
#[cfg(test)]
mod live_tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "contacts the real Threema directory server"]
    async fn echoecho_exists_and_bogus_id_is_invalid() {
        let client = HttpsClient::new().unwrap();
        let config = Config::from(libthreema::common::config::ConfigEnvironment::Production);
        let results = lookup(&client, &config, &["ECHOECHO".into(), "ZZZZZZZ9".into()])
            .await
            .unwrap();
        let Lookup::Found(echo) = &results[0].1 else {
            panic!("ECHOECHO not found")
        };
        eprintln!("ECHOECHO: {}", to_json(echo));
        assert!(matches!(results[1].1, Lookup::Invalid));
    }
}
