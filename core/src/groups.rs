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
/// isn't the notes group it looked like.
pub(crate) const MIGRATION_SYNC: &str = r#"
    ALTER TABLE groups ADD COLUMN kicked INTEGER NOT NULL DEFAULT 0;
    UPDATE groups
       SET members = CASE WHEN members = '[]' THEN '["' || creator || '"]'
                          ELSE '["' || creator || '",' || substr(members, 2) END
     WHERE left_group = 0
       AND creator != (SELECT threema_id FROM identity WHERE id = 1)
       AND NOT EXISTS (SELECT 1 FROM json_each(groups.members) AS m WHERE m.value = groups.creator);"#;

/// _Recently resynced_ lasts an hour (both directions).
const RESYNC_INTERVAL_MS: i64 = 60 * 60 * 1000;

/// Outbox kinds of group control messages (`body` in brackets).
pub const OUTBOX_GROUP_SETUP: &str = "groupSetup"; // {members: [id]}
pub const OUTBOX_GROUP_NAME: &str = "groupName"; // {name}
pub const OUTBOX_GROUP_LEAVE: &str = "groupLeave";
pub const OUTBOX_GROUP_SYNC_REQUEST: &str = "groupSyncRequest";
pub const OUTBOX_GROUP_DELETE_PICTURE: &str = "groupDeleteProfilePicture";
pub const OUTBOX_GROUP_SET_PICTURE: &str = "groupSetProfilePicture"; // {blob, key, size}
pub const OUTBOX_GROUP_CALL_START: &str = "groupCallStart"; // {gck (hex), url, version}

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

    pub fn to_json(&self, user: &str, display_name: impl Fn(&str) -> String) -> Value {
        let creator = self.identity.creator_identity.to_string();
        json!({
            "key": self.key,
            "name": self.name,
            "creator": creator,
            "isCreator": creator == user,
            "left": self.left,
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

    /// Remove a group and its conversation (only once it has been left).
    pub fn delete_group(&self, key: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM groups WHERE key = ?1", [key])?;
        self.conn.execute(
            "DELETE FROM pending_receivers WHERE conversation = ?1",
            [key],
        )?;
        self.delete_conversation(key)
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

    /// `peer` rejected `message_id` (lost FS session): if it is one of our group messages (or a
    /// group outbox entry), it has to get it again. Returns whether something was requeued.
    pub fn requeue_group_receiver(&self, peer: &str, message_id: u64) -> Result<bool, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT conversation FROM messages
                 WHERE outgoing = 1 AND message_id = ?1 AND conversation LIKE 'g:%'
             UNION SELECT conversation FROM outbox
                 WHERE message_id = ?1 AND conversation LIKE 'g:%'",
        )?;
        let conversations: Vec<String> = statement
            .query_map([message_id.cast_signed()], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        for conversation in &conversations {
            self.set_pending_receivers(conversation, message_id, &[peer.to_owned()])?;
            self.conn.execute(
                "UPDATE messages SET state = 0, sent_at = NULL WHERE conversation = ?1 AND outgoing = 1
                     AND message_id = ?2 AND state = 1",
                params![conversation, message_id.cast_signed()],
            )?;
        }
        Ok(!conversations.is_empty())
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
        self.conn.execute(
            "INSERT OR REPLACE INTO group_resyncs (key, at) VALUES (?1, ?2)",
            params![mark, now],
        )?;
        Ok(true)
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
    /// A group call was announced (checked and stored by the app).
    pub call_start: Option<libthreema::model::message::GroupCallStartMessage>,
}

/// Group receive steps; `user` is our identity, `contact_exists` whether we have a contact (any
/// acquaintance) for an identity, `blocked` whether the user blocked one explicitly (ADR 0019),
/// `name_of` its display name (for status lines).
///
/// `from_sync`: another device, the leader, reflected this message after running these steps
/// itself and answering whatever needed an answer. Group control is discarded then, like Android's
/// `executeMessageStepsFromSync`: the leader reflects a `GroupSync` for it (csp.struct.yml), and
/// that makes the status lines. Content is simply stored, and nothing goes out from here.
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
            }
            outcome.send = true;
            return Ok(outcome);
        }
        _ => {}
    }

    // Common Group Receive Steps. A group we don't know is none of a blocked creator's business:
    // asking them would bring it back (ADR 0019).
    let Some(mut group) = group else {
        if !user_is_creator && !blocked(&creator)? {
            sync_request(store, &key, &creator, &mut outcome)?;
        }
        return Ok(outcome);
    };
    let reply_to = vec![sender.to_owned()];
    if group.left && !from_sync {
        if user_is_creator {
            store.group_outbox_add(
                &key,
                &reply_to,
                OUTBOX_GROUP_SETUP,
                &json!({ "members": [] }),
            )?;
        } else {
            store.group_outbox_add(&key, &reply_to, OUTBOX_GROUP_LEAVE, &json!({}))?;
        }
        outcome.send = true;
        return Ok(outcome);
    }
    if !group.members.iter().any(|m| m == sender) && !from_sync {
        if user_is_creator {
            store.group_outbox_add(
                &key,
                &reply_to,
                OUTBOX_GROUP_SETUP,
                &json!({ "members": [] }),
            )?;
            outcome.send = true;
        } else {
            sync_request(store, &key, &creator, &mut outcome)?;
        }
        return Ok(outcome);
    }

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
            if let Some(original) =
                own_message_of(store, &key, sender, edit.message_id.0)?.filter(|m| m.kind == "text")
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
                .filter(|m| !matches!(m.kind.as_str(), "status" | "deleted"))
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
                outcome.download_picture = Some(key.clone());
            }
        }
        GroupMessageBody::DeleteProfilePicture(_) => {
            if group.picture.is_some() || group.picture_version != 0 {
                group.picture = None;
                group.picture_version = 0;
                store.upsert_group(&group)?;
                outcome.groups_changed = true;
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

/// _Group Sync Request Steps_ (we are not the creator).
fn sync_request(
    store: &Store,
    key: &str,
    creator: &str,
    outcome: &mut GroupOutcome,
) -> Result<(), StoreError> {
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
    Ok(())
}

/// The creator sends a group's state to `receivers`: setup, name, picture.
pub fn send_state(store: &Store, group: &Group, receivers: &[String]) -> Result<(), StoreError> {
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
    )?;
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
    let members: BTreeSet<String> = members
        .into_iter()
        .map(|m| m.to_string())
        .filter(|m| *m != creator)
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
        }
        let code = if members.is_empty() {
            "groupDisbanded"
        } else {
            "groupRemoved"
        };
        status(store, key, code, outcome)?;
        outcome.groups_changed = true;
        return Ok(());
    }
    // Everyone but us, the creator included.
    let mut new_members: Vec<String> = members.into_iter().filter(|m| m != user).collect();
    new_members.insert(0, creator.clone());
    outcome.unknown_members = new_members
        .iter()
        .filter(|m| !contact_exists(m))
        .cloned()
        .collect();
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
            if rejoined {
                status(store, key, "groupJoined", outcome)?;
            } else {
                for member in added {
                    status(
                        store,
                        key,
                        &format!("groupMemberAdded|{}", name_of(&member)),
                        outcome,
                    )?;
                }
                for member in removed {
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
            .execute_batch("ALTER TABLE groups DROP COLUMN kicked;")
            .unwrap();
        store.conn.execute_batch(MIGRATION_SYNC).unwrap();

        assert_eq!(before(&damaged), vec![CREATOR, BOB, "00000EVE"]);
        assert_eq!(before(&emptied), vec![CREATOR]);
        assert_eq!(before(&notes), notes_before);
        assert_eq!(before(&fine), fine_before);
        assert_eq!(before(&left), left_before);
        assert!(!store.group_kicked(&left).unwrap());
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
