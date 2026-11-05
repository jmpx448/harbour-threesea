// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Web: contacts, groups, distribution lists and the profile, changed from the browser
//! (`create/`, `update/`, `delete/` of `contact`, `group`, `distributionList`, `update/profile`,
//! `request/groupSync`, `request/contactDetail`). Each runs our own commands, then tells our UI;
//! the browsers hear of the change from the next refresh, like of any other.

use rmpv::Value;
use serde_json::json;

use crate::{
    app::App,
    providers::CoreEvent,
    web::{
        arp::{self, Incoming, map},
        images,
        saltyrtc::field,
    },
};

type Code = &'static str;

/// A string field: `Some(None)` for an explicit `null` (clear it), `None` if left out.
fn optional<'a>(data: Option<&'a Value>, name: &str) -> Option<Option<&'a str>> {
    let value = field(data?, name)?;
    Some(value.as_str())
}

fn members(data: Option<&Value>) -> Option<Vec<String>> {
    Some(
        field(data?, "members")?
            .as_array()?
            .iter()
            .filter_map(|member| member.as_str().map(str::to_owned))
            .collect(),
    )
}

impl App {
    /// One receiver in the browser's model.
    fn web_receiver(&self, kind: &str, id: &str) -> Option<Value> {
        self.web_receivers_of(kind)
            .ok()?
            .into_iter()
            .find(|receiver| field(receiver, "id").and_then(Value::as_str) == Some(id))
    }

    /// Tell our UI that receivers of `kind` changed.
    fn web_receivers_changed(&mut self, kind: &str) {
        let event = match kind {
            "contact" => CoreEvent::ContactsChanged,
            "group" => CoreEvent::GroupsChanged,
            _ => CoreEvent::Call(json!({ "event": "chatsChanged" })),
        };
        let _ = self.media_events.send(event);
    }

    /// A picture from the browser as our JPEG (512 px) for `cmd` (`setGroupPicture`,
    /// `setProfilePicture`), or its removal (`remove`) for an explicit `null`.
    async fn web_picture(
        &mut self,
        data: Option<&Value>,
        set: serde_json::Value,
        remove: serde_json::Value,
    ) -> Result<(), Code> {
        let Some(avatar) = data.and_then(|data| field(data, "avatar")) else {
            return Ok(());
        };
        if avatar.is_nil() {
            return self
                .web_command(remove)
                .await
                .map(|_| ())
                .map_err(|_| "badRequest");
        }
        let jpeg = avatar
            .as_slice()
            .and_then(|bytes| images::scaled_jpeg(bytes, images::AVATAR_HIGH_MAX, 85))
            .ok_or("badRequest")?;
        let mut set = set;
        set["id"] = json!(0);
        let reply: serde_json::Value = serde_json::from_str(
            &self
                .handle_with(&set.to_string(), vec![zeroize::Zeroizing::new(jpeg)])
                .await,
        )
        .unwrap_or_default();
        if reply["ok"] == true {
            Ok(())
        } else {
            Err("badRequest")
        }
    }

    /// Management requests; `None` if `incoming` isn't one.
    pub(crate) async fn web_manage(
        &mut self,
        incoming: &Incoming<'_>,
    ) -> Option<Result<Value, Code>> {
        let confirm = || arp::message("update", "confirm", None, None);
        let data = incoming.data;
        // Private chats stay out of the browser's reach (ADR 0013): also for managing them, not
        // only for reading (security audit 2026-10-27, S02).
        let invalid = match (incoming.kind, incoming.sub_type) {
            ("update", "contact") => Some("invalidContact"),
            ("update" | "delete", "group") | ("request", "groupSync") => Some("invalidGroup"),
            ("update" | "delete", "distributionList") => Some("invalidDistributionList"),
            _ => None,
        };
        if let Some(code) = invalid
            && self.web_receiver_key(incoming.arg("id")).is_err()
        {
            return Some(Err(code));
        }
        Some(match (incoming.kind, incoming.sub_type) {
            ("create", "contact") => {
                let Some(identity) = data
                    .and_then(|d| field(d, "identity"))
                    .and_then(Value::as_str)
                else {
                    return Some(Err("invalidIdentity"));
                };
                let identity = identity.to_ascii_uppercase();
                match self
                    .web_command(json!({"cmd": "addContact", "identity": identity}))
                    .await
                {
                    Ok(_) => {
                        self.web_receivers_changed("contact");
                        Ok(arp::message(
                            "create",
                            "contact",
                            Some(map(vec![("identity", identity.as_str().into())])),
                            Some(map(vec![(
                                "receiver",
                                self.web_receiver("contact", &identity)
                                    .unwrap_or(Value::Nil),
                            )])),
                        ))
                    }
                    Err(_) => Err("invalidIdentity"),
                }
            }
            ("update", "contact") => {
                let Some(id) = incoming.arg("id").map(str::to_owned) else {
                    return Some(Err("invalidContact"));
                };
                let first = optional(data, "firstName");
                let last = optional(data, "lastName");
                if first.is_some() || last.is_some() {
                    let current = self.store.contact(&id).ok().flatten();
                    let keep = |new: Option<Option<&str>>, old: Option<String>| match new {
                        Some(value) => value.map(str::to_owned),
                        None => old,
                    };
                    let command = json!({"cmd": "updateContact", "identity": id,
                        "firstName": keep(first, current.as_ref().and_then(|c| c.first_name.clone())),
                        "lastName": keep(last, current.as_ref().and_then(|c| c.last_name.clone()))});
                    if self.web_command(command).await.is_err() {
                        return Some(Err("invalidContact"));
                    }
                }
                self.web_receivers_changed("contact");
                Ok(arp::message(
                    "update",
                    "contact",
                    Some(map(vec![("id", id.as_str().into())])),
                    Some(map(vec![(
                        "receiver",
                        self.web_receiver("contact", &id).unwrap_or(Value::Nil),
                    )])),
                ))
            }
            ("create", "group") => {
                let name = data
                    .and_then(|d| field(d, "name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let Some(members) = members(data) else {
                    return Some(Err("badRequest"));
                };
                match self
                    .web_command(json!({"cmd": "createGroup", "name": name, "members": members}))
                    .await
                {
                    Ok(group) => {
                        let key = group["key"].as_str().unwrap_or_default().to_owned();
                        let _ = self
                            .web_picture(
                                data,
                                json!({"cmd": "setGroupPicture", "key": key}),
                                json!({"cmd": "removeGroupPicture", "key": key}),
                            )
                            .await;
                        self.web_receivers_changed("group");
                        Ok(arp::message(
                            "create",
                            "group",
                            None,
                            Some(map(vec![(
                                "receiver",
                                self.web_receiver("group", &key).unwrap_or(Value::Nil),
                            )])),
                        ))
                    }
                    Err(kind) => Err(if kind == "groupNameTooLong" {
                        "valueTooLong"
                    } else {
                        "badRequest"
                    }),
                }
            }
            ("update", "group") => {
                let Some(key) = incoming.arg("id").map(str::to_owned) else {
                    return Some(Err("invalidGroup"));
                };
                if let Some(members) = members(data) {
                    if members.is_empty() {
                        return Some(Err("noMembers"));
                    }
                    if self
                        .web_command(
                            json!({"cmd": "setGroupMembers", "key": key, "members": members}),
                        )
                        .await
                        .is_err()
                    {
                        return Some(Err("notAllowed"));
                    }
                }
                if let Some(name) = optional(data, "name") {
                    let name = name.unwrap_or_default();
                    if self
                        .web_command(json!({"cmd": "renameGroup", "key": key, "name": name}))
                        .await
                        .is_err()
                    {
                        return Some(Err("notAllowed"));
                    }
                }
                if let Err(code) = self
                    .web_picture(
                        data,
                        json!({"cmd": "setGroupPicture", "key": key}),
                        json!({"cmd": "removeGroupPicture", "key": key}),
                    )
                    .await
                {
                    return Some(Err(code));
                }
                self.web_receivers_changed("group");
                Ok(arp::message(
                    "update",
                    "group",
                    Some(map(vec![("id", key.as_str().into())])),
                    Some(map(vec![(
                        "receiver",
                        self.web_receiver("group", &key).unwrap_or(Value::Nil),
                    )])),
                ))
            }
            ("delete", "group") => {
                let Some(key) = incoming.arg("id").map(str::to_owned) else {
                    return Some(Err("invalidGroup"));
                };
                let left = self
                    .store
                    .group(&key)
                    .ok()
                    .flatten()
                    .is_some_and(|group| group.left);
                if !left
                    && self
                        .web_command(json!({"cmd": "leaveGroup", "key": key}))
                        .await
                        .is_err()
                {
                    return Some(Err("invalidGroup"));
                }
                if incoming.arg("deleteType") == Some("delete")
                    && self
                        .web_command(json!({"cmd": "deleteConversation", "identity": key}))
                        .await
                        .is_err()
                {
                    return Some(Err("invalidGroup"));
                }
                self.web_receivers_changed("group");
                let _ = self
                    .media_events
                    .send(CoreEvent::Call(json!({ "event": "chatsChanged" })));
                Ok(confirm())
            }
            ("request", "groupSync") => {
                let Some(key) = incoming.arg("id").map(str::to_owned) else {
                    return Some(Err("invalidGroup"));
                };
                Ok(match self.web_group_sync(&key) {
                    Ok(()) => confirm(),
                    Err(code) => return Some(Err(code)),
                })
            }
            ("create", "distributionList") => {
                let name = data
                    .and_then(|d| field(d, "name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let Some(members) = members(data) else {
                    return Some(Err("badRequest"));
                };
                match self
                    .web_command(
                        json!({"cmd": "createDistributionList", "name": name, "members": members}),
                    )
                    .await
                {
                    Ok(list) => {
                        let key = list["key"].as_str().unwrap_or_default().to_owned();
                        self.web_receivers_changed("distributionList");
                        Ok(arp::message(
                            "create",
                            "distributionList",
                            None,
                            Some(map(vec![(
                                "receiver",
                                self.web_receiver("distributionList", &key)
                                    .unwrap_or(Value::Nil),
                            )])),
                        ))
                    }
                    Err(_) => Err("badRequest"),
                }
            }
            ("update", "distributionList") => {
                let Some(key) = incoming.arg("id").map(str::to_owned) else {
                    return Some(Err("invalidDistributionList"));
                };
                let members = members(data);
                if members.as_ref().is_some_and(Vec::is_empty) {
                    return Some(Err("noMembers"));
                }
                let name = optional(data, "name").map(|name| name.unwrap_or_default().to_owned());
                if self
                    .web_command(json!({"cmd": "updateDistributionList", "key": key, "name": name, "members": members}))
                    .await
                    .is_err()
                {
                    return Some(Err("invalidDistributionList"));
                }
                self.web_receivers_changed("distributionList");
                Ok(arp::message(
                    "update",
                    "distributionList",
                    Some(map(vec![("id", key.as_str().into())])),
                    Some(map(vec![(
                        "receiver",
                        self.web_receiver("distributionList", &key)
                            .unwrap_or(Value::Nil),
                    )])),
                ))
            }
            ("delete", "distributionList") => {
                let Some(key) = incoming.arg("id").map(str::to_owned) else {
                    return Some(Err("invalidDistributionList"));
                };
                if self
                    .web_command(json!({"cmd": "deleteConversation", "identity": key}))
                    .await
                    .is_err()
                {
                    return Some(Err("invalidDistributionList"));
                }
                self.web_receivers_changed("distributionList");
                Ok(confirm())
            }
            ("update", "profile") => {
                if let Some(nickname) = optional(data, "publicNickname") {
                    let nickname = nickname.unwrap_or_default();
                    if self
                        .web_command(json!({"cmd": "setNickname", "nickname": nickname}))
                        .await
                        .is_err()
                    {
                        return Some(Err("badRequest"));
                    }
                }
                if let Err(code) = self
                    .web_picture(
                        data,
                        json!({"cmd": "setProfilePicture"}),
                        json!({"cmd": "removeProfilePicture"}),
                    )
                    .await
                {
                    return Some(Err(code));
                }
                let _ = self
                    .media_events
                    .send(CoreEvent::Call(json!({ "event": "profileChanged" })));
                Ok(confirm())
            }
            // We keep no address book details with contacts.
            ("request", "contactDetail") => {
                let identity = incoming.arg("identity").unwrap_or_default();
                if self.store.contact(identity).ok().flatten().is_none() {
                    return Some(Err("invalidContact"));
                }
                Ok(arp::message(
                    "response",
                    "contactDetail",
                    Some(map(vec![("identity", identity.into())])),
                    Some(map(vec![("receiver", map(vec![]))])),
                ))
            }
            _ => return None,
        })
    }

    /// Group sync: the creator sends the group's state to all members again; a member asks the
    /// creator for it.
    fn web_group_sync(&self, key: &str) -> Result<(), Code> {
        let group = self.store.group(key).ok().flatten().ok_or("invalidGroup")?;
        if group.left {
            return Err("invalidGroup");
        }
        let user = self.user().map_err(|_| "invalidGroup")?;
        let creator = group.identity.creator_identity.to_string();
        let result = if creator == user {
            crate::groups::send_state(&self.store, &group, &group.members)
        } else {
            self.store.group_outbox_add(
                key,
                &[creator],
                crate::groups::OUTBOX_GROUP_SYNC_REQUEST,
                &json!({}),
            )
        };
        result.map_err(|_| "badRequest")?;
        self.tell_connection(crate::connection::ToConnection::SendPending);
        self.send_group_pictures();
        Ok(())
    }
}
