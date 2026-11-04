// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Web: the app's data in the app remote protocol's models (`schema/v2.yaml`:
//! Contact, Group, DistributionList, Conversation, Message) and the updates the browser
//! subscribed to (`update/conversation`, `update/messages`, `update/receivers`,
//! `update/typing`). Private chats (ADR 0009) never reach the browser.
//!
//! IDs: receivers by our keys (Threema ID, `g:…`, `d:…`), messages by their local row ID as a
//! string, which also sorts them.

use std::collections::{HashMap, HashSet};

use libthreema::{model::contact::Contact, protobuf::d2d_sync::contact as pc};
use rmpv::Value;
use sha2::{Digest as _, Sha256};

use crate::{
    app::{ApiError, App},
    contacts,
    distribution_lists::DistributionList,
    group_call_handshake::public_bytes,
    groups::Group,
    messages::{Message, OutgoingState},
    providers::CoreEvent,
    web::arp::{self, map},
};

/// Messages per page.
const PAGE: u32 = 50;

/// When the browsers were last brought up to date.
#[derive(Default)]
pub(crate) struct RefreshState {
    /// The store's change count and the subscriptions' shape at the last refresh.
    last: Option<(u64, usize)>,
}

/// What a connected browser subscribed to, and what it has been sent.
#[derive(Default)]
pub(crate) struct Subscriptions {
    receivers: bool,
    conversations: bool,
    /// Chats whose messages it shows: the newest message ID sent.
    chats: HashMap<String, i64>,
    /// The chat list as last sent: newest message ID, unread count, starred per chat.
    listed: HashMap<String, (LastMessage, i64, bool)>,
}

#[cfg(test)]
impl Subscriptions {
    pub(crate) fn shows(&self, chat: &str) -> bool {
        self.chats.contains_key(chat)
    }
}

/// A chat's newest message as far as its list entry shows it: id, last change, state (a state
/// change, e.g. delivered, must reach the browser's chat list too).
type LastMessage = (i64, i64, i64);

/// A chat as listed for the browser: key, its Conversation model, newest message, unread
/// count, starred.
type Listed = (String, Value, LastMessage, i64, bool);

/// `contact`, `group` or `distributionList`.
pub(crate) fn receiver_type(key: &str) -> &'static str {
    if crate::groups::parse_key(key).is_some() {
        "group"
    } else if crate::distribution_lists::is_key(key) {
        "distributionList"
    } else {
        "contact"
    }
}

/// A colour per receiver, like Android's ID colours (a fixed palette, picked by a hash).
fn color(id: &str) -> String {
    const PALETTE: [&str; 16] = [
        "#d84315", "#ef6c00", "#f9a825", "#9e9d24", "#558b2f", "#2e7d32", "#00897b", "#00838f",
        "#0277bd", "#1565c0", "#283593", "#4527a0", "#6a1b9a", "#ad1457", "#c62828", "#6d4c41",
    ];
    PALETTE[usize::from(Sha256::digest(id.as_bytes())[0]) % PALETTE.len()].to_owned()
}

fn secs(ms: i64) -> Value {
    Value::from(ms / 1000)
}

/// A call's line: "result|outgoing|seconds" (app_calls.rs), worded like our chat.
fn call_text(record: &str) -> String {
    let mut parts = record.split('|');
    let result = parts.next().unwrap_or_default();
    let outgoing = parts.next() == Some("1");
    let seconds: i64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    match result {
        "finished" => format!(
            "{} call, {}:{:02}",
            if outgoing { "Outgoing" } else { "Incoming" },
            seconds / 60,
            seconds % 60
        ),
        "missed" => "Missed call".into(),
        "rejected" if outgoing => "Call declined".into(),
        "rejected" => "You declined the call".into(),
        "busy" => "Call not answered: busy".into(),
        "unanswered" => "Call not answered".into(),
        "disabled" => "Call not answered: calls are off".into(),
        "aborted" => "Call cancelled".into(),
        _ => "Call failed".into(),
    }
}

/// A call ("result|outgoing|seconds") in the protocol's VoIP model: status 1 missed, 2 finished,
/// 3 rejected (reason 0 unknown, 1 busy, 2 timeout, 3 rejected), 4 aborted.
fn voip(record: &str) -> Value {
    let mut parts = record.split('|');
    let result = parts.next().unwrap_or_default();
    let seconds: i64 = parts.nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let rejected = |reason: i64| map(vec![("status", 3.into()), ("reason", reason.into())]);
    match result {
        "finished" => map(vec![("status", 2.into()), ("duration", seconds.into())]),
        "missed" => map(vec![("status", 1.into())]),
        "busy" => rejected(1),
        "unanswered" => rejected(2),
        "rejected" => rejected(3),
        "disabled" => rejected(0),
        _ => map(vec![("status", 4.into())]),
    }
}

/// Status lines as Threema Web shows them (our UI translates the codes; the browser can't).
fn status_text(code: &str) -> String {
    let (kind, argument) = code.split_once('|').unwrap_or((code, ""));
    match kind {
        "distributionListCreated" => "You created this distribution list.".into(),
        "groupCreated" => "You created this group.".into(),
        "groupJoined" => "You were added to this group.".into(),
        "groupRemoved" => "You were removed from this group.".into(),
        "groupNoLongerMember" => "You are no longer a member of this group.".into(),
        "groupDisbanded" => "This group was dissolved.".into(),
        "groupLeft" => "You left this group.".into(),
        "groupRenamed" => format!("Group renamed to {argument}."),
        "groupMemberAdded" => format!("{argument} was added."),
        "groupMemberRemoved" => format!("{argument} was removed."),
        "groupMemberLeft" => format!("{argument} left the group."),
        "fsEstablished" => "Forward secrecy is active for this chat.".into(),
        "fsReset" => {
            "Forward secrecy was reset. This can happen when the contact changes devices or \
             reinstalls the app."
                .into()
        }
        "fsMessageRejected" => {
            "Forward secrecy was reset. Affected messages are sent again.".into()
        }
        "fsMessageWithoutFs" => {
            "A message without forward secrecy was received. Check with the contact that they \
             turned it off on purpose."
                .into()
        }
        "fsUnavailable" => {
            "Forward secrecy is off in this chat because this contact's app is not using it at the \
             moment."
                .into()
        }
        "fsTerminated" => "The forward secrecy session with this contact ended.".into(),
        "call" => call_text(argument),
        _ => code.to_owned(),
    }
}

impl App {
    /// An error here leaves the browser without an answer: what isn't known to be public isn't
    /// sent.
    fn web_private_chats(&self) -> Result<HashSet<String>, ApiError> {
        Ok(self.store.private_chats()?.into_iter().collect())
    }

    fn web_contact(
        &self,
        contact: &Contact,
        private: &HashSet<String>,
        blocked: &[String],
    ) -> Value {
        let id = contact.identity.to_string();
        let is_private = private.contains(&id);
        let mut fields = vec![
            ("id", Value::from(id.clone())),
            ("displayName", contacts::display_name(contact).into()),
            ("color", color(&id).into()),
            (
                "verificationLevel",
                (contact.verification_level as i64 + 1).into(),
            ),
            (
                "state",
                match contact.activity_state {
                    pc::ActivityState::Active => "ACTIVE",
                    pc::ActivityState::Inactive => "INACTIVE",
                    pc::ActivityState::Invalid => "INVALID",
                }
                .into(),
            ),
            ("featureMask", contact.feature_mask.0.into()),
            (
                "isWork",
                (contact.identity_type == pc::IdentityType::Work).into(),
            ),
            (
                "identityType",
                i64::from(contact.identity_type == pc::IdentityType::Work).into(),
            ),
            (
                "publicKey",
                Value::Binary(public_bytes(&contact.public_key).to_vec()),
            ),
            ("isBlocked", blocked.contains(&id).into()),
            ("locked", is_private.into()),
            ("visible", (!is_private).into()),
            (
                "hidden",
                (contact.acquaintance_level == pc::AcquaintanceLevel::GroupOrDeleted).into(),
            ),
            (
                "access",
                map(vec![
                    ("canDelete", false.into()),
                    ("canChangeAvatar", false.into()),
                    ("canChangeFirstName", true.into()),
                    ("canChangeLastName", true.into()),
                ]),
            ),
        ];
        for (name, value) in [
            ("firstName", &contact.first_name),
            ("lastName", &contact.last_name),
            ("publicNickname", &contact.nickname),
        ] {
            if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
                fields.push((name, value.into()));
            }
        }
        map(fields)
    }

    /// Private groups and lists don't get here (`web_receivers_of`).
    fn web_group(&self, group: &Group, user: &str) -> Value {
        // What the browser may offer, like Android: the creator manages the group.
        let admin = group.identity.creator_identity.to_string() == user;
        let mut members = vec![Value::from(user)];
        members.extend(group.members.iter().map(|m| Value::from(m.as_str())));
        map(vec![
            ("id", group.key.clone().into()),
            ("name", group.name.clone().into()),
            ("displayName", self.group_title(group).into()),
            ("color", color(&group.key).into()),
            ("createdAt", secs(group.created_at)),
            ("disabled", group.left.into()),
            ("locked", false.into()),
            ("visible", true.into()),
            ("members", Value::Array(members)),
            (
                "administrator",
                group.identity.creator_identity.to_string().into(),
            ),
            (
                "access",
                map(vec![
                    ("canDelete", (admin || group.left).into()),
                    ("canChangeAvatar", (admin && !group.left).into()),
                    ("canChangeName", (admin && !group.left).into()),
                    ("canChangeMembers", (admin && !group.left).into()),
                    ("canLeave", (!group.left).into()),
                    ("canSync", (admin && !group.left).into()),
                ]),
            ),
        ])
    }

    fn web_list(&self, list: &DistributionList) -> Value {
        map(vec![
            ("id", list.key.clone().into()),
            ("displayName", self.list_title(list).into()),
            ("color", color(&list.key).into()),
            ("locked", false.into()),
            ("visible", true.into()),
            (
                "members",
                Value::Array(
                    list.members
                        .iter()
                        .map(|m| Value::from(m.as_str()))
                        .collect(),
                ),
            ),
            (
                "access",
                map(vec![
                    ("canDelete", true.into()),
                    ("canChangeMembers", true.into()),
                ]),
            ),
        ])
    }

    fn web_blocked(&self) -> Vec<String> {
        self.app_settings()
            .map(|settings| settings.blocked_contacts)
            .unwrap_or_default()
    }

    /// The receivers of one type (`contact`, `group`, `distributionList`). A private group or
    /// list stays on the device: its name and members would say what the mark hides. Contacts
    /// stay, marked `locked` and not `visible`: the browser needs them for group members.
    pub(crate) fn web_receivers_of(&self, kind: &str) -> Result<Vec<Value>, ApiError> {
        let private = self.web_private_chats()?;
        let user = self
            .store
            .identity()?
            .ok_or(ApiError::NoIdentity)?
            .threema_id;
        Ok(match kind {
            "contact" => {
                let blocked = self.web_blocked();
                self.store
                    .contacts()?
                    .iter()
                    .filter(|c| c.identity.to_string() != user)
                    .map(|c| self.web_contact(c, &private, &blocked))
                    .collect()
            }
            "group" => self
                .store
                .groups()?
                .iter()
                .filter(|g| !private.contains(&g.key))
                .map(|g| self.web_group(g, &user))
                .collect(),
            _ => self
                .store
                .distribution_lists()?
                .iter()
                .filter(|l| !private.contains(&l.key))
                .map(|l| self.web_list(l))
                .collect(),
        })
    }

    /// `response/receivers`.
    pub(crate) fn web_receivers(&self) -> Result<Value, ApiError> {
        Ok(map(vec![
            ("contact", Value::Array(self.web_receivers_of("contact")?)),
            ("group", Value::Array(self.web_receivers_of("group")?)),
            (
                "distributionList",
                Value::Array(self.web_receivers_of("distributionList")?),
            ),
        ]))
    }

    /// One message in the browser's model.
    pub(crate) fn web_message(&self, message: &Message, user: &str) -> Value {
        let json = message.to_json();
        let mut fields = vec![
            ("id", Value::from(message.id.to_string())),
            ("sortKey", message.id.into()),
            ("date", secs(message.created_at)),
            ("isOutbox", message.outgoing.into()),
        ];
        // Like Android: the contact for 1:1 chats (also for our own messages; the browser looks
        // it up among the contacts), the sender (or us) in groups.
        let partner = if message.outgoing {
            if receiver_type(&message.conversation) == "contact" {
                message.conversation.clone()
            } else {
                user.to_owned()
            }
        } else {
            match &message.sender {
                Some(sender) => sender.clone(),
                // Status lines are ours; in groups the browser shows a sender, so: us.
                None if receiver_type(&message.conversation) == "contact" => {
                    message.conversation.clone()
                }
                None => user.to_owned(),
            }
        };
        fields.push(("partnerId", partner.into()));
        let mut status = false;
        match message.kind.as_str() {
            "text" => {
                fields.push(("type", "text".into()));
                fields.push(("body", json["text"].as_str().unwrap_or_default().into()));
                if let Some(quote) = self.web_quote(message, user) {
                    fields.push(("quote", quote));
                }
            }
            "file" => {
                let body = &json["body"];
                let media_type = body["mediaType"].as_str().unwrap_or_default();
                let kind = match crate::messages::media_kind(&message.body) {
                    "image" | "gif" => "image",
                    "video" => "video",
                    "voice" => "audio",
                    _ => "file",
                };
                fields.push(("type", kind.into()));
                if let Some(caption) = body["caption"].as_str().filter(|c| !c.is_empty()) {
                    fields.push(("caption", caption.into()));
                }
                if let Some(thumbnail) = self.web_thumbnail_field(message.id) {
                    fields.push(("thumbnail", thumbnail));
                }
                fields.push((
                    "file",
                    map(vec![
                        ("name", body["name"].as_str().unwrap_or("file").into()),
                        ("size", body["size"].as_i64().unwrap_or(0).into()),
                        ("type", media_type.into()),
                        ("inApp", false.into()),
                    ]),
                ));
            }
            "location" => {
                let body = &json["body"];
                fields.push(("type", "location".into()));
                fields.push((
                    "location",
                    map(vec![
                        ("lat", body["latitude"].as_f64().unwrap_or(0.0).into()),
                        ("lon", body["longitude"].as_f64().unwrap_or(0.0).into()),
                        ("accuracy", body["accuracy"].as_f64().unwrap_or(0.0).into()),
                        (
                            "description",
                            body["name"].as_str().unwrap_or_default().into(),
                        ),
                        (
                            "address",
                            body["address"].as_str().unwrap_or_default().into(),
                        ),
                    ]),
                ));
            }
            "poll" | crate::polls::KIND_CLOSED => {
                fields.push(("type", "ballot".into()));
            }
            "deleted" => {
                status = true;
                fields.push(("type", "text".into()));
                fields.push(("body", "This message was deleted.".into()));
            }
            // Calls as Threema Web's call messages (icon, missed/duration), like Android.
            "status" if message.body.starts_with("call|") => {
                // Its direction is in the record ("call|result|1|…": outgoing).
                let outgoing = message.body.split('|').nth(2) == Some("1");
                for (name, value) in &mut fields {
                    if *name == "isOutbox" {
                        *value = outgoing.into();
                    }
                }
                fields.push(("type", "voipStatus".into()));
                fields.push(("voip", voip(&message.body["call|".len()..])));
            }
            _ => {
                status = true;
                fields.push(("type", "text".into()));
                fields.push(("body", status_text(&message.body).into()));
            }
        }
        fields.push(("isStatus", status.into()));
        if status {
            fields.push(("statusType", "text".into()));
        } else if message.outgoing {
            let state = match message.state {
                s if s == OutgoingState::Pending as i64 => "sending",
                s if s == OutgoingState::Sent as i64 => "sent",
                s if s == OutgoingState::Delivered as i64 => "delivered",
                s if s == OutgoingState::Read as i64 => "read",
                _ => "send-failed",
            };
            fields.push(("state", state.into()));
        } else {
            fields.push(("unread", message.read_at.is_none().into()));
            // Our own thumbs reaction is Threema Web's agree / disagree.
            let own = message
                .reactions
                .as_array()
                .into_iter()
                .flatten()
                .find(|reaction| {
                    reaction["sender"] == ""
                        && matches!(reaction["emoji"].as_str(), Some("👍" | "👎"))
                });
            if let Some(own) = own {
                fields.push((
                    "state",
                    if own["emoji"] == "👍" {
                        "user-ack"
                    } else {
                        "user-dec"
                    }
                    .into(),
                ));
            }
        }
        if let Some(reactions) = emoji_reactions(message, user) {
            fields.push(("emojiReactions", reactions));
        }
        if let Some(edited) = message.edited_at
            && message.kind != "deleted"
        {
            fields.push(("lastEditedAt", edited.into()));
        }
        map(fields)
    }

    /// Threema Web's quote object on a text message that quotes another: who wrote the
    /// quoted message, its text (a caption or a stand-in for media) and its id here, so the
    /// browser shows the quote and can jump to it. Nothing when the quoted message is not
    /// stored (a quote carries only the id); the browser then shows just the reply.
    fn web_quote(&self, message: &Message, user: &str) -> Option<Value> {
        let quote_id = crate::messages::split_quote(&message.body).0?;
        // Quotes we wrote before 2026-10-27 named the id byte-reversed: those still resolve.
        let quoted = [quote_id, quote_id.swap_bytes()].iter().find_map(|id| {
            [false, true].iter().find_map(|outgoing| {
                self.store
                    .message_by_id(&message.conversation, *outgoing, *id)
                    .ok()
                    .flatten()
            })
        })?;
        let identity = if quoted.outgoing {
            user.to_owned()
        } else {
            match &quoted.sender {
                Some(sender) => sender.clone(),
                None => quoted.conversation.clone(),
            }
        };
        let text = match quoted.kind.as_str() {
            "text" => crate::messages::split_quote(&quoted.body).1.to_owned(),
            "file" => {
                let body: serde_json::Value =
                    serde_json::from_str(&quoted.body).unwrap_or_default();
                match body["caption"].as_str().filter(|c| !c.is_empty()) {
                    Some(caption) => caption.to_owned(),
                    // The browser renders plain text: a pictogram stands in for the media,
                    // like Android's quote previews.
                    None => match crate::messages::media_kind(&quoted.body) {
                        "image" | "gif" => "📷".to_owned(),
                        "video" => "🎥".to_owned(),
                        "voice" => "🎤".to_owned(),
                        _ => format!("📎 {}", body["name"].as_str().unwrap_or_default()),
                    },
                }
            }
            "location" => {
                let body: serde_json::Value =
                    serde_json::from_str(&quoted.body).unwrap_or_default();
                format!("📍 {}", body["name"].as_str().unwrap_or_default())
            }
            "poll" | crate::polls::KIND_CLOSED => {
                let body: serde_json::Value =
                    serde_json::from_str(&quoted.body).unwrap_or_default();
                format!("📊 {}", body["description"].as_str().unwrap_or_default())
            }
            _ => return None,
        };
        Some(map(vec![
            ("identity", identity.into()),
            ("text", text.trim().into()),
            ("messageId", quoted.id.to_string().into()),
        ]))
    }

    /// The chat list (without private chats), newest first; key, newest message ID, unread.
    fn web_conversation_list(&self) -> Result<Vec<Listed>, ApiError> {
        let user = self
            .store
            .identity()?
            .ok_or(ApiError::NoIdentity)?
            .threema_id;
        let private = self.web_private_chats()?;
        let pinned = self.store.pinned_chats()?;
        let mut list: Vec<Listed> = Vec::new();
        for (last, unread) in self.store.conversations()? {
            if private.contains(&last.conversation) {
                continue;
            }
            let key = last.conversation.clone();
            let count = self.store.message_count(&key)?;
            let mut fields = vec![
                ("type", receiver_type(&key).into()),
                ("id", key.clone().into()),
                ("position", 0.into()),
                ("messageCount", count.into()),
                ("unreadCount", unread.into()),
                ("isStarred", pinned.contains(&key).into()),
            ];
            // A chat without messages has no latest message (our list keeps a marker for it:
            // `empty` for an emptied chat, `none` for a group or list with no history yet).
            if last.kind != "empty" && last.kind != "none" {
                fields.push(("latestMessage", self.web_message(&last, &user)));
            }
            let value = map(fields);
            let starred = pinned.contains(&key);
            list.push((
                key,
                value,
                (last.id, last.updated_at, last.state),
                unread,
                starred,
            ));
        }
        // Starred (our pinned) first, like our own list.
        list.sort_by_key(|(key, ..)| pinned.iter().position(|p| p == key).unwrap_or(usize::MAX));
        for (position, (_, value, ..)) in list.iter_mut().enumerate() {
            if let Value::Map(fields) = value {
                for (name, field) in fields.iter_mut() {
                    if name.as_str() == Some("position") {
                        *field = position.into();
                    }
                }
            }
        }
        Ok(list)
    }

    /// `response/conversations`; subscribes the session to the list.
    pub(crate) fn web_conversations(&mut self, session: i64) -> Result<Value, ApiError> {
        let list = self.web_conversation_list()?;
        let subscriptions = self.web.subscriptions.entry(session).or_default();
        subscriptions.conversations = true;
        subscriptions.listed = list
            .iter()
            .map(|(key, _, last, unread, starred)| (key.clone(), (*last, *unread, *starred)))
            .collect();
        Ok(Value::Array(
            list.into_iter().map(|(_, value, ..)| value).collect(),
        ))
    }

    /// The receiver's key from `args` (`type`, `id`), unless it is private or unknown.
    pub(crate) fn web_receiver_key(&self, id: Option<&str>) -> Result<String, ApiError> {
        let id = id.ok_or(ApiError::WebSessionNotFound)?;
        let key = self.conversation(id)?.0;
        if self.store.is_private(&key)? {
            return Err(ApiError::Locked);
        }
        Ok(key)
    }

    /// `response/messages`: the newest page, or the one before `ref_id`; subscribes the session
    /// to the chat. Returns the args and data.
    pub(crate) fn web_messages(
        &mut self,
        session: i64,
        key: &str,
        ref_id: Option<&str>,
    ) -> Result<(Value, Value), ApiError> {
        let user = self
            .store
            .identity()?
            .ok_or(ApiError::NoIdentity)?
            .threema_id;
        let mut messages = match ref_id.and_then(|id| id.parse::<i64>().ok()) {
            Some(before) => self
                .store
                .conversation_messages_before(key, before, PAGE + 1)?,
            None => self.store.conversation_messages(key, PAGE + 1)?,
        };
        let more = messages.len() > PAGE as usize;
        if more {
            messages.remove(0);
        }
        if ref_id.is_none() {
            let newest = messages.last().map_or(0, |m| m.id);
            self.web
                .subscriptions
                .entry(session)
                .or_default()
                .chats
                .insert(key.to_owned(), newest);
        }
        let args = map(vec![
            ("type", receiver_type(key).into()),
            ("id", key.into()),
            ("more", more.into()),
        ]);
        let data = Value::Array(
            messages
                .iter()
                .map(|m| self.web_message(m, &user))
                .collect(),
        );
        Ok((args, data))
    }

    /// `request/receivers` was answered: the session gets receiver updates.
    pub(crate) fn web_subscribe_receivers(&mut self, session: i64) {
        self.web.subscriptions.entry(session).or_default().receivers = true;
    }

    /// Something in the core changed: tell the subscribed browsers.
    pub(crate) fn web_on_core_event(&mut self, event: &CoreEvent) {
        if self.web.subscriptions.is_empty() {
            return;
        }
        let user = match self.store.identity() {
            Ok(Some(identity)) => identity.threema_id,
            _ => return,
        };
        let mut out: Vec<(i64, Value)> = Vec::new();
        match event {
            CoreEvent::MessagesChanged(messages) => {
                for (session, subscriptions) in &self.web.subscriptions {
                    let mut changed: HashMap<&str, Vec<Value>> = HashMap::new();
                    for message in messages {
                        // Subscribed before the chat became private: nothing more of it (audit
                        // 2026-10-27, S03; unknown counts as private).
                        if self.store.is_private(&message.conversation).unwrap_or(true) {
                            continue;
                        }
                        if subscriptions
                            .chats
                            .get(&message.conversation)
                            .is_some_and(|newest| message.id <= *newest)
                        {
                            changed
                                .entry(message.conversation.as_str())
                                .or_default()
                                .push(self.web_message(message, &user));
                        }
                    }
                    for (key, data) in changed {
                        out.push((*session, messages_update(key, "modified", data)));
                    }
                }
            }
            CoreEvent::ContactsChanged
            | CoreEvent::GroupsChanged
            | CoreEvent::PrivateChanged { .. } => {
                let kind = match event {
                    CoreEvent::GroupsChanged => "group",
                    CoreEvent::PrivateChanged { conversation, .. }
                        if conversation.starts_with("g:") =>
                    {
                        "group"
                    }
                    CoreEvent::PrivateChanged { conversation, .. }
                        if conversation.starts_with("d:") =>
                    {
                        "distributionList"
                    }
                    _ => "contact",
                };
                if let Ok(receivers) = self.web_receivers_of(kind) {
                    for (session, subscriptions) in &self.web.subscriptions {
                        if subscriptions.receivers {
                            out.push((
                                *session,
                                arp::message(
                                    "update",
                                    "receivers",
                                    Some(map(vec![("type", kind.into())])),
                                    Some(Value::Array(receivers.clone())),
                                ),
                            ));
                        }
                    }
                }
            }
            CoreEvent::Typing { identity, typing }
                if !self.store.is_private(identity).unwrap_or(true) =>
            {
                for session in self.web.subscriptions.keys() {
                    out.push((
                        *session,
                        arp::message(
                            "update",
                            "typing",
                            Some(map(vec![("id", identity.as_str().into())])),
                            Some(map(vec![("isTyping", (*typing).into())])),
                        ),
                    ));
                }
            }
            _ => {}
        }
        for (session, message) in out {
            self.send_web(session, message);
        }
    }

    /// After every command and event: new messages in the chats the browsers show, and changes
    /// of the chat list.
    pub(crate) fn web_refresh(&mut self) {
        if !self.web.pending_blobs.is_empty() {
            self.web_blobs_changed();
        }
        if self.web.subscriptions.is_empty() {
            return;
        }
        // Nothing written and no new subscription since the last refresh: nothing to tell
        // (it ran after every command and event, rebuilding the chat list; deep audit P02).
        let shape: usize = self
            .web
            .subscriptions
            .values()
            .map(|s| {
                s.chats.len()
                    + s.listed.len()
                    + usize::from(s.conversations)
                    + usize::from(s.receivers)
            })
            .sum::<usize>()
            + self.web.subscriptions.len();
        let now = (self.store.changes(), shape);
        if self.web.refresh.last == Some(now) {
            return;
        }
        let Ok(Some(identity)) = self.store.identity() else {
            return;
        };
        // Which chats are private isn't known: nothing goes out, the next refresh tries again.
        let (Ok(list), Ok(private)) = (self.web_conversation_list(), self.web_private_chats())
        else {
            return;
        };
        self.web.refresh.last = Some(now);
        let user = identity.threema_id;
        let mut out: Vec<(i64, Value)> = Vec::new();
        let sessions: Vec<i64> = self.web.subscriptions.keys().copied().collect();
        for session in sessions {
            let Some(subscriptions) = self.web.subscriptions.get(&session) else {
                continue;
            };
            let chats: Vec<(String, i64)> = subscriptions
                .chats
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect();
            let mut newest_sent = Vec::new();
            let mut revoked = Vec::new();
            for (key, newest) in chats {
                // Private now: the browser's view of it ends (S03).
                if private.contains(&key) {
                    revoked.push(key);
                    continue;
                }
                let new = self
                    .store
                    .conversation_messages_after(&key, newest)
                    .unwrap_or_default();
                if let Some(last) = new.last() {
                    newest_sent.push((key.clone(), last.id));
                    let data = new.iter().map(|m| self.web_message(m, &user)).collect();
                    out.push((session, messages_update(&key, "new", data)));
                }
            }
            let subscriptions = self.web.subscriptions.entry(session).or_default();
            for (key, newest) in newest_sent {
                subscriptions.chats.insert(key, newest);
            }
            for key in revoked {
                subscriptions.chats.remove(&key);
            }
            if subscriptions.conversations {
                let mut listed = HashMap::new();
                for (key, value, last, unread, starred) in &list {
                    listed.insert(key.clone(), (*last, *unread, *starred));
                    let mode = match subscriptions.listed.get(key) {
                        None => "new",
                        Some(state) if *state != (*last, *unread, *starred) => "modified",
                        Some(_) => continue,
                    };
                    out.push((
                        session,
                        arp::message(
                            "update",
                            "conversation",
                            Some(map(vec![("mode", mode.into())])),
                            Some(value.clone()),
                        ),
                    ));
                }
                for key in subscriptions
                    .listed
                    .keys()
                    .filter(|key| !listed.contains_key(*key))
                {
                    out.push((
                        session,
                        arp::message(
                            "update",
                            "conversation",
                            Some(map(vec![("mode", "removed".into())])),
                            Some(map(vec![
                                ("type", receiver_type(key).into()),
                                ("id", key.as_str().into()),
                            ])),
                        ),
                    ));
                }
                subscriptions.listed = listed;
            }
        }
        for (session, message) in out {
            self.send_web(session, message);
        }
    }
}

/// Reactions grouped by emoji (Android's buckets): the most recently used emoji first, each
/// with its reactors, least recent first; our own reaction under our ID.
fn emoji_reactions(message: &Message, user: &str) -> Option<Value> {
    let reactions = message.reactions.as_array()?;
    let mut buckets: Vec<(String, Vec<String>)> = Vec::new();
    // Oldest first: a later reaction moves its emoji to the front.
    for reaction in reactions {
        let (Some(emoji), Some(sender)) = (reaction["emoji"].as_str(), reaction["sender"].as_str())
        else {
            continue;
        };
        let identity = if sender.is_empty() { user } else { sender }.to_owned();
        let identities = match buckets.iter().position(|(e, _)| e == emoji) {
            Some(position) => buckets.remove(position).1,
            None => Vec::new(),
        };
        let mut identities = identities;
        identities.push(identity);
        buckets.insert(0, (emoji.to_owned(), identities));
    }
    (!buckets.is_empty()).then(|| {
        Value::Array(
            buckets
                .into_iter()
                .map(|(emoji, identities)| {
                    map(vec![
                        ("emoji", emoji.into()),
                        (
                            "identities",
                            Value::Array(identities.into_iter().map(Value::from).collect()),
                        ),
                    ])
                })
                .collect(),
        )
    })
}

fn messages_update(key: &str, mode: &str, data: Vec<Value>) -> Value {
    arp::message(
        "update",
        "messages",
        Some(map(vec![
            ("type", receiver_type(key).into()),
            ("id", key.into()),
            ("mode", mode.into()),
        ])),
        Some(Value::Array(data)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::StartConfig, store::StoredIdentity, web::saltyrtc::field};

    #[tokio::test(flavor = "current_thread")]
    async fn chats_and_messages_for_the_browser() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(
            &StartConfig {
                data_dir: dir.path().to_owned(),
                app_version: "0.1.0".into(),
                language: "en".into(),
                country: "CH".into(),
                model: "test".into(),
                os_version: "5.1".into(),
                pictures_dir: None,
                calls: false,
                log_level: None,
                update_check: false,
            },
            &[3; 32],
        )
        .unwrap();
        app.store
            .set_identity(&StoredIdentity {
                threema_id: "AAAAAAAA".into(),
                client_key: zeroize::Zeroizing::new([7; 32]),
                server_group: "a".into(),
            })
            .unwrap();
        for id in ["BBBBBBBB", "CCCCCCCC"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(id))
                .unwrap();
        }
        for n in 1..=60 {
            app.store
                .insert_incoming("BBBBBBBB", n, "text", &format!("hi {n}"), n.cast_signed())
                .unwrap();
        }
        app.store
            .insert_incoming("CCCCCCCC", 1, "text", "secret", 1)
            .unwrap();
        app.store.set_private("CCCCCCCC", true).unwrap();

        // Contacts both; chats only the one that isn't private (groups and lists:
        // `private_groups_and_lists_stay_out_of_the_browser`).
        let receivers = app.web_receivers().unwrap();
        let contacts = field(&receivers, "contact").unwrap().as_array().unwrap();
        assert_eq!(contacts.len(), 2);
        let conversations = app.web_conversations(1).unwrap();
        let conversations = conversations.as_array().unwrap();
        assert_eq!(conversations.len(), 1);
        assert_eq!(
            field(&conversations[0], "id"),
            Some(&Value::from("BBBBBBBB"))
        );
        assert_eq!(
            field(&conversations[0], "unreadCount"),
            Some(&Value::from(60))
        );
        assert!(app.web_receiver_key(Some("CCCCCCCC")).is_err());

        assert_eq!(status_text("call|finished|1|83"), "Outgoing call, 1:23");
        let mut reacted = app
            .store
            .conversation_messages("BBBBBBBB", 1)
            .unwrap()
            .remove(0);
        reacted.reactions = serde_json::json!([
            {"sender": "BBBBBBBB", "emoji": "👍"},
            {"sender": "", "emoji": "❤️"},
            {"sender": "", "emoji": "👍"},
        ]);
        let buckets = emoji_reactions(&reacted, "AAAAAAAA").unwrap();
        let buckets = buckets.as_array().unwrap();
        assert_eq!(field(&buckets[0], "emoji"), Some(&Value::from("👍")));
        assert_eq!(
            field(&buckets[0], "identities"),
            Some(&Value::Array(vec!["BBBBBBBB".into(), "AAAAAAAA".into()]))
        );
        assert_eq!(field(&buckets[1], "emoji"), Some(&Value::from("❤️")));
        assert_eq!(status_text("groupRenamed|Club"), "Group renamed to Club.");

        // The newest page, then the older rest.
        let (args, data) = app.web_messages(1, "BBBBBBBB", None).unwrap();
        assert_eq!(field(&args, "more"), Some(&Value::from(true)));
        let page = data.as_array().unwrap();
        assert_eq!(page.len(), 50);
        assert_eq!(field(&page[49], "body"), Some(&Value::from("hi 60")));
        let oldest = field(&page[0], "id").unwrap().as_str().unwrap().to_owned();
        let (args, data) = app.web_messages(1, "BBBBBBBB", Some(&oldest)).unwrap();
        assert_eq!(field(&args, "more"), Some(&Value::from(false)));
        assert_eq!(data.as_array().unwrap().len(), 10);

        // A reply carries its quote: who wrote the quoted message, its text, its id for the
        // jump. A quote of a message that is not here is left out (only the reply shows).
        let quoted_row = app
            .store
            .conversation_messages("BBBBBBBB", 60)
            .unwrap()
            .iter()
            .find(|m| m.message_id == 5)
            .unwrap()
            .id;
        app.store
            .insert_incoming(
                "BBBBBBBB",
                70,
                "text",
                &crate::messages::with_quote(Some(5), "re"),
                70,
            )
            .unwrap();
        app.store
            .insert_incoming(
                "BBBBBBBB",
                71,
                "text",
                &crate::messages::with_quote(Some(0xDEAD), "re gone"),
                71,
            )
            .unwrap();
        let rows = app.store.conversation_messages("BBBBBBBB", 2).unwrap();
        let reply = app.web_message(&rows[0], "AAAAAAAA");
        assert_eq!(field(&reply, "body"), Some(&Value::from("re")));
        let quote = field(&reply, "quote").unwrap();
        assert_eq!(field(quote, "identity"), Some(&Value::from("BBBBBBBB")));
        assert_eq!(field(quote, "text"), Some(&Value::from("hi 5")));
        assert_eq!(
            field(quote, "messageId"),
            Some(&Value::from(quoted_row.to_string()))
        );
        let dangling = app.web_message(&rows[1], "AAAAAAAA");
        assert_eq!(field(&dangling, "body"), Some(&Value::from("re gone")));
        assert!(field(&dangling, "quote").is_none());

        // A new message reaches the subscribed browser's chat (nothing is running, so only the
        // bookkeeping can be checked).
        app.store
            .insert_incoming("BBBBBBBB", 61, "text", "new", 61)
            .unwrap();
        app.web_refresh();
        let newest = app.store.conversation_messages("BBBBBBBB", 1).unwrap()[0].id;
        assert_eq!(app.web.subscriptions[&1].chats["BBBBBBBB"], newest);
    }
}
