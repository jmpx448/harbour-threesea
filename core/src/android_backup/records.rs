// SPDX-License-Identifier: AGPL-3.0-only
//! Typed records from the archive's main CSVs, read like Android reads them back
//! (`RestoreService.createContactModel` / `createGroupModel` / `createDistributionListModel`):
//! by header name, with the same version gates. Malformed rows are counted, never guessed.

use zeroize::Zeroize as _;

use super::csv::Table;

/// `contacts.csv`: `publickey` is lowercase hex (not base64 like Threema Safe), `verification`
/// the enum name, `hidden` the group-only acquaintance level.
pub struct Contact {
    pub identity: String,
    pub public_key: [u8; 32],
    pub verification: i64,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub nickname: Option<String>,
    pub hidden: bool,
    pub archived: bool,
    /// `identity_id` (version 19+): the archive-local alias picture and message file names use.
    pub alias: Option<String>,
}

/// All valid contacts and how many rows were unreadable.
pub fn contacts(table: &Table) -> (Vec<Contact>, usize) {
    let mut list = Vec::new();
    let mut invalid = 0;
    for row in &table.rows {
        let parsed = (|| {
            let identity =
                crate::app_safe::normalize_identity(table.get(row, "identity").unwrap_or(""))?;
            let public_key: [u8; 32] = data_encoding::HEXLOWER_PERMISSIVE
                .decode(table.get(row, "publickey").unwrap_or("").as_bytes())
                .ok()?
                .try_into()
                .ok()?;
            let text = |column: &str| {
                table
                    .get(row, column)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            };
            Some(Contact {
                identity,
                public_key,
                verification: match table.get(row, "verification") {
                    Some("FULLY_VERIFIED") => 2,
                    Some("SERVER_VERIFIED") => 1,
                    _ => 0,
                },
                first_name: text("firstname"),
                last_name: text("lastname"),
                nickname: text("nick_name"),
                hidden: table.get_bool(row, "hidden"),
                archived: table.get_bool(row, "archived"),
                alias: text("identity_id"),
            })
        })();
        match parsed {
            Some(contact) => list.push(contact),
            None => invalid += 1,
        }
    }
    (list, invalid)
}

/// `groups.csv`. The group identity is the creator plus the 8-byte API id (lowercase hex of
/// the little-endian bytes, like Threema Safe); `group_uid` is the archive-local alias
/// (version 19+; older backups name avatar/message files `<api id>-<creator>`,
/// `BackupUtils.buildGroupUid`).
pub struct Group {
    pub group_id: u64,
    pub creator: String,
    pub name: String,
    pub created_at: i64,
    pub members: Vec<String>,
    pub archived: bool,
    /// The legacy `deleted` flag (not written since format 27; such groups are skipped).
    pub deleted: bool,
    /// `user_state` (version 25+): 0 member, 1 kicked, 2 left. Older backups derive it from
    /// the member list.
    pub user_state: Option<i64>,
    pub uid: Option<String>,
}

impl Group {
    /// The names this group's avatar and message files carry in the archive.
    pub fn aliases(&self) -> Vec<String> {
        match &self.uid {
            Some(uid) => vec![uid.clone()],
            None => vec![format!(
                "{}-{}",
                crate::app_safe::safe_id(self.group_id),
                self.creator
            )],
        }
    }
}

pub fn groups(table: &Table) -> (Vec<Group>, usize) {
    let mut list = Vec::new();
    let mut invalid = 0;
    for row in &table.rows {
        let parsed = (|| {
            let creator =
                crate::app_safe::normalize_identity(table.get(row, "creator").unwrap_or(""))?;
            let group_id = crate::app_safe::parse_safe_id(table.get(row, "id").unwrap_or(""))?;
            Some(Group {
                group_id,
                creator,
                name: table.get(row, "groupname").unwrap_or("").to_owned(),
                // Android clamps the negative timestamps of an old bug to 0.
                created_at: table.get_ms(row, "created_at").unwrap_or(0).max(0),
                members: table
                    .get_list(row, "members")
                    .iter()
                    .filter_map(|member| crate::app_safe::normalize_identity(member))
                    .collect(),
                archived: table.get_bool(row, "archived"),
                deleted: table.has_column("deleted") && table.get_bool(row, "deleted"),
                user_state: table
                    .has_column("user_state")
                    .then(|| table.get_ms(row, "user_state"))
                    .flatten(),
                uid: table
                    .get(row, "group_uid")
                    .filter(|uid| !uid.is_empty())
                    .map(str::to_owned),
            })
        })();
        match parsed {
            Some(group) => list.push(group),
            None => invalid += 1,
        }
    }
    (list, invalid)
}

/// `distribution_list.csv`: `id` is Android's local row id (a plain number).
pub struct DistributionList {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub members: Vec<String>,
    pub archived: bool,
}

pub fn distribution_lists(table: &Table) -> (Vec<DistributionList>, usize) {
    let mut list = Vec::new();
    let mut invalid = 0;
    for row in &table.rows {
        let parsed = (|| {
            Some(DistributionList {
                id: table.get(row, "id")?.parse().ok()?,
                name: table
                    .get(row, "distribution_list_name")
                    .unwrap_or("")
                    .to_owned(),
                created_at: table.get_ms(row, "created_at").unwrap_or(0).max(0),
                members: table
                    .get_list(row, "distribution_members")
                    .iter()
                    .filter_map(|member| crate::app_safe::normalize_identity(member))
                    .collect(),
                archived: table.get_bool(row, "archived"),
            })
        })();
        match parsed {
            Some(entry) => list.push(entry),
            None => invalid += 1,
        }
    }
    (list, invalid)
}

/// `messagestae` names that mean an outgoing message never made it out: imported as failed,
/// never into the sending queue (`RestoreService.setMessageState` + handoff rule).
const UNSENT_STATES: [&str; 6] = [
    "PENDING",
    "TRANSCODING",
    "UPLOADING",
    "SENDING",
    "SENDFAILED",
    "FS_KEY_MISMATCH",
];

/// One row of a `message_*`/`group_message_*`/`distribution_list_message_*` CSV, read like
/// `RestoreService.fillMessageModel`.
pub struct Message {
    pub uid: String,
    /// `apiid` (Threema's wire-order hex), absent or unreadable for local-only rows.
    pub api_id: Option<u64>,
    pub outgoing: bool,
    pub read: bool,
    /// Raw `messagestae` name.
    pub state: String,
    /// The sender column (group and list files).
    pub sender: Option<String>,
    /// Raw `type` name (TEXT, FILE, deprecated IMAGE, …).
    pub kind: String,
    pub body: String,
    pub is_status: bool,
    pub caption: Option<String>,
    pub quoted_api_id: Option<u64>,
    /// Display time: the remote `posted_at` where known, else the local `created_at`.
    pub at: i64,
    pub modified_at: Option<i64>,
    pub delivered_at: Option<i64>,
    pub read_at: Option<i64>,
    pub edited_at: Option<i64>,
    pub deleted_at: Option<i64>,
    pub starred: bool,
    /// `g_msg_states`: member → USERACK/USERDEC (group thumbs before reactions existed).
    pub group_states: Vec<(String, String)>,
}

/// The body of a file message holds the blob's key as hex; text bodies are private too. The
/// text goes on to the store from here, but this copy doesn't linger.
impl Drop for Message {
    fn drop(&mut self) {
        self.body.zeroize();
        self.caption.zeroize();
    }
}

impl Message {
    pub fn unsent(&self) -> bool {
        UNSENT_STATES.contains(&self.state.as_str())
    }
}

/// Threema's message-id strings (`MessageId.toString`, also `apiid` and quote ids in backups):
/// 16 hex digits of the 8 bytes in wire order.
pub fn parse_api_id(hex: &str) -> Option<u64> {
    let bytes: [u8; 8] = data_encoding::HEXLOWER_PERMISSIVE
        .decode(hex.as_bytes())
        .ok()?
        .try_into()
        .ok()?;
    Some(u64::from_le_bytes(bytes))
}

pub fn messages(table: &Table) -> (Vec<Message>, usize) {
    let mut list = Vec::new();
    let mut invalid = 0;
    for row in &table.rows {
        let parsed = (|| {
            let uid = table.get(row, "uid")?.to_owned();
            if uid.is_empty() {
                return None;
            }
            let posted = table.get_ms(row, "posted_at");
            let created = table.get_ms(row, "created_at");
            let text = |column: &str| {
                table
                    .get(row, column)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            };
            Some(Message {
                api_id: table.get(row, "apiid").and_then(parse_api_id),
                outgoing: table.get_bool(row, "isoutbox"),
                read: table.get_bool(row, "isread"),
                state: table.get(row, "messagestae").unwrap_or("").to_owned(),
                sender: text("identity"),
                kind: table.get(row, "type").unwrap_or("").to_owned(),
                body: table.get(row, "body").unwrap_or("").to_owned(),
                is_status: table.get_bool(row, "isstatusmessage"),
                caption: text("caption"),
                quoted_api_id: table
                    .get(row, "quoted_message_apiid")
                    .and_then(parse_api_id),
                at: posted.or(created).unwrap_or(0).max(0),
                modified_at: table.get_ms(row, "modified_at"),
                delivered_at: table.get_ms(row, "delivered_at"),
                read_at: table.get_ms(row, "read_at"),
                edited_at: table.get_ms(row, "edited_at"),
                deleted_at: table.get_ms(row, "deleted_at"),
                // DisplayTag.DISPLAY_TAG_STARRED is bit 1.
                starred: table
                    .get(row, "display_tags")
                    .and_then(|tags| tags.parse::<i64>().ok())
                    .is_some_and(|tags| tags & 1 != 0),
                group_states: table
                    .get(row, "g_msg_states")
                    .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                    .and_then(|value| {
                        value.as_object().map(|map| {
                            map.iter()
                                .filter_map(|(member, state)| {
                                    Some((member.clone(), state.as_str()?.to_owned()))
                                })
                                .collect()
                        })
                    })
                    .unwrap_or_default(),
                uid,
            })
        })();
        match parsed {
            Some(message) => list.push(message),
            None => invalid += 1,
        }
    }
    (list, invalid)
}

/// A row of `contact_reactions.csv` / `group_reactions.csv` (format 26+).
pub struct Reaction {
    /// The conversation: a contact identity, or the group's (creator, api id).
    pub contact: Option<String>,
    pub group: Option<(String, u64)>,
    pub api_message_id: u64,
    pub sender: String,
    pub emoji: String,
    pub reacted_at: i64,
}

pub fn reactions(table: &Table, group: bool) -> (Vec<Reaction>, usize) {
    let mut list = Vec::new();
    let mut invalid = 0;
    for row in &table.rows {
        let parsed = (|| {
            Some(Reaction {
                contact: if group {
                    None
                } else {
                    Some(crate::app_safe::normalize_identity(
                        table.get(row, "identity").unwrap_or(""),
                    )?)
                },
                group: if group {
                    Some((
                        crate::app_safe::normalize_identity(
                            table.get(row, "group_creator_identity").unwrap_or(""),
                        )?,
                        crate::app_safe::parse_safe_id(
                            table.get(row, "api_group_id").unwrap_or(""),
                        )?,
                    ))
                } else {
                    None
                },
                api_message_id: parse_api_id(table.get(row, "api_message_id").unwrap_or(""))?,
                sender: crate::app_safe::normalize_identity(
                    table.get(row, "sender_identity").unwrap_or(""),
                )?,
                emoji: table
                    .get(row, "emoji_sequence")
                    .filter(|emoji| !emoji.is_empty() && emoji.len() <= 64)?
                    .to_owned(),
                reacted_at: table.get_ms(row, "reacted_at").unwrap_or(0).max(0),
            })
        })();
        match parsed {
            Some(reaction) => list.push(reaction),
            None => invalid += 1,
        }
    }
    (list, invalid)
}
