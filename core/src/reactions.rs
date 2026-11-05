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

/// A message's reactions from [`REACTIONS_COLUMN`]; those that aren't an emoji stay out (older
/// versions stored whatever a peer sent).
pub(crate) fn shown_reactions(json: Option<&str>) -> Value {
    let mut reactions: Vec<Value> = json
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    reactions.retain(|reaction| reaction["emoji"].as_str().is_some_and(is_emoji));
    Value::Array(reactions)
}

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
/// A delivery receipt for a received message: `{messageId}` (hex).
pub const OUTBOX_DELIVERY_RECEIPT: &str = "deliveryReceipt";

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

/// Most elements a joined emoji has (a kiss or a family has four).
const MAX_JOINED_EMOJI: usize = 4;

/// Whether `text` is one emoji and nothing else: a reaction shows under someone's message as if
/// the app said it, so a peer's arbitrary text must not get there (Android sends only real
/// emoji). By form rather than from a list, so emoji newer than our pictures still count: a flag,
/// a keycap, or pictographs with their skin tones and variation selectors, joined.
pub fn is_emoji(text: &str) -> bool {
    const JOINER: char = '\u{200D}';
    const KEYCAP: char = '\u{20E3}';
    const VARIATION: char = '\u{FE0F}';
    let regional = |c: char| matches!(c, '\u{1F1E6}'..='\u{1F1FF}');
    let modifier = |c: char| matches!(c, '\u{FE0E}' | VARIATION | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}');
    let pictograph = |c: char| {
        !regional(c)
            && !modifier(c)
            && matches!(c,
                '\u{1F000}'..='\u{1FAFF}'
                | '\u{2190}'..='\u{21FF}'
                | '\u{2300}'..='\u{23FF}'
                | '\u{25A0}'..='\u{25FF}'
                | '\u{2600}'..='\u{27BF}'
                | '\u{2B00}'..='\u{2BFF}'
                | '\u{A9}' | '\u{AE}' | '\u{203C}' | '\u{2049}' | '\u{2122}' | '\u{2139}'
                | '\u{24C2}' | '\u{2934}' | '\u{2935}' | '\u{3030}' | '\u{303D}' | '\u{3297}'
                | '\u{3299}')
    };
    let chars: Vec<char> = text.chars().collect();
    match chars.as_slice() {
        [a, b] if regional(*a) && regional(*b) => return true,
        ['0'..='9' | '#' | '*', rest @ ..] => {
            return matches!(rest, [KEYCAP] | [VARIATION, KEYCAP]);
        }
        _ => {}
    }
    // Enclosed letters stand alone: a joiner would string them into a word.
    let joinable = !chars.iter().any(|c| matches!(c, '\u{1F100}'..='\u{1F1FF}'));
    let mut elements = 0;
    let mut base_next = true;
    for c in chars {
        if base_next {
            if !pictograph(c) {
                return false;
            }
            elements += 1;
            base_next = false;
        } else if c == JOINER {
            base_next = true;
        } else if !modifier(c) {
            return false;
        }
    }
    !base_next && (elements == 1 || (joinable && elements <= MAX_JOINED_EMOJI))
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

    /// Apply (`apply`) or withdraw a reaction; returns the changed message. What isn't an emoji
    /// is dropped (`None`).
    pub fn set_reaction(
        &self,
        message: i64,
        sender: &str,
        emoji: &str,
        apply: bool,
        reacted_at: i64,
    ) -> Result<Option<Message>, StoreError> {
        if !is_emoji(emoji) {
            tracing::debug!("reaction that isn't an emoji dropped");
            return Ok(None);
        }
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

    /// A reaction from a backup: added if it isn't there, one already there stays as it is, and the
    /// message isn't touched. Returns whether it was added; what isn't an emoji isn't.
    pub fn add_imported_reaction(
        &self,
        message: i64,
        sender: &str,
        emoji: &str,
        reacted_at: i64,
    ) -> Result<bool, StoreError> {
        if !is_emoji(emoji) {
            tracing::debug!("reaction that isn't an emoji dropped");
            return Ok(false);
        }
        Ok(self.conn.execute(
            "INSERT OR IGNORE INTO reactions (message, sender, emoji, reacted_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![message, sender, emoji, reacted_at],
        )? > 0)
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
        let rows = statement.query_map([], outbox_entry)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// A conversation's outbox entries queued no later than `created_at`, oldest first.
    pub fn outbox_until(
        &self,
        conversation: &str,
        created_at: i64,
    ) -> Result<Vec<OutboxEntry>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id, conversation, message_id, created_at, kind, body FROM outbox
                 WHERE conversation = ?1 AND created_at <= ?2 ORDER BY id",
        )?;
        let rows = statement.query_map(params![conversation, created_at], outbox_entry)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Remove everything queued for a contact that is gone. (Not with a group's or a chat's
    /// deletion: a group's setup, leave and "for everyone" entries still have to go out.)
    pub fn outbox_clear(&self, conversation: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM outbox WHERE conversation = ?1", [conversation])?;
        Ok(())
    }

    /// One outbox entry.
    pub fn outbox_entry(
        &self,
        conversation: &str,
        message_id: u64,
    ) -> Result<Option<OutboxEntry>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, conversation, message_id, created_at, kind, body FROM outbox
                     WHERE conversation = ?1 AND message_id = ?2",
                params![conversation, message_id.cast_signed()],
                outbox_entry,
            )
            .optional()?)
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

fn outbox_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxEntry> {
    Ok(OutboxEntry {
        id: row.get(0)?,
        conversation: row.get(1)?,
        message_id: row.get::<_, i64>(2)?.cast_unsigned(),
        created_at: row.get(3)?,
        kind: row.get(4)?,
        body: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or(Value::Null),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn only_a_single_emoji_is_a_reaction() {
        for emoji in [
            "\u{1F44D}",                                                    // thumbs up
            "\u{1F44D}\u{1F3FF}",                                           // with a skin tone
            "\u{2764}\u{FE0F}",                                             // heart
            "\u{1F1E8}\u{1F1ED}",                                           // flag
            "1\u{FE0F}\u{20E3}",                                            // keycap
            "\u{1F17F}\u{FE0F}",                                            // enclosed letter
            "\u{1F3F4}\u{E0067}\u{E0062}\u{E007F}",                         // tag flag
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}", // family
        ] {
            assert!(is_emoji(emoji), "{emoji}");
        }
        for text in [
            "",
            "Paid in full",
            "Verified",
            "OK",
            "1",
            "\u{1F44D}\u{1F44D}",          // two
            "\u{1F44D} ",                  // with a space
            "\u{1F44D}x",                  // with a letter
            "\u{1F3FF}",                   // a skin tone alone
            "\u{1F1E8}",                   // half a flag
            "\u{1F1E8}\u{1F1ED}\u{1F1E9}", // a flag and a half
            "\u{200D}\u{1F44D}",           // starting with a joiner
            "\u{1F44D}\u{200D}",           // ending with one
            // "PAID" and "OK" in enclosed letters, joined so they would count as one.
            "\u{1F17F}\u{200D}\u{1F170}\u{200D}\u{1F178}\u{200D}\u{1F173}",
            "\u{1F17E}\u{200D}\u{1F17A}",
            // More joined than any real emoji.
            "\u{1F44D}\u{200D}\u{1F44D}\u{200D}\u{1F44D}\u{200D}\u{1F44D}\u{200D}\u{1F44D}",
        ] {
            assert!(!is_emoji(text), "{text}");
        }
    }

    /// Every emoji the app has a picture for (Threema's own list) can be reacted with.
    #[test]
    fn every_emoji_of_the_table_counts() {
        let table = include_str!("../../qml/components/EmojiTable.js");
        let table = table.split_once("var table = {").unwrap().1;
        let mut seen = 0;
        for entry in table.split(",\"").map(|e| e.trim_start_matches('"')) {
            let Some((key, _)) = entry.split_once("\":\"") else {
                continue;
            };
            let emoji: String = key
                .split('-')
                .map(|c| char::from_u32(u32::from_str_radix(c, 16).unwrap()).unwrap())
                .collect();
            assert!(is_emoji(&emoji), "{key}");
            seen += 1;
        }
        assert!(seen > 3000, "{seen}");
    }

    /// A peer's text: nothing stored, and what an older version stored doesn't come back out.
    #[test]
    fn text_as_a_reaction_is_dropped_and_old_ones_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let id = store
            .insert_incoming("BBBBBBBB", 1, "text", "hi", 1)
            .unwrap()
            .unwrap()
            .id;
        assert!(
            store
                .set_reaction(id, "BBBBBBBB", "Paid in full", true, 2)
                .unwrap()
                .is_none()
        );
        let thumbs = store
            .set_reaction(id, "BBBBBBBB", "\u{1F44D}", true, 3)
            .unwrap()
            .unwrap();
        assert_eq!(thumbs.reactions.as_array().unwrap().len(), 1);
        store
            .conn
            .execute(
                "INSERT INTO reactions (message, sender, emoji, reacted_at)
                 VALUES (?1, 'BBBBBBBB', 'Verified', 4)",
                [id],
            )
            .unwrap();
        let message = store.message(id).unwrap().unwrap();
        assert_eq!(message.reactions.as_array().unwrap().len(), 1);
        assert_eq!(message.reactions[0]["emoji"], "\u{1F44D}");
    }

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
