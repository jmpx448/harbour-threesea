// SPDX-License-Identifier: AGPL-3.0-only
//! Message storage: 1:1 conversations, delivery states, used nonces.

use rusqlite::{OptionalExtension as _, Row, params};
use serde_json::{Value, json};

use crate::store::{Store, StoreError};

/// Schema for messages and nonces (store migration 3).
pub(crate) const MIGRATION: &str = "
    CREATE TABLE messages (
        id            INTEGER PRIMARY KEY,
        conversation  TEXT NOT NULL,
        message_id    INTEGER NOT NULL,
        outgoing      INTEGER NOT NULL,
        kind          TEXT NOT NULL,
        body          TEXT NOT NULL,
        created_at    INTEGER NOT NULL,
        updated_at    INTEGER NOT NULL,
        state         INTEGER NOT NULL,
        UNIQUE (conversation, outgoing, message_id)
    );
    CREATE INDEX messages_by_conversation ON messages (conversation, created_at);
    CREATE TABLE nonces (
        nonce BLOB PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;";

/// When a message was last edited, or (kind `deleted`) removed (store migration 6).
pub(crate) const MIGRATION_EDITS: &str = "ALTER TABLE messages ADD COLUMN edited_at INTEGER;";

/// Starred messages (Android's `DISPLAY_TAG_STARRED`; local only).
pub(crate) const MIGRATION_STARRED: &str =
    "ALTER TABLE messages ADD COLUMN starred INTEGER NOT NULL DEFAULT 0;";

/// When an outgoing message was accepted by the server, delivered and read; when an incoming
/// one arrived here and was read (store migration 17, Android's message details). Unknown for
/// older messages and for states skipped (a "read" receipt without "received" before it).
pub(crate) const MIGRATION_TIMES: &str = "
    ALTER TABLE messages ADD COLUMN sent_at INTEGER;
    ALTER TABLE messages ADD COLUMN delivered_at INTEGER;
    ALTER TABLE messages ADD COLUMN read_at INTEGER;
    ALTER TABLE messages ADD COLUMN received_at INTEGER;";

/// Chats emptied by the user (Android's "Empty chat"): they stay in the chat list without
/// messages, from the time they were emptied (store migration 20).
pub(crate) const MIGRATION_EMPTIED: &str = "
    CREATE TABLE emptied_chats (
        conversation  TEXT PRIMARY KEY NOT NULL,
        at            INTEGER NOT NULL
    ) WITHOUT ROWID;";

/// Indexes for the chat list and a chat's pages (store migration 26): a chat's messages in local
/// order (its newest one, a page of them) and its unread ones, without scanning the table.
pub(crate) const MIGRATION_INDEXES: &str = "
    CREATE INDEX messages_by_conversation_id ON messages (conversation, id);
    CREATE INDEX messages_unread ON messages (conversation) WHERE outgoing = 0 AND state = 0;";

/// A history position separate from the row id (store migration 28). Live messages keep their
/// arrival/sending order: the trigger gives every ordinary insert `sort_key = id << 20`, so
/// nothing changes for them and every existing insert path stays as it is. Imported history
/// (an Android backup) is placed before/between live messages with explicit keys in the gaps.
/// Row ids stay stable (media files and UI references use them) and keep meaning "inserted
/// later" — which, with imports, is no longer "later in the conversation". Key 0 is reserved
/// for "let the trigger assign".
/// Search index (P02, deep audit 2026-10-27): a trigram full-text index over what search
/// looks at — a text's body, a file's caption and name, a place's name and address, a poll's
/// question — inside the encrypted database, kept in step by triggers so no write path has
/// to know. Contentless: it holds only the index, the rows stay in `messages`. Messages with
/// mentions get a partial index: their names are not in the body, so search always checks
/// them. Search uses the index only to find candidates; the exact check stays in Rust.
pub(crate) const MIGRATION_SEARCH: &str = "
    CREATE VIRTUAL TABLE messages_search USING fts5(
        content, content='', contentless_delete=1, tokenize='trigram');
    INSERT INTO messages_search (rowid, content)
        SELECT id, CASE WHEN kind = 'text' THEN body
                        WHEN json_valid(body) THEN concat_ws(' ',
                            json_extract(body, '$.caption'), json_extract(body, '$.name'),
                            json_extract(body, '$.address'), json_extract(body, '$.description'))
                        ELSE '' END
        FROM messages WHERE kind IN ('text', 'file', 'location', 'poll');
    CREATE INDEX messages_mentions ON messages (id) WHERE body LIKE '%@[%';
    CREATE TRIGGER messages_search_insert AFTER INSERT ON messages
        WHEN NEW.kind IN ('text', 'file', 'location', 'poll')
    BEGIN
        INSERT INTO messages_search (rowid, content) VALUES (NEW.id,
            CASE WHEN NEW.kind = 'text' THEN NEW.body
                 WHEN json_valid(NEW.body) THEN concat_ws(' ',
                     json_extract(NEW.body, '$.caption'), json_extract(NEW.body, '$.name'),
                     json_extract(NEW.body, '$.address'), json_extract(NEW.body, '$.description'))
                 ELSE '' END);
    END;
    CREATE TRIGGER messages_search_update AFTER UPDATE OF kind, body ON messages
    BEGIN
        DELETE FROM messages_search WHERE rowid = OLD.id;
        INSERT INTO messages_search (rowid, content)
            SELECT NEW.id,
                CASE WHEN NEW.kind = 'text' THEN NEW.body
                     WHEN json_valid(NEW.body) THEN concat_ws(' ',
                         json_extract(NEW.body, '$.caption'), json_extract(NEW.body, '$.name'),
                         json_extract(NEW.body, '$.address'), json_extract(NEW.body, '$.description'))
                     ELSE '' END
            WHERE NEW.kind IN ('text', 'file', 'location', 'poll');
    END;
    CREATE TRIGGER messages_search_delete AFTER DELETE ON messages
    BEGIN
        DELETE FROM messages_search WHERE rowid = OLD.id;
    END;";

/// One row per chat with what the chat list needs without looking at the messages themselves
/// (store migration 33, the deep audit's per-chat summary): the newest message in history
/// order and how many incoming messages are unread. SQL triggers keep it in step — like the
/// search index, so no write path has to know about it, which was the risk in maintaining it
/// by hand.
///
/// `last_sort` holds that message's `sort_key`. A live insert has none yet (the `sort_key`
/// trigger assigns it right after), so the insert trigger may leave the summary pointing at
/// the previous message and the `sort_key` trigger fixes it a moment later, inside the same
/// statement. A message never changes its conversation, so no trigger watches that column.
pub(crate) const MIGRATION_CHAT_SUMMARIES: &str = "
    CREATE TABLE chat_summaries (
        chat       TEXT PRIMARY KEY NOT NULL,
        last_id    INTEGER NOT NULL,
        last_sort  INTEGER NOT NULL,
        unread     INTEGER NOT NULL
    ) WITHOUT ROWID;

    INSERT INTO chat_summaries (chat, last_id, last_sort, unread)
        SELECT conversation,
               (SELECT n.id FROM messages n WHERE n.conversation = m.conversation
                ORDER BY n.sort_key DESC, n.id DESC LIMIT 1),
               (SELECT n.sort_key FROM messages n WHERE n.conversation = m.conversation
                ORDER BY n.sort_key DESC, n.id DESC LIMIT 1),
               sum(CASE WHEN outgoing = 0 AND state = 0 THEN 1 ELSE 0 END)
        FROM messages m GROUP BY conversation;

    CREATE TRIGGER chat_summaries_insert AFTER INSERT ON messages
    BEGIN
        INSERT INTO chat_summaries (chat, last_id, last_sort, unread)
            VALUES (NEW.conversation, NEW.id, NEW.sort_key,
                    CASE WHEN NEW.outgoing = 0 AND NEW.state = 0 THEN 1 ELSE 0 END)
        ON CONFLICT(chat) DO UPDATE SET
            unread = unread + (CASE WHEN NEW.outgoing = 0 AND NEW.state = 0 THEN 1 ELSE 0 END),
            last_id = CASE WHEN NEW.sort_key > last_sort
                                OR (NEW.sort_key = last_sort AND NEW.id > last_id)
                           THEN NEW.id ELSE last_id END,
            last_sort = max(last_sort, NEW.sort_key);
    END;

    CREATE TRIGGER chat_summaries_sort AFTER UPDATE OF sort_key ON messages
    BEGIN
        UPDATE chat_summaries SET last_id = NEW.id, last_sort = NEW.sort_key
         WHERE chat = NEW.conversation
           AND (NEW.sort_key > last_sort OR (NEW.sort_key = last_sort AND NEW.id > last_id));
        UPDATE chat_summaries
           SET last_id = (SELECT n.id FROM messages n WHERE n.conversation = NEW.conversation
                          ORDER BY n.sort_key DESC, n.id DESC LIMIT 1),
               last_sort = (SELECT n.sort_key FROM messages n WHERE n.conversation = NEW.conversation
                            ORDER BY n.sort_key DESC, n.id DESC LIMIT 1)
         WHERE chat = NEW.conversation AND last_id = NEW.id;
    END;

    CREATE TRIGGER chat_summaries_state AFTER UPDATE OF state, outgoing ON messages
    BEGIN
        UPDATE chat_summaries
           SET unread = max(0, unread
                - (CASE WHEN OLD.outgoing = 0 AND OLD.state = 0 THEN 1 ELSE 0 END)
                + (CASE WHEN NEW.outgoing = 0 AND NEW.state = 0 THEN 1 ELSE 0 END))
         WHERE chat = NEW.conversation;
    END;

    CREATE TRIGGER chat_summaries_delete AFTER DELETE ON messages
    BEGIN
        UPDATE chat_summaries
           SET unread = max(0, unread
                    - (CASE WHEN OLD.outgoing = 0 AND OLD.state = 0 THEN 1 ELSE 0 END)),
               last_id = CASE WHEN last_id = OLD.id
                              THEN coalesce((SELECT n.id FROM messages n
                                             WHERE n.conversation = OLD.conversation
                                             ORDER BY n.sort_key DESC, n.id DESC LIMIT 1), -1)
                              ELSE last_id END,
               last_sort = CASE WHEN last_id = OLD.id
                                THEN coalesce((SELECT n.sort_key FROM messages n
                                               WHERE n.conversation = OLD.conversation
                                               ORDER BY n.sort_key DESC, n.id DESC LIMIT 1), 0)
                                ELSE last_sort END
         WHERE chat = OLD.conversation;
        -- Nothing left in the chat: no summary either.
        DELETE FROM chat_summaries WHERE chat = OLD.conversation AND last_id = -1;
    END;";

/// Chats the user marked unread by hand (store migration 32), like Android's conversation tag
/// MARKED_AS_UNREAD: local only (Android does not reflect it either), cleared as soon as the
/// chat is read or a new message arrives in it.
pub(crate) const MIGRATION_MARKED_UNREAD: &str = "
    CREATE TABLE marked_unread_chats (
        conversation  TEXT PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;";

pub(crate) const MIGRATION_POSITIONS: &str = "
    ALTER TABLE messages ADD COLUMN sort_key INTEGER NOT NULL DEFAULT 0;
    UPDATE messages SET sort_key = id << 20;
    CREATE INDEX messages_position ON messages (conversation, sort_key, id);
    CREATE TRIGGER messages_sort_key AFTER INSERT ON messages
        WHEN NEW.sort_key = 0
    BEGIN
        UPDATE messages SET sort_key = NEW.id << 20 WHERE id = NEW.id;
    END;";

/// Edits and deletions for everyone are only offered for our messages younger than this
/// (spec: applied on the sender side only).
pub const EDIT_DELETE_MAX_AGE_MS: i64 = 6 * 60 * 60 * 1000;

/// Delivery state of an outgoing message (Threema's ticks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum OutgoingState {
    /// Waiting to be sent (offline or not acknowledged yet).
    Pending = 0,
    /// Accepted by the chat server.
    Sent = 1,
    /// Received by the recipient's device.
    Delivered = 2,
    /// Read by the recipient.
    Read = 3,
    /// Could not be sent (e.g. recipient revoked).
    Failed = 4,
}

/// Incoming message state.
pub const INCOMING_UNREAD: i64 = 0;
pub const INCOMING_READ: i64 = 1;

/// A stored message.
#[derive(Debug, Clone)]
pub struct Message {
    pub id: i64,
    pub conversation: String,
    pub message_id: u64,
    pub outgoing: bool,
    pub kind: String,
    pub body: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub state: i64,
    /// Edited (or, kind `deleted`, removed) at, ms.
    pub edited_at: Option<i64>,
    /// Sender of an incoming group message (`None` for 1:1 and our own).
    pub sender: Option<String>,
    /// `[{sender, emoji}]`, oldest first; `sender` is empty for our own reactions.
    pub reactions: Value,
    pub starred: bool,
    /// Outgoing: accepted by the server, delivered, read (ms; `None` if unknown).
    pub sent_at: Option<i64>,
    pub delivered_at: Option<i64>,
    /// Outgoing: read by the recipient; incoming: read by us.
    pub read_at: Option<i64>,
    /// Incoming: arrived on this device.
    pub received_at: Option<i64>,
    /// History position within the conversation (ties broken by `id`): live messages in
    /// arrival order, imported history slotted between them. Internal; not part of the UI
    /// JSON.
    pub sort_key: i64,
    /// Outgoing: a send has begun, so someone may have it even while it is pending (see
    /// [`MIGRATION_WRITTEN_OUT`]).
    pub written_out: bool,
}

/// Received 1:1 messages stored on a leader whose reflection isn't confirmed
/// ([`Store::await_reflection`]).
pub(crate) const MIGRATION_AWAITING_REFLECTION: &str = "
    CREATE TABLE awaiting_reflection (
        sender     TEXT NOT NULL,
        message_id INTEGER NOT NULL,
        PRIMARY KEY (sender, message_id)
    ) WITHOUT ROWID;";

fn columns() -> String {
    format!(
        "id, conversation, message_id, outgoing, kind, body, created_at, updated_at, state, \
         edited_at, sender, {}, starred, sent_at, delivered_at, read_at, received_at, sort_key, \
         written_out",
        crate::reactions::REACTIONS_COLUMN
    )
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        conversation: row.get(1)?,
        message_id: row.get::<_, i64>(2)?.cast_unsigned(),
        outgoing: row.get(3)?,
        kind: row.get(4)?,
        body: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        state: row.get(8)?,
        edited_at: row.get(9)?,
        sender: row.get(10)?,
        reactions: crate::reactions::shown_reactions(row.get::<_, Option<String>>(11)?.as_deref()),
        starred: row.get(12)?,
        sent_at: row.get(13)?,
        delivered_at: row.get(14)?,
        read_at: row.get(15)?,
        received_at: row.get(16)?,
        sort_key: row.get(17)?,
        written_out: row.get(18)?,
    })
}

/// Store migration 36. `written_out`: a send of this outgoing message has begun, so someone may
/// have it although it isn't acknowledged. The queue doesn't survive a reconnect, and a resend
/// under the same ID is dropped by whoever has the message, so this has to be on record for edits
/// to know who may hold the old text. What was waiting during the upgrade counts as begun. And
/// placeholders an older version put back into the queue come out of it.
pub(crate) const MIGRATION_WRITTEN_OUT: &str = "
    ALTER TABLE messages ADD COLUMN written_out INTEGER NOT NULL DEFAULT 0;
    UPDATE messages SET written_out = 1 WHERE outgoing = 1 AND state = 0;
    UPDATE messages SET state = 1 WHERE outgoing = 1 AND state = 0 AND kind = 'deleted';
    DELETE FROM pending_receivers WHERE (conversation, message_id) IN
        (SELECT conversation, message_id FROM messages WHERE outgoing = 1 AND kind = 'deleted');";

/// Whether a received edit applies to this kind of message (the proto's _Edit applies to_): a
/// text, or a file's caption.
pub fn edit_applies_to(kind: &str) -> bool {
    matches!(kind, "text" | "file")
}

/// Whether a deletion for everyone applies to this kind of message, sent or received (Android's
/// `doesMessageTypeAllowRemoteDeletion`). A poll isn't one: it has to stay what votes refer to.
pub fn delete_applies_to(kind: &str) -> bool {
    matches!(kind, "text" | "location" | "file")
}

/// Threema's quote format (Android `QuoteUtil`, "quote v2"): `> quote #<message id hex>\n\n`
/// before the text. Returns the quoted message id and the text without the prefix. The hex is
/// the id's 8 bytes in wire order (little endian, as Android's `MessageId.toString()` and
/// libthreema's `Display`), not the number: printed as a number, Android couldn't find the
/// quoted message ("no longer available").
pub fn split_quote(text: &str) -> (Option<u64>, &str) {
    let parse = || {
        let rest = text.strip_prefix("> quote #")?;
        let (id, body) = (rest.get(..16)?, rest.get(16..)?);
        if !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        let bytes: [u8; 8] = data_encoding::HEXLOWER
            .decode(id.as_bytes())
            .ok()?
            .try_into()
            .ok()?;
        Some((u64::from_le_bytes(bytes), body.strip_prefix("\n\n")?))
    };
    match parse() {
        Some((id, body)) => (Some(id), body),
        None => (None, text),
    }
}

/// `text` quoting message `quote` (if any), in Threema's format.
pub fn with_quote(quote: Option<u64>, text: &str) -> String {
    match quote {
        Some(id) => format!(
            "> quote #{}\n\n{text}",
            data_encoding::HEXLOWER.encode(&id.to_le_bytes())
        ),
        None => text.to_owned(),
    }
}

/// The stored JSON of a location message (as received, or `sendLocation`) as libthreema's.
pub fn location_message(body: &str) -> Option<libthreema::model::message::LocationMessage> {
    let value: Value = serde_json::from_str(body).ok()?;
    let text = |field: &str| {
        value[field]
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let latitude = value["latitude"]
        .as_f64()
        .filter(|l| (-90.0..=90.0).contains(l))?;
    let longitude = value["longitude"]
        .as_f64()
        .filter(|l| (-180.0..=180.0).contains(l))?;
    // The wire format's single line after the coordinates is the address; libthreema's encoder
    // drops a name that comes without one. The place's name is what the recipient has to see.
    let (name, address) = match (text("name"), text("address")) {
        (Some(name), None) => (None, Some(name)),
        fields => fields,
    };
    Some(libthreema::model::message::LocationMessage {
        latitude,
        longitude,
        accuracy_m: value["accuracy"].as_f64().filter(|a| *a >= 0.0),
        name,
        address,
    })
}

/// What a file message is in the media gallery's filters (Android
/// `MimeUtil.getContentTypeFromFileData`): `gif`, `image`, `video`, `voice` (audio shown as
/// media), `audio` (sent as a file) or `file`. `body` is the stored file JSON.
pub fn media_kind(body: &str) -> &'static str {
    let value: Value = serde_json::from_str(body).unwrap_or_default();
    let media_type = value["mediaType"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if media_type == "image/gif" {
        "gif"
    } else if media_type.starts_with("image/") {
        "image"
    } else if media_type.starts_with("video/") {
        "video"
    } else if media_type.starts_with("audio/") {
        if value["rendering"] == "file" {
            "audio"
        } else {
            "voice"
        }
    } else {
        "file"
    }
}

/// All kinds, in the filter's order.
pub const MEDIA_KINDS: [&str; 6] = ["image", "gif", "video", "voice", "audio", "file"];

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The chat list's stand-in for a chat without messages (id 0): kind `empty` for a chat the
/// user emptied, kind `none` for a group or distribution list with no history yet.
fn marker(conversation: String, kind: &str, at: i64) -> Message {
    Message {
        id: 0,
        conversation,
        message_id: 0,
        outgoing: false,
        kind: kind.into(),
        body: String::new(),
        created_at: at,
        updated_at: at,
        state: INCOMING_READ,
        edited_at: None,
        sender: None,
        reactions: Value::Array(Vec::new()),
        starred: false,
        sent_at: None,
        delivered_at: None,
        read_at: None,
        received_at: None,
        sort_key: 0,
        written_out: false,
    }
}

impl Message {
    /// JSON for the UI.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "conversation": self.conversation,
            "messageId": format!("{:016x}", self.message_id),
            "outgoing": self.outgoing,
            "kind": self.kind,
            "text": if self.kind == "text" { Value::from(split_quote(&self.body).1) } else { Value::Null },
            "quoteId": if self.kind == "text" {
                split_quote(&self.body).0.map_or(Value::Null, |id| Value::from(format!("{id:016x}")))
            } else {
                Value::Null
            },
            // Status lines carry a code, other kinds JSON.
            "body": match self.kind.as_str() {
                "text" => Value::Null,
                "status" => Value::from(self.body.clone()),
                "file" => crate::file_types::shown_body(
                    serde_json::from_str(&self.body).unwrap_or(Value::Null),
                ),
                _ => serde_json::from_str(&self.body).unwrap_or(Value::Null),
            },
            "createdAt": self.created_at,
            "arrivedAt": self.arrived_at(),
            "updatedAt": self.updated_at,
            "state": self.state,
            "editedAt": self.edited_at,
            "sender": self.sender,
            "reactions": self.reactions,
            "starred": self.starred,
            "sentAt": self.sent_at,
            "deliveredAt": self.delivered_at,
            "readAt": self.read_at,
            "receivedAt": self.received_at,
            // The history position: the chat page tells by it whether a row outside its window
            // is new. Fits a JavaScript number: a row id shifted by 20 bits (see the migration).
            "sortKey": self.sort_key,
            // Pending, but someone may have it already: changing it takes an edit message.
            "writtenOut": self.written_out,
        })
    }
}

impl Message {
    /// When this message reached this device: an incoming `created_at` is the sender's clock,
    /// which may be off by any amount or set on purpose. What retention and the chat list go by.
    /// Rows with no arrival time (ours, imported ones, status lines) have only the one clock.
    pub fn arrived_at(&self) -> i64 {
        self.received_at.unwrap_or(self.created_at)
    }
}

/// A historical message as the Android-backup import records it (see
/// [`Store::insert_imported`]).
pub struct ImportedMessage<'a> {
    pub conversation: &'a str,
    pub message_id: u64,
    pub outgoing: bool,
    pub kind: &'a str,
    pub body: &'a str,
    pub created_at: i64,
    pub updated_at: i64,
    pub state: i64,
    pub edited_at: Option<i64>,
    pub sender: Option<&'a str>,
    pub starred: bool,
    pub sent_at: Option<i64>,
    pub delivered_at: Option<i64>,
    pub read_at: Option<i64>,
    pub received_at: Option<i64>,
    pub sort_key: i64,
}

impl Store {
    /// Insert an incoming message. Returns `None` if it already exists (server redelivery).
    pub fn insert_incoming(
        &self,
        conversation: &str,
        message_id: u64,
        kind: &str,
        body: &str,
        created_at: i64,
    ) -> Result<Option<Message>, StoreError> {
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO messages
                 (conversation, message_id, outgoing, kind, body, created_at, updated_at, state,
                  received_at)
             VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, ?7, ?6)",
            params![
                conversation,
                message_id.cast_signed(),
                kind,
                body,
                created_at,
                now_ms(),
                INCOMING_UNREAD
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        // A new message makes the hand-set mark pointless: the unread count says it now.
        self.set_marked_unread(conversation, false)?;
        self.message(self.conn.last_insert_rowid())
    }

    /// Insert an outgoing message in state pending.
    pub fn insert_outgoing(
        &self,
        conversation: &str,
        message_id: u64,
        kind: &str,
        body: &str,
    ) -> Result<Message, StoreError> {
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO messages
                 (conversation, message_id, outgoing, kind, body, created_at, updated_at, state)
             VALUES (?1, ?2, 1, ?3, ?4, ?5, ?5, ?6)",
            params![
                conversation,
                message_id.cast_signed(),
                kind,
                body,
                now,
                OutgoingState::Pending as i64
            ],
        )?;
        self.message(self.conn.last_insert_rowid())?
            .ok_or(StoreError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
    }

    /// A message another device of the group sent (multi-device): stored as sent, so it is not
    /// sent again from here. `None` if it is already here.
    pub fn insert_reflected_outgoing(
        &self,
        conversation: &str,
        message_id: u64,
        kind: &str,
        body: &str,
        created_at: i64,
    ) -> Result<Option<Message>, StoreError> {
        if self
            .message_by_id(conversation, true, message_id)?
            .is_some()
        {
            return Ok(None);
        }
        self.conn.execute(
            "INSERT INTO messages
                 (conversation, message_id, outgoing, kind, body, created_at, updated_at, state)
             VALUES (?1, ?2, 1, ?3, ?4, ?5, ?5, ?6)",
            params![
                conversation,
                message_id.cast_signed(),
                kind,
                body,
                created_at,
                OutgoingState::Sent as i64
            ],
        )?;
        self.message(self.conn.last_insert_rowid())
    }

    /// Mark these incoming messages read (read on another device of the group).
    pub fn mark_incoming_read(
        &self,
        conversation: &str,
        message_ids: &[u64],
        at: i64,
    ) -> Result<Vec<Message>, StoreError> {
        let mut changed = Vec::new();
        for message_id in message_ids {
            let updated = self.conn.execute(
                "UPDATE messages SET state = ?3, updated_at = ?4, read_at = ?4
                 WHERE conversation = ?1 AND outgoing = 0 AND message_id = ?2 AND state = ?5",
                params![
                    conversation,
                    message_id.cast_signed(),
                    INCOMING_READ,
                    at,
                    INCOMING_UNREAD
                ],
            )?;
            if updated > 0
                && let Some(message) = self.message_by_id(conversation, false, *message_id)?
            {
                changed.push(message);
            }
        }
        Ok(changed)
    }

    pub fn message(&self, id: i64) -> Result<Option<Message>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {} FROM messages WHERE id = ?1", columns()),
                [id],
                from_row,
            )
            .optional()?)
    }

    /// Was this incoming message id from `sender` seen before?
    pub fn has_incoming(&self, sender: &str, message_id: u64) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM messages WHERE conversation = ?1 AND outgoing = 0 AND message_id = ?2",
                params![sender, message_id.cast_signed()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// `has_incoming`'s row isn't known to the other devices of the group yet: the mediator hasn't
    /// confirmed its reflection. Until it has, the server's next copy has to pass again.
    pub fn await_reflection(&self, sender: &str, message_id: u64) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO awaiting_reflection (sender, message_id) VALUES (?1, ?2)",
            params![sender, message_id.cast_signed()],
        )?;
        Ok(())
    }

    pub fn awaits_reflection(&self, sender: &str, message_id: u64) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM awaiting_reflection WHERE sender = ?1 AND message_id = ?2",
                params![sender, message_id.cast_signed()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn reflection_done(&self, sender: &str, message_id: u64) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM awaiting_reflection WHERE sender = ?1 AND message_id = ?2",
            params![sender, message_id.cast_signed()],
        )?;
        Ok(())
    }

    /// Raise the state of outgoing messages (never lowers it: a late "delivered" must not undo
    /// "read"). Returns the changed messages.
    pub fn raise_outgoing_state(
        &self,
        conversation: &str,
        message_ids: &[u64],
        state: OutgoingState,
    ) -> Result<Vec<Message>, StoreError> {
        let mut changed = Vec::new();
        for message_id in message_ids {
            let updated = self.conn.execute(
                "UPDATE messages SET state = ?3, updated_at = ?4,
                     sent_at = CASE WHEN ?3 = ?6 THEN coalesce(sent_at, ?4) ELSE sent_at END,
                     delivered_at = CASE WHEN ?3 = ?7 THEN ?4 ELSE delivered_at END,
                     read_at = CASE WHEN ?3 = ?8 THEN ?4 ELSE read_at END
                 WHERE conversation = ?1 AND outgoing = 1 AND message_id = ?2
                   AND state < ?3 AND state != ?5",
                params![
                    conversation,
                    message_id.cast_signed(),
                    state as i64,
                    now_ms(),
                    OutgoingState::Failed as i64,
                    OutgoingState::Sent as i64,
                    OutgoingState::Delivered as i64,
                    OutgoingState::Read as i64
                ],
            )?;
            if updated > 0
                && let Some(id) = self
                    .conn
                    .query_row(
                        "SELECT id FROM messages WHERE conversation = ?1 AND outgoing = 1 AND message_id = ?2",
                        params![conversation, message_id.cast_signed()],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?
                {
                    changed.extend(self.message(id)?);
                }
        }
        Ok(changed)
    }

    /// A status line in a conversation (e.g. forward security changes); stored as read.
    pub fn insert_status(
        &self,
        conversation: &str,
        code: &str,
    ) -> Result<Option<Message>, StoreError> {
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO messages
                 (conversation, message_id, outgoing, kind, body, created_at, updated_at, state)
             VALUES (?1, ?2, 0, 'status', ?3, ?4, ?4, ?5)",
            params![
                conversation,
                rand::random::<i64>(),
                code,
                now,
                INCOMING_READ
            ],
        )?;
        self.message(self.conn.last_insert_rowid())
    }

    /// Whether the chat's newest row is the status line `code`.
    pub fn newest_is_status(&self, conversation: &str, code: &str) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT kind = 'status' AND body = ?2 FROM messages WHERE conversation = ?1
                 ORDER BY id DESC LIMIT 1",
                params![conversation, code],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// The chat with `peer` shows the call `call_id` (`call|…|callId`) among its last few calls
    /// (Android looks at the last 4 too).
    pub fn call_recorded(&self, peer: &str, call_id: u32) -> Result<bool, StoreError> {
        let mut statement = self.conn.prepare_cached(
            "SELECT body FROM messages
             WHERE conversation = ?1 AND kind = 'status' AND body LIKE 'call|%'
             ORDER BY id DESC LIMIT 4",
        )?;
        let call_id = call_id.to_string();
        let bodies = statement.query_map(params![peer], |row| row.get::<_, String>(0))?;
        for body in bodies {
            if body?.split('|').nth(4) == Some(call_id.as_str()) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Put a sent outgoing message back into the queue (the peer rejected it, e.g. after a lost
    /// forward security session). Returns the changed message; one deleted for everyone meanwhile
    /// has nothing left to send and stays as it is.
    pub fn reset_outgoing(
        &self,
        conversation: &str,
        message_id: u64,
    ) -> Result<Vec<Message>, StoreError> {
        let changed = self.conn.execute(
            "UPDATE messages SET state = ?3, updated_at = ?4,
                 sent_at = NULL, delivered_at = NULL, read_at = NULL
             WHERE conversation = ?1 AND outgoing = 1 AND message_id = ?2 AND kind <> 'deleted'",
            params![
                conversation,
                message_id.cast_signed(),
                OutgoingState::Pending as i64,
                now_ms()
            ],
        )?;
        if changed == 0 {
            return Ok(Vec::new());
        }
        let mut statement = self.conn.prepare(&format!(
            "SELECT {} FROM messages WHERE conversation = ?1 AND outgoing = 1 AND message_id = ?2",
            columns()
        ))?;
        let rows =
            statement.query_map(params![conversation, message_id.cast_signed()], from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// A send of this outgoing message begins: from here on someone may have it. Returns the
    /// message the first time, for the chat: changing it now takes more than it did.
    pub fn set_written_out(&self, id: i64) -> Result<Option<Message>, StoreError> {
        let changed = self.conn.execute(
            "UPDATE messages SET written_out = 1 WHERE id = ?1 AND written_out = 0",
            [id],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.message(id)
    }

    pub fn set_outgoing_failed(&self, id: i64) -> Result<Option<Message>, StoreError> {
        self.conn.execute(
            "UPDATE messages SET state = ?2, updated_at = ?3 WHERE id = ?1 AND outgoing = 1",
            params![id, OutgoingState::Failed as i64, now_ms()],
        )?;
        self.message(id)
    }

    /// Outgoing messages that still need sending, oldest first.
    pub fn pending_outgoing(&self) -> Result<Vec<Message>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {} FROM messages WHERE outgoing = 1 AND state = 0 AND kind <> 'deleted'
             ORDER BY id",
            columns()
        ))?;
        let rows = statement.query_map([], from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Messages of a conversation in history order, oldest first (`limit` newest ones). Live
    /// messages sit in arrival/sending order (never by `created_at`: the sender's clock can be
    /// off); imported history is slotted between them by `sort_key`, ties broken by `id`.
    pub fn conversation_messages(
        &self,
        conversation: &str,
        limit: u32,
    ) -> Result<Vec<Message>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {} FROM (SELECT * FROM messages WHERE conversation = ?1
                                    ORDER BY sort_key DESC, id DESC LIMIT ?2)
             ORDER BY sort_key, id",
            columns()
        ))?;
        let rows = statement.query_map(params![conversation, limit], from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The history position of a message in its conversation.
    fn position_of(&self, conversation: &str, id: i64) -> Result<Option<(i64, i64)>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT sort_key, id FROM messages WHERE conversation = ?1 AND id = ?2",
                params![conversation, id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// The newest `limit` messages of a conversation older **in history** than `before` (a
    /// local id), oldest first: a page further back (the chat page, Threema Web).
    pub fn conversation_messages_before(
        &self,
        conversation: &str,
        before: i64,
        limit: u32,
    ) -> Result<Vec<Message>, StoreError> {
        let Some((key, id)) = self.position_of(conversation, before)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(&format!(
            "SELECT {} FROM (SELECT * FROM messages WHERE conversation = ?1
                                    AND (sort_key < ?2 OR (sort_key = ?2 AND id < ?3))
                                    ORDER BY sort_key DESC, id DESC LIMIT ?4)
             ORDER BY sort_key, id",
            columns()
        ))?;
        let rows = statement.query_map(params![conversation, key, id, limit], from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The page of messages **newer** than `after` in history order, oldest first: the chat
    /// page scrolling forward again after it let the newest messages go (its window is
    /// bounded). Not [`Self::conversation_messages_after`], which is change detection by row
    /// id and answers everything at once.
    pub fn conversation_messages_newer(
        &self,
        conversation: &str,
        after: i64,
        limit: u32,
    ) -> Result<Vec<Message>, StoreError> {
        let Some((key, id)) = self.position_of(conversation, after)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(&format!(
            "SELECT {} FROM messages WHERE conversation = ?1
                    AND (sort_key > ?2 OR (sort_key = ?2 AND id > ?3))
             ORDER BY sort_key, id LIMIT ?4",
            columns()
        ))?;
        let rows = statement.query_map(params![conversation, key, id, limit], from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// How many incoming messages are unread, and the first of them in history order: the
    /// chat page opens there and draws the new-messages line above it.
    pub fn unread_position(&self, conversation: &str) -> Result<(i64, Option<i64>), StoreError> {
        let count = self.conn.query_row(
            "SELECT count(*) FROM messages
             WHERE conversation = ?1 AND outgoing = 0 AND state = ?2",
            params![conversation, INCOMING_UNREAD],
            |row| row.get(0),
        )?;
        let first = self
            .conn
            .query_row(
                "SELECT id FROM messages
                 WHERE conversation = ?1 AND outgoing = 0 AND state = ?2
                 ORDER BY sort_key, id LIMIT 1",
                params![conversation, INCOMING_UNREAD],
                |row| row.get(0),
            )
            .optional()?;
        Ok((count, first))
    }

    /// Messages of a conversation **inserted** after `after` (a local id), in history order:
    /// change detection for live updates, deliberately by row id — an import refreshes the UI
    /// with full reloads instead.
    pub fn conversation_messages_after(
        &self,
        conversation: &str,
        after: i64,
    ) -> Result<Vec<Message>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {} FROM messages WHERE conversation = ?1 AND id > ?2 ORDER BY sort_key, id",
            columns()
        ))?;
        let rows = statement.query_map(params![conversation, after], from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn message_count(&self, conversation: &str) -> Result<i64, StoreError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation = ?1",
            [conversation],
            |row| row.get(0),
        )?)
    }

    /// A window of a conversation around the message `around` (a local id), oldest first: up
    /// to `side` messages before it in history and up to `side` after it, to show a search hit
    /// or a quoted message however old it is. The flag says whether the window reaches the
    /// newest message; the chat pages on from there. A message that isn't there any more (or
    /// isn't of this conversation) gives the newest messages.
    pub fn conversation_messages_around(
        &self,
        conversation: &str,
        around: i64,
        side: u32,
    ) -> Result<(Vec<Message>, bool), StoreError> {
        if self.position_of(conversation, around)?.is_none() {
            let newest = side.saturating_mul(2).saturating_add(1);
            return Ok((self.conversation_messages(conversation, newest)?, true));
        }
        let mut window = self.conversation_messages_before(conversation, around, side)?;
        window.extend(self.message(around)?);
        // One more than asked for tells whether anything lies beyond the window.
        let mut newer =
            self.conversation_messages_newer(conversation, around, side.saturating_add(1))?;
        let reaches_newest = newer.len() <= side as usize;
        newer.truncate(side as usize);
        window.append(&mut newer);
        Ok((window, reaches_newest))
    }

    /// Texts, captions, file, place and poll names containing `query` (case-insensitive),
    /// newest first; in one conversation or all. `names` resolves mentioned IDs (`@[ID]` is
    /// searched as "@Name"). Returns each message with the matching text (for the preview).
    /// SQLCipher decrypts on read, so this runs on plaintext inside the core only.
    pub fn search_messages(
        &self,
        query: &str,
        conversation: Option<&str>,
        limit: u32,
        names: impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<(Message, String)>, StoreError> {
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        // SQL narrows it down (messages with mentions always pass: names are not in the body).
        // From three characters on, the trigram index finds the candidates; shorter queries
        // can't use it and scan, as before.
        let indexed = query.chars().count() >= 3;
        let candidates = if indexed {
            "id IN (SELECT rowid FROM messages_search WHERE messages_search MATCH ?1
                    UNION SELECT id FROM messages WHERE body LIKE '%@[%')"
        } else {
            "(body LIKE ?1 ESCAPE '\\' OR body LIKE '%@[%')"
        };
        let mut statement = self.conn.prepare(&format!(
            "SELECT {} FROM messages
             WHERE kind IN ('text', 'file', 'location', 'poll')
               AND {candidates}
               AND (?2 IS NULL OR conversation = ?2)
             ORDER BY created_at DESC, id DESC",
            columns()
        ))?;
        // A quoted phrase: the trigram tokenizer matches it as a substring, case-insensitive.
        let phrase = format!("\"{}\"", query.replace('"', "\"\""));
        let rows = statement.query_map(
            params![if indexed { &phrase } else { &pattern }, conversation],
            from_row,
        )?;
        let needle = query.to_lowercase();
        let with_names = |text: &str| -> String {
            let mut text = text.to_owned();
            for id in crate::mentions::identities(&text.clone()) {
                if let Some(name) = names(id) {
                    text = text.replace(&format!("@[{id}]"), &format!("@{name}"));
                }
            }
            text
        };
        let mut found = Vec::new();
        for message in rows {
            let message = message?;
            // Only user-visible fields count, not JSON keys, blob ids or the quote marker.
            // The hit is returned raw (the UI shows mentions in its language, "@Me"…).
            let fields: Vec<String> = match message.kind.as_str() {
                "text" => vec![split_quote(&message.body).1.to_owned()],
                _ => {
                    let value: Value = serde_json::from_str(&message.body).unwrap_or_default();
                    ["caption", "name", "address", "description"]
                        .iter()
                        .filter_map(|field| value[field].as_str())
                        .map(str::to_owned)
                        .collect()
                }
            };
            if let Some(hit) = fields
                .into_iter()
                .find(|f| with_names(f).to_lowercase().contains(&needle))
            {
                found.push((message, hit));
                if found.len() >= limit as usize {
                    break;
                }
            }
        }
        Ok(found)
    }

    pub fn set_starred(&self, id: i64, starred: bool) -> Result<Option<Message>, StoreError> {
        self.conn.execute(
            "UPDATE messages SET starred = ?2 WHERE id = ?1",
            params![id, starred],
        )?;
        self.message(id)
    }

    /// Starred messages of all chats (or one), newest first.
    pub fn starred_messages(&self, conversation: Option<&str>) -> Result<Vec<Message>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {} FROM messages WHERE starred = 1 AND (?1 IS NULL OR conversation = ?1)
             ORDER BY created_at DESC, id DESC",
            columns()
        ))?;
        let rows = statement.query_map(params![conversation], from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The file messages of a conversation whose [`media_kind`] is in `kinds`, newest first:
    /// the chat's media gallery (Android's `mediagallery`).
    pub fn conversation_media(
        &self,
        conversation: &str,
        kinds: &[&str],
    ) -> Result<Vec<Message>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {} FROM messages WHERE conversation = ?1 AND kind = 'file'
             ORDER BY sort_key DESC, id DESC",
            columns()
        ))?;
        let rows = statement.query_map(params![conversation], from_row)?;
        let mut media = Vec::new();
        for message in rows {
            let message = message?;
            if kinds.contains(&media_kind(&message.body)) {
                media.push(message);
            }
        }
        Ok(media)
    }

    /// Mark incoming messages of a conversation as read; returns their message ids.
    /// Chats marked unread by hand.
    pub fn marked_unread_chats(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT conversation FROM marked_unread_chats")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_marked_unread(&self, conversation: &str, unread: bool) -> Result<(), StoreError> {
        if unread {
            self.conn.execute(
                "INSERT OR IGNORE INTO marked_unread_chats (conversation) VALUES (?1)",
                [conversation],
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM marked_unread_chats WHERE conversation = ?1",
                [conversation],
            )?;
        }
        Ok(())
    }

    pub fn mark_conversation_read(&self, conversation: &str) -> Result<Vec<u64>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT message_id FROM messages WHERE conversation = ?1 AND outgoing = 0 AND state = ?2",
        )?;
        let ids: Vec<u64> = statement
            .query_map(params![conversation, INCOMING_UNREAD], |row| {
                row.get::<_, i64>(0).map(i64::cast_unsigned)
            })?
            .collect::<Result<_, _>>()?;
        self.conn.execute(
            "UPDATE messages SET state = ?3, updated_at = ?4, read_at = ?4
             WHERE conversation = ?1 AND outgoing = 0 AND state = ?2",
            params![conversation, INCOMING_UNREAD, INCOMING_READ, now_ms()],
        )?;
        // Reading the chat takes the hand-set mark away, as on Android.
        self.set_marked_unread(conversation, false)?;
        Ok(ids)
    }

    /// The key of every chat that holds a message, private ones too.
    pub fn chats_with_messages(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self.conn.prepare("SELECT chat FROM chat_summaries")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One row per conversation: last message and unread count, newest first. Chats without
    /// messages get a stand-in row (id 0): emptied chats and those a backup brought without their
    /// messages (kind `empty`, from when they were emptied or last changed) and groups and distribution lists with no history yet (kind `none`, from their
    /// creation) — unlike a contact, they are chats of their own as soon as they exist
    /// (Android lists every group too; a Safe restore brings groups without any messages).
    pub fn conversations(&self) -> Result<Vec<(Message, i64)>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            // One row per chat straight from the summary the triggers keep (migration 33):
            // its newest message by history position and its unread count, no scan over the
            // messages. The order is when that message arrived (`Message::arrived_at`; a sender's
            // wrong clock must not pin a chat or bury a new message), with a deterministic
            // tie-break.
            "SELECT {}, s.unread
             FROM chat_summaries s JOIN messages m ON m.id = s.last_id
             ORDER BY COALESCE(m.received_at, m.created_at) DESC, m.id DESC",
            columns()
        ))?;
        let rows = statement.query_map([], |row| Ok((from_row(row)?, row.get::<_, i64>(19)?)))?;
        let mut list: Vec<(Message, i64)> = rows.collect::<Result<_, _>>()?;
        let mut markers: Vec<Message> = Vec::new();
        let mut statement = self.conn.prepare(
            "SELECT conversation, at FROM emptied_chats e
             WHERE NOT EXISTS (SELECT 1 FROM messages m WHERE m.conversation = e.conversation)",
        )?;
        let emptied = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (conversation, at) in emptied {
            markers.push(marker(conversation, "empty", at));
        }
        let starters = self
            .groups()?
            .into_iter()
            .map(|group| (group.key, group.created_at))
            .chain(
                self.distribution_lists()?
                    .into_iter()
                    .map(|l| (l.key, l.created_at)),
            );
        for (key, at) in starters {
            if list.iter().any(|(m, _)| m.conversation == key)
                || markers.iter().any(|m| m.conversation == key)
            {
                continue;
            }
            markers.push(marker(key, "none", at));
        }
        // Placed by their time among the real rows; the order among markers is deterministic.
        markers.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.conversation.cmp(&b.conversation))
        });
        for entry in markers {
            let place = list
                .iter()
                .position(|(m, _)| m.arrived_at() < entry.arrived_at())
                .unwrap_or(list.len());
            list.insert(place, (entry, 0));
        }
        Ok(list)
    }

    /// Delete a chat: its messages and what is kept for the chat as such, its pin, archive and
    /// unread marks and its draft, so that it starts anew if it comes back (ADR 0022). What belongs
    /// to the contact or group (notification choice, tone) and the private mark stay; see
    /// [`Self::empty_conversation`] to keep the chat.
    pub fn delete_conversation(&self, conversation: &str) -> Result<(), StoreError> {
        let step = self.step()?;
        self.delete_chat_rows(conversation)?;
        step.commit()?;
        Ok(())
    }

    /// [`Self::delete_conversation`]'s rows, in the caller's transaction.
    pub(crate) fn delete_chat_rows(&self, conversation: &str) -> Result<(), StoreError> {
        for table in [
            "messages",
            "emptied_chats",
            "pinned_chats",
            "archived_chats",
            "marked_unread_chats",
        ] {
            self.conn.execute(
                &format!("DELETE FROM {table} WHERE conversation = ?1"),
                [conversation],
            )?;
        }
        self.conn.execute(
            "DELETE FROM settings WHERE key IN ('draft:' || ?1, 'draftQuote:' || ?1)",
            [conversation],
        )?;
        Ok(())
    }

    /// Delete a chat's messages but keep the chat in the list (Android's "Empty chat").
    pub fn empty_conversation(&self, conversation: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM messages WHERE conversation = ?1",
            [conversation],
        )?;
        self.conn.execute(
            "INSERT OR REPLACE INTO emptied_chats (conversation, at) VALUES (?1, ?2)",
            params![conversation, now_ms()],
        )?;
        Ok(())
    }

    /// A chat a backup brings without its messages, last changed at `at` (ms): listed from then on
    /// like an emptied one, as Android lists restored chats by their `lastUpdate`. A chat that
    /// already has a time of its own keeps it; no usable time, no entry.
    pub fn note_restored_chat(
        &self,
        conversation: &str,
        at: Option<i64>,
    ) -> Result<(), StoreError> {
        let Some(at) = at.filter(|at| *at > 0) else {
            return Ok(());
        };
        self.conn.execute(
            "INSERT OR IGNORE INTO emptied_chats (conversation, at) VALUES (?1, ?2)",
            params![conversation, at],
        )?;
        Ok(())
    }

    /// A message by its Threema id and direction in a conversation.
    pub fn message_by_id(
        &self,
        conversation: &str,
        outgoing: bool,
        message_id: u64,
    ) -> Result<Option<Message>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {} FROM messages WHERE conversation = ?1 AND outgoing = ?2 AND message_id = ?3",
                    columns()
                ),
                params![conversation, outgoing, message_id.cast_signed()],
                from_row,
            )
            .optional()?)
    }

    /// We wrote something in this conversation (status lines are not messages).
    pub fn has_outgoing_content(&self, conversation: &str) -> Result<bool, StoreError> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM messages
                 WHERE conversation = ?1 AND outgoing = 1 AND kind <> 'status')",
            [conversation],
            |row| row.get(0),
        )?)
    }

    /// When the first message from others came in this conversation (status lines are not
    /// messages).
    pub fn earliest_incoming_content(&self, conversation: &str) -> Result<Option<i64>, StoreError> {
        Ok(self.conn.query_row(
            "SELECT MIN(created_at) FROM messages
                 WHERE conversation = ?1 AND outgoing = 0 AND kind <> 'status'",
            [conversation],
            |row| row.get(0),
        )?)
    }

    /// Replace a message's text, or a file's caption; `edited_at` `None` for a message nobody has
    /// seen yet. Other kinds stay as they are.
    pub fn edit_message(
        &self,
        id: i64,
        text: &str,
        edited_at: Option<i64>,
    ) -> Result<Option<Message>, StoreError> {
        let Some(original) = self.message(id)? else {
            return Ok(None);
        };
        match original.kind.as_str() {
            "text" => {
                // An edit without the quote prefix keeps the quote of the original.
                let text = match split_quote(text).0 {
                    None => with_quote(split_quote(&original.body).0, text),
                    Some(_) => text.to_owned(),
                };
                self.conn.execute(
                    "UPDATE messages SET body = ?2, edited_at = coalesce(?3, edited_at),
                         updated_at = ?4
                     WHERE id = ?1",
                    params![id, text, edited_at, now_ms()],
                )?;
            }
            "file" => {
                self.conn.execute(
                    "UPDATE messages SET body = json_set(body, '$.caption', ?2),
                         edited_at = coalesce(?3, edited_at), updated_at = ?4
                     WHERE id = ?1 AND json_valid(body)",
                    params![id, text, edited_at, now_ms()],
                )?;
            }
            _ => {}
        }
        self.message(id)
    }

    /// Replace a message with a "message deleted" placeholder (content and reactions gone).
    pub fn mark_deleted(&self, id: i64, deleted_at: i64) -> Result<Option<Message>, StoreError> {
        self.conn
            .execute("DELETE FROM reactions WHERE message = ?1", [id])?;
        self.conn
            .execute("DELETE FROM files WHERE message = ?1", [id])?;
        // Whatever refers to it by ID has nothing to find any more: a poll takes no more votes.
        self.conn
            .execute("DELETE FROM polls WHERE message = ?1", [id])?;
        // Nothing of it is left to send, to anyone.
        self.conn.execute(
            "DELETE FROM pending_receivers WHERE (conversation, message_id) IN
                 (SELECT conversation, message_id FROM messages WHERE id = ?1 AND outgoing = 1)",
            [id],
        )?;
        self.conn.execute(
            "UPDATE messages SET kind = 'deleted', body = '', edited_at = ?2, updated_at = ?3,
                 state = CASE WHEN outgoing = 1 AND state = ?4 THEN ?5 ELSE state END
             WHERE id = ?1",
            params![
                id,
                deleted_at,
                now_ms(),
                OutgoingState::Pending as i64,
                OutgoingState::Sent as i64
            ],
        )?;
        self.message(id)
    }

    /// Remove a message from this device only (or one that was never sent).
    pub fn remove_message(&self, id: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM pending_receivers WHERE (conversation, message_id) IN
                 (SELECT conversation, message_id FROM messages WHERE id = ?1 AND outgoing = 1)",
            [id],
        )?;
        self.conn
            .execute("DELETE FROM messages WHERE id = ?1", [id])?;
        Ok(())
    }

    /// `(sort_key, created_at)` of a conversation in history order: the skeleton the import
    /// interleaves old messages into.
    pub fn conversation_positions(
        &self,
        conversation: &str,
    ) -> Result<Vec<(i64, i64)>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT sort_key, created_at FROM messages WHERE conversation = ?1
             ORDER BY sort_key, id",
        )?;
        let rows = statement.query_map([conversation], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Insert one historical message exactly as recorded, at an explicit history position:
    /// the import's dedicated path — never queued for sending (outgoing failures stay
    /// failures), no receipts, no notifications. Returns the new row id, or `None` when
    /// `(conversation, outgoing, message_id)` already exists (the import's deduplication; the
    /// existing row, e.g. a live edit or tombstone, always wins).
    pub fn insert_imported(&self, m: &ImportedMessage<'_>) -> Result<Option<i64>, StoreError> {
        debug_assert!(m.sort_key != 0, "0 is reserved for the trigger");
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO messages
                 (conversation, message_id, outgoing, kind, body, created_at, updated_at,
                  state, edited_at, sender, starred, sent_at, delivered_at, read_at,
                  received_at, sort_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                m.conversation,
                m.message_id.cast_signed(),
                m.outgoing,
                m.kind,
                m.body,
                m.created_at,
                m.updated_at,
                m.state,
                m.edited_at,
                m.sender,
                m.starred,
                m.sent_at,
                m.delivered_at,
                m.read_at,
                m.received_at,
                m.sort_key
            ],
        )?;
        Ok((changed > 0).then(|| self.conn.last_insert_rowid()))
    }

    /// Used before? Raw nonces (24 bytes, ours) and hashed ones (32 bytes, imported from an
    /// Android backup) share the table, like Android's `exists(nonce=? OR nonce=?)`.
    pub fn has_nonce(&self, nonce: &[u8]) -> Result<bool, StoreError> {
        self.has_nonce_in("nonces", nonce)
    }

    pub(crate) fn has_nonce_in(&self, table: &str, nonce: &[u8]) -> Result<bool, StoreError> {
        let hashed = match self.own_threema_id()? {
            Some(identity) => hash_nonce(&identity, nonce),
            None => nonce.to_vec(),
        };
        Ok(self
            .conn
            .query_row(
                &format!("SELECT 1 FROM {table} WHERE nonce = ?1 OR nonce = ?2"),
                params![nonce, hashed],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The own Threema ID alone (the key of hashed nonces), without loading the client key.
    pub(crate) fn own_threema_id(&self) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT threema_id FROM identity WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Add already hashed nonces (Android backup import) in one transaction; returns how many
    /// were new.
    pub fn add_hashed_nonces(&self, d2d: bool, hashed: &[[u8; 32]]) -> Result<usize, StoreError> {
        let table = if d2d { "d2d_nonces" } else { "nonces" };
        let tx = self.conn.unchecked_transaction()?;
        let mut added = 0;
        {
            let mut statement = tx.prepare(&format!(
                "INSERT OR IGNORE INTO {table} (nonce) VALUES (?1)"
            ))?;
            for nonce in hashed {
                added += statement.execute([&nonce[..]])?;
            }
        }
        tx.commit()?;
        Ok(added)
    }

    /// Every used CSP nonce (device join).
    pub fn all_nonces(&self) -> Result<Vec<Vec<u8>>, StoreError> {
        let mut statement = self.conn.prepare("SELECT nonce FROM nonces")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn add_nonces(&self, nonces: &[&[u8]]) -> Result<(), StoreError> {
        for nonce in nonces {
            self.conn
                .execute("INSERT OR IGNORE INTO nonces (nonce) VALUES (?1)", [nonce])?;
        }
        Ok(())
    }
}

/// Android's hashed nonce (`NonceFactory`/`DatabaseNonceStore`): HMAC-SHA256 over the nonce,
/// keyed with the own Threema ID string. Already hashed (32-byte) entries pass through.
pub fn hash_nonce(identity: &str, nonce: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac as _};
    if nonce.len() == 32 {
        return nonce.to_vec();
    }
    let mut mac =
        Hmac::<sha2::Sha256>::new_from_slice(identity.as_bytes()).expect("HMAC takes any key");
    mac.update(nonce);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_index_follows_every_change() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let hits = |query: &str| -> Vec<i64> {
            store
                .search_messages(query, None, 50, |id| {
                    (id == "0000ANNA").then(|| "Anna".into())
                })
                .unwrap()
                .into_iter()
                .map(|(m, _)| m.id)
                .collect()
        };
        let text = store
            .insert_incoming("00000BOB", 1, "text", "Grüße aus Zürich", 10)
            .unwrap()
            .unwrap();
        let file = store
            .insert_incoming(
                "00000BOB",
                2,
                "file",
                r#"{"caption":"Holiday photos","name":"beach.jpg","blobId":"zurich"}"#,
                11,
            )
            .unwrap()
            .unwrap();
        let mention = store
            .insert_incoming("00000BOB", 3, "text", "ask @[0000ANNA] later", 12)
            .unwrap()
            .unwrap();
        // Case-insensitive beyond ASCII, a substring in the middle of a word.
        assert_eq!(hits("ZÜRICH"), vec![text.id]);
        assert_eq!(hits("üri"), vec![text.id]);
        // A file by caption and by name, never by a JSON key or blob id.
        assert_eq!(hits("holiday"), vec![file.id]);
        assert_eq!(hits("beach"), vec![file.id]);
        assert!(hits("blobId").is_empty());
        // A mention by the name it shows.
        assert_eq!(hits("Anna"), vec![mention.id]);
        // Edits and deletions reach the index.
        store
            .edit_message(text.id, "Grüße aus Bern", Some(20))
            .unwrap();
        assert!(hits("Zürich").is_empty());
        assert_eq!(hits("bern"), vec![text.id]);
        store.remove_message(file.id).unwrap();
        assert!(hits("holiday").is_empty());
        // Shorter than a trigram: scanned, still found.
        assert_eq!(hits("Be"), vec![text.id]);
    }

    #[test]
    fn search() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let hello = store
            .insert_incoming("00000BOB", 1, "text", "Hello Sailfish", 10)
            .unwrap()
            .unwrap();
        store
            .insert_incoming("00000BOB", 2, "text", "100% sure", 11)
            .unwrap();
        store
            .insert_incoming(
                "00000BOB",
                3,
                "file",
                r#"{"caption":"sailing","blob":"sail"}"#,
                12,
            )
            .unwrap();
        store
            .insert_incoming(
                "00000ANN",
                4,
                "text",
                "> quote #0123456789abcdef\n\nmore sail",
                13,
            )
            .unwrap();
        store
            .insert_incoming("00000ANN", 5, "text", "Hi @[00000BOB]!", 14)
            .unwrap();
        let names = |id: &str| (id == "00000BOB").then(|| "Bob".to_owned());
        let search = |query: &str, conversation: Option<&str>| {
            store
                .search_messages(query, conversation, 10, names)
                .unwrap()
        };
        let ids = |found: Vec<(Message, String)>| {
            found.iter().map(|(m, _)| m.message_id).collect::<Vec<_>>()
        };
        assert_eq!(ids(search("SAIL", None)), vec![4, 3, 1]);
        assert_eq!(ids(search("sail", Some("00000BOB"))), vec![3, 1]);
        assert_eq!(ids(search("0%", None)), vec![2]);
        // Not in user-visible fields: the quote marker, JSON keys.
        assert!(search("quote", None).is_empty());
        assert!(search("blob", None).is_empty());
        // Mentions by name; the hit is the visible text.
        assert_eq!(search("@bob", None)[0].1, "Hi @[00000BOB]!");
        assert!(search("00000BOB", None).is_empty());
        assert_eq!(search("sailing", None)[0].1, "sailing");
        // Stars.
        assert!(!hello.starred);
        assert!(store.set_starred(hello.id, true).unwrap().unwrap().starred);
        assert_eq!(store.starred_messages(None).unwrap().len(), 1);
        assert!(store.starred_messages(Some("00000ANN")).unwrap().is_empty());
        // A hit can be opened: the window around it holds it.
        let (around, _) = store
            .conversation_messages_around("00000BOB", hello.id, 1)
            .unwrap();
        assert_eq!(around.iter().filter(|m| m.id == hello.id).count(), 1);
    }

    /// The chat page tells a new row from a change to one outside its window by `sortKey`:
    /// a receipt or a star on an old message must not look newer than what came after it, and
    /// only a message that really is new may be higher than everything.
    #[test]
    fn a_change_keeps_its_place_in_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let key = |m: &Message| m.to_json()["sortKey"].as_i64().unwrap();
        let old = store
            .insert_outgoing("00000BOB", 1, "text", "old of ours")
            .unwrap();
        let newest = store
            .insert_incoming("00000BOB", 2, "text", "newest", 20)
            .unwrap()
            .unwrap();
        assert!(key(&old) < key(&newest));

        let receipt = store
            .raise_outgoing_state("00000BOB", &[1], OutgoingState::Read)
            .unwrap();
        assert_eq!(receipt.len(), 1);
        assert_eq!(key(&receipt[0]), key(&old), "a receipt doesn't move it");
        let starred = store.set_starred(old.id, true).unwrap().unwrap();
        assert_eq!(key(&starred), key(&old), "nor does anything else");

        let added = store
            .insert_incoming("00000BOB", 3, "text", "new", 5)
            .unwrap()
            .unwrap();
        assert!(key(&added) > key(&newest), "whatever its own clock says");
    }

    /// A jump to an old message used to load everything newer than it at once.
    #[test]
    fn a_window_around_a_message_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let ids: Vec<i64> = (0..100_u64)
            .map(|n| {
                store
                    .insert_incoming("00000BOB", n + 1, "text", "x", 10)
                    .unwrap()
                    .unwrap()
                    .id
            })
            .collect();
        store
            .insert_incoming("00000ANN", 1, "text", "other chat", 10)
            .unwrap();
        let window = |around: i64| {
            let (messages, reaches_newest) = store
                .conversation_messages_around("00000BOB", around, 5)
                .unwrap();
            let ids: Vec<i64> = messages.iter().map(|m| m.id).collect();
            (ids, reaches_newest)
        };

        // In the middle: the message and five on either side, in history order.
        let (middle, reaches_newest) = window(ids[10]);
        assert_eq!(middle, ids[5..=15]);
        assert!(!reaches_newest);
        // At the oldest end: nothing to fill the older side with.
        assert_eq!(window(ids[0]).0, ids[..=5]);
        // Exactly five left, and fewer: the window reaches the newest message.
        let (near_end, reaches_newest) = window(ids[94]);
        assert_eq!(near_end, ids[89..=99]);
        assert!(reaches_newest);
        assert_eq!(window(ids[99]).0, ids[94..]);
        assert!(window(ids[99]).1);
        // One more than fits: it doesn't.
        assert!(!window(ids[93]).1);

        // Gone, or of another chat: the newest messages, not someone else's.
        let other = store.conversation_messages("00000ANN", 1).unwrap()[0].id;
        for target in [other, -1] {
            let (newest, reaches_newest) = window(target);
            assert_eq!(newest, ids[89..]);
            assert!(reaches_newest);
        }
    }

    #[test]
    fn location_bodies() {
        let location = location_message(
            r#"{"latitude":49.61,"longitude":6.13,"accuracy":12.5,"name":" Place ","address":""}"#,
        )
        .unwrap();
        assert_eq!(location.latitude, 49.61);
        assert_eq!(location.accuracy_m, Some(12.5));
        // A name alone goes out as the address, the one line the recipient gets.
        assert_eq!(location.name, None);
        assert_eq!(location.address.as_deref(), Some("Place"));
        let both = location_message(
            r#"{"latitude":49.61,"longitude":6.13,"name":"Place","address":"Street 1"}"#,
        )
        .unwrap();
        assert_eq!(both.name.as_deref(), Some("Place"));
        assert_eq!(both.address.as_deref(), Some("Street 1"));
        let none = location_message(r#"{"latitude":49.61,"longitude":6.13}"#).unwrap();
        assert_eq!((none.name, none.address), (None, None));
        assert!(location_message(r#"{"latitude":91,"longitude":0}"#).is_none());
        assert!(location_message(r#"{"latitude":1}"#).is_none());
    }

    /// The place's name reaches the recipient: it is in the bytes that are sent.
    #[test]
    fn location_name_is_sent() {
        use libthreema::{
            csp_e2e::message::task::outgoing::encode_message_body,
            model::message::{OutgoingContactMessageBody, OutgoingMessageBody},
        };
        let location =
            location_message(r#"{"latitude":49.61,"longitude":6.13,"name":"Place"}"#).unwrap();
        let (_, data) =
            encode_message_body(&OutgoingMessageBody::Contact(OutgoingContactMessageBody {
                receiver_identity: "ABCD1234".parse().unwrap(),
                body: libthreema::model::message::ContactMessageBody::Location(location),
            }));
        let data = String::from_utf8(data).unwrap();
        assert_eq!(data.lines().nth(1), Some("Place"), "{data}");
    }

    #[test]
    fn message_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let out = store.insert_outgoing("ECHOECHO", 7, "text", "hi").unwrap();
        assert_eq!(out.state, OutgoingState::Pending as i64);
        assert_eq!(store.pending_outgoing().unwrap().len(), 1);

        let changed = store
            .raise_outgoing_state("ECHOECHO", &[7], OutgoingState::Read)
            .unwrap();
        assert_eq!(changed.len(), 1);
        // Never lowered.
        assert!(
            store
                .raise_outgoing_state("ECHOECHO", &[7], OutgoingState::Delivered)
                .unwrap()
                .is_empty()
        );
        assert!(store.pending_outgoing().unwrap().is_empty());

        let incoming = store
            .insert_incoming("ECHOECHO", 9, "text", "hi", now_ms() + 1)
            .unwrap();
        assert!(incoming.is_some());
        assert!(
            store
                .insert_incoming("ECHOECHO", 9, "text", "hi", now_ms())
                .unwrap()
                .is_none(),
            "duplicate"
        );
        assert!(store.has_incoming("ECHOECHO", 9).unwrap());

        let conversations = store.conversations().unwrap();
        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].0.message_id, 9);
        assert_eq!(conversations[0].1, 1, "one unread");
        assert_eq!(store.mark_conversation_read("ECHOECHO").unwrap(), vec![9]);
        assert_eq!(store.conversations().unwrap()[0].1, 0);
        assert_eq!(
            store.conversation_messages("ECHOECHO", 50).unwrap().len(),
            2
        );

        store.add_nonces(&[&[5; 24]]).unwrap();
        assert!(store.has_nonce(&[5; 24]).unwrap());
        assert!(!store.has_nonce(&[6; 24]).unwrap());
    }

    /// The per-chat summary (migration 33) must say exactly what a recomputation says, after
    /// every kind of change: new messages (live and placed in the history like an import),
    /// reading a chat, deleting single messages, emptying and deleting chats.
    /// The chat page's window: the page before a message and the page after it, both in
    /// history order, so scrolling back and forward again shows the same messages.
    #[test]
    fn pages_before_and_after_a_message() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        for n in 1..=20_u64 {
            store
                .insert_incoming(
                    "AAAAAAAA",
                    n,
                    "text",
                    &format!("m{n}"),
                    i64::try_from(n).unwrap(),
                )
                .unwrap();
        }
        let all = store.conversation_messages("AAAAAAAA", 50).unwrap();
        assert_eq!(all.len(), 20);
        let middle = all[10].id; // m11

        let older = store
            .conversation_messages_before("AAAAAAAA", middle, 5)
            .unwrap();
        assert_eq!(
            older.iter().map(|m| m.body.clone()).collect::<Vec<_>>(),
            vec!["m6", "m7", "m8", "m9", "m10"],
            "oldest first, right up to the message"
        );
        let newer = store
            .conversation_messages_newer("AAAAAAAA", middle, 5)
            .unwrap();
        assert_eq!(
            newer.iter().map(|m| m.body.clone()).collect::<Vec<_>>(),
            vec!["m12", "m13", "m14", "m15", "m16"],
            "oldest first, right after the message"
        );
        // The ends, and a message that is not there.
        assert!(
            store
                .conversation_messages_newer("AAAAAAAA", all[19].id, 5)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .conversation_messages_before("AAAAAAAA", all[0].id, 5)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .conversation_messages_newer("AAAAAAAA", 9999, 5)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn chat_summaries_follow_every_change() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let chats: Vec<String> = (0..6).map(|i| format!("CHAT{i:04}")).collect();
        // Deterministic pseudo-random choices (xorshift), so a failure can be reproduced.
        let mut seed = 0x2026_1029_u64;
        let mut next = move |bound: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % bound
        };
        let mut message_id = 0_u64;
        for step in 0..1500_u64 {
            let chat = &chats[usize::try_from(next(6)).unwrap()];
            match next(10) {
                0..=3 => {
                    message_id += 1;
                    store
                        .insert_incoming(
                            chat,
                            message_id,
                            "text",
                            "hi",
                            i64::try_from(step).unwrap(),
                        )
                        .unwrap();
                }
                4 => {
                    message_id += 1;
                    store
                        .insert_outgoing(chat, message_id, "text", "ho")
                        .unwrap();
                }
                5 => {
                    // History placed between live messages, as an import does.
                    if let Some(id) = store
                        .conversation_messages(chat, 50)
                        .unwrap()
                        .first()
                        .map(|m| m.id)
                    {
                        store
                            .conn
                            .execute(
                                "UPDATE messages SET sort_key = ?2 WHERE id = ?1",
                                rusqlite::params![id, i64::try_from(next(1 << 40)).unwrap()],
                            )
                            .unwrap();
                    }
                }
                6 => {
                    store.mark_conversation_read(chat).unwrap();
                }
                7 => {
                    if let Some(id) = store
                        .conversation_messages(chat, 50)
                        .unwrap()
                        .last()
                        .map(|m| m.id)
                    {
                        store.remove_message(id).unwrap();
                    }
                }
                8 => {
                    if step % 60 == 0 {
                        store.empty_conversation(chat).unwrap();
                    }
                }
                _ => {
                    if step % 150 == 0 {
                        store.delete_conversation(chat).unwrap();
                    }
                }
            }
            if step % 37 == 0 {
                check_summaries(&store, step);
            }
        }
        check_summaries(&store, u64::MAX);
        // Something was actually there at the end, so the check means something.
        let left: i64 = store
            .conn
            .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert!(left > 100, "only {left} messages survived the run");
        assert!(!store.conversations().unwrap().is_empty());
    }

    /// Compares the summary table against what a full scan would say.
    fn check_summaries(store: &Store, step: u64) {
        let expected: Vec<(String, i64, i64, i64)> = {
            let mut statement = store
                .conn
                .prepare(
                    "SELECT conversation,
                            (SELECT n.id FROM messages n WHERE n.conversation = m.conversation
                             ORDER BY n.sort_key DESC, n.id DESC LIMIT 1),
                            (SELECT n.sort_key FROM messages n WHERE n.conversation = m.conversation
                             ORDER BY n.sort_key DESC, n.id DESC LIMIT 1),
                            sum(CASE WHEN outgoing = 0 AND state = 0 THEN 1 ELSE 0 END)
                     FROM messages m GROUP BY conversation ORDER BY conversation",
                )
                .unwrap();
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .unwrap();
            rows.map(Result::unwrap).collect()
        };
        let actual: Vec<(String, i64, i64, i64)> = {
            let mut statement = store
                .conn
                .prepare(
                    "SELECT chat, last_id, last_sort, unread FROM chat_summaries ORDER BY chat",
                )
                .unwrap();
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .unwrap();
            rows.map(Result::unwrap).collect()
        };
        assert_eq!(expected, actual, "summaries differ at step {step}");
    }

    #[test]
    fn marking_a_chat_unread_lasts_until_read_or_a_new_message() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        store
            .insert_incoming("AAAAAAAA", 1, "text", "hi", 1)
            .unwrap();
        store.mark_conversation_read("AAAAAAAA").unwrap();
        store.set_marked_unread("AAAAAAAA", true).unwrap();
        assert_eq!(store.marked_unread_chats().unwrap(), vec!["AAAAAAAA"]);
        // Reading the chat clears it.
        store.mark_conversation_read("AAAAAAAA").unwrap();
        assert!(store.marked_unread_chats().unwrap().is_empty());
        // So does a new message (its own unread count says it from then on).
        store.set_marked_unread("AAAAAAAA", true).unwrap();
        store
            .insert_incoming("AAAAAAAA", 2, "text", "and this", 2)
            .unwrap();
        assert!(store.marked_unread_chats().unwrap().is_empty());
        // Taking the mark away by hand works too.
        store.set_marked_unread("AAAAAAAA", true).unwrap();
        store.set_marked_unread("AAAAAAAA", false).unwrap();
        assert!(store.marked_unread_chats().unwrap().is_empty());
    }

    #[test]
    fn emptied_chats_stay_listed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        store
            .insert_incoming("AAAAAAAA", 1, "text", "old", 1)
            .unwrap();
        store
            .insert_incoming("BBBBBBBB", 2, "text", "hi", 2)
            .unwrap();
        store.empty_conversation("BBBBBBBB").unwrap();
        let list = store.conversations().unwrap();
        let kinds: Vec<(&str, &str)> = list
            .iter()
            .map(|(m, _)| (m.conversation.as_str(), m.kind.as_str()))
            .collect();
        // Emptied now, so newer than A's message.
        assert_eq!(kinds, [("BBBBBBBB", "empty"), ("AAAAAAAA", "text")]);
        assert!(
            store
                .conversation_messages("BBBBBBBB", 10)
                .unwrap()
                .is_empty()
        );
        // A new message replaces the stand-in.
        store
            .insert_incoming("BBBBBBBB", 3, "text", "again", now_ms())
            .unwrap();
        let list = store.conversations().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0.kind, "text");
        // Deleting forgets it.
        store.delete_conversation("BBBBBBBB").unwrap();
        assert_eq!(store.conversations().unwrap().len(), 1);
    }

    /// When the person writes again, the chat starts anew; emptying it keeps it as it is.
    #[test]
    fn a_deleted_chat_takes_its_marks_along_and_an_emptied_one_keeps_them() {
        use crate::chat_notifications::Policy;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        store.set_archived("BBBBBBBB", true).unwrap();
        store.set_marked_unread("BBBBBBBB", true).unwrap();
        store.set_setting("draft:BBBBBBBB", "unsent").unwrap();
        store.set_setting("draftQuote:BBBBBBBB", "ff").unwrap();
        store
            .set_chat_notifications("BBBBBBBB", Some((Policy::Never, None)))
            .unwrap();
        store.set_chat_tone("BBBBBBBB", Some("a.ogg")).unwrap();
        store.empty_conversation("BBBBBBBB").unwrap();
        assert_eq!(store.archived_chats().unwrap(), ["BBBBBBBB"]);
        assert_eq!(store.marked_unread_chats().unwrap().len(), 1);
        assert!(store.setting("draft:BBBBBBBB").unwrap().is_some());

        store.delete_conversation("BBBBBBBB").unwrap();
        assert!(store.archived_chats().unwrap().is_empty());
        assert!(store.marked_unread_chats().unwrap().is_empty());
        assert_eq!(store.setting("draft:BBBBBBBB").unwrap(), None);
        assert_eq!(store.setting("draftQuote:BBBBBBBB").unwrap(), None);
        // (A pin ends an archive mark, a private mark both.)
        store.set_pinned("BBBBBBBB", true).unwrap();
        store.delete_conversation("BBBBBBBB").unwrap();
        assert!(store.pinned_chats().unwrap().is_empty());
        assert!(store.conversations().unwrap().is_empty(), "no empty row");
        store.set_private("BBBBBBBB", true).unwrap();
        store.delete_conversation("BBBBBBBB").unwrap();
        // These belong to the contact, and the mark keeps hiding what comes next.
        assert!(store.chat_notifications("BBBBBBBB").unwrap().is_some());
        assert_eq!(store.chat_tones().unwrap().len(), 1);
        assert!(store.is_private("BBBBBBBB").unwrap());
    }

    #[test]
    fn imported_history_sits_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        // Live traffic first: ids 1 and 2, times 5000 and 6000.
        let live1 = store
            .insert_incoming("00000BOB", 1, "text", "live one", 5000)
            .unwrap()
            .unwrap();
        let live2 = store
            .insert_incoming("00000BOB", 2, "text", "live two", 6000)
            .unwrap()
            .unwrap();
        assert_eq!(live1.sort_key, live1.id << 20, "trigger assigned");

        // Imported history around them: older, between, and newer than everything.
        let positions = store.conversation_positions("00000BOB").unwrap();
        assert_eq!(positions.len(), 2);
        let mut placer = crate::android_backup::place::Placer::new(positions, 3);
        let import = |message_id: u64, body: &str, at: i64, key: i64| {
            store
                .insert_imported(&ImportedMessage {
                    conversation: "00000BOB",
                    message_id,
                    outgoing: false,
                    kind: "text",
                    body,
                    created_at: at,
                    updated_at: at,
                    state: INCOMING_READ,
                    edited_at: None,
                    sender: None,
                    starred: false,
                    sent_at: None,
                    delivered_at: None,
                    read_at: Some(at),
                    received_at: None,
                    sort_key: key,
                })
                .unwrap()
        };
        let key_old = placer.place(100);
        let key_mid = placer.place(5500);
        let key_new = placer.place(9000);
        assert!(import(100, "ancient", 100, key_old).is_some());
        assert!(import(101, "between", 5500, key_mid).is_some());
        assert!(import(102, "newest", 9000, key_new).is_some());
        // Deduplicated: the same (conversation, direction, message id) is not added again.
        assert!(import(101, "between again", 5500, key_mid + 1000).is_none());

        // The chat reads in history order even though the imported rows have higher ids.
        let page = store.conversation_messages("00000BOB", 50).unwrap();
        let texts: Vec<&str> = page.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(
            texts,
            ["ancient", "live one", "between", "live two", "newest"]
        );

        // Pages follow history, with no gaps or duplicates across the cursor.
        let newest_two = store.conversation_messages("00000BOB", 2).unwrap();
        assert_eq!(newest_two[0].body, "live two");
        assert_eq!(newest_two[1].body, "newest");
        let older = store
            .conversation_messages_before("00000BOB", newest_two[0].id, 2)
            .unwrap();
        assert_eq!(older[0].body, "live one");
        assert_eq!(older[1].body, "between");
        // A window around a message follows history as well, across imported and live rows.
        let (around, reaches_newest) = store
            .conversation_messages_around("00000BOB", older[1].id, 1)
            .unwrap();
        let texts: Vec<&str> = around.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(texts, ["live one", "between", "live two"]);
        assert!(!reaches_newest, "the imported newest one lies beyond it");

        // The UI gets the same order: a row keeps its place in history in its JSON, and only a
        // message newer than everything has the highest key.
        let key = |m: &Message| m.to_json()["sortKey"].as_i64().unwrap();
        let shown = store.conversation_messages("00000BOB", 50).unwrap();
        assert!(shown.windows(2).all(|pair| key(&pair[0]) < key(&pair[1])));
        assert!(key(&live1) < key(shown.last().unwrap()), "an old live row");

        // Change detection stays by insertion: everything imported after the last live row.
        let changed = store
            .conversation_messages_after("00000BOB", live2.id)
            .unwrap();
        assert_eq!(changed.len(), 3);

        // The chat list's preview is the newest by history — the imported "newest" here —
        // and a fresh live message takes over again.
        assert_eq!(store.conversations().unwrap()[0].0.body, "newest");
        store
            .insert_incoming("00000BOB", 3, "text", "fresh", 9500)
            .unwrap();
        assert_eq!(store.conversations().unwrap()[0].0.body, "fresh");

        // A live message with a skewed (old) clock still lands at the end: arrival order.
        store
            .insert_incoming("00000BOB", 4, "text", "skewed clock", 100)
            .unwrap();
        let all = store.conversation_messages("00000BOB", 50).unwrap();
        assert_eq!(all.last().unwrap().body, "skewed clock");
    }

    #[test]
    fn chats_without_messages_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let identity = libthreema::common::GroupIdentity {
            creator_identity: libthreema::common::ThreemaId::try_from("00000BOB").unwrap(),
            group_id: 7,
        };
        let group = crate::groups::Group {
            key: crate::groups::key(&identity),
            identity,
            name: "Team".into(),
            members: vec!["00000BOB".into()],
            left: false,
            created_at: 50,
            picture: None,
            picture_version: 0,
        };
        store.upsert_group(&group).unwrap();
        let list = crate::distribution_lists::DistributionList {
            key: "d:00000000000000ff".into(),
            name: "List".into(),
            members: Vec::new(),
            created_at: 10,
        };
        store.upsert_distribution_list(&list).unwrap();
        store
            .insert_incoming("00000BOB", 1, "text", "hi", 20)
            .unwrap();
        store
            .conn
            .execute("UPDATE messages SET received_at = created_at", [])
            .unwrap();

        // The group and the list are listed without any message, placed by creation date
        // (a contact without messages is not).
        let rows = store.conversations().unwrap();
        let kinds: Vec<(&str, &str)> = rows
            .iter()
            .map(|(m, _)| (m.conversation.as_str(), m.kind.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                (group.key.as_str(), "none"),
                ("00000BOB", "text"),
                ("d:00000000000000ff", "none")
            ]
        );
        assert_eq!(rows[0].1, 0, "no unread");

        // A real message replaces the stand-in; emptying keeps the chat as `empty`, not `none`.
        store
            .insert_incoming(&group.key, 2, "text", "yo", 60)
            .unwrap();
        assert_eq!(store.conversations().unwrap()[0].0.kind, "text");
        store.empty_conversation(&group.key).unwrap();
        let rows = store.conversations().unwrap();
        let state = rows
            .iter()
            .find(|(m, _)| m.conversation == group.key)
            .unwrap();
        assert_eq!(state.0.kind, "empty");
    }

    /// The chat list goes by when a message arrived. A sender's clock a year ahead doesn't pin
    /// their chat on top, and a message dated long ago still comes up as the new one it is.
    #[test]
    fn chat_list_goes_by_arrival_not_by_the_senders_clock() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let now = now_ms();
        let year = 365 * 86_400_000;
        let order = |store: &Store| -> Vec<String> {
            let rows = store.conversations().unwrap();
            rows.into_iter().map(|(m, _)| m.conversation).collect()
        };
        let ahead = store
            .insert_incoming("AAAAAAAA", 1, "text", "from the future", now + year)
            .unwrap()
            .unwrap();
        assert_eq!(ahead.arrived_at(), ahead.received_at.unwrap());
        store
            .conn
            .execute(
                "UPDATE messages SET received_at = ?1 WHERE id = ?2",
                params![now - 1000, ahead.id],
            )
            .unwrap();
        store
            .insert_incoming("BBBBBBBB", 2, "text", "from the past", now - year)
            .unwrap();
        assert_eq!(order(&store), ["BBBBBBBB", "AAAAAAAA"]);

        // Our own message has no arrival time and goes by the one clock it has.
        store
            .insert_outgoing("CCCCCCCC", 3, "text", "ours")
            .unwrap();
        let own = store.conversations().unwrap().remove(0).0;
        assert_eq!(own.conversation, "CCCCCCCC");
        assert_eq!(own.arrived_at(), own.created_at);
        assert_eq!(
            own.to_json()["arrivedAt"],
            serde_json::json!(own.created_at)
        );

        // A chat emptied just now compares its time with the same value: it stays above a message
        // dated far ahead that arrived before it.
        store
            .insert_incoming("DDDDDDDD", 4, "text", "bye", now)
            .unwrap();
        store.empty_conversation("DDDDDDDD").unwrap();
        store
            .conn
            .execute("UPDATE emptied_chats SET at = ?1", [now + 10_000])
            .unwrap();
        assert_eq!(order(&store)[0], "DDDDDDDD");
    }

    #[test]
    fn conversation_media() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let file = |media_type: &str, rendering: &str| {
            json!({ "mediaType": media_type, "rendering": rendering }).to_string()
        };
        let mut ids = Vec::new();
        for (n, (media_type, rendering)) in [
            ("image/jpeg", "media"),
            ("image/GIF", "media"),
            ("video/mp4", "media"),
            ("audio/aac", "media"),
            ("audio/mpeg", "file"),
            ("application/pdf", "file"),
        ]
        .into_iter()
        .enumerate()
        {
            let message = store
                .insert_outgoing("ECHOECHO", n as u64, "file", &file(media_type, rendering))
                .unwrap();
            ids.push(message.id);
        }
        store
            .insert_outgoing("ECHOECHO", 9, "text", "image/jpeg")
            .unwrap();
        store
            .insert_outgoing("00000BOB", 10, "file", &file("image/png", "media"))
            .unwrap();
        let kinds: Vec<&str> = store
            .conversation_media("ECHOECHO", &MEDIA_KINDS)
            .unwrap()
            .iter()
            .map(|m| media_kind(&m.body))
            .collect();
        assert_eq!(
            kinds,
            ["file", "audio", "voice", "video", "gif", "image"],
            "newest first"
        );
        let picked: Vec<i64> = store
            .conversation_media("ECHOECHO", &["image", "video"])
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(picked, vec![ids[2], ids[0]]);
    }

    /// A message deleted for everyone while it was still going out takes nothing with it: no
    /// clock, no queue entry, and a Reject of it stays without effect. Removing one leaves no
    /// receivers behind.
    #[test]
    fn a_deleted_message_has_nothing_left_to_send() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let sent = store.insert_outgoing("ECHOECHO", 7, "text", "hi").unwrap();
        store
            .raise_outgoing_state("ECHOECHO", &[7], OutgoingState::Sent)
            .unwrap();
        store.mark_deleted(sent.id, 9).unwrap();
        assert!(store.reset_outgoing("ECHOECHO", 7).unwrap().is_empty());
        let row = store.message(sent.id).unwrap().unwrap();
        assert_eq!(row.state, OutgoingState::Sent as i64);

        let going = "g:ECHOECHO:0000000000000001";
        let pending = store.insert_outgoing(going, 8, "text", "hi").unwrap();
        store
            .set_pending_receivers(going, 8, &["ABCD1234".to_owned()])
            .unwrap();
        let row = store.mark_deleted(pending.id, 9).unwrap().unwrap();
        assert_eq!(row.state, OutgoingState::Sent as i64);
        assert!(store.pending_receivers(going, 8).unwrap().is_empty());
        assert!(store.pending_outgoing().unwrap().is_empty());

        // What an older version left: a placeholder back in the queue.
        store
            .conn
            .execute("UPDATE messages SET state = 0 WHERE id = ?1", [pending.id])
            .unwrap();
        assert!(store.pending_outgoing().unwrap().is_empty());

        let removed = store.insert_outgoing(going, 10, "text", "hi").unwrap();
        store
            .set_pending_receivers(going, 10, &["ABCD1234".to_owned()])
            .unwrap();
        store.remove_message(removed.id).unwrap();
        assert!(store.pending_receivers(going, 10).unwrap().is_empty());
    }

    /// Upgrading: what is waiting may be out already, what was sent needn't say so, and a
    /// placeholder an older version put back into the queue comes out of it.
    #[test]
    fn the_written_out_migration_is_careful_with_whats_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let (waiting, sent, stuck) = {
            let store = Store::open(&path, &[1; 32]).unwrap();
            let waiting = store.insert_outgoing("ECHOECHO", 1, "text", "a").unwrap();
            let sent = store.insert_outgoing("ECHOECHO", 2, "text", "b").unwrap();
            store
                .raise_outgoing_state("ECHOECHO", &[2], OutgoingState::Sent)
                .unwrap();
            let stuck = store.insert_outgoing("ECHOECHO", 3, "deleted", "").unwrap();
            store
                .set_pending_receivers("ECHOECHO", 3, &["ABCD1234".to_owned()])
                .unwrap();
            store
                .conn
                .execute_batch(
                    "DROP TABLE awaiting_reflection;
                     ALTER TABLE messages DROP COLUMN written_out;
                     PRAGMA user_version = 35;",
                )
                .unwrap();
            (waiting.id, sent.id, stuck.id)
        };
        let store = Store::open(&path, &[1; 32]).unwrap();
        let row = |id| store.message(id).unwrap().unwrap();
        assert!(row(waiting).written_out);
        assert!(!row(sent).written_out);
        assert_eq!(row(stuck).state, OutgoingState::Sent as i64);
        assert!(store.pending_receivers("ECHOECHO", 3).unwrap().is_empty());
    }

    #[test]
    fn message_times() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let out = store.insert_outgoing("ECHOECHO", 7, "text", "hi").unwrap();
        assert_eq!(
            (out.sent_at, out.delivered_at, out.read_at),
            (None, None, None)
        );

        let sent = store
            .raise_outgoing_state("ECHOECHO", &[7], OutgoingState::Sent)
            .unwrap()
            .remove(0);
        assert!(sent.sent_at.is_some());
        assert_eq!(sent.delivered_at, None);
        // "Read" without "received" first: delivery time stays unknown.
        let read = store
            .raise_outgoing_state("ECHOECHO", &[7], OutgoingState::Read)
            .unwrap()
            .remove(0);
        assert_eq!(read.sent_at, sent.sent_at);
        assert_eq!(read.delivered_at, None);
        assert!(read.read_at.is_some());
        // Back in the queue (peer rejected it): sent again later, so the times start over.
        let reset = store.reset_outgoing("ECHOECHO", 7).unwrap().remove(0);
        assert_eq!((reset.sent_at, reset.read_at), (None, None));

        let incoming = store
            .insert_incoming("ECHOECHO", 9, "text", "hi", 5)
            .unwrap()
            .unwrap();
        assert_eq!(incoming.created_at, 5, "the sender's time");
        assert!(incoming.received_at.is_some());
        assert_eq!(incoming.read_at, None);
        store.mark_conversation_read("ECHOECHO").unwrap();
        let read = store.message(incoming.id).unwrap().unwrap();
        assert!(read.read_at.is_some());
        let json = read.to_json();
        assert_eq!(json["readAt"], serde_json::json!(read.read_at));
        assert_eq!(json["receivedAt"], serde_json::json!(read.received_at));
    }

    #[test]
    fn edit_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let incoming = store
            .insert_incoming("ECHOECHO", 7, "text", "helo", 1)
            .unwrap()
            .unwrap();
        // The sender's own message only: same id, other direction is not found.
        assert!(store.message_by_id("ECHOECHO", true, 7).unwrap().is_none());
        let found = store.message_by_id("ECHOECHO", false, 7).unwrap().unwrap();
        assert_eq!(found.id, incoming.id);

        let edited = store
            .edit_message(incoming.id, "hello", Some(5))
            .unwrap()
            .unwrap();
        assert_eq!(edited.body, "hello");
        assert_eq!(edited.edited_at, Some(5));
        // A silent edit (never sent) keeps the marker as it was.
        let edited = store
            .edit_message(incoming.id, "hello!", None)
            .unwrap()
            .unwrap();
        assert_eq!(edited.edited_at, Some(5));

        store.set_reaction(incoming.id, "", "👍", true, 6).unwrap();
        let deleted = store.mark_deleted(incoming.id, 9).unwrap().unwrap();
        assert_eq!(
            (deleted.kind.as_str(), deleted.body.as_str()),
            ("deleted", "")
        );
        assert_eq!(deleted.edited_at, Some(9));
        assert_eq!(deleted.reactions, serde_json::json!([]));
        // Deleted messages can't be edited or reacted to.
        assert_eq!(
            store
                .edit_message(incoming.id, "x", Some(10))
                .unwrap()
                .unwrap()
                .body,
            ""
        );
        assert!(store.reactable_message("ECHOECHO", 7).unwrap().is_none());

        store.remove_message(incoming.id).unwrap();
        assert!(store.message(incoming.id).unwrap().is_none());
    }

    /// Another client's edit of a caption takes the caption's place, with the edited mark and in
    /// the search; the rest of the file stays. Nothing else about a message can be edited.
    #[test]
    fn an_edit_changes_a_files_caption() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let body = serde_json::json!({"mediaType": "image/jpeg", "size": 5, "caption": "old"});
        let file = store
            .insert_incoming("ECHOECHO", 7, "file", &body.to_string(), 1)
            .unwrap()
            .unwrap();
        let edited = store
            .edit_message(file.id, "the right date", Some(5))
            .unwrap()
            .unwrap();
        let stored: serde_json::Value = serde_json::from_str(&edited.body).unwrap();
        assert_eq!(stored["caption"], "the right date");
        assert_eq!(stored["size"], 5);
        assert_eq!(edited.edited_at, Some(5));
        assert_eq!(
            store
                .search_messages("right", None, 10, |_| None)
                .unwrap()
                .len(),
            1
        );
        assert!(
            store
                .search_messages("old", None, 10, |_| None)
                .unwrap()
                .is_empty()
        );

        assert!(edit_applies_to("text") && edit_applies_to("file"));
        for kind in ["location", "poll", "pollClosed", "status", "deleted"] {
            assert!(!edit_applies_to(kind), "{kind}");
        }
        let location = store
            .insert_incoming("ECHOECHO", 8, "location", "{}", 1)
            .unwrap()
            .unwrap();
        let same = store
            .edit_message(location.id, "x", Some(5))
            .unwrap()
            .unwrap();
        assert_eq!((same.body.as_str(), same.edited_at), ("{}", None));
    }

    #[test]
    fn quotes() {
        // As Android writes it: the id's bytes 01 23 … ef in wire order.
        let text = "> quote #0123456789abcdef\n\nYes!";
        assert_eq!(split_quote(text), (Some(0xefcd_ab89_6745_2301), "Yes!"));
        assert_eq!(with_quote(Some(0xefcd_ab89_6745_2301), "Yes!"), text);
        // libthreema's own rendering of that id agrees.
        assert_eq!(
            libthreema::common::MessageId(0xefcd_ab89_6745_2301).to_string(),
            "0123456789abcdef"
        );
        for plain in [
            "> quote #0123\n\nx",
            "> quote #0123456789ABCDEF\n\nx",
            "> quote #0123456789abcdef x",
            "hi",
        ] {
            assert_eq!(split_quote(plain), (None, plain));
        }
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let m = store.insert_outgoing("ECHOECHO", 1, "text", text).unwrap();
        let edited = store.edit_message(m.id, "No!", Some(2)).unwrap().unwrap();
        assert_eq!(edited.body, "> quote #0123456789abcdef\n\nNo!");
        assert_eq!(edited.to_json()["text"], "No!");
        assert_eq!(edited.to_json()["quoteId"], "efcdab8967452301");
    }

    /// Timing of the chat list and a chat's page on a large store (`cargo test -p threesea-core
    /// --release -- --ignored --nocapture large_store`).
    #[test]
    #[ignore = "timing, not a check"]
    fn large_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        store.conn.execute_batch("BEGIN").unwrap();
        for i in 0..60_000u64 {
            let chat = format!("CHAT{:04}", i % 60);
            if i % 3 == 0 {
                store.insert_outgoing(&chat, i, "text", "{}").unwrap();
            } else {
                store.insert_incoming(&chat, i, "text", "{}", 0).unwrap();
            }
        }
        store.conn.execute_batch("COMMIT").unwrap();
        let start = std::time::Instant::now();
        let list = store.conversations().unwrap();
        eprintln!("conversations: {} in {:?}", list.len(), start.elapsed());
        let start = std::time::Instant::now();
        let page = store.conversation_messages("CHAT0007", 200).unwrap();
        eprintln!(
            "conversation_messages: {} in {:?}",
            page.len(),
            start.elapsed()
        );
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// P02 measurement (deep audit 2026-10-27): the chat list and search on a large history.
    /// `cargo test -p threesea-core --release --lib bench_large_history -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark"]
    fn bench_large_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let (chats, per_chat) = (1000_u64, 100_u64);
        let tx = store.conn.unchecked_transaction().unwrap();
        for chat in 0..chats {
            let conversation = format!("C{chat:07}");
            for n in 0..per_chat {
                store
                    .insert_incoming(
                        &conversation,
                        chat * per_chat + n,
                        "text",
                        &format!("message {n} in chat {chat}"),
                        i64::try_from(chat * per_chat + n).unwrap(),
                    )
                    .unwrap();
            }
        }
        tx.commit().unwrap();
        let time = |name: &str, f: &dyn Fn()| {
            f();
            let mut runs: Vec<f64> = (0..5)
                .map(|_| {
                    let start = std::time::Instant::now();
                    f();
                    start.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            runs.sort_by(f64::total_cmp);
            println!("{name}: {:.2} ms (median of 5)", runs[2]);
        };
        time("conversations", &|| {
            assert_eq!(store.conversations().unwrap().len(), 1000);
        });
        time("search, no match", &|| {
            assert!(
                store
                    .search_messages("zzzz", None, 50, |_| None)
                    .unwrap()
                    .is_empty()
            );
        });
        time("search, 50 hits of many", &|| {
            assert_eq!(
                store
                    .search_messages("chat 99", None, 50, |_| None)
                    .unwrap()
                    .len(),
                50
            );
        });
        time("search, 2 characters (scan)", &|| {
            store.search_messages("zz", None, 50, |_| None).unwrap();
        });
    }
}
