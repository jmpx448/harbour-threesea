// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Safe backup JSON, version 1.
//!
//! Spec: `third_party/libthreema/threema-protocols/src/safe-backup.schema.yml`.
//! Writer/reader behaviour mirrors upstream `ThreemaSafeServiceImpl.java` (6.5.3-1216): optional
//! fields are omitted when absent, readers tolerate missing/unknown fields (other clients may
//! write more), and defaults match Android's.

use serde::{Deserialize, Serialize};

/// Format version we write and the highest we read.
pub const VERSION: u32 = 1;

/// The whole backup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub info: Info,
    pub user: User,
    #[serde(default)]
    pub contacts: Option<Vec<Contact>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<Group>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distributionlists: Option<Vec<DistributionList>>,
    #[serde(default)]
    pub settings: Settings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Info {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    /// Base64 of the 32-byte client key.
    pub privatekey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    /// Base64 JPEG (q=60, ≤ 400×400).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_pic: Option<String>,
    /// `[null]` = nobody, `["*"]` = everyone, else an allow list of IDs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_pic_release: Option<Vec<Option<String>>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    /// `mobile` (E.164 without `+`) or `email`.
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub identity: String,
    /// Base64; only written for fully verified (2) or revoked contacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publickey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    /// 0 unverified, 1 server verified, 2 fully verified.
    pub verification: u8,
    #[serde(default)]
    pub work_verified: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firstname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lastname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update: Option<i64>,
    #[serde(default, rename = "private")]
    pub is_private: bool,
    /// 0 default, 1 send, 2 don't send.
    #[serde(default)]
    pub read_receipts: u8,
    #[serde(default)]
    pub typing_indicators: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    /// 8 bytes, lowercase hex.
    pub id: String,
    pub creator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groupname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    pub members: Vec<String>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update: Option<i64>,
    #[serde(default, rename = "private")]
    pub is_private: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistributionList {
    /// Hex of the 8-byte little-endian local id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    pub members: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update: Option<i64>,
    #[serde(default, rename = "private")]
    pub is_private: bool,
}

const fn yes() -> bool {
    true
}

/// App settings; read defaults are Android's.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub sync_contacts: bool,
    #[serde(default)]
    pub block_unknown: bool,
    #[serde(default = "yes")]
    pub read_receipts: bool,
    #[serde(default = "yes")]
    pub send_typing: bool,
    #[serde(default = "yes")]
    pub threema_calls: bool,
    #[serde(default)]
    pub relay_threema_calls: bool,
    #[serde(default)]
    pub disable_screenshots: bool,
    #[serde(default)]
    pub incognito_keyboard: bool,
    #[serde(default)]
    pub blocked_contacts: Vec<String>,
    #[serde(default)]
    pub sync_excluded_ids: Vec<String>,
    #[serde(default)]
    pub recent_emojis: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sync_contacts: false,
            block_unknown: false,
            read_receipts: true,
            send_typing: true,
            threema_calls: true,
            relay_threema_calls: false,
            disable_screenshots: false,
            incognito_keyboard: false,
            blocked_contacts: Vec::new(),
            sync_excluded_ids: Vec::new(),
            recent_emojis: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_android_style_backup_and_tolerates_unknown_fields() {
        let json = r#"{
            "info": {"version": 1, "device": "6.5.3A/de_CH", "futureField": 1},
            "user": {"privatekey": "ZWNob2VjaG9lY2hvZWNob2VjaG9lY2hvZWNob2VjaG8=",
                     "nickname": "julia", "profilePicRelease": [null],
                     "links": [{"type": "mobile", "value": "41791234567"}]},
            "contacts": [{"identity": "ECHOECHO", "verification": 2,
                          "publickey": "AAAA", "private": true, "createdAt": 1}],
            "groups": [{"id": "0011223344556677", "creator": "ECHOECHO",
                        "members": ["ECHOECHO"], "deleted": false}],
            "distributionlists": [{"name": "Friends", "members": []}],
            "settings": {"syncContacts": true}
        }"#;
        let backup: Backup = serde_json::from_str(json).unwrap();
        assert_eq!(backup.user.nickname.as_deref(), Some("julia"));
        assert_eq!(backup.user.profile_pic_release, Some(vec![None]));
        let contacts = backup.contacts.unwrap();
        assert!(contacts[0].is_private);
        assert_eq!(contacts[0].verification, 2);
        assert!(backup.settings.sync_contacts);
        assert!(backup.settings.read_receipts, "Android default");
    }

    #[test]
    fn writes_required_keys_and_omits_absent_optionals() {
        let backup = Backup {
            info: Info {
                version: VERSION,
                device: Some("0.1.0S/en_US".into()),
            },
            user: User {
                privatekey: "x".into(),
                nickname: None,
                profile_pic: None,
                profile_pic_release: None,
                links: Vec::new(),
            },
            contacts: Some(Vec::new()),
            groups: Some(Vec::new()),
            distributionlists: Some(Vec::new()),
            settings: Settings::default(),
        };
        let value = serde_json::to_value(&backup).unwrap();
        assert_eq!(value["info"]["version"], 1);
        assert!(value["user"].get("nickname").is_none());
        assert_eq!(value["settings"]["syncContacts"], false);
        assert_eq!(value["contacts"], serde_json::json!([]));
    }
}
