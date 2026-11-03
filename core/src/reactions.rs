// SPDX-License-Identifier: AGPL-3.0-only
//! Emoji reactions on messages, and the outbox for outgoing messages that are not shown as a
//! message of their own (reactions, edits, deletions).

use libthreema::model::message::LegacyReactionType;
use rusqlite::{OptionalExtension as _, params};
use serde_json::Value;

use crate::{
    messages::{Message, now_ms},
    store::{Store, StoreError},
};

/// Schema for reactions and the outbox (store migration 5).
pub(crate) const MIGRATION: &str = "
    CREATE TABLE reactions (
        message     INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
        sender      TEXT NOT NULL,
        emoji       TEXT NOT NULL,
        reacted_at  INTEGER NOT NULL,
        PRIMARY KEY (message, sender, emoji)
    ) WITHOUT ROWID;
    CREATE TABLE outbox (
        id            INTEGER PRIMARY KEY,
        conversation  TEXT NOT NULL,
        message_id    INTEGER NOT NULL,
        created_at    INTEGER NOT NULL,
        kind          TEXT NOT NULL,
        body          TEXT NOT NULL,
        UNIQUE (conversation, message_id)
    );";

/// `sender` of our own reactions.
pub const OWN_SENDER: &str = "";

/// The JSON array of a message's reactions (`[{sender, emoji}]`, oldest first) as a column of
/// `SELECT … FROM messages`. `id` resolves to the outer message: `reactions` has no `id` column.
pub(crate) const REACTIONS_COLUMN: &str = "(SELECT json_group_array(json_object('sender', sender, \
     'emoji', emoji)) FROM (SELECT sender, emoji FROM reactions WHERE message = id \
     ORDER BY reacted_at, emoji))";

/// A message waiting in the outbox.
#[derive(Debug, Clone)]
pub struct OutboxEntry {
    pub id: i64,
    pub conversation: String,
    pub message_id: u64,
    pub created_at: i64,
    pub kind: String,
    pub body: Value,
}

/// `kind` of a new-style reaction (`body`: `{messageId, emoji, apply}`).
pub const OUTBOX_REACTION: &str = "reaction";
/// `kind` of a legacy reaction (`body`: `{messageId, emoji, apply: true}`, emoji 👍 or 👎).
pub const OUTBOX_LEGACY_REACTION: &str = "legacyReaction";
/// `kind` of an edit (`body`: `{messageId, text}`).
pub const OUTBOX_EDIT: &str = "edit";
/// `kind` of a deletion for everyone (`body`: `{messageId}`).
pub const OUTBOX_DELETE: &str = "delete";

/// The _Legacy Reaction Mapping Steps_: thumbs up/down (any skin tone) map to the
/// acknowledge/decline delivery receipts that peers without `REACTION_SUPPORT` understand.
pub fn legacy_reaction(emoji: &str) -> Option<LegacyReactionType> {
    let base = emoji.trim_end_matches([
        '\u{1F3FB}',
        '\u{1F3FC}',
        '\u{1F3FD}',
        '\u{1F3FE}',
        '\u{1F3FF}',
    ]);
    // At most one skin tone modifier.
    if emoji.chars().count() - base.chars().count() > 1 {
        return None;
    }
    match base {
        "\u{1F44D}" => Some(LegacyReactionType::Acknowledge),
        "\u{1F44E}" => Some(LegacyReactionType::Decline),
        _ => None,
    }
}

/// The emoji shown for a received legacy reaction.
pub fn legacy_emoji(reaction: LegacyReactionType) -> &'static str {
    match reaction {
        LegacyReactionType::Acknowledge => "\u{1F44D}",
        LegacyReactionType::Decline => "\u{1F44E}",
    }
}

impl Store {
    /// The local id of a reactable message `message_id` in `conversation` (either direction;
    /// status lines are not reactable).
    pub fn reactable_message(
        &self,
        conversation: &str,
        message_id: u64,
    ) -> Result<Option<i64>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM messages
                 WHERE conversation = ?1 AND message_id = ?2 AND kind NOT IN ('status', 'deleted')
                 ORDER BY outgoing LIMIT 1",
                params![conversation, message_id.cast_signed()],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Apply (`apply`) or withdraw a reaction; returns the changed message.
    pub fn set_reaction(
        &self,
        message: i64,
        sender: &str,
        emoji: &str,
        apply: bool,
        reacted_at: i64,
    ) -> Result<Option<Message>, StoreError> {
        if apply {
            self.conn.execute(
                "INSERT INTO reactions (message, sender, emoji, reacted_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT DO UPDATE SET reacted_at = excluded.reacted_at",
                params![message, sender, emoji, reacted_at],
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM reactions WHERE message = ?1 AND sender = ?2 AND emoji = ?3",
                params![message, sender, emoji],
            )?;
        }
        self.touch(message)
    }

    /// A legacy reaction replaces all reactions of the sender on the message.
    pub fn replace_reactions(
        &self,
        message: i64,
        sender: &str,
        emoji: &str,
        reacted_at: i64,
    ) -> Result<Option<Message>, StoreError> {
        self.conn.execute(
            "DELETE FROM reactions WHERE message = ?1 AND sender = ?2",
            params![message, sender],
        )?;
        self.set_reaction(message, sender, emoji, true, reacted_at)
    }

    fn touch(&self, message: i64) -> Result<Option<Message>, StoreError> {
        self.conn.execute(
            "UPDATE messages SET updated_at = ?2 WHERE id = ?1",
            params![message, now_ms()],
        )?;
        self.message(message)
    }

    /// Queue a message for sending; `message_id` is its fresh Threema message id.
    pub fn outbox_add(
        &self,
        conversation: &str,
        message_id: u64,
        kind: &str,
        body: &Value,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO outbox (conversation, message_id, created_at, kind, body)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                conversation,
                message_id.cast_signed(),
                now_ms(),
                kind,
                body.to_string()
            ],
        )?;
        Ok(())
    }

    /// Everything in the outbox, oldest first.
    pub fn outbox(&self) -> Result<Vec<OutboxEntry>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id, conversation, message_id, created_at, kind, body FROM outbox ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(OutboxEntry {
                id: row.get(0)?,
                conversation: row.get(1)?,
                message_id: row.get::<_, i64>(2)?.cast_unsigned(),
                created_at: row.get(3)?,
                kind: row.get(4)?,
                body: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or(Value::Null),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Remove a sent (acknowledged) or unsendable outbox entry.
    pub fn outbox_remove(&self, conversation: &str, message_id: u64) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM outbox WHERE conversation = ?1 AND message_id = ?2",
            params![conversation, message_id.cast_signed()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn legacy_mapping() {
        assert!(matches!(
            legacy_reaction("👍"),
            Some(LegacyReactionType::Acknowledge)
        ));
        assert!(matches!(
            legacy_reaction("👍🏿"),
            Some(LegacyReactionType::Acknowledge)
        ));
        assert!(matches!(
            legacy_reaction("👎🏻"),
            Some(LegacyReactionType::Decline)
        ));
        assert!(legacy_reaction("👍🏻🏻").is_none());
        assert!(legacy_reaction("❤️").is_none());
        assert!(legacy_reaction("").is_none());
    }

    #[test]
    fn reactions_and_outbox() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let message = store.insert_outgoing("ECHOECHO", 7, "text", "hi").unwrap();
        assert_eq!(
            store.reactable_message("ECHOECHO", 7).unwrap(),
            Some(message.id)
        );
        assert_eq!(store.reactable_message("ECHOECHO", 8).unwrap(), None);

        store
            .set_reaction(message.id, "ECHOECHO", "❤️", true, 1)
            .unwrap();
        store
            .set_reaction(message.id, OWN_SENDER, "👍", true, 2)
            .unwrap();
        let changed = store
            .set_reaction(message.id, "ECHOECHO", "😂", true, 3)
            .unwrap()
            .unwrap();
        assert_eq!(
            changed.reactions,
            json!([
                {"sender": "ECHOECHO", "emoji": "❤️"},
                {"sender": "", "emoji": "👍"},
                {"sender": "ECHOECHO", "emoji": "😂"},
            ])
        );
        // A legacy reaction replaces the sender's reactions.
        let changed = store
            .replace_reactions(message.id, "ECHOECHO", "👎", 4)
            .unwrap()
            .unwrap();
        assert_eq!(
            changed.reactions,
            json!([{"sender": "", "emoji": "👍"}, {"sender": "ECHOECHO", "emoji": "👎"}])
        );
        let changed = store
            .set_reaction(message.id, OWN_SENDER, "👍", false, 5)
            .unwrap()
            .unwrap();
        assert_eq!(
            changed.reactions,
            json!([{"sender": "ECHOECHO", "emoji": "👎"}])
        );
        // In the message list too.
        let list = store.conversation_messages("ECHOECHO", 10).unwrap();
        assert_eq!(list[0].reactions, changed.reactions);
        // Deleted with the conversation.
        store.delete_conversation("ECHOECHO").unwrap();
        let left: i64 = store
            .conn
            .query_row("SELECT count(*) FROM reactions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);

        store
            .outbox_add("ECHOECHO", 9, OUTBOX_REACTION, &json!({"messageId": "7"}))
            .unwrap();
        let outbox = store.outbox().unwrap();
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].message_id, 9);
        assert_eq!(outbox[0].body["messageId"], "7");
        store.outbox_remove("ECHOECHO", 9).unwrap();
        assert!(store.outbox().unwrap().is_empty());
    }
}
