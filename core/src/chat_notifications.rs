// SPDX-License-Identifier: AGPL-3.0-only
//! Per-chat notification settings, like Android's `NotificationsActivity` (the D2D sync's
//! `NotificationTriggerPolicyOverride`): muted (`never`, optionally until a time) or, for groups,
//! only when we are mentioned (`mentioned`). Kept per conversation key (Threema ID or group key)
//! in our own table, since groups have no such field in libthreema's model.

use rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::{
    mentions,
    messages::{Message, now_ms},
    store::{Store, StoreError},
};

/// Schema (store migration 12).
pub(crate) const MIGRATION: &str = "
    CREATE TABLE chat_notifications (
        conversation  TEXT PRIMARY KEY NOT NULL,
        policy        TEXT NOT NULL,
        until         INTEGER
    ) WITHOUT ROWID;";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// No notifications (until a time, if set).
    Never,
    /// Groups: only when we or everyone are mentioned.
    Mentioned,
}

impl Policy {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "never" => Some(Self::Never),
            "mentioned" => Some(Self::Mentioned),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Mentioned => "mentioned",
        }
    }
}

impl Store {
    /// The chat's setting, if any and not expired.
    pub fn chat_notifications(
        &self,
        conversation: &str,
    ) -> Result<Option<(Policy, Option<i64>)>, StoreError> {
        let row: Option<(String, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT policy, until FROM chat_notifications WHERE conversation = ?1",
                [conversation],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(policy, until)| {
            let policy = Policy::parse(&policy)?;
            match until {
                Some(until) if until <= now_ms() => None,
                _ => Some((policy, until)),
            }
        }))
    }

    /// Every chat whose notifications are off or limited right now (the chat list's bell):
    /// one query instead of one per chat.
    pub fn muted_chats(&self) -> Result<std::collections::HashSet<String>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT conversation FROM chat_notifications
             WHERE policy IN ('never', 'mentioned') AND (until IS NULL OR until > ?1)",
        )?;
        let rows = statement.query_map([now_ms()], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// `None`: notifications on (the default).
    pub fn set_chat_notifications(
        &self,
        conversation: &str,
        setting: Option<(Policy, Option<i64>)>,
    ) -> Result<(), StoreError> {
        match setting {
            Some((policy, until)) => self.conn.execute(
                "INSERT OR REPLACE INTO chat_notifications (conversation, policy, until)
                 VALUES (?1, ?2, ?3)",
                params![conversation, policy.as_str(), until],
            )?,
            None => self.conn.execute(
                "DELETE FROM chat_notifications WHERE conversation = ?1",
                [conversation],
            )?,
        };
        Ok(())
    }

    /// `{policy: "never"|"mentioned"|null, until: ms|null}` for the UI.
    pub fn chat_notifications_json(&self, conversation: &str) -> Value {
        match self.chat_notifications(conversation).ok().flatten() {
            Some((policy, until)) => json!({ "policy": policy.as_str(), "until": until }),
            None => json!({ "policy": null, "until": null }),
        }
    }

    /// Whether an incoming message should notify (the global switch is the UI's).
    pub fn should_notify(&self, message: &Message) -> bool {
        // Of the status lines only missed calls (app_calls.rs) are worth a notification.
        if message.outgoing
            || (message.kind == "status" && !message.body.starts_with("call|missed|"))
        {
            return false;
        }
        match self.chat_notifications(&message.conversation) {
            Ok(None) | Err(_) => true,
            Ok(Some((Policy::Never, _))) => false,
            Ok(Some((Policy::Mentioned, _))) => self.mentions_me(message),
        }
    }

    fn mentions_me(&self, message: &Message) -> bool {
        let json = message.to_json();
        let text = json["text"]
            .as_str()
            .or_else(|| json["body"]["caption"].as_str())
            .unwrap_or_default();
        let own = self.identity().ok().flatten().map(|own| own.threema_id);
        mentions::identities(text)
            .into_iter()
            .any(|id| id == mentions::EVERYONE || own.as_deref() == Some(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_expire() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        assert_eq!(store.chat_notifications("00000BOB").unwrap(), None);
        store
            .set_chat_notifications("00000BOB", Some((Policy::Never, None)))
            .unwrap();
        assert_eq!(
            store.chat_notifications("00000BOB").unwrap(),
            Some((Policy::Never, None))
        );
        store
            .set_chat_notifications("00000BOB", Some((Policy::Never, Some(now_ms() - 1))))
            .unwrap();
        assert_eq!(store.chat_notifications("00000BOB").unwrap(), None);
        let later = now_ms() + 3_600_000;
        store
            .set_chat_notifications("g:abc", Some((Policy::Mentioned, Some(later))))
            .unwrap();
        assert_eq!(
            store.chat_notifications_json("g:abc"),
            json!({ "policy": "mentioned", "until": later })
        );
        store.set_chat_notifications("g:abc", None).unwrap();
        assert_eq!(store.chat_notifications("g:abc").unwrap(), None);
    }

    #[test]
    fn notify_decision() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let plain = store
            .insert_incoming("g:abc", 1, "text", "Hello", now_ms())
            .unwrap()
            .unwrap();
        let everyone = store
            .insert_incoming("g:abc", 2, "text", "Hi @[@@@@@@@@]", now_ms())
            .unwrap()
            .unwrap();
        assert!(store.should_notify(&plain));
        store
            .set_chat_notifications("g:abc", Some((Policy::Mentioned, None)))
            .unwrap();
        assert!(!store.should_notify(&plain));
        assert!(store.should_notify(&everyone));
        store
            .set_chat_notifications("g:abc", Some((Policy::Never, None)))
            .unwrap();
        assert!(!store.should_notify(&everyone));
    }
}
