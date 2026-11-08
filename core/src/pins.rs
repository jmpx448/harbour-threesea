// SPDX-License-Identifier: AGPL-3.0-only
//! Pinned chats, on top of the chat list in the user's own order (Android pins conversations;
//! the order is ours, the user asked for it). Local only: table `pinned_chats` (migration 13).

use rusqlite::params;

use crate::store::{Store, StoreError};

/// Schema (store migration 13).
pub(crate) const MIGRATION: &str = "
    CREATE TABLE pinned_chats (
        conversation  TEXT PRIMARY KEY NOT NULL,
        position      INTEGER NOT NULL
    ) WITHOUT ROWID;";

impl Store {
    /// Pinned conversations, in order.
    pub fn pinned_chats(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT conversation FROM pinned_chats ORDER BY position")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn save_pinned(&self, order: &[String]) -> Result<(), StoreError> {
        let step = self.step()?;
        self.conn.execute("DELETE FROM pinned_chats", [])?;
        for (position, conversation) in order.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO pinned_chats (conversation, position) VALUES (?1, ?2)",
                params![conversation, i64::try_from(position).unwrap_or(i64::MAX)],
            )?;
        }
        step.commit()?;
        Ok(())
    }

    /// Pin (at the end of the pinned chats) or unpin.
    pub fn set_pinned(&self, conversation: &str, pinned: bool) -> Result<(), StoreError> {
        let mut order = self.pinned_chats()?;
        order.retain(|c| c != conversation);
        if pinned {
            order.push(conversation.to_owned());
        }
        self.save_pinned(&order)
    }

    /// Move a pinned chat by `offset` places (negative: up), clamped to the list.
    pub fn move_pinned(&self, conversation: &str, offset: i64) -> Result<(), StoreError> {
        let mut order = self.pinned_chats()?;
        let Some(from) = order.iter().position(|c| c == conversation) else {
            return Ok(());
        };
        let last = i64::try_from(order.len()).unwrap_or(i64::MAX) - 1;
        let to = (i64::try_from(from).unwrap_or(0) + offset).clamp(0, last);
        let item = order.remove(from);
        order.insert(usize::try_from(to).unwrap_or(0), item);
        self.save_pinned(&order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        for c in ["A", "B", "C"] {
            store.set_pinned(c, true).unwrap();
        }
        store.move_pinned("C", -1).unwrap();
        assert_eq!(store.pinned_chats().unwrap(), ["A", "C", "B"]);
        store.move_pinned("A", -5).unwrap();
        store.move_pinned("B", 3).unwrap();
        assert_eq!(store.pinned_chats().unwrap(), ["A", "C", "B"]);
        store.set_pinned("A", false).unwrap();
        store.set_pinned("A", true).unwrap();
        assert_eq!(store.pinned_chats().unwrap(), ["C", "B", "A"]);
        store.move_pinned("nope", 1).unwrap();
    }
}
