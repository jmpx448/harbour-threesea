// SPDX-License-Identifier: AGPL-3.0-only
//! libthreema storage interfaces (`model::provider`) backed by our SQLCipher [`Store`].
//!
//! Everything runs on the core thread; the store is shared via `Rc`. Changes the UI must hear
//! about are collected in an [`EventSink`] and emitted after each processed message.

use core::cell::RefCell;
use std::rc::Rc;

use libthreema::{
    common::{Delta, MessageId, Nonce, ThreemaId},
    model::{
        contact::{Contact, ContactUpdate},
        message::{
            ContactMessageBody, DeliveryReceiptType, IncomingMessage, IncomingMessageBody,
            ReactionAction, WebSessionResumeMessage,
        },
        provider::{
            ContactProvider, ConversationProvider, NonceStorage, ProfilePicture, ProviderError,
            SettingsProvider, ShortcutProvider,
        },
    },
    protobuf::d2d_sync::contact as pc,
};
use serde_json::{Value, json};

use crate::{
    messages::{Message, OutgoingState},
    reactions,
    store::{Store, keys},
};

/// UI-relevant changes produced while processing incoming messages.
#[derive(Debug, Clone)]
pub enum CoreEvent {
    /// A new incoming message was stored (incl. whether a "received" receipt should be sent).
    MessageAdded {
        message: Message,
        send_delivery_receipt: bool,
    },
    /// Outgoing messages changed state (receipts, server acks).
    MessagesChanged(Vec<Message>),
    /// A contact is typing (or stopped).
    Typing { identity: String, typing: bool },
    /// Contacts were added or changed (e.g. a new sender, a nickname).
    ContactsChanged,
    /// Internal: a forward security control message to send to `peer`.
    FsControl { peer: String, envelope: Vec<u8> },
    /// Internal: a forward security status (turned into a status message by the connection).
    FsStatus {
        peer: String,
        status: libthreema::csp_e2e::fs::FsStatus,
    },
    /// Groups were created or changed.
    GroupsChanged,
    /// A chat became private or stopped being private, from anywhere: the UI, a linked device,
    /// a restore.
    PrivateChanged { conversation: String, private: bool },
    /// Internal: something was queued in the outbox (group replies): send it.
    OutboxChanged,
    /// Internal: group members without a contact; fetch them from the directory.
    UnknownMembers(Vec<String>),
    /// Internal: a group's picture changed; download it (group key).
    DownloadGroupPicture(String),
    /// Internal: a contact's picture changed (descriptor persisted in `picture_downloads`);
    /// download it.
    DownloadContactPicture(String),
    /// Our own profile (picture) changed.
    ProfileChanged,
    /// An Android-backup import event for the UI (already shaped as `{"event": …}`); also
    /// used for the bulk-reload events an import ends with.
    AndroidBackup(Value),
    /// Multi-device was switched on or off (feature mask, Linked devices page).
    MultiDeviceChanged,
    /// A held notification is due (a forwarded message still unread after its grace time).
    Notify(Message),
    /// The chat server saw this ID log in with another device cookie (another device used
    /// it, or a restore): the UI warns, like Android's rogue-device warning.
    DeviceCookieChanged,
    /// A call event for the UI (its JSON as it is, app_calls.rs).
    Call(Value),
    /// Internal: a group call was announced in a group (app_group_calls.rs).
    GroupCallStart {
        conversation: String,
        start: libthreema::model::message::GroupCallStartMessage,
    },
    /// Internal: a call signalling message from `peer` (ADR 0012).
    CallSignal {
        peer: String,
        kind: CallSignalKind,
        json: Vec<u8>,
        at: i64,
    },
    /// Internal: a picture changed on another device of the group: fetch it from the blob
    /// mirror (`target`: `me`, `contact:<ID>` or a group key; `None` blob: removed).
    SyncPicture {
        target: String,
        blob: Option<crate::sync::SyncedBlob>,
    },
    /// Internal: Threema Web wants this session back (`web-session-resume` from `*3MAPUSH`):
    /// the hex SHA-256 of the browser's permanent key.
    WebSessionResume(String),
}

impl CoreEvent {
    /// JSON event for the bridge. While the app is locked, events about private chats carry no
    /// content, so it never reaches the UI layer (ADR 0009: hidden by the core, not only the
    /// UI): `messageAdded` keeps only what the generic "Private chat" notification needs,
    /// `messagesChanged` leaves their messages out, and their typing events are not forwarded.
    pub fn to_json(&self, store: &crate::store::Store, unlocked: bool) -> Value {
        let hidden =
            |conversation: &str| !unlocked && store.is_private(conversation).unwrap_or(false);
        match self {
            Self::MessageAdded { message, .. } => {
                let private = store.is_private(&message.conversation).unwrap_or(false);
                let message_json = if private && !unlocked {
                    // Statuses carry app-made codes; the body stays so the UI can tell a missed
                    // call from a message. A rename's new name is user content: without it.
                    json!({
                        "conversation": message.conversation,
                        "outgoing": message.outgoing,
                        "kind": message.kind,
                        "createdAt": message.created_at,
                        "body": if message.kind != "status" {
                            Value::Null
                        } else if message.body.starts_with("groupRenamed|") {
                            Value::from("groupRenamed")
                        } else {
                            Value::from(message.body.as_str())
                        },
                    })
                } else {
                    store.message_json(message)
                };
                json!({
                    "event": "messageAdded",
                    "message": message_json,
                    // Per-chat notification setting (muted, mentions only) applied; a
                    // forwarded message's notification may wait (`Notify` follows).
                    "notify": store.should_notify(message)
                        && !store.held_notifications.borrow().contains(&message.id),
                    // Private chat: notifications show neither sender nor text (ADR 0009).
                    "private": private,
                })
            }
            Self::MessagesChanged(messages) => {
                let shown: Vec<_> = messages
                    .iter()
                    .filter(|message| !hidden(&message.conversation))
                    .cloned()
                    .collect();
                json!({
                    "event": "messagesChanged",
                    "messages": store.messages_json(&shown),
                })
            }
            Self::Typing { identity, typing } if !hidden(identity) => {
                json!({"event": "typing", "identity": identity, "typing": typing})
            }
            Self::Typing { .. } => json!({"event": "internal"}),
            Self::ContactsChanged => json!({"event": "contactsChanged"}),
            Self::GroupsChanged => json!({"event": "groupsChanged"}),
            Self::PrivateChanged {
                conversation,
                private,
            } => json!({"event": "privateChanged", "identity": conversation, "private": private}),
            Self::ProfileChanged => json!({"event": "profileChanged"}),
            Self::AndroidBackup(event) => event.clone(),
            Self::MultiDeviceChanged => json!({"event": "multiDeviceChanged"}),
            Self::Notify(message) => {
                let private = store.is_private(&message.conversation).unwrap_or(false);
                json!({
                    "event": "messageNotify",
                    "message": if private && !unlocked {
                        // As for messageAdded: no content of a hidden private chat.
                        json!({ "conversation": message.conversation,
                                "kind": message.kind, "outgoing": false,
                                "createdAt": message.created_at,
                                "body": Value::Null })
                    } else {
                        store.message_json(message)
                    },
                    "notify": store.should_notify(message),
                    "private": private,
                })
            }
            Self::DeviceCookieChanged => json!({"event": "deviceCookieChanged"}),
            Self::Call(event) => event.clone(),
            // Handled inside the core.
            Self::FsControl { .. }
            | Self::FsStatus { .. }
            | Self::OutboxChanged
            | Self::UnknownMembers(_)
            | Self::DownloadGroupPicture(_)
            | Self::DownloadContactPicture(_)
            | Self::SyncPicture { .. }
            | Self::CallSignal { .. }
            | Self::GroupCallStart { .. }
            | Self::WebSessionResume(_) => json!({"event": "internal"}),
        }
    }
}

pub type EventSink = Rc<RefCell<Vec<CoreEvent>>>;

/// Storage failures (`Foreign` only exists in libthreema's binding builds).
pub(crate) fn foreign(error: impl std::fmt::Display) -> ProviderError {
    ProviderError::InvalidState(format!("storage: {error}"))
}

/// Nonce storage (incoming and outgoing nonces share one table, like Android).
pub struct StoreNonces(pub Rc<Store>);

impl NonceStorage for StoreNonces {
    fn has(&self, nonce: &Nonce) -> Result<bool, ProviderError> {
        self.0.has_nonce(&nonce.0).map_err(foreign)
    }

    fn add_many(&mut self, nonces: Vec<Nonce>) -> Result<(), ProviderError> {
        let slices: Vec<&[u8]> = nonces.iter().map(|n| &n.0[..]).collect();
        self.0.add_nonces(&slices).map_err(foreign)
    }
}

/// Settings (block unknown) from the app settings object.
pub struct StoreSettings(pub Rc<Store>);

impl SettingsProvider for StoreSettings {
    fn block_unknown_identities(&self) -> Result<bool, ProviderError> {
        let settings = self.0.setting(keys::APP_SETTINGS).map_err(foreign)?;
        Ok(settings
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v["blockUnknown"].as_bool())
            .unwrap_or(false))
    }
}

/// Nothing to wake (no Threema Web while restoring forward secrecy state, fs_store).
pub struct NoShortcuts;

impl ShortcutProvider for NoShortcuts {
    fn handle_web_session_resume(
        &mut self,
        _: WebSessionResumeMessage,
    ) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// Threema Web's wake-up (ADR 0013): the push relay sends `web-session-resume` with JSON
/// `{"wcs": <session hash>, …}` when a browser wants a stored session back.
pub struct WebShortcuts {
    pub events: EventSink,
}

impl ShortcutProvider for WebShortcuts {
    fn handle_web_session_resume(
        &mut self,
        message: WebSessionResumeMessage,
    ) -> Result<(), ProviderError> {
        tracing::info!("Threema Web wake-up received");
        let payload: Value = serde_json::from_slice(&message.push_payload).unwrap_or_default();
        match payload["wcs"].as_str() {
            Some(hash) => self
                .events
                .borrow_mut()
                .push(CoreEvent::WebSessionResume(hash.to_ascii_lowercase())),
            None => tracing::warn!("Threema Web wake-up without a session dropped"),
        }
        Ok(())
    }
}

/// Apply a [`ContactUpdate`] (libthreema's own `TryApply` is crate-private).
fn apply_update(contact: &mut Contact, update: ContactUpdate) {
    fn delta<T>(target: &mut Option<T>, delta: Delta<T>) {
        match delta {
            Delta::Unchanged => {}
            Delta::Update(value) => *target = Some(value),
            Delta::Remove => *target = None,
        }
    }
    delta(&mut contact.first_name, update.first_name);
    delta(&mut contact.last_name, update.last_name);
    delta(&mut contact.nickname, update.nickname);
    delta(
        &mut contact.read_receipt_policy_override,
        update.read_receipt_policy_override,
    );
    delta(
        &mut contact.typing_indicator_policy_override,
        update.typing_indicator_policy_override,
    );
    if let Some(v) = update.verification_level {
        contact.verification_level = v;
    }
    if let Some(v) = update.work_verification_level {
        contact.work_verification_level = v;
    }
    if let Some(v) = update.identity_type {
        contact.identity_type = v;
    }
    if let Some(v) = update.acquaintance_level {
        contact.acquaintance_level = v;
    }
    if let Some(v) = update.activity_state {
        contact.activity_state = v;
    }
    if let Some(v) = update.feature_mask {
        contact.feature_mask = v;
    }
    if let Some(v) = update.sync_state {
        contact.sync_state = v;
    }
}

/// Contacts.
pub struct StoreContacts {
    pub store: Rc<Store>,
    pub user_identity: ThreemaId,
    pub events: EventSink,
}

impl StoreContacts {
    fn check_not_user(&self, identity: ThreemaId) -> Result<(), ProviderError> {
        if identity == self.user_identity {
            return Err(ProviderError::InvalidParameter(
                "user's own identity passed to contact provider".into(),
            ));
        }
        Ok(())
    }

    fn blocked_identities(&self) -> Vec<String> {
        self.store
            .setting(keys::APP_SETTINGS)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| serde_json::from_value(v["blockedContacts"].clone()).ok())
            .unwrap_or_default()
    }
}

impl ContactProvider for StoreContacts {
    fn is_explicitly_blocked(&self, identity: ThreemaId) -> Result<bool, ProviderError> {
        Ok(self
            .blocked_identities()
            .iter()
            .any(|blocked| *blocked == identity.to_string()))
    }

    fn is_member_of_active_group(&self, identity: ThreemaId) -> Result<bool, ProviderError> {
        // With "block unknown senders" on, a group member who is no contact of ours may still
        // write to us, as on Android (libthreema's `CommunicationPermission`).
        self.store
            .is_group_member(&identity.to_string())
            .map_err(foreign)
    }

    fn has(&self, identity: ThreemaId) -> Result<bool, ProviderError> {
        self.check_not_user(identity)?;
        Ok(self
            .store
            .contact(&identity.to_string())
            .map_err(foreign)?
            .is_some())
    }

    fn has_many(&self, identities: &[ThreemaId]) -> Result<usize, ProviderError> {
        let mut count = 0;
        for identity in identities {
            if self.has(*identity)? {
                count += 1;
            }
        }
        Ok(count)
    }

    fn get(&self, identity: ThreemaId) -> Result<Option<Contact>, ProviderError> {
        self.check_not_user(identity)?;
        self.store.contact(&identity.to_string()).map_err(foreign)
    }

    fn add(&mut self, contacts: Vec<Contact>) -> Result<(), ProviderError> {
        for contact in &contacts {
            self.check_not_user(contact.identity)?;
            if self.has(contact.identity)? {
                return Err(ProviderError::InvalidState("contact already exists".into()));
            }
        }
        for contact in &contacts {
            self.store.upsert_contact(contact).map_err(foreign)?;
        }
        self.events.borrow_mut().push(CoreEvent::ContactsChanged);
        Ok(())
    }

    fn update(&mut self, contacts: Vec<ContactUpdate>) -> Result<(), ProviderError> {
        for update in contacts {
            self.check_not_user(update.identity)?;
            let mut contact = self
                .store
                .contact(&update.identity.to_string())
                .map_err(foreign)?
                .ok_or_else(|| {
                    ProviderError::InvalidState("contact to update does not exist".into())
                })?;
            apply_update(&mut contact, update);
            self.store.upsert_contact(&contact).map_err(foreign)?;
        }
        self.events.borrow_mut().push(CoreEvent::ContactsChanged);
        Ok(())
    }

    fn get_contact_defined_profile_picture(
        &self,
        identity: ThreemaId,
    ) -> Result<Option<ProfilePicture>, ProviderError> {
        self.check_not_user(identity)?;
        // Not asked for by any of libthreema's paths; we distribute and apply pictures
        // ourselves (profile_pictures.rs).
        Ok(None)
    }

    fn get_user_defined_profile_picture(
        &self,
        identity: ThreemaId,
    ) -> Result<Option<ProfilePicture>, ProviderError> {
        self.check_not_user(identity)?;
        Ok(None)
    }
}

/// 1:1 conversations.
pub struct StoreConversations {
    pub store: Rc<Store>,
    pub events: EventSink,
    /// Our own identity (group receive steps).
    pub user: ThreemaId,
}

/// The call signalling message types (csp.struct.yml `call-*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallSignalKind {
    Offer,
    Answer,
    IceCandidates,
    Hangup,
    Ringing,
}

impl StoreConversations {
    fn call_signal(
        &self,
        peer: &str,
        kind: CallSignalKind,
        json: Vec<u8>,
        at: i64,
    ) -> Result<(), ProviderError> {
        self.events.borrow_mut().push(CoreEvent::CallSignal {
            peer: peer.to_owned(),
            kind,
            json,
            at,
        });
        Ok(())
    }
}

impl ConversationProvider for StoreConversations {
    fn message_is_marked_used(
        &self,
        sender: ThreemaId,
        id: MessageId,
    ) -> Result<bool, ProviderError> {
        self.store
            .has_incoming(&sender.to_string(), id.0)
            .map_err(foreign)
    }

    fn set_typing_indicator(
        &mut self,
        sender: ThreemaId,
        is_typing: bool,
    ) -> Result<(), ProviderError> {
        self.events.borrow_mut().push(CoreEvent::Typing {
            identity: sender.to_string(),
            typing: is_typing,
        });
        Ok(())
    }

    fn add_or_update_incoming_message(
        &mut self,
        message: IncomingMessage,
    ) -> Result<(), ProviderError> {
        let body = match message.body {
            IncomingMessageBody::Contact(body) => body,
            IncomingMessageBody::Group(group) => {
                let store = &self.store;
                let group_identity = group.group_identity;
                let outcome = crate::groups::receive(
                    store,
                    &self.user.to_string(),
                    &message.sender_identity.to_string(),
                    message.id.0,
                    i64::try_from(message.created_at).unwrap_or(i64::MAX),
                    group,
                    &|identity| store.contact(identity).ok().flatten().is_some(),
                    &|identity| {
                        store.contact(identity).ok().flatten().map_or_else(
                            || identity.to_owned(),
                            |c| crate::contacts::display_name(&c),
                        )
                    },
                )
                .map_err(foreign)?;
                let mut events = self.events.borrow_mut();
                // A group message from someone whose picture we don't have: ask them, like a
                // 1:1 message does (bounded by the request backoff).
                if !outcome.added.is_empty()
                    && self
                        .store
                        .request_picture(&message.sender_identity.to_string())
                        .map_err(foreign)?
                {
                    events.push(CoreEvent::OutboxChanged);
                }
                for added in outcome.added {
                    events.push(CoreEvent::MessageAdded {
                        message: added,
                        send_delivery_receipt: false,
                    });
                }
                if !outcome.changed.is_empty() {
                    events.push(CoreEvent::MessagesChanged(outcome.changed));
                }
                if outcome.groups_changed {
                    events.push(CoreEvent::GroupsChanged);
                }
                if !outcome.unknown_members.is_empty() {
                    events.push(CoreEvent::UnknownMembers(outcome.unknown_members));
                }
                if outcome.send {
                    events.push(CoreEvent::OutboxChanged);
                }
                if let Some(key) = outcome.download_picture {
                    events.push(CoreEvent::DownloadGroupPicture(key));
                }
                if let Some(start) = outcome.call_start {
                    events.push(CoreEvent::GroupCallStart {
                        conversation: crate::groups::key(&group_identity),
                        start,
                    });
                }
                return Ok(());
            }
        };
        let sender = message.sender_identity.to_string();
        let contact = self
            .store
            .contact(&sender)
            .map_err(foreign)?
            .ok_or_else(|| ProviderError::InvalidState("sender contact does not exist".into()))?;
        let created_at = i64::try_from(message.created_at).unwrap_or(i64::MAX);
        let mut file_keys = None;
        let (kind, content) = match body {
            ContactMessageBody::Text(text) => ("text", text.text),
            ContactMessageBody::File(file) => {
                file_keys = Some(crate::app_media::file_keys(&file));
                ("file", crate::app_media::file_body(&file).to_string())
            }
            ContactMessageBody::Location(location) => (
                "location",
                json!({
                    "latitude": location.latitude,
                    "longitude": location.longitude,
                    "accuracy": location.accuracy_m,
                    "name": location.name,
                    "address": location.address,
                })
                .to_string(),
            ),
            ContactMessageBody::DeliveryReceipt(receipt) => {
                let state = match receipt.receipt_type {
                    DeliveryReceiptType::Received => OutgoingState::Delivered,
                    DeliveryReceiptType::Read => OutgoingState::Read,
                };
                let ids: Vec<u64> = receipt.message_ids.iter().map(|id| id.0).collect();
                let changed = self
                    .store
                    .raise_outgoing_state(&sender, &ids, state)
                    .map_err(foreign)?;
                if !changed.is_empty() {
                    self.events
                        .borrow_mut()
                        .push(CoreEvent::MessagesChanged(changed));
                }
                return Ok(());
            }
            // Common Reaction Receive Steps: unknown or unreactable referred messages are ignored.
            ContactMessageBody::Reaction(reaction) => {
                let (emoji, apply) = match &reaction.action {
                    ReactionAction::Apply(emoji) => (emoji, true),
                    ReactionAction::Withdraw(emoji) => (emoji, false),
                };
                if let Some(id) = self
                    .store
                    .reactable_message(&sender, reaction.message_id.0)
                    .map_err(foreign)?
                {
                    let changed = self
                        .store
                        .set_reaction(id, &sender, emoji, apply, created_at)
                        .map_err(foreign)?;
                    self.events
                        .borrow_mut()
                        .push(CoreEvent::MessagesChanged(changed.into_iter().collect()));
                }
                return Ok(());
            }
            ContactMessageBody::PollSetup(setup) => {
                let (added, changed) = crate::polls::receive_setup(
                    &self.store,
                    &sender,
                    &sender,
                    false,
                    message.id.0,
                    created_at,
                    setup.poll_id,
                    &setup.poll,
                )
                .map_err(foreign)?;
                let mut events = self.events.borrow_mut();
                if let Some(added) = added {
                    events.push(CoreEvent::MessageAdded {
                        message: added,
                        send_delivery_receipt: !message.overrides.disable_delivery_receipts,
                    });
                }
                events.extend(changed.map(|c| CoreEvent::MessagesChanged(vec![c])));
                return Ok(());
            }
            ContactMessageBody::PollVote(vote) => {
                let changed = crate::polls::receive_vote(
                    &self.store,
                    &sender,
                    &sender,
                    &vote.creator_identity.to_string(),
                    vote.poll_id,
                    &vote.choices,
                )
                .map_err(foreign)?;
                self.events
                    .borrow_mut()
                    .extend(changed.map(|c| CoreEvent::MessagesChanged(vec![c])));
                return Ok(());
            }
            // Contact-defined profile picture: the descriptor is kept until the download
            // succeeded (survives restarts and failures), then the app fetches it (no `done`:
            // persisted blob).
            ContactMessageBody::SetProfilePicture(picture) => {
                // Android's "Receive profile pictures", off: nothing is downloaded or stored.
                if !self.store.receive_pictures().map_err(foreign)? {
                    return Ok(());
                }
                self.store
                    .set_picture_download(
                        &sender,
                        &crate::groups::GroupPicture {
                            blob_id: picture.blob_id,
                            key: picture.key,
                            size: picture.size,
                        },
                    )
                    .map_err(foreign)?;
                self.events
                    .borrow_mut()
                    .push(CoreEvent::DownloadContactPicture(sender));
                return Ok(());
            }
            ContactMessageBody::DeleteProfilePicture(_) => {
                // The deletion also obsoletes any download still under way, and there is
                // nothing to request until they set a new picture (distributed by them).
                self.store
                    .cancel_picture_download(&sender)
                    .map_err(foreign)?;
                self.store
                    .pause_picture_requests(
                        &sender,
                        crate::profile_pictures::REQUEST_PAUSE_DELETED_MS,
                    )
                    .map_err(foreign)?;
                self.store
                    .set_contact_picture(&sender, None)
                    .map_err(foreign)?;
                self.events.borrow_mut().push(CoreEvent::ContactsChanged);
                return Ok(());
            }
            ContactMessageBody::RequestProfilePicture(_) => {
                self.store
                    .purge_picture_distribution(Some(&sender))
                    .map_err(foreign)?;
                return Ok(());
            }
            // Common Edit Message Receive Steps: only the original sender's text messages.
            ContactMessageBody::Edit(edit) => {
                if let Some(original) = self
                    .store
                    .message_by_id(&sender, false, edit.message_id.0)
                    .map_err(foreign)?
                    .filter(|original| original.kind == "text")
                {
                    let changed = self
                        .store
                        .edit_message(original.id, &edit.text, Some(created_at))
                        .map_err(foreign)?;
                    self.events
                        .borrow_mut()
                        .push(CoreEvent::MessagesChanged(changed.into_iter().collect()));
                }
                return Ok(());
            }
            // Common Delete Message Receive Steps: only the original sender's messages.
            ContactMessageBody::Delete(delete) => {
                if let Some(original) = self
                    .store
                    .message_by_id(&sender, false, delete.message_id.0)
                    .map_err(foreign)?
                    .filter(|original| !matches!(original.kind.as_str(), "status" | "deleted"))
                {
                    let changed = self
                        .store
                        .mark_deleted(original.id, created_at)
                        .map_err(foreign)?;
                    self.events
                        .borrow_mut()
                        .push(CoreEvent::MessagesChanged(changed.into_iter().collect()));
                }
                return Ok(());
            }
            // A legacy reaction (acknowledge/decline receipt) replaces the sender's reactions.
            ContactMessageBody::LegacyReaction(reaction) => {
                let emoji = reactions::legacy_emoji(reaction.reaction_type);
                let mut changed = Vec::new();
                for message_id in &reaction.message_ids {
                    if let Some(id) = self
                        .store
                        .reactable_message(&sender, message_id.0)
                        .map_err(foreign)?
                    {
                        changed.extend(
                            self.store
                                .replace_reactions(id, &sender, emoji, created_at)
                                .map_err(foreign)?,
                        );
                    }
                }
                if !changed.is_empty() {
                    self.events
                        .borrow_mut()
                        .push(CoreEvent::MessagesChanged(changed));
                }
                return Ok(());
            }
            ContactMessageBody::WebSessionResume(_) | ContactMessageBody::TypingIndicator(_) => {
                return Ok(());
            }
            // Calls (ADR 0012): the call logic decides what they mean.
            ContactMessageBody::CallOffer(m) => {
                return self.call_signal(&sender, CallSignalKind::Offer, m.json, created_at);
            }
            ContactMessageBody::CallAnswer(m) => {
                return self.call_signal(&sender, CallSignalKind::Answer, m.json, created_at);
            }
            ContactMessageBody::CallIceCandidate(m) => {
                return self.call_signal(
                    &sender,
                    CallSignalKind::IceCandidates,
                    m.json,
                    created_at,
                );
            }
            ContactMessageBody::CallHangup(m) => {
                return self.call_signal(&sender, CallSignalKind::Hangup, m.json, created_at);
            }
            ContactMessageBody::CallRinging(m) => {
                return self.call_signal(&sender, CallSignalKind::Ringing, m.json, created_at);
            }
        };
        if contact.acquaintance_level != pc::AcquaintanceLevel::Direct {
            tracing::info!("discarding message from non-direct contact");
            return Ok(());
        }
        if let Some(stored) = self
            .store
            .insert_incoming(&sender, message.id.0, kind, &content, created_at)
            .map_err(foreign)?
        {
            if self.store.request_picture(&sender).map_err(foreign)? {
                self.events.borrow_mut().push(CoreEvent::OutboxChanged);
            }
            if let Some(keys) = &file_keys {
                self.store.set_file_keys(stored.id, keys).map_err(foreign)?;
            }
            self.events.borrow_mut().push(CoreEvent::MessageAdded {
                message: stored,
                send_delivery_receipt: !message.overrides.disable_delivery_receipts,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::CoreEvent;

    #[test]
    fn held_notifications_wait_for_their_notify_event() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let message = store
            .insert_incoming("B0B0B0B0", 1, "text", "hi", 1)
            .unwrap()
            .unwrap();
        let added = CoreEvent::MessageAdded {
            message: message.clone(),
            send_delivery_receipt: false,
        };
        assert_eq!(
            added.to_json(&store, true)["notify"],
            true,
            "not held: at once"
        );
        // Forwarded by the leading device: the list shows it, the notification waits.
        store.held_notifications.borrow_mut().insert(message.id);
        assert_eq!(added.to_json(&store, true)["notify"], false);
        store.held_notifications.borrow_mut().remove(&message.id);
        let due = CoreEvent::Notify(message).to_json(&store, true);
        assert_eq!(due["event"], "messageNotify");
        assert_eq!(due["notify"], true);
        assert_eq!(due["message"]["text"], "hi");
    }
}
