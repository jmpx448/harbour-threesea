// SPDX-License-Identifier: AGPL-3.0-only
//! Groups: storage, the receive steps of csp.struct.yml (_Common Group Receive Steps_ and those of
//! each control message), and what has to be sent in reply.
//!
//! - A group's conversation key is `g:<CREATOR>:<group id, 16 hex digits>`; Threema IDs never
//!   contain `:`, so keys can't collide with 1:1 conversations.
//! - `members` are all participants **except the user**, including the creator unless the user is
//!   the creator. They are exactly the receivers of everything we send to the group.
//! - Group messages are sent to every member separately (same message id); `pending_receivers`
//!   holds who still has to get a message (or outbox entry), so it counts as sent once the server
//!   acknowledged it for all of them.

use std::collections::BTreeSet;

use libthreema::{
    common::{GroupIdentity, ThreemaId},
    model::message::{GroupMessageBody, IncomingGroupMessageBody},
    protobuf::d2d_sync::contact::ActivityState,
};
use rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::{
    messages::{Message, now_ms},
    reactions,
    store::{Store, StoreError},
};

/// Schema (store migration 8).
pub(crate) const MIGRATION: &str = "
    ALTER TABLE messages ADD COLUMN sender TEXT;
    CREATE TABLE groups (
        key         TEXT PRIMARY KEY NOT NULL,
        creator     TEXT NOT NULL,
        group_id    INTEGER NOT NULL,
        name        TEXT NOT NULL DEFAULT '',
        members     TEXT NOT NULL,
        left_group  INTEGER NOT NULL DEFAULT 0,
        created_at  INTEGER NOT NULL
    );
    CREATE TABLE pending_receivers (
        conversation  TEXT NOT NULL,
        message_id    INTEGER NOT NULL,
        receiver      TEXT NOT NULL,
        PRIMARY KEY (conversation, message_id, receiver)
    ) WITHOUT ROWID;
    CREATE TABLE group_resyncs (
        key   TEXT PRIMARY KEY NOT NULL,
        at    INTEGER NOT NULL
    ) WITHOUT ROWID;";

/// Group pictures (store migration 9): the blob we got or sent (`{blob, key, size}`, hex), and a
/// local version that changes whenever the picture on the device changes (0: none).
pub(crate) const MIGRATION_PICTURES: &str = "
    ALTER TABLE groups ADD COLUMN picture TEXT;
    ALTER TABLE groups ADD COLUMN picture_version INTEGER NOT NULL DEFAULT 0;";

/// Store migration 35. `kicked`: of the groups we are out of (`left_group`), the ones the creator
/// removed us from; those from before it can't be told apart and stay "left". And the creator back
/// into the groups an older version's group sync took them out of: without them, everything they
/// wrote was dropped as coming from a stranger, and nothing we wrote reached them. Empty lists
/// are how those syncs left a group they created by mistake; it is someone else's group, so it
/// isn't the notes group it looked like. And what the other devices
/// are still owed of a group change that needs the network first, kept so that a crash or a failed
/// lookup or upload can't lose it: `group_member_lookups`, members who have no contact yet
/// (`added`: they came with this change, the others get to see that), and `group_picture_syncs`,
/// groups whose new picture isn't on the mirror yet. The setting: contacts looked up for groups
/// used to reach no other device, see `sync::reflect_group_contacts_once`.
pub(crate) const MIGRATION_SYNC: &str = r#"
    ALTER TABLE groups ADD COLUMN kicked INTEGER NOT NULL DEFAULT 0;
    UPDATE groups
       SET members = CASE WHEN members = '[]' THEN '["' || creator || '"]'
                          ELSE '["' || creator || '",' || substr(members, 2) END
     WHERE left_group = 0
       AND creator != (SELECT threema_id FROM identity WHERE id = 1)
       AND NOT EXISTS (SELECT 1 FROM json_each(groups.members) AS m WHERE m.value = groups.creator);
    CREATE TABLE group_member_lookups (
        key       TEXT NOT NULL,
        identity  TEXT NOT NULL,
        added     INTEGER NOT NULL,
        PRIMARY KEY (key, identity)
    ) WITHOUT ROWID;
    CREATE TABLE group_picture_syncs (
        key  TEXT PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;
    INSERT OR REPLACE INTO settings (key, value) VALUES ('group_contacts_to_reflect', '1');"#;

/// _Recently resynced_ lasts an hour (both directions).
const RESYNC_INTERVAL_MS: i64 = 60 * 60 * 1000;

/// Outbox kinds of group control messages (`body` in brackets).
/// Android's `MAX_GROUP_SIZE`. A setup listing more is no group anyone can make, and each ID in it
/// would be one more for the directory.
const SETUP_MEMBERS_MAX: usize = 256;
/// How many IDs group lookups may ask the directory for within an hour: two groups of the largest
/// size. Anyone can name any ID in a setup, sync request or group, and a contact can do so again
/// and again; Threema tolerates clients that cause it no load. What is over waits for the next
/// round, which it isn't lost for: the members stay noted, and libthreema looks up who writes.
const MEMBER_LOOKUPS_PER_HOUR: usize = 2 * SETUP_MEMBERS_MAX;
const MEMBER_LOOKUP_WINDOW_MS: i64 = 60 * 60 * 1000;
/// How long an ID stays out of groups after the directory said it doesn't exist. They don't come
/// back, but one may yet be created.
const INVALID_ID_MEMORY_MS: i64 = 24 * 60 * 60 * 1000;
/// How many group sync requests wait at once at most; they normally leave with the next ack.
const SYNC_REQUESTS_QUEUED_MAX: i64 = 64;
pub const OUTBOX_GROUP_SETUP: &str = "groupSetup"; // {members: [id]}
pub const OUTBOX_GROUP_NAME: &str = "groupName"; // {name}
pub const OUTBOX_GROUP_LEAVE: &str = "groupLeave";
pub const OUTBOX_GROUP_SYNC_REQUEST: &str = "groupSyncRequest";
pub const OUTBOX_GROUP_DELETE_PICTURE: &str = "groupDeleteProfilePicture";
pub const OUTBOX_GROUP_SET_PICTURE: &str = "groupSetProfilePicture"; // {blob, key, size}
// {gck (hex), url, version, startedAt (ms, a repeat to someone new: when the call started)}
pub const OUTBOX_GROUP_CALL_START: &str = "groupCallStart";

/// A group picture on the blob server (persisted), as sent in `set-profile-picture`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupPicture {
    pub blob_id: libthreema::common::BlobId,
    pub key: [u8; 32],
    pub size: u32,
}

impl GroupPicture {
    pub fn to_json(&self) -> Value {
        json!({
            "blob": data_encoding::HEXLOWER.encode(&self.blob_id.0),
            "key": data_encoding::HEXLOWER.encode(&self.key),
            "size": self.size,
        })
    }

    pub fn from_json(value: &Value) -> Option<Self> {
        let hex = |field: &str| {
            data_encoding::HEXLOWER
                .decode(value[field].as_str()?.as_bytes())
                .ok()
        };
        Some(Self {
            blob_id: libthreema::common::BlobId(hex("blob")?.try_into().ok()?),
            key: hex("key")?.try_into().ok()?,
            size: u32::try_from(value["size"].as_u64()?).ok()?,
        })
    }
}

/// `members` as we keep them (see the module docs), from any list of identities: no duplicates,
/// not the user, the creator first unless that is the user. The sync protocol's lists leave the
/// creator out.
pub(crate) fn normalized_members(
    listed: impl IntoIterator<Item = String>,
    creator: &str,
    user: &str,
) -> Vec<String> {
    let mut members = Vec::new();
    if creator != user {
        members.push(creator.to_owned());
    }
    for member in listed {
        if member != user && !members.contains(&member) && member != creator {
            members.push(member);
        }
    }
    members
}

/// A stored group.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub key: String,
    pub identity: GroupIdentity,
    pub name: String,
    /// Everyone but the user (see the module docs).
    pub members: Vec<String>,
    /// Left, removed (kicked) or disbanded; [`Store::group_kicked`] tells whether we were removed.
    pub left: bool,
    pub created_at: i64,
    pub picture: Option<GroupPicture>,
    /// Changes when the picture on the device changes; 0: none on the device.
    pub picture_version: i64,
}

/// `g:<CREATOR>:<group id hex>`.
pub fn key(identity: &GroupIdentity) -> String {
    format!("g:{}:{:016x}", identity.creator_identity, identity.group_id)
}

/// The group identity of a conversation key, if it is a group's.
pub fn parse_key(key: &str) -> Option<GroupIdentity> {
    let rest = key.strip_prefix("g:")?;
    let (creator, id) = rest.split_once(':')?;
    if id.len() != 16 {
        return None;
    }
    Some(GroupIdentity {
        creator_identity: ThreemaId::try_from(creator).ok()?,
        group_id: u64::from_str_radix(id, 16).ok()?,
    })
}

impl Group {
    /// `members` as the sync protocol wants them: neither the creator nor the user.
    pub(crate) fn sync_members(&self, user: &str) -> Vec<String> {
        let creator = self.identity.creator_identity.as_str();
        self.members
            .iter()
            .filter(|member| *member != creator && *member != user)
            .cloned()
            .collect()
    }

    /// A notes group: Threema's group of one, read by our own devices alone (`members` never holds
    /// us). Android's `isNotesGroup`.
    pub fn is_notes(&self) -> bool {
        self.members.is_empty() && !self.left
    }

    pub fn to_json(&self, user: &str, display_name: impl Fn(&str) -> String) -> Value {
        let creator = self.identity.creator_identity.to_string();
        json!({
            "key": self.key,
            "name": self.name,
            "creator": creator,
            "isCreator": creator == user,
            "left": self.left,
            "isNotes": self.is_notes(),
            "members": self.members.iter().map(|m| json!({
                "identity": m,
                "displayName": display_name(m),
                "creator": *m == creator,
            })).collect::<Vec<_>>(),
            "createdAt": self.created_at,
            "pictureVersion": self.picture_version,
        })
    }
}

type GroupRow = (
    String,
    String,
    i64,
    String,
    String,
    bool,
    i64,
    Option<String>,
    i64,
);

fn row_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<GroupRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn to_group(raw: GroupRow) -> Option<Group> {
    let (key, creator, group_id, name, members, left, created_at, picture, picture_version) = raw;
    Some(Group {
        key,
        identity: GroupIdentity {
            creator_identity: ThreemaId::try_from(creator.as_str()).ok()?,
            group_id: group_id.cast_unsigned(),
        },
        name,
        members: serde_json::from_str(&members).unwrap_or_default(),
        left,
        created_at,
        picture: picture
            .and_then(|p| serde_json::from_str(&p).ok())
            .and_then(|p| GroupPicture::from_json(&p)),
        picture_version,
    })
}

const GROUP_COLUMNS: &str =
    "key, creator, group_id, name, members, left_group, created_at, picture, picture_version";

impl Store {
    pub fn group(&self, key: &str) -> Result<Option<Group>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {GROUP_COLUMNS} FROM groups WHERE key = ?1"),
                [key],
                row_group,
            )
            .optional()?
            .and_then(to_group))
    }

    pub fn groups(&self) -> Result<Vec<Group>, StoreError> {
        let mut statement = self
            .conn
            .prepare(&format!("SELECT {GROUP_COLUMNS} FROM groups ORDER BY name"))?;
        let rows = statement.query_map([], row_group)?;
        Ok(rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter_map(to_group)
            .collect())
    }

    /// Whether `identity` is a member of a group the user is still in. Android lets such a
    /// person write even while "block unknown senders" is on, and libthreema asks us for it
    /// (`ContactProvider::is_member_of_active_group`).
    pub fn is_group_member(&self, identity: &str) -> Result<bool, StoreError> {
        // The members are a JSON array in one column: the LIKE narrows the rows, the parsed
        // list decides (a substring could sit inside another ID).
        let mut statement = self.conn.prepare(
            "SELECT members FROM groups WHERE left_group = 0 AND members LIKE '%' || ?1 || '%'",
        )?;
        let rows = statement.query_map([identity], |row| row.get::<_, String>(0))?;
        for members in rows {
            if serde_json::from_str::<Vec<String>>(&members?)
                .unwrap_or_default()
                .iter()
                .any(|member| member == identity)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether any group names `identity` as creator or member, a group we left too: such a group
    /// still goes to the other devices, which need a contact for everyone it names.
    pub fn is_named_by_a_group(&self, identity: &str) -> Result<bool, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT creator, members FROM groups WHERE creator = ?1 OR members LIKE '%' || ?1 || '%'",
        )?;
        let rows = statement.query_map([identity], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (creator, members) = row?;
            if creator == identity
                || serde_json::from_str::<Vec<String>>(&members)
                    .unwrap_or_default()
                    .iter()
                    .any(|member| member == identity)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Groups whose picture descriptor is known but whose image never made it onto the device
    /// (version 0): their downloads are tried again on reconnect and by maintenance.
    pub fn groups_with_pending_picture(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT key FROM groups WHERE picture IS NOT NULL AND picture_version = 0")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn upsert_group(&self, group: &Group) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO groups (key, creator, group_id, name, members, left_group, created_at,
                                 picture, picture_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (key) DO UPDATE SET name = excluded.name, members = excluded.members,
                 left_group = excluded.left_group, picture = excluded.picture,
                 picture_version = excluded.picture_version,
                 kicked = CASE WHEN excluded.left_group = 0 THEN 0 ELSE kicked END",
            params![
                group.key,
                group.identity.creator_identity.to_string(),
                group.identity.group_id.cast_signed(),
                group.name,
                serde_json::to_string(&group.members).unwrap_or_else(|_| "[]".into()),
                group.left,
                group.created_at,
                group.picture.as_ref().map(|p| p.to_json().to_string()),
                group.picture_version
            ],
        )?;
        Ok(())
    }

    /// Whether the creator removed us from this group (as opposed to our leaving it).
    pub fn group_kicked(&self, key: &str) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT kicked FROM groups WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?
            .unwrap_or(false))
    }

    /// See [`Self::group_kicked`]. Being back in the group clears it (`upsert_group`).
    pub fn set_group_kicked(&self, key: &str, kicked: bool) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE groups SET kicked = ?2 WHERE key = ?1",
            params![key, kicked],
        )?;
        Ok(())
    }

    /// The directory said these IDs don't exist: a setup listing them again changes nothing, and
    /// they aren't asked for again.
    pub(crate) fn note_invalid_ids(&self, identities: &[String]) {
        let now = now_ms();
        let mut noted = self.invalid_ids.borrow_mut();
        noted.retain(|_, at| now - *at < INVALID_ID_MEMORY_MS);
        for identity in identities {
            noted.insert(identity.clone(), now);
        }
    }

    pub(crate) fn recently_invalid(&self, identity: &str) -> bool {
        self.invalid_ids
            .borrow()
            .get(identity)
            .is_some_and(|at| now_ms() - *at < INVALID_ID_MEMORY_MS)
    }

    /// How many of `wanted` group member lookups may go to the directory now; they count as made.
    pub(crate) fn member_lookup_allowance(&self, wanted: usize) -> usize {
        self.member_lookup_allowance_at(wanted, now_ms())
    }

    fn member_lookup_allowance_at(&self, wanted: usize, now: i64) -> usize {
        let (began, used) = self.member_lookups.get();
        // A clock set back starts a new hour rather than stopping the lookups for however long.
        let (began, used) = if (0..MEMBER_LOOKUP_WINDOW_MS).contains(&(now - began)) {
            (began, used)
        } else {
            (now, 0)
        };
        let allowed = wanted.min(MEMBER_LOOKUPS_PER_HOUR.saturating_sub(used));
        self.member_lookups.set((began, used + allowed));
        allowed
    }

    /// Whom something waits for and has no contact to be sent or shown with: the members of the
    /// groups we are in, and the receivers of whatever is queued (a sync request to a creator we
    /// don't know). What a failed lookup left behind is found here for the next try.
    pub(crate) fn unresolved_members(&self) -> Result<Vec<String>, StoreError> {
        let mut wanted = BTreeSet::new();
        for group in self.groups()?.into_iter().filter(|group| !group.left) {
            wanted.extend(group.members);
        }
        let mut statement = self
            .conn
            .prepare("SELECT DISTINCT receiver FROM pending_receivers")?;
        let receivers = statement.query_map([], |row| row.get::<_, String>(0))?;
        for receiver in receivers {
            wanted.insert(receiver?);
        }
        if let Some(user) = self.identity()? {
            wanted.remove(&user.threema_id);
        }
        let mut unresolved = Vec::new();
        for identity in wanted {
            if self.contact(&identity)?.is_none() {
                unresolved.push(identity);
            }
        }
        Ok(unresolved)
    }

    /// The directory doesn't know these IDs (revoked or never existed): they are in no group from
    /// here on and nothing waits for them. Android doesn't keep them in a group either. An outbox
    /// entry nobody is left for goes now; a message is raised by the next send, which sees it has
    /// nobody left. Returns whether a group changed.
    pub(crate) fn drop_invalid_members(&self, identities: &[String]) -> Result<bool, StoreError> {
        let mut invalid = Vec::new();
        for identity in identities {
            if self.contact(identity)?.is_none() {
                invalid.push(identity.as_str());
            }
        }
        if invalid.is_empty() {
            return Ok(false);
        }
        let step = self.step()?;
        let mut changed = false;
        for mut group in self.groups()? {
            let before = group.members.len();
            group.members.retain(|m| !invalid.contains(&m.as_str()));
            if group.members.len() != before {
                self.upsert_group(&group)?;
                changed = true;
            }
        }
        for identity in invalid {
            let mut statement = self
                .conn
                .prepare("SELECT DISTINCT message_id FROM pending_receivers WHERE receiver = ?1")?;
            let waiting: Vec<i64> = statement
                .query_map([identity], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            for message_id in waiting {
                for conversation in self.receiver_done(identity, message_id.cast_unsigned())? {
                    self.outbox_remove(&conversation, message_id.cast_unsigned())?;
                }
            }
        }
        step.commit()?;
        Ok(changed)
    }

    /// In a device group, a lookup that gets to someone only now, from the retry or a setup that
    /// changes nothing, also owes the other devices the list with them in it, as the lists so far
    /// have left them out.
    pub(crate) fn note_unresolved_member_lookups(&self) -> Result<(), StoreError> {
        if self.multi_device()?.is_none() {
            return Ok(());
        }
        for group in self.groups()?.into_iter().filter(|group| !group.left) {
            for member in &group.members {
                if self.contact(member)?.is_none() {
                    self.add_member_lookup(&group.key, member, false)?;
                }
            }
        }
        Ok(())
    }

    /// The other devices have to hear of `identity` in this group once its contact is here.
    pub(crate) fn add_member_lookup(
        &self,
        key: &str,
        identity: &str,
        added: bool,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO group_member_lookups (key, identity, added) VALUES (?1, ?2, ?3)
             ON CONFLICT (key, identity) DO UPDATE SET added = added OR excluded.added",
            params![key, identity, added],
        )?;
        Ok(())
    }

    /// Whoever such a lookup is still outstanding for.
    pub(crate) fn outstanding_member_lookups(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT DISTINCT identity FROM group_member_lookups ORDER BY identity")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The groups waiting for `identity`, each with whether it came with the change there; they
    /// stop waiting.
    pub(crate) fn take_member_lookups(
        &self,
        identity: &str,
    ) -> Result<Vec<(String, bool)>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT key, added FROM group_member_lookups WHERE identity = ?1")?;
        let rows = statement
            .query_map([identity], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        self.conn.execute(
            "DELETE FROM group_member_lookups WHERE identity = ?1",
            [identity],
        )?;
        Ok(rows)
    }

    /// A new picture of a group is on the device, set here or from its creator. One store step with
    /// the note that the other devices are owed it, so a crash or a failed upload can't lose that.
    pub(crate) fn store_new_group_picture(&self, group: &Group) -> Result<(), StoreError> {
        let step = self.step()?;
        self.upsert_group(group)?;
        if self.multi_device()?.is_some() {
            self.conn.execute(
                "INSERT OR IGNORE INTO group_picture_syncs (key) VALUES (?1)",
                [&group.key],
            )?;
        }
        step.commit()?;
        Ok(())
    }

    pub(crate) fn group_picture_syncs(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self.conn.prepare("SELECT key FROM group_picture_syncs")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// No longer owed; `false` if it already wasn't.
    pub(crate) fn group_picture_synced(&self, key: &str) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .execute("DELETE FROM group_picture_syncs WHERE key = ?1", [key])?
            > 0)
    }

    /// Remove a group and its conversation (only once it has been left). What the others still have
    /// to hear of our leaving stays queued, as it is sent by the key alone: without it they keep
    /// writing, and the creator's answer to the next sync request brings the group back. Anything
    /// else queued for the group goes with it.
    pub fn delete_group(&self, key: &str) -> Result<(), StoreError> {
        let step = self.step()?;
        self.conn
            .execute("DELETE FROM groups WHERE key = ?1", [key])?;
        for entry in self.outbox_until(key, i64::MAX)? {
            let farewell = entry.kind == OUTBOX_GROUP_LEAVE
                || (entry.kind == OUTBOX_GROUP_SETUP
                    && entry.body["members"].as_array().is_some_and(Vec::is_empty));
            if !farewell {
                self.outbox_remove(key, entry.message_id)?;
            }
        }
        self.conn.execute(
            "DELETE FROM pending_receivers WHERE conversation = ?1
                 AND message_id NOT IN (SELECT message_id FROM outbox WHERE conversation = ?1)",
            [key],
        )?;
        self.conn
            .execute("DELETE FROM group_member_lookups WHERE key = ?1", [key])?;
        self.conn
            .execute("DELETE FROM group_picture_syncs WHERE key = ?1", [key])?;
        self.cancel_synced_picture(key)?;
        // Its calls' keys, and the marks of who was resynced lately (`resync_allowed`).
        self.conn
            .execute("DELETE FROM group_calls WHERE conversation = ?1", [key])?;
        self.conn.execute(
            "DELETE FROM group_resyncs WHERE key = ?1 OR substr(key, 1, length(?1) + 1) = ?1 || '|'",
            [key],
        )?;
        self.forget_chat_choices(key)?;
        self.delete_chat_rows(key)?;
        step.commit()?;
        Ok(())
    }

    /// Insert an incoming group message from `sender`; `None` if seen before.
    pub fn insert_incoming_group(
        &self,
        conversation: &str,
        sender: &str,
        message_id: u64,
        kind: &str,
        body: &str,
        created_at: i64,
    ) -> Result<Option<Message>, StoreError> {
        let Some(message) =
            self.insert_incoming(conversation, message_id, kind, body, created_at)?
        else {
            return Ok(None);
        };
        self.conn.execute(
            "UPDATE messages SET sender = ?2 WHERE id = ?1",
            params![message.id, sender],
        )?;
        self.message(message.id)
    }

    /// Who still has to receive `message_id` of a group conversation.
    pub fn set_pending_receivers(
        &self,
        conversation: &str,
        message_id: u64,
        receivers: &[String],
    ) -> Result<(), StoreError> {
        for receiver in receivers {
            self.conn.execute(
                "INSERT OR IGNORE INTO pending_receivers (conversation, message_id, receiver)
                 VALUES (?1, ?2, ?3)",
                params![conversation, message_id.cast_signed(), receiver],
            )?;
        }
        Ok(())
    }

    pub fn pending_receivers(
        &self,
        conversation: &str,
        message_id: u64,
    ) -> Result<Vec<String>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT receiver FROM pending_receivers WHERE conversation = ?1 AND message_id = ?2
             ORDER BY receiver",
        )?;
        let rows = statement.query_map(params![conversation, message_id.cast_signed()], |row| {
            row.get(0)
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// `receiver` acknowledged `message_id`: returns the group conversations for which nobody is
    /// left to send it to.
    pub fn receiver_done(
        &self,
        receiver: &str,
        message_id: u64,
    ) -> Result<Vec<String>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT conversation FROM pending_receivers WHERE receiver = ?1 AND message_id = ?2",
        )?;
        let conversations: Vec<String> = statement
            .query_map(params![receiver, message_id.cast_signed()], |row| {
                row.get(0)
            })?
            .collect::<Result<_, _>>()?;
        self.conn.execute(
            "DELETE FROM pending_receivers WHERE receiver = ?1 AND message_id = ?2",
            params![receiver, message_id.cast_signed()],
        )?;
        let mut done = Vec::new();
        for conversation in conversations {
            if self
                .pending_receivers(&conversation, message_id)?
                .is_empty()
            {
                done.push(conversation);
            }
        }
        Ok(done)
    }

    /// Queue a group control/outbox message for these receivers.
    pub fn group_outbox_add(
        &self,
        key: &str,
        receivers: &[String],
        kind: &str,
        body: &Value,
    ) -> Result<(), StoreError> {
        if receivers.is_empty() {
            return Ok(());
        }
        let message_id = libthreema::common::MessageId::random().0;
        self.outbox_add(key, message_id, kind, body)?;
        self.set_pending_receivers(key, message_id, receivers)
    }

    /// `peer` rejected `message_id` without naming a group, as anyone does for a message we sent
    /// without naming one: if it is one of our group messages (or a group outbox entry) in a
    /// group `peer` is a member of, it has to get it again. Returns whether something was
    /// requeued. Whoever isn't a member gets nothing this way, however old an ID they know.
    pub fn requeue_group_receiver(&self, peer: &str, message_id: u64) -> Result<bool, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT conversation FROM messages
                 WHERE outgoing = 1 AND message_id = ?1 AND conversation LIKE 'g:%'
                     AND kind <> 'deleted'
             UNION SELECT conversation FROM outbox
                 WHERE message_id = ?1 AND conversation LIKE 'g:%'",
        )?;
        let conversations: Vec<String> = statement
            .query_map([message_id.cast_signed()], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        let mut requeued = false;
        for conversation in &conversations {
            let member = self
                .group(conversation)?
                .is_some_and(|group| !group.left && group.members.iter().any(|m| m == peer));
            if member {
                requeued |= self.requeue_for(conversation, peer, message_id)?;
            }
        }
        Ok(requeued)
    }

    /// Put a group message of ours back in the queue for `peer`; `false` if it was deleted for
    /// everyone meanwhile, with nothing left to send.
    fn requeue_for(
        &self,
        conversation: &str,
        peer: &str,
        message_id: u64,
    ) -> Result<bool, StoreError> {
        let deleted: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM messages WHERE conversation = ?1 AND outgoing = 1
                 AND message_id = ?2 AND kind = 'deleted')",
            params![conversation, message_id.cast_signed()],
            |row| row.get(0),
        )?;
        if deleted {
            return Ok(false);
        }
        self.set_pending_receivers(conversation, message_id, &[peer.to_owned()])?;
        self.conn.execute(
            "UPDATE messages SET state = 0, sent_at = NULL WHERE conversation = ?1 AND outgoing = 1
                 AND message_id = ?2 AND state = 1",
            params![conversation, message_id.cast_signed()],
        )?;
        Ok(true)
    }

    /// Whether `message_id` is in this group's chat or its outbox: `Some(true)` if we sent it.
    fn group_message_is_ours(
        &self,
        conversation: &str,
        message_id: u64,
    ) -> Result<Option<bool>, StoreError> {
        let id = message_id.cast_signed();
        let outgoing: Option<bool> = self
            .conn
            .query_row(
                "SELECT outgoing FROM messages WHERE conversation = ?1 AND message_id = ?2
                 UNION ALL SELECT 1 FROM outbox WHERE conversation = ?1 AND message_id = ?2
                 ORDER BY 1 DESC LIMIT 1",
                params![conversation, id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(outgoing)
    }

    /// Group outbox entries older than `before` stop waiting for a receiver with no contact, once
    /// even the retries on reconnect and in maintenance didn't get them one. An entry nobody is
    /// left for goes. Returns how many receivers gave up on.
    pub(crate) fn expire_unresolved_outbox(&self, before: i64) -> Result<usize, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT DISTINCT p.receiver, p.message_id FROM pending_receivers p
                 JOIN outbox o ON o.conversation = p.conversation AND o.message_id = p.message_id
                 WHERE o.created_at < ?1
                     AND p.receiver NOT IN (SELECT identity FROM contacts)",
        )?;
        let expired: Vec<(String, i64)> = statement
            .query_map([before], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (receiver, message_id) in &expired {
            for conversation in self.receiver_done(receiver, message_id.cast_unsigned())? {
                self.outbox_remove(&conversation, message_id.cast_unsigned())?;
            }
        }
        Ok(expired.len())
    }

    /// `recently resynced` marks: `true` (and marked now) if not marked within the last hour.
    fn resync_allowed(&self, mark: &str) -> Result<bool, StoreError> {
        let at: Option<i64> = self
            .conn
            .query_row(
                "SELECT at FROM group_resyncs WHERE key = ?1",
                [mark],
                |row| row.get(0),
            )
            .optional()?;
        let now = now_ms();
        if at.is_some_and(|at| now - at < RESYNC_INTERVAL_MS) {
            return Ok(false);
        }
        // Marks past the hour say nothing any more. The key holds what a sender made up (any group
        // ID will do), so they can't be left to pile up.
        self.conn.execute(
            "DELETE FROM group_resyncs WHERE at <= ?1",
            [now - RESYNC_INTERVAL_MS],
        )?;
        self.conn.execute(
            "INSERT OR REPLACE INTO group_resyncs (key, at) VALUES (?1, ?2)",
            params![mark, now],
        )?;
        Ok(true)
    }
}

/// What happened to a member (`GroupSync.Update.member_state_changes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberChange {
    Added,
    Kicked,
    Left,
}

/// What an incoming group message changed, for the user's other devices. A picture isn't here: it
/// is reflected when it is on this device (`download_group_picture`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupChange {
    /// A new group.
    pub created: bool,
    pub name: bool,
    pub members: bool,
    /// Who came and went; empty when we came back ourselves.
    pub member_changes: std::collections::BTreeMap<String, MemberChange>,
    /// We were removed, or are (still) a member after a setup.
    pub user_state: bool,
    pub picture_removed: bool,
}

impl GroupChange {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// What processing an incoming group message did.
#[derive(Debug, Default)]
pub struct GroupOutcome {
    /// Messages added or changed in conversations.
    pub added: Vec<Message>,
    pub changed: Vec<Message>,
    /// Something was queued to be sent (reply, sync request).
    pub send: bool,
    /// Member identities we have no contact for (fetch them from the directory).
    pub unknown_members: Vec<String>,
    /// A group was created or changed (UI: reload).
    pub groups_changed: bool,
    /// The group's picture changed: download it (group key).
    pub download_picture: Option<String>,
    /// The group's picture was deleted: its file goes too (group key).
    pub remove_picture: Option<String>,
    /// Who just got the group's state from us: a running call is for them to hear of too.
    pub state_sent_to: Vec<String>,
    /// A group call was announced (checked and stored by the app).
    pub call_start: Option<libthreema::model::message::GroupCallStartMessage>,
    /// What the other devices of the group have to be told.
    pub change: GroupChange,
    /// That went into the sync queue, in the same store step as the change.
    pub sync_queued: bool,
}

/// Group receive steps; `user` is our identity, `contact_exists` whether we have a contact (any
/// acquaintance) for an identity, `blocked` whether the user blocked one explicitly (ADR 0019),
/// `name_of` its display name (for status lines).
///
/// `from_sync`: another device, the leader, reflected this message after running these steps
/// itself and answering whatever needed an answer. Group control is discarded then, like Android's
/// `executeMessageStepsFromSync`: the leader reflects a `GroupSync` for it (csp.struct.yml), and
/// that makes the status lines. Content is simply stored, and nothing goes out from here.
///
/// While this device leads, group control runs as one store step together with the entry that tells
/// the other devices (csp.struct.yml's _MD_ steps): a crash can't keep the change and lose the
/// entry, or the other way round.
#[expect(clippy::too_many_arguments, reason = "the receive context")]
pub fn receive(
    store: &Store,
    user: &str,
    sender: &str,
    message_id: u64,
    created_at: i64,
    message: IncomingGroupMessageBody,
    from_sync: bool,
    contact_exists: &dyn Fn(&str) -> bool,
    blocked: &dyn Fn(&str) -> Result<bool, StoreError>,
    name_of: &dyn Fn(&str) -> String,
) -> Result<GroupOutcome, StoreError> {
    let group_key = key(&message.group_identity);
    let control = matches!(
        message.body,
        GroupMessageBody::Setup(_)
            | GroupMessageBody::Name(_)
            | GroupMessageBody::Leave(_)
            | GroupMessageBody::DeleteProfilePicture(_)
    );
    if from_sync || !control {
        return receive_steps(
            store,
            user,
            sender,
            message_id,
            created_at,
            message,
            from_sync,
            contact_exists,
            blocked,
            name_of,
        );
    }
    let step = store.step()?;
    let mut outcome = receive_steps(
        store,
        user,
        sender,
        message_id,
        created_at,
        message,
        from_sync,
        contact_exists,
        blocked,
        name_of,
    )?;
    if !outcome.change.is_empty() {
        outcome.sync_queued = crate::sync::queue_received_group_change(
            store,
            &group_key,
            &outcome.change,
            &outcome.unknown_members,
        )?;
    }
    step.commit()?;
    Ok(outcome)
}

#[expect(clippy::too_many_arguments, reason = "the receive context")]
fn receive_steps(
    store: &Store,
    user: &str,
    sender: &str,
    message_id: u64,
    created_at: i64,
    message: IncomingGroupMessageBody,
    from_sync: bool,
    contact_exists: &dyn Fn(&str) -> bool,
    blocked: &dyn Fn(&str) -> Result<bool, StoreError>,
    name_of: &dyn Fn(&str) -> String,
) -> Result<GroupOutcome, StoreError> {
    let identity = message.group_identity;
    let key = key(&identity);
    let creator = identity.creator_identity.to_string();
    let user_is_creator = creator == user;
    let mut outcome = GroupOutcome::default();
    let group = store.group(&key)?;

    if from_sync {
        if matches!(
            message.body,
            GroupMessageBody::Setup(_)
                | GroupMessageBody::Name(_)
                | GroupMessageBody::Leave(_)
                | GroupMessageBody::SyncRequest(_)
                | GroupMessageBody::SetProfilePicture(_)
                | GroupMessageBody::DeleteProfilePicture(_)
        ) {
            return Ok(outcome);
        }
        if group.is_none() {
            tracing::info!("reflected group message for an unknown group, discarded");
            return Ok(outcome);
        }
    }

    match message.body {
        GroupMessageBody::Setup(setup) => {
            receive_setup(
                store,
                user,
                &key,
                identity,
                setup.members,
                group,
                &mut outcome,
                contact_exists,
                blocked,
                name_of,
            )?;
            return Ok(outcome);
        }
        GroupMessageBody::Leave(_) => {
            if sender == creator {
                tracing::warn!("group-leave from the creator, discarded");
                return Ok(outcome);
            }
            let Some(mut group) = group.filter(|g| !g.left) else {
                if !user_is_creator && !blocked(&creator)? {
                    sync_request(store, &key, &creator, &mut outcome)?;
                }
                return Ok(outcome);
            };
            if !group.members.iter().any(|m| m == sender) {
                return Ok(outcome);
            }
            group.members.retain(|m| m != sender);
            store.upsert_group(&group)?;
            status(
                store,
                &key,
                &format!("groupMemberLeft|{}", name_of(sender)),
                &mut outcome,
            )?;
            outcome.groups_changed = true;
            // What was queued for them doesn't go now: something may have nobody else left.
            outcome.send = true;
            outcome.change.members = true;
            outcome
                .change
                .member_changes
                .insert(sender.to_owned(), MemberChange::Left);
            return Ok(outcome);
        }
        GroupMessageBody::SyncRequest(_) => {
            let Some(group) = group else {
                return Ok(outcome);
            };
            if !user_is_creator || !store.resync_allowed(&format!("{key}|{sender}"))? {
                return Ok(outcome);
            }
            let receivers = vec![sender.to_owned()];
            if group.left || !group.members.iter().any(|m| m == sender) {
                store.group_outbox_add(
                    &key,
                    &receivers,
                    OUTBOX_GROUP_SETUP,
                    &json!({ "members": [] }),
                )?;
            } else {
                send_state(store, &group, &receivers)?;
                outcome.state_sent_to = receivers;
            }
            outcome.send = true;
            return Ok(outcome);
        }
        _ => {}
    }

    let Some(mut group) = common_receive_steps(
        store,
        &key,
        &creator,
        user,
        sender,
        group,
        from_sync,
        blocked,
        &mut outcome,
    )?
    else {
        return Ok(outcome);
    };

    let content = |kind: &str, body: String| -> Result<Option<Message>, StoreError> {
        store.insert_incoming_group(&key, sender, message_id, kind, &body, created_at)
    };
    match message.body {
        GroupMessageBody::Name(name) => {
            if group.name != name.name {
                group.name = name.name;
                store.upsert_group(&group)?;
                status(
                    store,
                    &key,
                    &format!("groupRenamed|{}", group.name),
                    &mut outcome,
                )?;
                outcome.groups_changed = true;
                outcome.change.name = true;
            }
        }
        // Group calls (ADR 0012): the app checks the SFU URL and peeks the call.
        GroupMessageBody::CallStart(start) => outcome.call_start = Some(start),
        GroupMessageBody::Text(text) => outcome.added.extend(content("text", text.text)?),
        GroupMessageBody::Location(location) => outcome.added.extend(content(
            "location",
            json!({
                "latitude": location.latitude,
                "longitude": location.longitude,
                "accuracy": location.accuracy_m,
                "name": location.name,
                "address": location.address,
            })
            .to_string(),
        )?),
        GroupMessageBody::File(file) => {
            if let Some(stored) = content("file", crate::app_media::file_body(&file).to_string())? {
                store.set_file_keys(stored.id, &crate::app_media::file_keys(&file))?;
                outcome.added.push(stored);
            }
        }
        GroupMessageBody::Reaction(reaction) => {
            let (emoji, apply) = match &reaction.action {
                libthreema::model::message::ReactionAction::Apply(emoji) => (emoji, true),
                libthreema::model::message::ReactionAction::Withdraw(emoji) => (emoji, false),
            };
            if let Some(id) = store.reactable_message(&key, reaction.message_id.0)? {
                outcome
                    .changed
                    .extend(store.set_reaction(id, sender, emoji, apply, created_at)?);
            }
        }
        GroupMessageBody::LegacyReaction(reaction) => {
            let emoji = reactions::legacy_emoji(reaction.reaction_type);
            for referred in &reaction.message_ids {
                if let Some(id) = store.reactable_message(&key, referred.0)? {
                    outcome
                        .changed
                        .extend(store.replace_reactions(id, sender, emoji, created_at)?);
                }
            }
        }
        GroupMessageBody::Edit(edit) => {
            if let Some(original) = own_message_of(store, &key, sender, edit.message_id.0)?
                .filter(|m| crate::messages::edit_applies_to(&m.kind))
            {
                outcome.changed.extend(store.edit_message(
                    original.id,
                    &edit.text,
                    Some(created_at),
                )?);
            }
        }
        GroupMessageBody::Delete(delete) => {
            if let Some(original) = own_message_of(store, &key, sender, delete.message_id.0)?
                .filter(|m| crate::messages::delete_applies_to(&m.kind))
            {
                outcome
                    .changed
                    .extend(store.mark_deleted(original.id, created_at)?);
            }
        }
        // Exempt from blocking so that a blocked member's copy of a poll stays in step when we send,
        // but their poll doesn't show here (Android's `processIncomingGroupMessage`).
        GroupMessageBody::PollSetup(_) if blocked(sender)? => {
            tracing::info!("group poll from a blocked member, ignored");
        }
        GroupMessageBody::PollSetup(setup) => {
            let (added, changed) = crate::polls::receive_setup(
                store,
                &key,
                sender,
                true,
                message_id,
                created_at,
                setup.poll_id,
                &setup.poll,
            )?;
            outcome.added.extend(added);
            outcome.changed.extend(changed);
        }
        GroupMessageBody::PollVote(vote) => {
            outcome.changed.extend(crate::polls::receive_vote(
                store,
                &key,
                sender,
                &vote.creator_identity.to_string(),
                vote.poll_id,
                &vote.choices,
            )?);
        }
        // Downloaded by the app (no `done`: every member fetches it); applied once on the device.
        GroupMessageBody::SetProfilePicture(picture) => {
            let picture = GroupPicture {
                blob_id: picture.blob_id,
                key: picture.key,
                size: picture.size,
            };
            if group.picture.as_ref() != Some(&picture) || group.picture_version == 0 {
                group.picture = Some(picture);
                store.upsert_group(&group)?;
                // From the creator: newer than what another device said of it.
                store.cancel_synced_picture(&key)?;
                outcome.download_picture = Some(key.clone());
            }
        }
        GroupMessageBody::DeleteProfilePicture(_) => {
            if group.picture.is_some() || group.picture_version != 0 {
                group.picture = None;
                group.picture_version = 0;
                store.upsert_group(&group)?;
                store.cancel_synced_picture(&key)?;
                outcome.groups_changed = true;
                outcome.change.picture_removed = true;
                // A picture still owed to the other devices isn't any more.
                store.group_picture_synced(&key)?;
                outcome.remove_picture = Some(key.clone());
            }
        }
        GroupMessageBody::Setup(_)
        | GroupMessageBody::Leave(_)
        | GroupMessageBody::SyncRequest(_) => {}
    }
    Ok(outcome)
}

/// An incoming message of `sender` in a group (edits and deletions only apply to the sender's).
fn own_message_of(
    store: &Store,
    key: &str,
    sender: &str,
    message_id: u64,
) -> Result<Option<Message>, StoreError> {
    Ok(store
        .message_by_id(key, false, message_id)?
        .filter(|m| m.sender.as_deref() == Some(sender)))
}

fn status(
    store: &Store,
    key: &str,
    code: &str,
    outcome: &mut GroupOutcome,
) -> Result<(), StoreError> {
    outcome.added.extend(store.insert_status(key, code)?);
    Ok(())
}

/// What to do after a Reject that names a group.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GroupReject {
    /// Something was queued: send it.
    pub send: bool,
    /// The rejected message goes out again.
    pub resending: bool,
    /// `peer` is getting the group's state.
    pub state_sent: bool,
}

/// `peer` rejected `message_id` in the group `identity` (`Reject` in csp-e2e-fs.proto): the
/// _Common Group Receive Steps_ first, so that someone removed learns it from this too. A message
/// we don't have is a sync request if the group is ours (the group's state may be what got lost),
/// and one of ours goes out again to `peer`.
pub fn message_rejected(
    store: &Store,
    user: &str,
    peer: &str,
    identity: GroupIdentity,
    message_id: u64,
    blocked: &dyn Fn(&str) -> Result<bool, StoreError>,
) -> Result<GroupReject, StoreError> {
    let key = key(&identity);
    let creator = identity.creator_identity.to_string();
    let mut outcome = GroupOutcome::default();
    let group = store.group(&key)?;
    let Some(group) = common_receive_steps(
        store,
        &key,
        &creator,
        user,
        peer,
        group,
        false,
        blocked,
        &mut outcome,
    )?
    else {
        return Ok(GroupReject {
            send: outcome.send,
            resending: false,
            state_sent: false,
        });
    };
    match store.group_message_is_ours(&key, message_id)? {
        Some(true) => {
            let resending = store.requeue_for(&key, peer, message_id)?;
            Ok(GroupReject {
                send: resending,
                resending,
                state_sent: false,
            })
        }
        // Someone else's: not for us to send.
        Some(false) => Ok(GroupReject::default()),
        None if creator == user && store.resync_allowed(&format!("{key}|{peer}"))? => {
            send_state(store, &group, &[peer.to_owned()])?;
            Ok(GroupReject {
                send: true,
                resending: false,
                state_sent: true,
            })
        }
        None => Ok(GroupReject::default()),
    }
}

/// _Common Group Receive Steps_: the group, if `sender` is a member of one we are in. If not, what
/// the protocol asks for in that case is queued: the creator gets a sync request (not a blocked
/// creator, for a group we don't know: asking them would bring it back, ADR 0019), someone removed
/// or left behind an empty setup or our leave.
#[expect(clippy::too_many_arguments, reason = "the receive context")]
fn common_receive_steps(
    store: &Store,
    key: &str,
    creator: &str,
    user: &str,
    sender: &str,
    group: Option<Group>,
    from_sync: bool,
    blocked: &dyn Fn(&str) -> Result<bool, StoreError>,
    outcome: &mut GroupOutcome,
) -> Result<Option<Group>, StoreError> {
    let user_is_creator = creator == user;
    let Some(group) = group else {
        if !user_is_creator && !blocked(creator)? {
            sync_request(store, key, creator, outcome)?;
        }
        return Ok(None);
    };
    let reply_to = vec![sender.to_owned()];
    if group.left && !from_sync {
        if user_is_creator {
            store.group_outbox_add(
                key,
                &reply_to,
                OUTBOX_GROUP_SETUP,
                &json!({ "members": [] }),
            )?;
        } else {
            store.group_outbox_add(key, &reply_to, OUTBOX_GROUP_LEAVE, &json!({}))?;
        }
        outcome.send = true;
        return Ok(None);
    }
    if !group.members.iter().any(|m| m == sender) && !from_sync {
        if user_is_creator {
            store.group_outbox_add(
                key,
                &reply_to,
                OUTBOX_GROUP_SETUP,
                &json!({ "members": [] }),
            )?;
            outcome.send = true;
        } else {
            sync_request(store, key, creator, outcome)?;
        }
        return Ok(None);
    }
    Ok(Some(group))
}

/// _Group Sync Request Steps_ (we are not the creator).
fn sync_request(
    store: &Store,
    key: &str,
    creator: &str,
    outcome: &mut GroupOutcome,
) -> Result<(), StoreError> {
    // The directory told us there is no such creator a moment ago; asking them would mean asking
    // it again.
    if store.recently_invalid(creator) {
        return Ok(());
    }
    // Any contact can name any group ID and creator in a group message, and each new pair asks here.
    let queued: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM outbox WHERE kind = ?1",
        [OUTBOX_GROUP_SYNC_REQUEST],
        |row| row.get(0),
    )?;
    if queued >= SYNC_REQUESTS_QUEUED_MAX {
        tracing::warn!(queued, "too many group sync requests waiting, none added");
        return Ok(());
    }
    if !store.resync_allowed(key)? {
        tracing::info!("group recently resynced, no new sync request");
        return Ok(());
    }
    store.group_outbox_add(
        key,
        &[creator.to_owned()],
        OUTBOX_GROUP_SYNC_REQUEST,
        &json!({}),
    )?;
    outcome.send = true;
    // It goes out once they have a contact; if the directory doesn't know them, it goes altogether.
    if store.contact(creator)?.is_none() {
        outcome.unknown_members.push(creator.to_owned());
    }
    Ok(())
}

/// A group after a restore (Android's _Application Setup Steps_): the creator sends its state to
/// everyone, a member asks the creator for it, and a group we left stays quiet. So does a creator
/// on the block list (ADR 0019). A creator's picture isn't in the backup, so there is none to send
/// and nothing to say about it: unlike Android, this doesn't delete it for all members. Returns
/// whether something was queued.
pub(crate) fn resync_restored(
    store: &Store,
    group: &Group,
    user: &str,
) -> Result<bool, StoreError> {
    if group.left {
        return Ok(false);
    }
    let creator = group.identity.creator_identity.to_string();
    if creator != user {
        if store.is_explicitly_blocked(&creator)? {
            return Ok(false);
        }
        let mut outcome = GroupOutcome::default();
        sync_request(store, &group.key, &creator, &mut outcome)?;
        return Ok(outcome.send);
    }
    if group.members.is_empty() {
        return Ok(false);
    }
    if group.picture.is_some() {
        send_state(store, group, &group.members)?;
    } else {
        send_setup_and_name(store, group, &group.members)?;
    }
    Ok(true)
}

fn send_setup_and_name(
    store: &Store,
    group: &Group,
    receivers: &[String],
) -> Result<(), StoreError> {
    store.group_outbox_add(
        &group.key,
        receivers,
        OUTBOX_GROUP_SETUP,
        &json!({ "members": group.members }),
    )?;
    store.group_outbox_add(
        &group.key,
        receivers,
        OUTBOX_GROUP_NAME,
        &json!({ "name": group.name }),
    )
}

/// The creator sends a group's state to `receivers`: setup, name, picture.
pub fn send_state(store: &Store, group: &Group, receivers: &[String]) -> Result<(), StoreError> {
    send_setup_and_name(store, group, receivers)?;
    send_picture(store, group, receivers)
}

/// What comes after the setup for whoever is new to the group: its name and picture.
pub fn send_name_and_picture(
    store: &Store,
    group: &Group,
    receivers: &[String],
) -> Result<(), StoreError> {
    store.group_outbox_add(
        &group.key,
        receivers,
        OUTBOX_GROUP_NAME,
        &json!({ "name": group.name }),
    )?;
    send_picture(store, group, receivers)
}

fn send_picture(store: &Store, group: &Group, receivers: &[String]) -> Result<(), StoreError> {
    match &group.picture {
        Some(picture) => store.group_outbox_add(
            &group.key,
            receivers,
            OUTBOX_GROUP_SET_PICTURE,
            &picture.to_json(),
        ),
        None => store.group_outbox_add(
            &group.key,
            receivers,
            OUTBOX_GROUP_DELETE_PICTURE,
            &json!({}),
        ),
    }
}

#[expect(clippy::too_many_arguments, reason = "the receive context")]
fn receive_setup(
    store: &Store,
    user: &str,
    key: &str,
    identity: GroupIdentity,
    members: Vec<ThreemaId>,
    group: Option<Group>,
    outcome: &mut GroupOutcome,
    contact_exists: &dyn Fn(&str) -> bool,
    blocked: &dyn Fn(&str) -> Result<bool, StoreError>,
    name_of: &dyn Fn(&str) -> String,
) -> Result<(), StoreError> {
    let creator = identity.creator_identity.to_string();
    // group-setup is exempt from the general block check because of these dedicated steps: a blocked
    // creator can't bring us into a new group, or back into one we left (ADR 0019). For a group we
    // are in, their setup still counts.
    if group.as_ref().is_none_or(|g| g.left) && blocked(&creator)? {
        tracing::info!("group-setup for a group we aren't in from a blocked creator, discarded");
        return Ok(());
    }
    if members.len() > SETUP_MEMBERS_MAX {
        tracing::warn!(
            members = members.len(),
            "group-setup listing too many members, discarded"
        );
        return Ok(());
    }
    // Someone revoked isn't made a member, as nothing we send could ever reach them (Android). The
    // same for whom the directory didn't know a moment ago, or each resend would list them anew.
    let mut listed = BTreeSet::new();
    for member in members {
        let member = member.to_string();
        if member != creator
            && !store.recently_invalid(&member)
            && store
                .contact(&member)?
                .is_none_or(|c| c.activity_state != ActivityState::Invalid)
        {
            listed.insert(member);
        }
    }
    let members = listed;
    // Asked for even if nothing else changes: an earlier lookup may have failed.
    let unknown_members: Vec<String> = members
        .iter()
        .filter(|m| *m != user)
        .chain(std::iter::once(&creator))
        .filter(|m| !contact_exists(m))
        .cloned()
        .collect();
    if let Some(group) = &group {
        if group.left && members.is_empty() {
            return Ok(());
        }
        if !group.left {
            let mut current: BTreeSet<String> = group
                .members
                .iter()
                .filter(|m| **m != creator)
                .cloned()
                .collect();
            current.insert(user.to_owned());
            // A list without the creator is a change, whatever else it says: an older version's
            // group sync could have taken them out, and this setup is what brings them back.
            if current == members && group.members.contains(&creator) {
                if store.multi_device()?.is_some() {
                    for member in &unknown_members {
                        store.add_member_lookup(key, member, false)?;
                    }
                }
                outcome.unknown_members = unknown_members;
                return Ok(());
            }
        }
    }
    if !members.contains(user) {
        let Some(mut group) = group else {
            return Ok(());
        };
        let was_in = !group.left;
        group.left = true;
        store.upsert_group(&group)?;
        if was_in {
            store.set_group_kicked(key, true)?;
            outcome.change.user_state = true;
        }
        // A list without us says no more than that: the spec has creators send removed members an
        // empty list just like when dissolving, while the group may well go on.
        status(store, key, "groupNoLongerMember", outcome)?;
        outcome.groups_changed = true;
        return Ok(());
    }
    // Everyone but us, the creator included.
    let mut new_members: Vec<String> = members.into_iter().filter(|m| m != user).collect();
    new_members.insert(0, creator.clone());
    outcome.unknown_members = unknown_members;
    match group {
        None => {
            store.upsert_group(&Group {
                key: key.to_owned(),
                identity,
                name: String::new(),
                members: new_members,
                left: false,
                created_at: now_ms(),
                picture: None,
                picture_version: 0,
            })?;
            status(store, key, "groupJoined", outcome)?;
            outcome.change.created = true;
        }
        Some(mut group) => {
            let rejoined = group.left;
            // The creator was always in it: coming back after a damaged sync is no news.
            let added: Vec<String> = new_members
                .iter()
                .filter(|m| !rejoined && !group.members.contains(m) && **m != creator)
                .cloned()
                .collect();
            let removed: Vec<String> = if rejoined {
                Vec::new()
            } else {
                group
                    .members
                    .iter()
                    .filter(|m| !new_members.contains(m))
                    .cloned()
                    .collect()
            };
            group.members = new_members;
            group.left = false;
            store.upsert_group(&group)?;
            // What was queued for those gone doesn't go now: something may have nobody else left.
            outcome.send |= !removed.is_empty();
            outcome.change.members = true;
            outcome.change.user_state = true;
            for member in &added {
                outcome
                    .change
                    .member_changes
                    .insert(member.clone(), MemberChange::Added);
            }
            for member in &removed {
                outcome
                    .change
                    .member_changes
                    .insert(member.clone(), MemberChange::Kicked);
            }
            if rejoined {
                status(store, key, "groupJoined", outcome)?;
            } else {
                // Someone we have no contact for gets their line once the directory has said they
                // exist (`sync::members_looked_up`). Until then they may be made up, and a line
                // for each would be the sender's to fill the chat with. Without a contact they
                // have had no line yet to answer with a "removed".
                for member in added {
                    if contact_exists(&member) {
                        status(
                            store,
                            key,
                            &format!("groupMemberAdded|{}", name_of(&member)),
                            outcome,
                        )?;
                    } else {
                        store.add_member_lookup(key, &member, true)?;
                    }
                }
                for member in removed {
                    if contact_exists(&member) {
                        status(
                            store,
                            key,
                            &format!("groupMemberRemoved|{}", name_of(&member)),
                            outcome,
                        )?;
                    }
                }
            }
        }
    }
    outcome.groups_changed = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use libthreema::model::message::{
        GroupLeaveMessage, GroupNameMessage, GroupSetupMessage, GroupSyncRequestMessage,
        TextMessage,
    };

    use super::*;

    const USER: &str = "USER0001";
    const CREATOR: &str = "CREATOR1";
    const BOB: &str = "00000BOB";
    const EVE: &str = "00000EVE";
    const BLOCKED: &str = "BLOCKED1";

    fn id(s: &str) -> ThreemaId {
        ThreemaId::try_from(s).unwrap()
    }

    fn gid(creator: &str) -> GroupIdentity {
        GroupIdentity {
            creator_identity: id(creator),
            group_id: 0xabcd,
        }
    }

    fn run(store: &Store, sender: &str, creator: &str, body: GroupMessageBody) -> GroupOutcome {
        receive(
            store,
            USER,
            sender,
            rand::random(),
            1,
            IncomingGroupMessageBody {
                group_identity: gid(creator),
                body,
            },
            false,
            &|m| m != "UNKNOWN1",
            &|m| Ok(m == BLOCKED),
            &|m| format!("~{m}"),
        )
        .unwrap()
    }

    fn outbox_kinds(store: &Store) -> Vec<(String, Vec<String>)> {
        store
            .outbox()
            .unwrap()
            .into_iter()
            .map(|e| {
                let receivers = store
                    .pending_receivers(&e.conversation, e.message_id)
                    .unwrap();
                (e.kind, receivers)
            })
            .collect()
    }

    fn setup(members: &[&str]) -> GroupMessageBody {
        GroupMessageBody::Setup(GroupSetupMessage {
            members: members.iter().map(|m| id(m)).collect(),
        })
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        (dir, store)
    }

    #[test]
    fn keys() {
        let key = key(&gid(CREATOR));
        assert_eq!(key, "g:CREATOR1:000000000000abcd");
        assert_eq!(parse_key(&key), Some(gid(CREATOR)));
        assert_eq!(parse_key("ECHOECHO"), None);
    }

    #[test]
    fn unknown_group_asks_the_creator_once() {
        let (_dir, store) = store();
        let text = || GroupMessageBody::Text(TextMessage { text: "hi".into() });
        let outcome = run(&store, BOB, CREATOR, text());
        assert!(outcome.send && outcome.added.is_empty());
        assert_eq!(
            outbox_kinds(&store),
            vec![(OUTBOX_GROUP_SYNC_REQUEST.into(), vec![CREATOR.into()])]
        );
        // Recently resynced: no second request within the hour.
        assert!(!run(&store, BOB, CREATOR, text()).send);
    }

    #[test]
    fn a_blocked_creator_cannot_bring_us_into_a_new_group() {
        let (_dir, store) = store();
        let group_key = key(&gid(BLOCKED));

        // A new group from them: nothing appears, nobody is told.
        let outcome = run(&store, BLOCKED, BLOCKED, setup(&[USER, BOB]));
        assert!(outcome.added.is_empty() && !outcome.groups_changed && !outcome.send);
        assert!(store.group(&group_key).unwrap().is_none());

        // Group messages for it don't ask the blocked creator for it either.
        let text = GroupMessageBody::Text(TextMessage { text: "hi".into() });
        assert!(!run(&store, BOB, BLOCKED, text).send);
        let leave = GroupMessageBody::Leave(GroupLeaveMessage {});
        assert!(!run(&store, BOB, BLOCKED, leave).send);
        assert!(outbox_kinds(&store).is_empty());
    }

    #[test]
    fn a_blocked_creator_cannot_bring_us_back_into_a_group_we_left() {
        let (_dir, store) = store();
        let group_key = key(&gid(BLOCKED));
        store
            .upsert_group(&Group {
                key: group_key.clone(),
                identity: gid(BLOCKED),
                name: String::new(),
                members: vec![BLOCKED.into(), BOB.into()],
                left: true,
                created_at: 0,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        let outcome = run(&store, BLOCKED, BLOCKED, setup(&[USER, BOB]));
        assert!(outcome.added.is_empty() && !outcome.groups_changed);
        assert!(store.group(&group_key).unwrap().unwrap().left);
        // Nor does being removed from it again add a line.
        let outcome = run(&store, BLOCKED, BLOCKED, setup(&[BOB]));
        assert!(outcome.added.is_empty() && !outcome.groups_changed);
    }

    #[test]
    fn a_blocked_members_poll_does_not_show() {
        let (_dir, store) = store();
        let group_key = key(&gid(CREATOR));
        store
            .upsert_group(&Group {
                key: group_key.clone(),
                identity: gid(CREATOR),
                name: String::new(),
                members: vec![CREATOR.into(), BLOCKED.into(), BOB.into()],
                left: false,
                created_at: 0,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        let poll = |id: u64| {
            let poll = crate::polls::new_poll(
                id,
                "ABCD1234",
                "Lunch?",
                &["Pizza".into(), "Sushi".into()],
                false,
                true,
            )
            .unwrap();
            GroupMessageBody::PollSetup(libthreema::model::message::PollSetupMessage {
                poll_id: id,
                poll: crate::polls::setup_json(&poll),
            })
        };
        assert!(run(&store, BLOCKED, CREATOR, poll(1)).added.is_empty());
        assert!(
            store
                .conversation_messages(&group_key, 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(run(&store, BOB, CREATOR, poll(2)).added.len(), 1);
    }

    #[test]
    fn a_blocked_creator_still_changes_a_group_we_have() {
        let (_dir, store) = store();
        let group_key = key(&gid(BLOCKED));
        store
            .upsert_group(&Group {
                key: group_key.clone(),
                identity: gid(BLOCKED),
                name: String::new(),
                members: vec![BLOCKED.into(), BOB.into()],
                left: false,
                created_at: 0,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        assert!(run(&store, BLOCKED, BLOCKED, setup(&[USER])).groups_changed);
        assert_eq!(
            store.group(&group_key).unwrap().unwrap().members,
            vec![BLOCKED.to_owned()]
        );
    }

    #[test]
    fn group_membership_is_exact() {
        let (_dir, store) = store();
        store
            .upsert_group(&Group {
                key: key(&gid(CREATOR)),
                identity: gid(CREATOR),
                name: "Crew".into(),
                members: vec![CREATOR.into(), BOB.into()],
                left: false,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        assert!(store.is_group_member(BOB).unwrap());
        assert!(store.is_group_member(CREATOR).unwrap());
        // A substring of a member's id is not a member (the query's LIKE only narrows rows).
        assert!(!store.is_group_member("00000BO").unwrap());
        assert!(!store.is_group_member("ABCD1234").unwrap());
        // A group we are no longer in does not count.
        let mut left = store.group(&key(&gid(CREATOR))).unwrap().unwrap();
        left.left = true;
        store.upsert_group(&left).unwrap();
        assert!(!store.is_group_member(BOB).unwrap());
    }

    #[test]
    fn setup_creates_updates_and_removes() {
        let (_dir, store) = store();
        let key = key(&gid(CREATOR));
        let outcome = run(&store, CREATOR, CREATOR, setup(&[USER, BOB, "UNKNOWN1"]));
        assert!(outcome.groups_changed);
        assert_eq!(outcome.unknown_members, vec!["UNKNOWN1".to_owned()]);
        let group = store.group(&key).unwrap().unwrap();
        assert_eq!(group.members, vec![CREATOR, "00000BOB", "UNKNOWN1"]);
        assert!(!group.left);
        // Same setup again: no change.
        assert!(!run(&store, CREATOR, CREATOR, setup(&[BOB, USER, "UNKNOWN1"])).groups_changed);

        // Name and text from members; text from a stranger triggers a sync request.
        run(
            &store,
            CREATOR,
            CREATOR,
            GroupMessageBody::Name(GroupNameMessage {
                name: "Crew".into(),
            }),
        );
        assert_eq!(store.group(&key).unwrap().unwrap().name, "Crew");
        let added = run(
            &store,
            BOB,
            CREATOR,
            GroupMessageBody::Text(TextMessage {
                text: "ahoy".into(),
            }),
        )
        .added;
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].sender.as_deref(), Some(BOB));
        assert!(
            run(
                &store,
                "STRANGER",
                CREATOR,
                GroupMessageBody::Text(TextMessage { text: "x".into() })
            )
            .send
        );

        // Bob leaves.
        run(
            &store,
            BOB,
            CREATOR,
            GroupMessageBody::Leave(GroupLeaveMessage),
        );
        assert_eq!(
            store.group(&key).unwrap().unwrap().members,
            vec![CREATOR, "UNKNOWN1"]
        );

        // We are removed: left; then messages get a group-leave in reply.
        run(&store, CREATOR, CREATOR, setup(&["UNKNOWN1"]));
        assert!(store.group(&key).unwrap().unwrap().left);
        let before = store.outbox().unwrap().len();
        assert!(
            run(
                &store,
                "UNKNOWN1",
                CREATOR,
                GroupMessageBody::Text(TextMessage { text: "?".into() })
            )
            .send
        );
        let kinds = outbox_kinds(&store);
        assert_eq!(kinds.len(), before + 1);
        assert!(kinds.contains(&(OUTBOX_GROUP_LEAVE.into(), vec!["UNKNOWN1".into()])));
    }

    fn run_reflected(
        store: &Store,
        sender: &str,
        creator: &str,
        body: GroupMessageBody,
    ) -> GroupOutcome {
        receive(
            store,
            USER,
            sender,
            rand::random(),
            1,
            IncomingGroupMessageBody {
                group_identity: gid(creator),
                body,
            },
            true,
            &|_| true,
            &|_| Ok(false),
            &|m| format!("~{m}"),
        )
        .unwrap()
    }

    fn stored_group(store: &Store, creator: &str, members: &[&str], left: bool) -> String {
        let group_key = key(&gid(creator));
        store
            .upsert_group(&Group {
                key: group_key.clone(),
                identity: gid(creator),
                name: "Crew".into(),
                members: members.iter().map(ToString::to_string).collect(),
                left,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        group_key
    }

    #[test]
    fn reflected_group_control_changes_nothing_and_gets_no_answer() {
        let (_dir, store) = store();
        let own = stored_group(&store, USER, &[BOB], false);
        let before = store.group(&own).unwrap();
        let outcome = run_reflected(
            &store,
            BOB,
            USER,
            GroupMessageBody::SyncRequest(GroupSyncRequestMessage),
        );
        assert!(!outcome.send);
        assert!(outbox_kinds(&store).is_empty());

        let theirs = stored_group(&store, CREATOR, &[CREATOR, BOB], false);
        for body in [
            setup(&[USER]),
            GroupMessageBody::Name(GroupNameMessage { name: "New".into() }),
            GroupMessageBody::Leave(GroupLeaveMessage),
        ] {
            let sender = if matches!(body, GroupMessageBody::Leave(_)) {
                BOB
            } else {
                CREATOR
            };
            let outcome = run_reflected(&store, sender, CREATOR, body);
            assert!(outcome.added.is_empty() && !outcome.groups_changed);
            assert!(outcome.change.is_empty());
        }
        assert_eq!(store.group(&own).unwrap(), before);
        let group = store.group(&theirs).unwrap().unwrap();
        assert_eq!(group.name, "Crew");
        assert_eq!(group.members, vec![CREATOR, BOB]);
        assert!(outbox_kinds(&store).is_empty());
    }

    #[test]
    fn reflected_content_is_stored_without_asking_anyone() {
        let (_dir, store) = store();
        let text = || GroupMessageBody::Text(TextMessage { text: "hi".into() });
        // A group we don't know: not stored, and the creator isn't asked.
        assert!(run_reflected(&store, BOB, CREATOR, text()).added.is_empty());
        assert!(outbox_kinds(&store).is_empty());

        // The leader let it through, whatever our lists say.
        stored_group(&store, CREATOR, &[CREATOR], false);
        assert_eq!(run_reflected(&store, BOB, CREATOR, text()).added.len(), 1);
        stored_group(&store, CREATOR, &[CREATOR, BOB], true);
        assert_eq!(run_reflected(&store, BOB, CREATOR, text()).added.len(), 1);
        assert!(outbox_kinds(&store).is_empty());
    }

    #[test]
    fn a_setup_brings_a_missing_creator_back_without_a_line() {
        let (_dir, store) = store();
        let group_key = stored_group(&store, CREATOR, &[BOB], false);
        let outcome = run(&store, CREATOR, CREATOR, setup(&[USER, BOB]));
        assert_eq!(
            store.group(&group_key).unwrap().unwrap().members,
            vec![CREATOR, BOB]
        );
        assert!(outcome.added.is_empty());
    }

    #[test]
    fn being_removed_is_told_apart_from_leaving() {
        let (_dir, store) = store();
        let group_key = stored_group(&store, CREATOR, &[CREATOR, BOB], false);
        run(&store, CREATOR, CREATOR, setup(&[BOB]));
        assert!(store.group(&group_key).unwrap().unwrap().left);
        assert!(store.group_kicked(&group_key).unwrap());
        // Back in: no longer removed; every later write of the group keeps that.
        run(&store, CREATOR, CREATOR, setup(&[USER, BOB]));
        let mut group = store.group(&group_key).unwrap().unwrap();
        assert!(!group.left);
        assert!(!store.group_kicked(&group_key).unwrap());
        // Having left ourselves is not being removed, even when a setup without us follows.
        group.left = true;
        store.upsert_group(&group).unwrap();
        run(&store, CREATOR, CREATOR, setup(&[BOB]));
        assert!(!store.group_kicked(&group_key).unwrap());
    }

    #[test]
    fn the_migration_repairs_groups_a_sync_took_the_creator_out_of() {
        let (_dir, store) = store();
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: USER.into(),
                client_key: zeroize::Zeroizing::new([9; 32]),
                server_group: "01".into(),
            })
            .unwrap();
        let damaged = stored_group(&store, CREATOR, &[BOB, "00000EVE"], false);
        let emptied = {
            let group = Group {
                key: key(&GroupIdentity {
                    creator_identity: id(CREATOR),
                    group_id: 2,
                }),
                identity: GroupIdentity {
                    creator_identity: id(CREATOR),
                    group_id: 2,
                },
                name: String::new(),
                members: Vec::new(),
                left: false,
                created_at: 1,
                picture: None,
                picture_version: 0,
            };
            store.upsert_group(&group).unwrap();
            group.key
        };
        let notes = stored_group(&store, USER, &[], false);
        let fine = stored_group(&store, BOB, &[BOB, CREATOR], false);
        let left = stored_group(&store, BLOCKED, &[CREATOR], true);
        let before = |key: &str| store.group(key).unwrap().unwrap().members;
        let (notes_before, fine_before, left_before) =
            (before(&notes), before(&fine), before(&left));

        store
            .conn
            .execute_batch(
                "ALTER TABLE groups DROP COLUMN kicked;
                 DROP TABLE group_member_lookups; DROP TABLE group_picture_syncs;",
            )
            .unwrap();
        store.conn.execute_batch(MIGRATION_SYNC).unwrap();

        assert_eq!(before(&damaged), vec![CREATOR, BOB, "00000EVE"]);
        assert_eq!(before(&emptied), vec![CREATOR]);
        assert_eq!(before(&notes), notes_before);
        assert_eq!(before(&fine), fine_before);
        assert_eq!(before(&left), left_before);
        assert!(!store.group_kicked(&left).unwrap());
    }

    fn queued(store: &Store) -> usize {
        let mut count = 0;
        while let Some((id, _, _)) = store.next_sync().unwrap() {
            store.sync_done(id).unwrap();
            count += 1;
        }
        count
    }

    /// The whole life of a group as a leader sees it: `queued` after each message, and every step
    /// says so in its outcome.
    fn control_messages(store: &Store) -> Vec<(bool, usize)> {
        let group_key = key(&gid(CREATOR));
        store
            .upsert_contact(&crate::contacts::test_contact(CREATOR))
            .unwrap();
        let mut seen = Vec::new();
        let mut note = |outcome: GroupOutcome| {
            seen.push((outcome.sync_queued, queued(store)));
        };
        note(run(store, CREATOR, CREATOR, setup(&[USER, BOB])));
        note(run(
            store,
            CREATOR,
            CREATOR,
            setup(&[USER, BOB, "00000EVE"]),
        ));
        note(run(
            store,
            CREATOR,
            CREATOR,
            GroupMessageBody::Name(GroupNameMessage {
                name: "Crew".into(),
            }),
        ));
        note(run(
            store,
            "00000EVE",
            CREATOR,
            GroupMessageBody::Leave(GroupLeaveMessage),
        ));
        let mut group = store.group(&group_key).unwrap().unwrap();
        group.picture_version = 3;
        store.upsert_group(&group).unwrap();
        note(run(
            store,
            CREATOR,
            CREATOR,
            GroupMessageBody::DeleteProfilePicture(
                libthreema::model::message::DeleteProfilePictureMessage,
            ),
        ));
        note(run(store, CREATOR, CREATOR, setup(&[BOB])));
        // Not a change: nothing to tell.
        note(run(store, CREATOR, CREATOR, setup(&[BOB])));
        note(run(
            store,
            BOB,
            USER,
            GroupMessageBody::SyncRequest(GroupSyncRequestMessage),
        ));
        seen
    }

    #[test]
    fn every_group_change_is_queued_for_the_other_devices_while_in_a_device_group() {
        let (_dir, store) = store();
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: USER.into(),
                client_key: zeroize::Zeroizing::new([9; 32]),
                server_group: "01".into(),
            })
            .unwrap();
        store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        assert_eq!(
            control_messages(&store),
            vec![
                (true, 1),
                (true, 1),
                (true, 1),
                (true, 1),
                (true, 1),
                (true, 1),
                (false, 0),
                (false, 0),
            ]
        );
    }

    #[test]
    fn nothing_is_queued_without_a_device_group() {
        let (_dir, store) = store();
        assert!(
            control_messages(&store)
                .iter()
                .all(|step| *step == (false, 0))
        );
    }

    #[test]
    fn a_change_that_fails_to_queue_is_not_kept() {
        let (_dir, store) = store();
        // In a device group without an identity: the entry can't be built.
        store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        let attempt = receive(
            &store,
            USER,
            CREATOR,
            1,
            1,
            IncomingGroupMessageBody {
                group_identity: gid(CREATOR),
                body: setup(&[USER, BOB]),
            },
            false,
            &|_| true,
            &|_| Ok(false),
            &|m| m.to_owned(),
        );
        assert!(attempt.is_err());
        assert!(store.group(&key(&gid(CREATOR))).unwrap().is_none());
        assert!(store.next_sync().unwrap().is_none());
    }

    #[test]
    fn a_member_without_a_contact_waits_for_the_lookup() {
        let (_dir, store) = store();
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: USER.into(),
                client_key: zeroize::Zeroizing::new([9; 32]),
                server_group: "01".into(),
            })
            .unwrap();
        store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        store
            .upsert_contact(&crate::contacts::test_contact(CREATOR))
            .unwrap();
        // New group: nobody came "with the change".
        run(&store, CREATOR, CREATOR, setup(&[USER, "UNKNOWN1"]));
        assert_eq!(
            store.outstanding_member_lookups().unwrap(),
            vec!["UNKNOWN1"]
        );
        assert_eq!(
            store.take_member_lookups("UNKNOWN1").unwrap(),
            vec![(key(&gid(CREATOR)), false)]
        );
        assert!(store.outstanding_member_lookups().unwrap().is_empty());
        // Added to a group we have: they did.
        store
            .upsert_contact(&crate::contacts::test_contact(BLOCKED))
            .unwrap();
        let identity = GroupIdentity {
            creator_identity: id(BLOCKED),
            group_id: 9,
        };
        let theirs = Group {
            key: key(&identity),
            identity,
            name: String::new(),
            members: vec![BLOCKED.into()],
            left: false,
            created_at: 1,
            picture: None,
            picture_version: 0,
        };
        store.upsert_group(&theirs).unwrap();
        receive(
            &store,
            USER,
            BLOCKED,
            2,
            1,
            IncomingGroupMessageBody {
                group_identity: identity,
                body: setup(&[USER, "UNKNOWN1"]),
            },
            false,
            &|m| m != "UNKNOWN1",
            &|_| Ok(false),
            &|m| m.to_owned(),
        )
        .unwrap();
        assert_eq!(
            store.take_member_lookups("UNKNOWN1").unwrap(),
            vec![(theirs.key, true)]
        );
    }

    #[test]
    fn a_member_nobody_has_looked_up_yet_is_asked_for_by_an_identical_setup_too() {
        let (_dir, store) = store();
        run(&store, CREATOR, CREATOR, setup(&[USER, "UNKNOWN1"]));
        let again = run(&store, CREATOR, CREATOR, setup(&[USER, "UNKNOWN1"]));
        assert!(!again.groups_changed);
        assert_eq!(again.unknown_members, vec!["UNKNOWN1"]);
    }

    #[test]
    fn someone_revoked_is_not_made_a_member() {
        let (_dir, store) = store();
        let mut revoked = crate::contacts::test_contact(BOB);
        revoked.activity_state = ActivityState::Invalid;
        store.upsert_contact(&revoked).unwrap();
        run(&store, CREATOR, CREATOR, setup(&[USER, BOB, EVE]));
        let group = store.group(&key(&gid(CREATOR))).unwrap().unwrap();
        assert_eq!(group.members, vec![CREATOR, EVE]);
        // And the same list again is no change, so nobody gets a line about it.
        assert!(!run(&store, CREATOR, CREATOR, setup(&[USER, BOB, EVE])).groups_changed);
    }

    #[test]
    fn who_is_missing_a_contact_is_found_for_the_next_try() {
        let (_dir, store) = store();
        store
            .upsert_contact(&crate::contacts::test_contact(CREATOR))
            .unwrap();
        // In a group we are in, or waited for by something queued; not in a group we left.
        stored_group(&store, CREATOR, &[BOB], false);
        let left = GroupIdentity {
            creator_identity: id(CREATOR),
            group_id: 99,
        };
        store
            .upsert_group(&Group {
                key: key(&left),
                identity: left,
                name: String::new(),
                members: vec![EVE.to_owned()],
                left: true,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        store
            .group_outbox_add(
                "g:ABCD1234:1",
                &["ABCD1234".to_owned()],
                OUTBOX_GROUP_SYNC_REQUEST,
                &json!({}),
            )
            .unwrap();
        assert_eq!(store.unresolved_members().unwrap(), vec![BOB, "ABCD1234"]);
    }

    #[test]
    fn a_member_the_directory_does_not_know_stops_holding_up_the_group() {
        let (_dir, store) = store();
        store
            .upsert_contact(&crate::contacts::test_contact(BOB))
            .unwrap();
        let group_key = stored_group(&store, CREATOR, &[BOB, "UNKNOWN1"], false);
        // Bob has this one already; only the unknown member was left.
        store
            .set_pending_receivers(&group_key, 7, &["UNKNOWN1".to_owned()])
            .unwrap();
        store
            .outbox_add(&group_key, 7, OUTBOX_GROUP_NAME, &json!({"name": "x"}))
            .unwrap();
        // Bob still has to get this one.
        store
            .set_pending_receivers(&group_key, 8, &[BOB.to_owned(), "UNKNOWN1".to_owned()])
            .unwrap();
        store
            .outbox_add(&group_key, 8, OUTBOX_GROUP_NAME, &json!({"name": "y"}))
            .unwrap();
        // A message: nobody left, so the next send raises it (send_to_group).
        store
            .set_pending_receivers(&group_key, 9, &["UNKNOWN1".to_owned()])
            .unwrap();

        assert!(
            store
                .drop_invalid_members(&["UNKNOWN1".to_owned()])
                .unwrap()
        );

        let group = store.group(&group_key).unwrap().unwrap();
        assert_eq!(group.members, vec![BOB]);
        let entries: Vec<u64> = store
            .outbox()
            .unwrap()
            .iter()
            .map(|e| e.message_id)
            .collect();
        assert_eq!(entries, vec![8]);
        assert_eq!(store.pending_receivers(&group_key, 8).unwrap(), vec![BOB]);
        assert!(store.pending_receivers(&group_key, 9).unwrap().is_empty());
    }

    #[test]
    fn a_contact_that_came_meanwhile_is_not_dropped() {
        let (_dir, store) = store();
        store
            .upsert_contact(&crate::contacts::test_contact(BOB))
            .unwrap();
        let group_key = stored_group(&store, CREATOR, &[BOB], false);
        assert!(!store.drop_invalid_members(&[BOB.to_owned()]).unwrap());
        assert_eq!(store.group(&group_key).unwrap().unwrap().members, vec![BOB]);
    }

    #[test]
    fn a_sync_request_to_a_creator_without_a_contact_asks_for_their_lookup() {
        let (_dir, store) = store();
        let unknown = run(
            &store,
            BOB,
            CREATOR,
            GroupMessageBody::Text(TextMessage { text: "hi".into() }),
        );
        assert!(unknown.send);
        assert_eq!(unknown.unknown_members, vec![CREATOR]);

        // The directory doesn't know them: nothing stays behind.
        assert!(!store.drop_invalid_members(&[CREATOR.to_owned()]).unwrap());
        assert!(store.outbox().unwrap().is_empty());
        assert!(store.unresolved_members().unwrap().is_empty());

        // With a contact there is nothing to look up.
        store
            .upsert_contact(&crate::contacts::test_contact(EVE))
            .unwrap();
        let known = run(
            &store,
            BOB,
            EVE,
            GroupMessageBody::Text(TextMessage { text: "hi".into() }),
        );
        assert!(known.send);
        assert!(known.unknown_members.is_empty());
    }

    fn text_in_a_made_up_group(store: &Store, group_id: u64) -> GroupOutcome {
        receive(
            store,
            USER,
            BOB,
            rand::random(),
            1,
            IncomingGroupMessageBody {
                group_identity: GroupIdentity {
                    creator_identity: id(CREATOR),
                    group_id,
                },
                body: GroupMessageBody::Text(TextMessage { text: "x".into() }),
            },
            false,
            &|_| true,
            &|_| Ok(false),
            &|m| m.to_owned(),
        )
        .unwrap()
    }

    #[test]
    fn made_up_groups_can_only_queue_so_many_sync_requests() {
        let (_dir, store) = store();
        let max = u64::try_from(SYNC_REQUESTS_QUEUED_MAX).unwrap();
        for group_id in 0..max {
            assert!(text_in_a_made_up_group(&store, group_id).send);
        }
        let over = text_in_a_made_up_group(&store, max);
        assert!(!over.send);
        assert!(over.unknown_members.is_empty());
        assert_eq!(store.outbox().unwrap().len(), usize::try_from(max).unwrap());
        // Not marked as asked: with room again, it can ask.
        store.drop_invalid_members(&[CREATOR.to_owned()]).unwrap();
        assert!(store.outbox().unwrap().is_empty());
        assert!(text_in_a_made_up_group(&store, max).send);
    }

    #[test]
    fn a_deleted_group_leaves_nothing_of_itself() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let key = "g:ECHOECHO:0000000000000001";
        let other = "g:ECHOECHO:0000000000000002";
        for key in [key, other] {
            store.set_pinned(key, true).unwrap();
            store
                .set_setting(&format!("draft:{key}"), "unsent")
                .unwrap();
            store
                .set_chat_notifications(key, Some((crate::chat_notifications::Policy::Never, None)))
                .unwrap();
            store.set_chat_tone(key, Some("a.ogg")).unwrap();
            assert!(store.resync_allowed(key).unwrap());
            assert!(store.resync_allowed(&format!("{key}|00000BOB")).unwrap());
        }
        let call = crate::group_calls::RunningCall {
            call_id: [1; 32],
            conversation: key.into(),
            gck: [2; 32],
            sfu_base_url: "https://sfu.threema.ch".into(),
            protocol_version: 1,
            received_at: 5,
            failed: 0,
        };
        store.add_group_call(&call).unwrap();

        store.delete_group(key).unwrap();

        assert!(store.conn.is_autocommit(), "no transaction left open");
        assert!(store.group_calls(key).unwrap().is_empty());
        assert_eq!(store.pinned_chats().unwrap(), [other]);
        assert_eq!(store.setting(&format!("draft:{key}")).unwrap(), None);
        assert_eq!(store.chat_notifications(key).unwrap(), None);
        assert_eq!(store.chat_tones().unwrap().len(), 1);
        assert!(store.resync_allowed(key).unwrap());
        assert!(store.resync_allowed(&format!("{key}|00000BOB")).unwrap());
        // The other group's marks are its own.
        assert!(!store.resync_allowed(other).unwrap());
        assert!(!store.resync_allowed(&format!("{other}|00000BOB")).unwrap());
    }

    #[test]
    fn resync_marks_past_the_hour_go() {
        let (_dir, store) = store();
        store
            .conn
            .execute(
                "INSERT INTO group_resyncs (key, at) VALUES ('old', ?1)",
                [now_ms() - RESYNC_INTERVAL_MS - 1],
            )
            .unwrap();
        assert!(store.resync_allowed("new").unwrap());
        let marks: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM group_resyncs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(marks, 1);
    }

    #[test]
    fn an_old_outbox_entry_stops_waiting_for_someone_with_no_contact() {
        let (_dir, store) = store();
        store
            .upsert_contact(&crate::contacts::test_contact(BOB))
            .unwrap();
        let everyone = vec![BOB.to_owned(), "UNKNOWN1".to_owned()];
        let group_key = stored_group(&store, CREATOR, &[BOB], false);
        store
            .group_outbox_add(
                &group_key,
                &everyone,
                OUTBOX_GROUP_NAME,
                &json!({"name": "a"}),
            )
            .unwrap();
        store
            .group_outbox_add(
                "g:ABCD1234:1",
                &["UNKNOWN1".to_owned()],
                OUTBOX_GROUP_SYNC_REQUEST,
                &json!({}),
            )
            .unwrap();
        // Too recent: still waiting.
        assert_eq!(
            store.expire_unresolved_outbox(now_ms() - 60_000).unwrap(),
            0
        );
        assert_eq!(store.outbox().unwrap().len(), 2);

        assert_eq!(
            store.expire_unresolved_outbox(now_ms() + 60_000).unwrap(),
            2
        );
        // Bob still gets his; the request nobody is left for goes.
        let left = store.outbox().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(
            store
                .pending_receivers(&group_key, left[0].message_id)
                .unwrap(),
            vec![BOB]
        );
    }

    #[test]
    fn a_group_we_left_still_names_its_people() {
        let (_dir, store) = store();
        stored_group(&store, CREATOR, &[BOB], true);
        assert!(store.is_named_by_a_group(BOB).unwrap());
        assert!(store.is_named_by_a_group(CREATOR).unwrap());
        assert!(!store.is_named_by_a_group(EVE).unwrap());
        // Unlike a group we are in, it doesn't let them past "block unknown senders".
        assert!(!store.is_group_member(BOB).unwrap());
    }

    #[test]
    fn deleting_a_group_keeps_only_telling_them_we_are_gone() {
        let (_dir, store) = store();
        let group_key = stored_group(&store, CREATOR, &[BOB], true);
        let bob = vec![BOB.to_owned()];
        store
            .group_outbox_add(&group_key, &bob, OUTBOX_GROUP_NAME, &json!({"name": "x"}))
            .unwrap();
        store
            .group_outbox_add(&group_key, &bob, OUTBOX_GROUP_LEAVE, &json!({}))
            .unwrap();
        store
            .group_outbox_add(
                &group_key,
                &bob,
                OUTBOX_GROUP_SETUP,
                &json!({"members": []}),
            )
            .unwrap();
        store
            .group_outbox_add(
                &group_key,
                &bob,
                OUTBOX_GROUP_SETUP,
                &json!({"members": [BOB]}),
            )
            .unwrap();
        // A message: gone with the chat.
        store.set_pending_receivers(&group_key, 5, &bob).unwrap();

        store.delete_group(&group_key).unwrap();

        assert!(store.group(&group_key).unwrap().is_none());
        assert_eq!(
            outbox_kinds(&store),
            vec![
                (OUTBOX_GROUP_LEAVE.to_owned(), bob.clone()),
                (OUTBOX_GROUP_SETUP.to_owned(), bob.clone()),
            ]
        );
        assert!(store.pending_receivers(&group_key, 5).unwrap().is_empty());

        // Acknowledged, nothing is left of it.
        for entry in store.outbox().unwrap() {
            for conversation in store.receiver_done(BOB, entry.message_id).unwrap() {
                store
                    .outbox_remove(&conversation, entry.message_id)
                    .unwrap();
            }
        }
        assert!(store.outbox().unwrap().is_empty());
    }

    #[test]
    fn a_new_picture_is_owed_to_the_other_devices_only_in_a_device_group() {
        let (_dir, store) = store();
        let group_key = stored_group(&store, CREATOR, &[CREATOR], false);
        let group = store.group(&group_key).unwrap().unwrap();
        store.store_new_group_picture(&group).unwrap();
        assert!(store.group_picture_syncs().unwrap().is_empty());
        store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        store.store_new_group_picture(&group).unwrap();
        assert_eq!(
            store.group_picture_syncs().unwrap(),
            vec![group_key.clone()]
        );
        assert!(store.group_picture_synced(&group_key).unwrap());
        assert!(!store.group_picture_synced(&group_key).unwrap());
        assert!(store.group_picture_syncs().unwrap().is_empty());
    }

    #[test]
    fn a_deleted_picture_is_no_longer_owed_and_its_file_goes() {
        let (_dir, store) = store();
        store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: USER.into(),
                client_key: zeroize::Zeroizing::new([9; 32]),
                server_group: "01".into(),
            })
            .unwrap();
        let group_key = stored_group(&store, CREATOR, &[CREATOR], false);
        let mut group = store.group(&group_key).unwrap().unwrap();
        group.picture_version = 2;
        store.store_new_group_picture(&group).unwrap();
        let outcome = run(
            &store,
            CREATOR,
            CREATOR,
            GroupMessageBody::DeleteProfilePicture(
                libthreema::model::message::DeleteProfilePictureMessage,
            ),
        );
        assert_eq!(outcome.remove_picture, Some(group_key));
        assert!(store.group_picture_syncs().unwrap().is_empty());
    }

    #[test]
    fn nothing_owed_stays_behind_a_deleted_group_or_a_wipe() {
        let (_dir, store) = store();
        let owed = |store: &Store| {
            (
                store.outstanding_member_lookups().unwrap().len(),
                store.group_picture_syncs().unwrap().len(),
            )
        };
        store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        let group_key = stored_group(&store, CREATOR, &[CREATOR], true);
        let note = |store: &Store| {
            let group = store.group(&group_key).unwrap();
            store
                .store_new_group_picture(group.as_ref().unwrap())
                .unwrap();
            store.add_member_lookup(&group_key, BOB, true).unwrap();
        };
        note(&store);
        assert_eq!(owed(&store), (1, 1));
        store.delete_group(&group_key).unwrap();
        assert_eq!(owed(&store), (0, 0));

        stored_group(&store, CREATOR, &[CREATOR], true);
        note(&store);
        store.wipe().unwrap();
        assert_eq!(owed(&store), (0, 0));
    }

    #[test]
    fn a_later_setup_does_not_take_back_that_someone_came() {
        let (_dir, store) = store();
        stored_group(&store, CREATOR, &[CREATOR], false);
        let group_key = key(&gid(CREATOR));
        store.add_member_lookup(&group_key, BOB, true).unwrap();
        store.add_member_lookup(&group_key, BOB, false).unwrap();
        assert_eq!(
            store.take_member_lookups(BOB).unwrap(),
            vec![(group_key.clone(), true)]
        );
        store.add_member_lookup(&group_key, BOB, false).unwrap();
        store.add_member_lookup(&group_key, BOB, true).unwrap();
        assert_eq!(
            store.take_member_lookups(BOB).unwrap(),
            vec![(group_key, true)]
        );
    }

    #[test]
    fn what_a_message_changed_is_said_for_the_other_devices() {
        let (_dir, store) = store();
        let outcome = run(&store, CREATOR, CREATOR, setup(&[USER, BOB]));
        assert!(outcome.change.created);

        let outcome = run(
            &store,
            CREATOR,
            CREATOR,
            GroupMessageBody::Name(GroupNameMessage {
                name: "Crew".into(),
            }),
        );
        assert!(outcome.change.name && !outcome.change.created);

        let outcome = run(&store, CREATOR, CREATOR, setup(&[USER, "UNKNOWN1"]));
        assert!(outcome.change.members && outcome.change.user_state);
        assert_eq!(
            outcome.change.member_changes,
            [
                (BOB.to_owned(), MemberChange::Kicked),
                ("UNKNOWN1".to_owned(), MemberChange::Added),
            ]
            .into_iter()
            .collect()
        );

        let outcome = run(
            &store,
            "UNKNOWN1",
            CREATOR,
            GroupMessageBody::Leave(GroupLeaveMessage),
        );
        assert!(outcome.change.members && !outcome.change.user_state);
        assert_eq!(
            outcome.change.member_changes.get("UNKNOWN1"),
            Some(&MemberChange::Left)
        );

        let group_key = key(&gid(CREATOR));
        let mut group = store.group(&group_key).unwrap().unwrap();
        group.picture_version = 3;
        store.upsert_group(&group).unwrap();
        let outcome = run(
            &store,
            CREATOR,
            CREATOR,
            GroupMessageBody::DeleteProfilePicture(
                libthreema::model::message::DeleteProfilePictureMessage,
            ),
        );
        assert!(outcome.change.picture_removed);

        // Plain content changes nothing the others need to hear of from us.
        let outcome = run(
            &store,
            CREATOR,
            CREATOR,
            GroupMessageBody::Text(TextMessage { text: "hi".into() }),
        );
        assert!(outcome.change.is_empty());

        let outcome = run(&store, CREATOR, CREATOR, setup(&[]));
        assert!(outcome.change.user_state && !outcome.change.members);
    }

    fn own_group(creator: &str, members: &[&str]) -> Group {
        Group {
            key: key(&gid(creator)),
            identity: gid(creator),
            name: "Group".into(),
            members: members.iter().map(|m| (*m).to_owned()).collect(),
            left: false,
            created_at: 1,
            picture: None,
            picture_version: 0,
        }
    }

    /// A text of ours in the group, sent: its id.
    fn sent_text(store: &Store, group: &Group) -> u64 {
        let message_id: u64 = rand::random();
        store
            .conn
            .execute(
                "INSERT INTO messages
                     (conversation, message_id, outgoing, kind, body, created_at, updated_at, state)
                 VALUES (?1, ?2, 1, 'text', 'hi', 1, 1, 1)",
                params![group.key, message_id.cast_signed()],
            )
            .unwrap();
        message_id
    }

    fn rejected(store: &Store, peer: &str, creator: &str, message_id: u64) -> GroupReject {
        message_rejected(store, USER, peer, gid(creator), message_id, &|m| {
            Ok(m == BLOCKED)
        })
        .unwrap()
    }

    #[test]
    fn a_reject_without_a_group_only_requeues_for_members() {
        let (_dir, store) = store();
        let mut group = own_group(USER, &[BOB]);
        store.upsert_group(&group).unwrap();
        let message_id = sent_text(&store, &group);
        let pending =
            |store: &Store, group: &Group| store.pending_receivers(&group.key, message_id).unwrap();

        // Someone who isn't a member, however they came by the ID.
        assert!(
            !store
                .requeue_group_receiver("STRANGER", message_id)
                .unwrap()
        );
        assert!(pending(&store, &group).is_empty());
        assert_eq!(
            store.pending_outgoing().unwrap().len(),
            0,
            "the message stays sent"
        );

        // A group we left is nobody's to ask.
        group.left = true;
        store.upsert_group(&group).unwrap();
        assert!(!store.requeue_group_receiver(BOB, message_id).unwrap());

        // A member gets it again.
        group.left = false;
        store.upsert_group(&group).unwrap();
        assert!(store.requeue_group_receiver(BOB, message_id).unwrap());
        assert_eq!(pending(&store, &group), vec![BOB.to_owned()]);
        assert_eq!(store.pending_outgoing().unwrap().len(), 1);
    }

    /// A message deleted for everyone has nothing left to send: a late Reject of it brings
    /// back neither a queue entry nor the clock, and doesn't say it is sent again.
    #[test]
    fn a_reject_of_a_message_deleted_for_everyone_changes_nothing() {
        let (_dir, store) = store();
        let group = own_group(USER, &[BOB]);
        store.upsert_group(&group).unwrap();
        let message_id = sent_text(&store, &group);
        let row = store
            .message_by_id(&group.key, true, message_id)
            .unwrap()
            .unwrap();
        store.mark_deleted(row.id, 5).unwrap();

        assert!(!store.requeue_group_receiver(BOB, message_id).unwrap());
        let reject = rejected(&store, BOB, USER, message_id);
        assert!(!reject.send && !reject.resending);
        assert!(
            store
                .pending_receivers(&group.key, message_id)
                .unwrap()
                .is_empty()
        );
        let row = store.message(row.id).unwrap().unwrap();
        assert_eq!(row.state, crate::messages::OutgoingState::Sent as i64);
        assert!(store.pending_outgoing().unwrap().is_empty());
    }

    #[test]
    fn a_reject_naming_the_group_goes_by_the_group() {
        let (_dir, store) = store();
        let group = own_group(USER, &[BOB]);
        store.upsert_group(&group).unwrap();
        let message_id = sent_text(&store, &group);

        // Ours, to a member: out again, to them alone.
        assert_eq!(
            rejected(&store, BOB, USER, message_id),
            GroupReject {
                send: true,
                resending: true,
                state_sent: false,
            }
        );
        assert_eq!(
            store.pending_receivers(&group.key, message_id).unwrap(),
            vec![BOB.to_owned()]
        );
    }

    #[test]
    fn a_rejected_message_we_dont_have_is_a_sync_request_if_the_group_is_ours() {
        let (_dir, store) = store();
        store.upsert_group(&own_group(USER, &[BOB])).unwrap();
        assert_eq!(
            rejected(&store, BOB, USER, 1),
            GroupReject {
                send: true,
                resending: false,
                state_sent: true,
            }
        );
        let kinds: Vec<_> = outbox_kinds(&store).into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            kinds,
            vec![
                OUTBOX_GROUP_SETUP,
                OUTBOX_GROUP_NAME,
                OUTBOX_GROUP_DELETE_PICTURE
            ]
        );
        // Not once for each of a burst.
        assert_eq!(rejected(&store, BOB, USER, 2), GroupReject::default());
        assert_eq!(outbox_kinds(&store).len(), 3);
    }

    #[test]
    fn a_rejected_message_we_dont_have_in_someone_elses_group_is_nothing_to_do() {
        let (_dir, store) = store();
        store
            .upsert_group(&own_group(CREATOR, &[BOB, CREATOR]))
            .unwrap();
        assert_eq!(rejected(&store, BOB, CREATOR, 1), GroupReject::default());
        assert!(outbox_kinds(&store).is_empty());
    }

    #[test]
    fn someone_elses_message_is_not_ours_to_send_again() {
        let (_dir, store) = store();
        let group = own_group(CREATOR, &[BOB, CREATOR]);
        store.upsert_group(&group).unwrap();
        let message_id = sent_text(&store, &group);
        store
            .conn
            .execute("UPDATE messages SET outgoing = 0", [])
            .unwrap();
        assert_eq!(
            rejected(&store, BOB, CREATOR, message_id),
            GroupReject::default()
        );
        assert!(
            store
                .pending_receivers(&group.key, message_id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn someone_removed_learns_it_from_their_reject() {
        let (_dir, store) = store();
        let group = own_group(USER, &[BOB]);
        store.upsert_group(&group).unwrap();
        let message_id = sent_text(&store, &group);
        assert_eq!(
            rejected(&store, "STRANGER", USER, message_id),
            GroupReject {
                send: true,
                resending: false,
                state_sent: false,
            }
        );
        assert!(
            store
                .pending_receivers(&group.key, message_id)
                .unwrap()
                .is_empty()
        );
        let last = store.outbox().unwrap().pop().unwrap();
        assert_eq!(last.kind, OUTBOX_GROUP_SETUP);
        assert_eq!(last.body["members"], json!([]));
    }

    #[test]
    fn a_reject_for_a_group_we_dont_know_asks_its_creator_unless_blocked() {
        let (_dir, store) = store();
        assert_eq!(
            rejected(&store, BOB, CREATOR, 1),
            GroupReject {
                send: true,
                resending: false,
                state_sent: false,
            }
        );
        let (_dir, store) = self::store();
        assert_eq!(rejected(&store, BOB, BLOCKED, 1), GroupReject::default());
        assert!(outbox_kinds(&store).is_empty());
    }

    #[test]
    fn who_gets_a_groups_state_is_said_for_the_running_call() {
        let (_dir, store) = store();
        let group = own_group(USER, &[BOB]);
        store.upsert_group(&group).unwrap();
        let answered = run(
            &store,
            BOB,
            USER,
            GroupMessageBody::SyncRequest(GroupSyncRequestMessage),
        );
        assert_eq!(answered.state_sent_to, vec![BOB]);

        // An empty setup, for someone not in it, isn't the group's state.
        let not_a_member = run(
            &store,
            "STRANGER",
            USER,
            GroupMessageBody::SyncRequest(GroupSyncRequestMessage),
        );
        assert!(not_a_member.send);
        assert!(not_a_member.state_sent_to.is_empty());
    }

    #[test]
    fn a_setup_listing_more_than_a_group_can_hold_changes_nothing() {
        let (_dir, store) = store();
        let names: Vec<String> = (0..256).map(|n| format!("A{n:07}")).collect();
        let mut listed: Vec<&str> = names.iter().map(String::as_str).collect();
        listed[0] = USER;

        let too_many: Vec<&str> = listed.iter().copied().chain(["B0000000"]).collect();
        let outcome = run(&store, CREATOR, CREATOR, setup(&too_many));
        assert!(outcome.unknown_members.is_empty());
        assert!(store.group(&key(&gid(CREATOR))).unwrap().is_none());

        assert!(run(&store, CREATOR, CREATOR, setup(&listed)).groups_changed);
    }

    #[test]
    fn an_id_the_directory_does_not_know_stays_out_of_a_resent_setup() {
        let (_dir, store) = store();
        run(&store, CREATOR, CREATOR, setup(&[USER, BOB, "UNKNOWN1"]));
        let group_key = key(&gid(CREATOR));
        store
            .drop_invalid_members(&["UNKNOWN1".to_owned()])
            .unwrap();
        store.note_invalid_ids(&["UNKNOWN1".to_owned()]);

        let lines = store.conversation_messages(&group_key, 100).unwrap().len();
        let again = run(&store, CREATOR, CREATOR, setup(&[USER, BOB, "UNKNOWN1"]));
        assert!(!again.groups_changed);
        assert!(again.unknown_members.is_empty());
        assert_eq!(
            store.group(&group_key).unwrap().unwrap().members,
            vec![CREATOR, BOB]
        );
        assert_eq!(
            store.conversation_messages(&group_key, 100).unwrap().len(),
            lines
        );
    }

    #[test]
    fn a_creator_the_directory_does_not_know_is_not_asked_again() {
        let (_dir, store) = store();
        store.note_invalid_ids(&[CREATOR.to_owned()]);
        let outcome = run(
            &store,
            BOB,
            CREATOR,
            GroupMessageBody::Text(TextMessage { text: "hi".into() }),
        );
        assert!(!outcome.send);
        assert!(outcome.unknown_members.is_empty());
        assert!(store.outbox().unwrap().is_empty());
    }

    #[test]
    fn someone_added_gets_their_line_once_the_directory_knows_them() {
        let (_dir, store) = store();
        let group_key = key(&gid(CREATOR));
        run(&store, CREATOR, CREATOR, setup(&[USER, BOB]));
        let added_lines = || -> Vec<String> {
            store
                .conversation_messages(&group_key, 100)
                .unwrap()
                .into_iter()
                .map(|message| message.body)
                .filter(|body| body.starts_with("groupMemberAdded"))
                .collect()
        };

        // The directory doesn't know them: no line, ever.
        let outcome = run(&store, CREATOR, CREATOR, setup(&[USER, BOB, "UNKNOWN1"]));
        assert!(outcome.added.is_empty());
        let looked_up = crate::sync::members_looked_up(
            &store,
            vec![("UNKNOWN1".to_owned(), crate::contacts::Lookup::Invalid)],
        )
        .unwrap();
        assert!(looked_up.lines.is_empty());
        assert!(added_lines().is_empty());

        // The directory knows them.
        run(&store, CREATOR, CREATOR, setup(&[USER, BOB]));
        run(&store, CREATOR, CREATOR, setup(&[USER, BOB, "UNKNOWN1"]));
        assert!(added_lines().is_empty());
        let looked_up = crate::sync::members_looked_up(
            &store,
            vec![(
                "UNKNOWN1".to_owned(),
                crate::contacts::Lookup::Found(Box::new(crate::contacts::test_contact("UNKNOWN1"))),
            )],
        )
        .unwrap();
        assert_eq!(looked_up.lines.len(), 1);
        assert_eq!(added_lines(), vec!["groupMemberAdded|UNKNOWN1"]);
        assert!(store.outstanding_member_lookups().unwrap().is_empty());
    }

    #[test]
    fn someone_gone_before_the_lookup_gets_no_line() {
        let (_dir, store) = store();
        run(&store, CREATOR, CREATOR, setup(&[USER, BOB]));
        run(&store, CREATOR, CREATOR, setup(&[USER, BOB, "UNKNOWN1"]));
        run(&store, CREATOR, CREATOR, setup(&[USER, BOB]));
        let looked_up = crate::sync::members_looked_up(
            &store,
            vec![(
                "UNKNOWN1".to_owned(),
                crate::contacts::Lookup::Found(Box::new(crate::contacts::test_contact("UNKNOWN1"))),
            )],
        )
        .unwrap();
        assert!(looked_up.lines.is_empty());
    }

    #[test]
    fn member_lookups_are_limited_per_hour() {
        let (_dir, store) = store();
        let start = 1_000_000;
        assert_eq!(store.member_lookup_allowance_at(300, start), 300);
        assert_eq!(store.member_lookup_allowance_at(300, start + 1), 212);
        assert_eq!(store.member_lookup_allowance_at(1, start + 2), 0);
        // The next hour.
        assert_eq!(
            store.member_lookup_allowance_at(5, start + MEMBER_LOOKUP_WINDOW_MS),
            5
        );
        // A clock set back doesn't hold them up.
        assert_eq!(store.member_lookup_allowance_at(7, start), 7);
    }

    #[test]
    fn creator_answers_sync_requests() {
        let (_dir, store) = store();
        let group = Group {
            key: key(&gid(USER)),
            identity: gid(USER),
            name: "Mine".into(),
            members: vec![BOB.into()],
            left: false,
            created_at: 1,
            picture: None,
            picture_version: 0,
        };
        store.upsert_group(&group).unwrap();
        assert!(
            run(
                &store,
                BOB,
                USER,
                GroupMessageBody::SyncRequest(GroupSyncRequestMessage)
            )
            .send
        );
        let kinds: Vec<String> = outbox_kinds(&store).into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            kinds,
            vec![
                OUTBOX_GROUP_SETUP,
                OUTBOX_GROUP_NAME,
                OUTBOX_GROUP_DELETE_PICTURE
            ]
        );
        // A non-member gets an empty setup.
        assert!(
            run(
                &store,
                "STRANGER",
                USER,
                GroupMessageBody::SyncRequest(GroupSyncRequestMessage)
            )
            .send
        );
        let last = store.outbox().unwrap().pop().unwrap();
        assert_eq!(last.body["members"], json!([]));
        // Pending receivers are cleared per receiver.
        let entry = &store.outbox().unwrap()[0];
        assert_eq!(
            store.receiver_done(BOB, entry.message_id).unwrap(),
            vec![group.key.clone()]
        );
    }
}
