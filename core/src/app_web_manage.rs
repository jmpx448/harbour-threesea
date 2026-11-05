// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Web: contacts, groups, distribution lists and the profile, changed from the browser
//! (`create/`, `update/`, `delete/` of `contact`, `group`, `distributionList`, `update/profile`,
//! `request/groupSync`, `request/contactDetail`). Each runs our own commands, then tells our UI;
//! the browsers hear of the change from the next refresh, like of any other.

use rmpv::Value;
use serde_json::json;

use crate::{
    app::App,
    app_web_data::receiver_type,
    providers::CoreEvent,
    web::{
        arp::{self, Incoming, map},
        images,
        saltyrtc::field,
    },
};

type Code = &'static str;

/// Threema's limit for a contact's first and last name, in bytes.
const CONTACT_NAME_BYTES_MAX: usize = 256;

/// Threema's limit for our nickname, in bytes. The browser counts UTF-16 units and would keep
/// showing a nickname that `setNickname` cuts.
const NICKNAME_BYTES_MAX: usize = 32;

/// The browser's code for a failed command that takes a name: a name too long has its own.
fn name_error(kind: &str, otherwise: Code) -> Code {
    if kind == "messageTooLong" {
        "valueTooLong"
    } else {
        otherwise
    }
}

/// The contact an `update/contact` is about: threema-web and Android name it `identity`.
fn contact_arg<'a>(incoming: &Incoming<'a>) -> Option<&'a str> {
    incoming.arg("identity").or_else(|| incoming.arg("id"))
}

/// What a request says of a picture.
enum Picture {
    Unchanged,
    Removed,
    New(Vec<u8>),
}

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

    /// A picture from the browser, made into our JPEG (512 px); done before anything changes, so
    /// that one that can't be read refuses the request as a whole.
    fn web_picture(data: Option<&Value>) -> Result<Picture, Code> {
        let Some(avatar) = data.and_then(|data| field(data, "avatar")) else {
            return Ok(Picture::Unchanged);
        };
        if avatar.is_nil() {
            return Ok(Picture::Removed);
        }
        avatar
            .as_slice()
            .and_then(|bytes| images::scaled_jpeg(bytes, images::AVATAR_HIGH_MAX, 85))
            .map(Picture::New)
            .ok_or("invalidAvatar")
    }

    /// Run `set` (`setGroupPicture`, `setProfilePicture`) or `remove` for what `web_picture` found.
    async fn web_apply_picture(
        &mut self,
        picture: Picture,
        set: serde_json::Value,
        remove: serde_json::Value,
    ) -> Result<(), Code> {
        match picture {
            Picture::Unchanged => Ok(()),
            Picture::Removed => self
                .web_command(remove)
                .await
                .map(|_| ())
                .map_err(|_| "badRequest"),
            Picture::New(jpeg) => {
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
            ("update", "contact") => Some(("invalidContact", contact_arg(incoming))),
            ("update" | "delete", "group") | ("request", "groupSync") => {
                Some(("invalidGroup", incoming.arg("id")))
            }
            ("update" | "delete", "distributionList") => {
                Some(("invalidDistributionList", incoming.arg("id")))
            }
            _ => None,
        };
        if let Some((code, target)) = invalid
            && self.web_receiver_key(target).is_err()
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
                let Ok(id) = self.web_receiver_key(contact_arg(incoming)) else {
                    return Some(Err("invalidContact"));
                };
                if receiver_type(&id) != "contact" {
                    return Some(Err("invalidContact"));
                }
                let first = optional(data, "firstName");
                let last = optional(data, "lastName");
                let too_long = |name: Option<Option<&str>>| {
                    name.flatten()
                        .is_some_and(|name| name.trim().len() > CONTACT_NAME_BYTES_MAX)
                };
                if too_long(first) || too_long(last) {
                    return Some(Err("valueTooLong"));
                }
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
                    Some(map(vec![("identity", id.as_str().into())])),
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
                        let picture = Self::web_picture(data).unwrap_or(Picture::Unchanged);
                        let _ = self
                            .web_apply_picture(
                                picture,
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
                    Err(kind) => Err(name_error(&kind, "badRequest")),
                }
            }
            ("update", "group") => {
                let Some(key) = incoming.arg("id").map(str::to_owned) else {
                    return Some(Err("invalidGroup"));
                };
                // Before anything changes: a name too long leaves the members as they were.
                if optional(data, "name")
                    .flatten()
                    .is_some_and(|name| Self::check_name(name).is_err())
                {
                    return Some(Err("valueTooLong"));
                }
                let picture = match Self::web_picture(data) {
                    Ok(picture) => picture,
                    Err(code) => return Some(Err(code)),
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
                    .web_apply_picture(
                        picture,
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
                    Err(kind) => Err(name_error(&kind, "badRequest")),
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
                if let Err(kind) = self
                    .web_command(json!({"cmd": "updateDistributionList", "key": key, "name": name, "members": members}))
                    .await
                {
                    return Some(Err(name_error(&kind, "invalidDistributionList")));
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
                let picture = match Self::web_picture(data) {
                    Ok(picture) => picture,
                    Err(code) => return Some(Err(code)),
                };
                if let Some(nickname) = optional(data, "publicNickname") {
                    let nickname = nickname.unwrap_or_default();
                    if nickname.trim().len() > NICKNAME_BYTES_MAX {
                        return Some(Err("valueTooLong"));
                    }
                    if self
                        .web_command(json!({"cmd": "setNickname", "nickname": nickname}))
                        .await
                        .is_err()
                    {
                        return Some(Err("badRequest"));
                    }
                }
                if let Err(code) = self
                    .web_apply_picture(
                        picture,
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
