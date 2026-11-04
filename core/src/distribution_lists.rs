// SPDX-License-Identifier: AGPL-3.0-only
//! Distribution lists, like Android's (`DistributionListMessageReceiver`): a chat of its own
//! whose messages are sent to every member as a normal 1:1 message (which also shows in the
//! member's chat). Local only, nothing in the protocol: table `distribution_lists`
//! (migration 14), conversation key `d:<16 hex>`.

use rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::{
    app::{ApiError, App},
    messages::now_ms,
    store::{Store, StoreError},
};

/// Schema (store migration 14).
pub(crate) const MIGRATION: &str = "
    CREATE TABLE distribution_lists (
        key         TEXT PRIMARY KEY NOT NULL,
        name        TEXT NOT NULL,
        members     TEXT NOT NULL,
        created_at  INTEGER NOT NULL
    ) WITHOUT ROWID;";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistributionList {
    pub key: String,
    pub name: String,
    pub members: Vec<String>,
    pub created_at: i64,
}

pub fn is_key(key: &str) -> bool {
    key.strip_prefix("d:")
        .is_some_and(|hex| hex.len() == 16 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub fn new_key() -> String {
    format!("d:{:016x}", rand::random::<u64>())
}

impl DistributionList {
    pub fn to_json(&self, display_name: impl Fn(&str) -> String) -> Value {
        json!({
            "key": self.key,
            "name": self.name,
            "members": self.members.iter().map(|m| json!({
                "identity": m,
                "displayName": display_name(m),
            })).collect::<Vec<_>>(),
            "createdAt": self.created_at,
        })
    }
}

impl Store {
    pub fn distribution_list(&self, key: &str) -> Result<Option<DistributionList>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT key, name, members, created_at FROM distribution_lists WHERE key = ?1",
                [key],
                |row| {
                    let members: String = row.get(2)?;
                    Ok(DistributionList {
                        key: row.get(0)?,
                        name: row.get(1)?,
                        members: serde_json::from_str(&members).unwrap_or_default(),
                        created_at: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn upsert_distribution_list(&self, list: &DistributionList) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO distribution_lists (key, name, members, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                list.key,
                list.name,
                serde_json::to_string(&list.members).unwrap_or_else(|_| "[]".into()),
                list.created_at
            ],
        )?;
        Ok(())
    }

    pub fn distribution_lists(&self) -> Result<Vec<DistributionList>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT key FROM distribution_lists ORDER BY created_at")?;
        let keys: Vec<String> = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        let mut lists = Vec::with_capacity(keys.len());
        for key in keys {
            lists.extend(self.distribution_list(&key)?);
        }
        Ok(lists)
    }

    /// The list and its chat.
    pub fn delete_distribution_list(&self, key: &str) -> Result<(), StoreError> {
        let step = self.conn.unchecked_transaction()?;
        self.conn
            .execute("DELETE FROM distribution_lists WHERE key = ?1", [key])?;
        self.forget_chat_choices(key)?;
        self.delete_chat_rows(key)?;
        step.commit()?;
        Ok(())
    }

    pub fn create_distribution_list(
        &self,
        name: &str,
        members: Vec<String>,
    ) -> Result<DistributionList, StoreError> {
        let list = DistributionList {
            key: new_key(),
            name: name.to_owned(),
            members,
            created_at: now_ms(),
        };
        self.upsert_distribution_list(&list)?;
        Ok(list)
    }
}

impl App {
    /// A list as the UI gets it; `displayName` and `members` like a group's.
    pub(crate) fn distribution_list_json(&self, key: &str) -> Result<Value, ApiError> {
        let list = self
            .store
            .distribution_list(key)?
            .ok_or(ApiError::GroupNotFound)?;
        let mut json = list.to_json(|m| self.display_name_of(m));
        json["displayName"] = self.list_title(&list).into();
        json["pinned"] = self
            .store
            .pinned_chats()
            .is_ok_and(|pinned| pinned.contains(&list.key))
            .into();
        Ok(json)
    }

    /// Its name, else its members' names.
    pub(crate) fn list_title(&self, list: &DistributionList) -> String {
        crate::app_groups::title(&list.name, &list.members, &|m| self.display_name_of(m))
    }

    pub(crate) fn create_distribution_list(
        &mut self,
        name: &str,
        members: &[String],
    ) -> Result<Value, ApiError> {
        let user = self.user()?;
        let name = Self::check_name(name)?;
        let members = self.member_list(members, &user)?;
        let list = self.store.create_distribution_list(&name, members)?;
        // So the new list shows in the chat list right away.
        self.store
            .insert_status(&list.key, "distributionListCreated")?;
        tracing::info!(members = list.members.len(), "distribution list created");
        self.distribution_list_json(&list.key)
    }

    pub(crate) fn update_distribution_list(
        &mut self,
        key: &str,
        name: Option<&str>,
        members: Option<&[String]>,
    ) -> Result<Value, ApiError> {
        let mut list = self
            .store
            .distribution_list(key)?
            .ok_or(ApiError::GroupNotFound)?;
        if let Some(name) = name {
            list.name = Self::check_name(name)?;
        }
        if let Some(members) = members {
            let user = self.user()?;
            list.members = self.member_list(members, &user)?;
        }
        self.store.upsert_distribution_list(&list)?;
        self.distribution_list_json(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let mut list = store
            .create_distribution_list("Family", vec!["00000BOB".into(), "00000ANN".into()])
            .unwrap();
        assert!(is_key(&list.key));
        assert!(!is_key("g:ABC:0123456789abcdef"));
        assert_eq!(
            store.distribution_list(&list.key).unwrap(),
            Some(list.clone())
        );
        list.members.pop();
        list.name = "Bob".into();
        store.upsert_distribution_list(&list).unwrap();
        assert_eq!(
            store.distribution_list(&list.key).unwrap().unwrap().members,
            ["00000BOB"]
        );
        store.delete_distribution_list(&list.key).unwrap();
        assert_eq!(store.distribution_list(&list.key).unwrap(), None);
    }
}
