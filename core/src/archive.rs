// SPDX-License-Identifier: AGPL-3.0-only
//! Archived chats, like Android's archive: out of the chat list into "Archived chats", back
//! when a new message arrives. Local only: table `archived_chats` (migration 16).

use crate::store::{Store, StoreError};

/// Schema (store migration 16).
pub(crate) const MIGRATION: &str = "
    CREATE TABLE archived_chats (
        conversation  TEXT PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;";

impl Store {
    pub fn archived_chats(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT conversation FROM archived_chats")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Bring a chat back from the archive; `true` if it was there.
    pub fn unarchive(&self, conversation: &str) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "DELETE FROM archived_chats WHERE conversation = ?1",
            [conversation],
        )? > 0)
    }

    /// Archive (also unpins: a pinned chat in the archive makes no sense) or bring back.
    pub fn set_archived(&self, conversation: &str, archived: bool) -> Result<(), StoreError> {
        if archived {
            self.conn.execute(
                "INSERT OR IGNORE INTO archived_chats (conversation) VALUES (?1)",
                [conversation],
            )?;
            self.set_pinned(conversation, false)?;
        } else {
            self.conn.execute(
                "DELETE FROM archived_chats WHERE conversation = ?1",
                [conversation],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        store.set_pinned("A", true).unwrap();
        store.set_archived("A", true).unwrap();
        store.set_archived("A", true).unwrap();
        assert_eq!(store.archived_chats().unwrap(), ["A"]);
        assert!(store.pinned_chats().unwrap().is_empty());
        assert!(!store.unarchive("B").unwrap(), "was not there");
        assert!(store.unarchive("A").unwrap());
        assert!(!store.unarchive("A").unwrap(), "not any more");
        store.set_archived("A", true).unwrap();
        store.set_archived("A", false).unwrap();
        assert!(store.archived_chats().unwrap().is_empty());
    }
}
