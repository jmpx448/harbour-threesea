// SPDX-License-Identifier: AGPL-3.0-only
//! libthreema storage interfaces (`model::provider`) backed by our SQLCipher [`Store`].
//!
//! Everything runs on the core thread; the store is shared via `Rc`. Changes the UI must hear
//! about are collected in an [`EventSink`] and emitted once a message is stored, before anything
//! that can fail on the network.

use core::cell::RefCell;
use std::rc::Rc;

use libthreema::{
    common::{Delta, GroupIdentity, MessageId, Nonce, ThreemaId, config::Config},
    model::{
        contact::{Contact, ContactUpdate},
        message::{
            ContactMessageBody, DeliveryReceiptType, IncomingMessage, IncomingMessageBody,
            MessageOverrides, ReactionAction, WebSessionResumeMessage,
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
    store::{Store, StoreError, keys},
};

/// UI-relevant changes produced while processing incoming messages.
#[derive(Debug, Clone)]
pub enum CoreEvent {
    /// A new incoming message was stored (incl. whether a "received" receipt should be sent).
    MessageAdded { message: Message },
    /// A delivery receipt waits in the outbox (see [`StoreConversations::queue_receipt`]).
    ReceiptQueued { receiver: String, receipt_id: u64 },
    /// Outgoing messages changed state (receipts, server acks).
    MessagesChanged(Vec<Message>),
    /// A contact is typing (or stopped).
    Typing { identity: String, typing: bool },
    /// Contacts were added or changed (e.g. a new sender, a nickname).
    ContactsChanged,
    /// Internal: a forward security control message to send to `peer`.
    FsControl {
        peer: String,
        /// Known when this answers a message of `peer`, who may not be a contact yet.
        peer_key: Option<libthreema::common::keys::PublicKey>,
        envelope: Vec<u8>,
    },
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
    /// Internal: something went into the sync queue while handling a message. The connection
    /// reflects it before going on to the next message.
    SyncQueued,
    /// Internal: a group's picture was deleted; its file goes too (group key).
    RemoveGroupPicture(String),
    /// Internal: a group's picture changed; download it (group key).
    DownloadGroupPicture(String),
    /// Internal: someone is to get a group's picture from us: `group_picture_sends`.
    SendGroupPictures,
    /// Internal: a contact's picture changed (descriptor persisted in `picture_downloads`);
    /// download it.
    DownloadContactPicture(String),
    /// Our own profile (picture) changed.
    ProfileChanged,
    /// The settings were changed on another device: what shows them loads them again.
    SettingsChanged,
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
    /// The store fails while receiving (or works again): once each way.
    StorageProblem { active: bool },
    /// A call event for the UI (its JSON as it is, app_calls.rs).
    Call(Value),
    /// Internal: a group call was announced in a group (app_group_calls.rs).
    /// Internal: these people just got a group's state from us: if a call runs, they hear of it.
    AnnounceGroupCall {
        conversation: String,
        receivers: Vec<String>,
    },
    GroupCallStart {
        conversation: String,
        /// Who announced it.
        caller: String,
        start: libthreema::model::message::GroupCallStartMessage,
    },
    /// Internal: a peek showed that the call announced by this status line runs (`call_id`, the
    /// group's running call): the notification goes out unless we are in that call.
    GroupCallRunning { message: Message, call_id: [u8; 32] },
    /// Internal: a call signalling message from `peer` (ADR 0012).
    CallSignal {
        peer: String,
        kind: CallSignalKind,
        json: Vec<u8>,
    },
    /// Internal: a picture changed on another device of the group, what it said is in
    /// `synced_pictures`: fetch it from the blob mirror (`target`: `me`, `contact:<ID>` or a group
    /// key).
    SyncPicture { target: String },
    /// Internal: a group was deleted on another device: its picture and media go here too.
    GroupDeleted(String),
    /// Internal: Threema Web wants this session back (`web-session-resume` from `*3MAPUSH`):
    /// the hex SHA-256 of the browser's permanent key.
    WebSessionResume(String),
}

/// What the UI may see of a message of a hidden private chat. Statuses carry app-made codes, so
/// that it can tell a missed call from a message; what group status lines say after the code
/// (a new name, a member's or caller's name) is user content and stays behind.
fn hidden_body(message: &Message) -> Value {
    if message.kind != "status" {
        Value::Null
    } else if message.body.starts_with("group") {
        Value::from(message.body.split('|').next().unwrap_or_default())
    } else {
        Value::from(message.body.as_str())
    }
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
                    json!({
                        "conversation": message.conversation,
                        "outgoing": message.outgoing,
                        "kind": message.kind,
                        "createdAt": message.created_at,
                        "body": hidden_body(message),
                    })
                } else {
                    store.message_json(message)
                };
                json!({
                    "event": "messageAdded",
                    "message": message_json,
                    // Per-chat notification setting (muted, mentions only) applied; a
                    // forwarded message's notification may wait (`Notify` follows), and so does
                    // a group call's until a peek shows it runs.
                    "notify": store.should_notify(message)
                        && !message.body.starts_with(crate::chat_notifications::GROUP_CALL_STARTED)
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
            Self::SettingsChanged => json!({"event": "settingsChanged"}),
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
                                "body": hidden_body(message) })
                    } else {
                        store.message_json(message)
                    },
                    "notify": store.should_notify(message),
                    "private": private,
                })
            }
            Self::DeviceCookieChanged => json!({"event": "deviceCookieChanged"}),
            Self::StorageProblem { active } => json!({"event": "storageProblem", "active": active}),
            // Who is in a call of a private group is its content like any other.
            Self::Call(event)
                if event["event"] == "groupCall"
                    && event["conversation"].as_str().is_some_and(hidden) =>
            {
                json!({"event": "internal"})
            }
            Self::Call(event) => event.clone(),
            // Handled inside the core.
            Self::FsControl { .. }
            | Self::FsStatus { .. }
            | Self::OutboxChanged
            | Self::ReceiptQueued { .. }
            | Self::UnknownMembers(_)
            | Self::SyncQueued
            | Self::RemoveGroupPicture(_)
            | Self::DownloadGroupPicture(_)
            | Self::SendGroupPictures
            | Self::DownloadContactPicture(_)
            | Self::SyncPicture { .. }
            | Self::GroupDeleted(_)
            | Self::CallSignal { .. }
            | Self::AnnounceGroupCall { .. }
            | Self::GroupCallStart { .. }
            | Self::GroupCallRunning { .. }
            | Self::WebSessionResume(_) => json!({"event": "internal"}),
        }
    }
}

pub type EventSink = Rc<RefCell<Vec<CoreEvent>>>;

/// How a storage failure shows in what libthreema makes of a [`ProviderError`]: all it passes on
/// is text.
const STORAGE_MARK: &str = "storage[";

/// Storage failures (`Foreign` only exists in libthreema's binding builds).
pub(crate) fn foreign(error: StoreError) -> ProviderError {
    ProviderError::InvalidState(format!("{STORAGE_MARK}{}]: {error}", error.sqlite_code()))
}

/// The SQLite code if this error text says that the store failed ([`foreign`]), wherever libthreema
/// has put it on the way.
pub(crate) fn storage_code(error: &str) -> Option<i32> {
    let (_, rest) = error.split_once(STORAGE_MARK)?;
    let (code, _) = rest.split_once("]: ")?;
    code.parse().ok()
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
        block_unknown(&self.0).map_err(foreign)
    }
}

fn block_unknown(store: &Store) -> Result<bool, StoreError> {
    let settings = store.setting(keys::APP_SETTINGS)?;
    Ok(settings
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["blockUnknown"].as_bool())
        .unwrap_or(false))
}

/// The _Identity Blocked Steps_ (Android's `IdentityBlockedSteps`, libthreema's crate-private
/// `communication_permission`): special contacts never are; everyone on the block list is; with
/// "block unknown senders" on, so is anyone who is neither a direct contact nor in an active group
/// with us, predefined contacts excepted.
pub fn identity_blocked(
    store: &Store,
    config: &Config,
    identity: &str,
) -> Result<bool, StoreError> {
    let predefined = ThreemaId::try_from(identity)
        .ok()
        .and_then(|id| config.predefined_contacts.get(&id));
    if predefined.is_some_and(|contact| contact.special) {
        return Ok(false);
    }
    if store.is_explicitly_blocked(identity)? {
        return Ok(true);
    }
    if predefined.is_some() || !block_unknown(store)? {
        return Ok(false);
    }
    let direct = store
        .contact(identity)?
        .is_some_and(|c| c.acquaintance_level == pc::AcquaintanceLevel::Direct);
    Ok(!direct && !store.is_group_member(identity)?)
}

/// Whether a message may go to `receiver` at all (the receiver filter of Android's
/// `OutgoingCspMessageHandle`): never to a revoked ID, and to a blocked one only if it is exempt
/// from blocking (`exempt`), such as what keeps a group in step.
pub fn may_receive(
    store: &Store,
    config: &Config,
    receiver: &str,
    exempt: bool,
) -> Result<bool, StoreError> {
    if store
        .contact(receiver)?
        .is_some_and(|c| c.activity_state == pc::ActivityState::Invalid)
    {
        return Ok(false);
    }
    Ok(exempt || !identity_blocked(store, config, receiver)?)
}

/// Whom a forward secrecy control message for `receiver` goes to: a contact, unless revoked, by
/// its stored key, else someone who just wrote to us (`peer_key`). Such a stranger's first message
/// comes after their Init, so their Accept would otherwise be lost: their next message is then
/// answered in a state they aren't in yet, which they reject, on every first conversation. Not
/// under _block unknown_, though, where their answer would only show that we are online.
pub fn control_receiver(
    store: &Store,
    config: &Config,
    receiver: &str,
    peer_key: Option<libthreema::common::keys::PublicKey>,
) -> Result<Option<(ThreemaId, libthreema::common::keys::PublicKey)>, StoreError> {
    if let Some(contact) = store.contact(receiver)? {
        return Ok((contact.activity_state != pc::ActivityState::Invalid)
            .then_some((contact.identity, contact.public_key)));
    }
    let (Ok(identity), Some(key)) = (ThreemaId::try_from(receiver), peer_key) else {
        return Ok(None);
    };
    Ok((!identity_blocked(store, config, receiver)?).then_some((identity, key)))
}

/// The group members `message_id` still has to go to. Whoever may not receive it is taken off the
/// list for good, so that a message queued before the block doesn't reach them and doesn't wait
/// for them either. So is whoever is no member any more, if `members_only`: they left or were
/// removed after it was queued. Not for what goes to someone outside by its nature, nor once the
/// group is gone: then only such things are left.
pub fn group_receivers(
    store: &Store,
    config: &Config,
    key: &str,
    message_id: u64,
    exempt: bool,
    members_only: bool,
) -> Result<Vec<String>, StoreError> {
    let group = if members_only {
        store.group(key)?
    } else {
        None
    };
    let mut kept = Vec::new();
    for receiver in store.pending_receivers(key, message_id)? {
        if may_receive(store, config, &receiver, exempt)?
            && group.as_ref().is_none_or(|g| g.members.contains(&receiver))
        {
            kept.push(receiver);
        } else {
            store.receiver_done(&receiver, message_id)?;
        }
    }
    Ok(kept)
}

/// Forward secrecy status lines appear in a chat with someone we have a contact for and don't
/// block. Anyone else's control messages must not create or bump a chat.
pub fn shows_fs_status(store: &Store, config: &Config, peer: &str) -> Result<bool, StoreError> {
    Ok(store.contact(peer)?.is_some() && !identity_blocked(store, config, peer)?)
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
}

impl ContactProvider for StoreContacts {
    fn is_explicitly_blocked(&self, identity: ThreemaId) -> Result<bool, ProviderError> {
        self.store
            .is_explicitly_blocked(&identity.to_string())
            .map_err(foreign)
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
    /// The messages come reflected from another device, not from the chat server: that device
    /// has answered them and told the others.
    pub from_sync: bool,
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
    ) -> Result<(), ProviderError> {
        self.events.borrow_mut().push(CoreEvent::CallSignal {
            peer: peer.to_owned(),
            kind,
            json,
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
        let sender = sender.to_string();
        Ok(self.store.has_incoming(&sender, id.0).map_err(foreign)?
            && !self
                .store
                .awaits_reflection(&sender, id.0)
                .map_err(foreign)?)
    }

    fn is_in_group(&self, group_identity: GroupIdentity) -> Result<bool, ProviderError> {
        Ok(self
            .store
            .group(&crate::groups::key(&group_identity))
            .map_err(foreign)?
            .is_some_and(|group| !group.left))
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

    /// One store step, whatever the message takes: what fails here comes again from the server,
    /// and a row left behind would make that look like a duplicate, without the rest of what the
    /// message said (a file's keys, a group message's sender). Nothing is announced unless it holds.
    fn add_or_update_incoming_message(
        &mut self,
        message: IncomingMessage,
    ) -> Result<(), ProviderError> {
        let announced = self.events.borrow().len();
        let stored = self.store.step().map_err(foreign).and_then(|step| {
            self.store_incoming(message)?;
            step.commit().map_err(foreign)
        });
        if stored.is_err() {
            self.events.borrow_mut().truncate(announced);
        }
        stored
    }
}

impl StoreConversations {
    fn store_incoming(&self, message: IncomingMessage) -> Result<(), ProviderError> {
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
                    self.from_sync,
                    &|identity| store.contact(identity).ok().flatten().is_some(),
                    &|identity| store.is_explicitly_blocked(identity),
                    &|identity| {
                        store.contact(identity).ok().flatten().map_or_else(
                            || identity.to_owned(),
                            |c| crate::contacts::display_name(&c),
                        )
                    },
                )
                .map_err(foreign)?;
                let mut events = self.events.borrow_mut();
                for added in outcome.added {
                    events.push(CoreEvent::MessageAdded { message: added });
                }
                if !outcome.changed.is_empty() {
                    events.push(CoreEvent::MessagesChanged(outcome.changed));
                }
                if outcome.groups_changed {
                    events.push(CoreEvent::GroupsChanged);
                }
                if !outcome.state_sent_to.is_empty() {
                    events.push(CoreEvent::SendGroupPictures);
                    events.push(CoreEvent::AnnounceGroupCall {
                        conversation: crate::groups::key(&group_identity),
                        receivers: outcome.state_sent_to,
                    });
                }
                if !outcome.unknown_members.is_empty() {
                    // In this step: the setup won't come again, whatever becomes of the lookup.
                    self.store
                        .owe_member_lookups(&outcome.unknown_members)
                        .map_err(foreign)?;
                    events.push(CoreEvent::UnknownMembers(outcome.unknown_members));
                }
                if outcome.sync_queued {
                    events.push(CoreEvent::SyncQueued);
                }
                if outcome.send {
                    events.push(CoreEvent::OutboxChanged);
                }
                if let Some(key) = outcome.download_picture {
                    events.push(CoreEvent::DownloadGroupPicture(key));
                }
                if let Some(key) = outcome.remove_picture {
                    events.push(CoreEvent::RemoveGroupPicture(key));
                }
                if let Some((caller, start)) = outcome.call_start {
                    events.push(CoreEvent::GroupCallStart {
                        conversation: crate::groups::key(&group_identity),
                        caller,
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
                let is_new = added.is_some();
                if let Some(added) = added {
                    events.push(CoreEvent::MessageAdded { message: added });
                }
                events.extend(changed.map(|c| CoreEvent::MessagesChanged(vec![c])));
                drop(events);
                if is_new {
                    self.queue_receipt(&sender, &message.overrides, message.id.0)?;
                    self.note_awaiting_reflection(&sender, message.id.0)?;
                }
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
                // The deletion also obsoletes any download still under way.
                self.store
                    .cancel_picture_download(&sender)
                    .map_err(foreign)?;
                self.store
                    .cancel_synced_picture(&crate::profile_pictures::contact_picture_name(&sender))
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
            // Common Edit Message Receive Steps: only the original sender's texts and captions.
            ContactMessageBody::Edit(edit) => {
                if let Some(original) = self
                    .store
                    .message_by_id(&sender, false, edit.message_id.0)
                    .map_err(foreign)?
                    .filter(|original| crate::messages::edit_applies_to(&original.kind))
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
                    .filter(|original| crate::messages::delete_applies_to(&original.kind))
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
                return self.call_signal(&sender, CallSignalKind::Offer, m.json);
            }
            ContactMessageBody::CallAnswer(m) => {
                return self.call_signal(&sender, CallSignalKind::Answer, m.json);
            }
            ContactMessageBody::CallIceCandidate(m) => {
                return self.call_signal(&sender, CallSignalKind::IceCandidates, m.json);
            }
            ContactMessageBody::CallHangup(m) => {
                return self.call_signal(&sender, CallSignalKind::Hangup, m.json);
            }
            ContactMessageBody::CallRinging(m) => {
                return self.call_signal(&sender, CallSignalKind::Ringing, m.json);
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
            if let Some(keys) = &file_keys {
                self.store.set_file_keys(stored.id, keys).map_err(foreign)?;
            }
            self.events
                .borrow_mut()
                .push(CoreEvent::MessageAdded { message: stored });
            self.queue_receipt(&sender, &message.overrides, message.id.0)?;
            self.note_awaiting_reflection(&sender, message.id.0)?;
        }
        Ok(())
    }

    /// A 1:1 message just stored on a device that leads: libthreema reflects it next. Should the
    /// connection die before the mediator confirms that, the server sends the message again, and
    /// it mustn't count as a known one ([`Self::message_is_marked_used`]), or the other devices
    /// never get it. A group message has no such check in its way.
    fn note_awaiting_reflection(&self, sender: &str, message_id: u64) -> Result<(), ProviderError> {
        if !self.from_sync && self.store.multi_device().map_err(foreign)?.is_some() {
            self.store
                .await_reflection(sender, message_id)
                .map_err(foreign)?;
        }
        Ok(())
    }

    /// The delivery receipt for a message just stored: in the same store step, as its delivery
    /// won't come again. A connection that dies before it went out sends it after the next login.
    /// Not for what another device received: that device has answered.
    fn queue_receipt(
        &self,
        sender: &str,
        overrides: &MessageOverrides,
        message_id: u64,
    ) -> Result<(), ProviderError> {
        if overrides.disable_delivery_receipts {
            // For when it is read.
            self.store
                .set_no_receipts(sender, message_id)
                .map_err(foreign)?;
            return Ok(());
        }
        if self.from_sync {
            return Ok(());
        }
        let receipt_id = MessageId::random().0;
        self.store
            .outbox_add(
                sender,
                receipt_id,
                reactions::OUTBOX_DELIVERY_RECEIPT,
                &json!({ "messageId": format!("{message_id:016x}") }),
            )
            .map_err(foreign)?;
        self.events.borrow_mut().push(CoreEvent::ReceiptQueued {
            receiver: sender.to_owned(),
            receipt_id,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use libthreema::{
        common::config::ConfigEnvironment,
        model::message::{GroupMessageBody, IncomingGroupMessageBody, TextMessage},
    };

    use super::*;

    const MEMBER: &str = "B0B0B0B0";
    const OTHER: &str = "C0C0C0C0";
    const STRANGER: &str = "D0D0D0D0";

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        (dir, store)
    }

    fn config() -> Config {
        Config::from(ConfigEnvironment::Production)
    }

    fn settings(store: &Store, value: &Value) {
        store
            .set_setting(keys::APP_SETTINGS, &value.to_string())
            .unwrap();
    }

    fn contact(store: &Store, id: &str, level: pc::AcquaintanceLevel, state: pc::ActivityState) {
        let mut contact = crate::contacts::test_contact(id);
        contact.acquaintance_level = level;
        contact.activity_state = state;
        store.upsert_contact(&contact).unwrap();
    }

    fn group(store: &Store, members: &[&str], left: bool) -> String {
        let identity = GroupIdentity {
            creator_identity: ThreemaId::try_from("ABCD1234").unwrap(),
            group_id: 7,
        };
        let key = crate::groups::key(&identity);
        store
            .upsert_group(&crate::groups::Group {
                key: key.clone(),
                identity,
                name: String::new(),
                members: members.iter().map(ToString::to_string).collect(),
                left,
                created_at: 0,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        key
    }

    #[test]
    fn a_storage_failure_keeps_its_code_through_libthreema() {
        let error = foreign(StoreError::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(13),
            None,
        )));
        let text = libthreema::csp_e2e::CspE2eProtocolError::from(error).to_string();
        assert_eq!(storage_code(&text), Some(13), "{text}");

        for other in ["Desync error: no storage", "storage[]: x", "storage[1", ""] {
            assert_eq!(storage_code(other), None, "{other}");
        }
    }

    /// A message of this kind fails at the store write `trigger` refuses, comes again once that
    /// works, and is then complete: what `check` looks at is there, and it is announced once.
    fn redelivery_is_complete(
        kind_of_write: &str,
        sender: &str,
        conversation: &str,
        body: &dyn Fn() -> IncomingMessageBody,
        check: &dyn Fn(&Store, &Message),
    ) {
        let (_dir, store) = store();
        contact(
            &store,
            sender,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Active,
        );
        if conversation != sender {
            group(&store, &["ABCD1234", sender], false);
        }
        store
            .conn
            .execute_batch(&format!(
                "CREATE TRIGGER refuse {kind_of_write} BEGIN SELECT RAISE(ABORT, 'disk full'); END;"
            ))
            .unwrap();
        let store = Rc::new(store);
        let events: EventSink = Rc::default();
        let mut conversations = StoreConversations {
            store: Rc::clone(&store),
            events: Rc::clone(&events),
            user: ThreemaId::try_from("USER0001").unwrap(),
            from_sync: false,
        };
        let message = || IncomingMessage {
            sender_identity: ThreemaId::try_from(sender).unwrap(),
            id: MessageId(9),
            overrides: libthreema::model::message::MessageOverrides::default(),
            created_at: 1,
            body: body(),
        };

        assert!(
            conversations
                .add_or_update_incoming_message(message())
                .is_err()
        );
        assert!(store.conn.is_autocommit(), "no transaction left open");
        assert!(events.borrow().is_empty(), "nothing announced");
        assert!(
            store
                .message_by_id(conversation, false, 9)
                .unwrap()
                .is_none(),
            "nothing left that makes the next delivery look like a duplicate"
        );

        store.conn.execute_batch("DROP TRIGGER refuse").unwrap();
        conversations
            .add_or_update_incoming_message(message())
            .unwrap();
        let stored = store
            .message_by_id(conversation, false, 9)
            .unwrap()
            .unwrap();
        check(&store, &stored);
        let added = events
            .borrow()
            .iter()
            .filter(|event| matches!(event, CoreEvent::MessageAdded { .. }))
            .count();
        assert_eq!(added, 1);
    }

    fn file() -> libthreema::model::message::FileMessage {
        libthreema::model::message::FileMessage {
            rendering_type: libthreema::model::message::FileRenderingType::Media,
            key: [7; 32],
            blob_id: libthreema::common::BlobId([7; 16]),
            media_type: "image/jpeg".into(),
            name: None,
            size: 1000,
            thumbnail_blob_id: None,
            thumbnail_media_type: None,
            caption: None,
            correlation_id: None,
            metadata: libthreema::model::message::FileMetadata::default(),
        }
    }

    fn group_body(body: GroupMessageBody) -> IncomingMessageBody {
        IncomingMessageBody::Group(IncomingGroupMessageBody {
            group_identity: GroupIdentity {
                creator_identity: ThreemaId::try_from("ABCD1234").unwrap(),
                group_id: 7,
            },
            body,
        })
    }

    #[test]
    fn a_file_whose_keys_failed_comes_again_with_its_keys() {
        redelivery_is_complete(
            "BEFORE INSERT ON files",
            MEMBER,
            MEMBER,
            &|| IncomingMessageBody::Contact(ContactMessageBody::File(file())),
            &|store, stored| assert!(store.file_keys(stored.id).unwrap().is_some()),
        );
    }

    #[test]
    fn a_group_text_whose_sender_failed_comes_again_with_its_sender() {
        let key = crate::groups::key(&GroupIdentity {
            creator_identity: ThreemaId::try_from("ABCD1234").unwrap(),
            group_id: 7,
        });
        redelivery_is_complete(
            "BEFORE UPDATE OF sender ON messages",
            MEMBER,
            &key,
            &|| group_body(GroupMessageBody::Text(TextMessage { text: "hi".into() })),
            &|_, stored| assert_eq!(stored.sender.as_deref(), Some(MEMBER)),
        );
    }

    #[test]
    fn a_group_file_whose_keys_failed_comes_again_with_keys_and_sender() {
        let key = crate::groups::key(&GroupIdentity {
            creator_identity: ThreemaId::try_from("ABCD1234").unwrap(),
            group_id: 7,
        });
        redelivery_is_complete(
            "BEFORE INSERT ON files",
            MEMBER,
            &key,
            &|| group_body(GroupMessageBody::File(file())),
            &|store, stored| {
                assert!(store.file_keys(stored.id).unwrap().is_some());
                assert_eq!(stored.sender.as_deref(), Some(MEMBER));
            },
        );
    }

    /// The group steps have store steps of their own, which have to work inside the message's. A
    /// message is no reason to ask its sender for a picture: they send it with their messages.
    #[test]
    fn a_message_from_someone_without_a_picture_asks_for_nothing() {
        let (_dir, store) = store();
        contact(
            &store,
            MEMBER,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Active,
        );
        group(&store, &["ABCD1234", MEMBER], false);
        let store = Rc::new(store);
        deliver(
            &store,
            MEMBER,
            1,
            group_body(GroupMessageBody::Text(TextMessage { text: "hi".into() })),
        );
        assert!(store.conn.is_autocommit());
        assert!(store.outbox().unwrap().is_empty());
    }

    /// The server won't send a stored message again, so its receipt waits in the store, once. What
    /// another device received, that device has answered.
    #[test]
    fn a_stored_message_leaves_its_receipt_in_the_store() {
        let (_dir, store) = store();
        contact(
            &store,
            MEMBER,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Active,
        );
        let store = Rc::new(store);
        let events: EventSink = Rc::default();
        let receive = |id: u64, from_sync: bool, no_receipt: bool| {
            let overrides = MessageOverrides {
                disable_delivery_receipts: no_receipt,
            };
            StoreConversations {
                store: Rc::clone(&store),
                events: Rc::clone(&events),
                user: ThreemaId::try_from("USER0001").unwrap(),
                from_sync,
            }
            .add_or_update_incoming_message(IncomingMessage {
                sender_identity: ThreemaId::try_from(MEMBER).unwrap(),
                id: MessageId(id),
                overrides,
                created_at: 1,
                body: IncomingMessageBody::Contact(ContactMessageBody::Text(TextMessage {
                    text: "hi".into(),
                })),
            })
            .unwrap();
        };
        let receipts = || {
            store
                .outbox()
                .unwrap()
                .into_iter()
                .filter(|entry| entry.kind == reactions::OUTBOX_DELIVERY_RECEIPT)
                .map(|entry| entry.body["messageId"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };

        receive(9, false, false);
        assert_eq!(receipts(), ["0000000000000009"]);
        assert!(events.borrow().iter().any(|event| matches!(
            event,
            CoreEvent::ReceiptQueued { receiver, .. } if receiver == MEMBER
        )));
        // A repeat isn't new.
        receive(9, false, false);
        receive(10, true, false);
        receive(11, false, true);
        assert_eq!(receipts(), ["0000000000000009"]);
        // Asked for none: the read receipt remembers it.
        let without: Vec<u64> = store
            .mark_conversation_read(MEMBER)
            .unwrap()
            .into_iter()
            .filter(|message| message.no_receipts)
            .map(|message| message.message_id)
            .collect();
        assert_eq!(without, [11]);
    }

    #[test]
    fn a_message_counts_as_known_once_the_other_devices_have_it() {
        let (_dir, store) = store();
        contact(
            &store,
            MEMBER,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Active,
        );
        let store = Rc::new(store);
        let text = || {
            IncomingMessageBody::Contact(ContactMessageBody::Text(TextMessage {
                text: "hi".into(),
            }))
        };
        let sender = ThreemaId::try_from(MEMBER).unwrap();
        let conversations = |from_sync| StoreConversations {
            store: Rc::clone(&store),
            events: Rc::default(),
            user: ThreemaId::try_from("USER0001").unwrap(),
            from_sync,
        };
        let known = |id| {
            conversations(false)
                .message_is_marked_used(sender, MessageId(id))
                .unwrap()
        };

        // No other devices: known at once.
        deliver(&store, MEMBER, 1, text());
        assert!(known(1));

        store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        deliver(&store, MEMBER, 2, text());
        assert!(!known(2), "the mediator hasn't confirmed it");
        assert_eq!(store.conversation_messages(MEMBER, 10).unwrap().len(), 2);
        deliver(&store, MEMBER, 2, text());
        assert_eq!(store.conversation_messages(MEMBER, 10).unwrap().len(), 2);
        store.reflection_done(MEMBER, 2).unwrap();
        assert!(known(2));

        // What another device reflected to us is that device's to tell.
        let mut follower = conversations(true);
        follower
            .add_or_update_incoming_message(IncomingMessage {
                sender_identity: sender,
                id: MessageId(3),
                overrides: libthreema::model::message::MessageOverrides::default(),
                created_at: 1,
                body: text(),
            })
            .unwrap();
        assert!(known(3));
    }

    /// `body` arriving from `sender`, as the connection hands it over.
    fn deliver(store: &Rc<Store>, sender: &str, id: u64, body: IncomingMessageBody) {
        let mut conversations = StoreConversations {
            store: Rc::clone(store),
            events: Rc::default(),
            user: ThreemaId::try_from("USER0001").unwrap(),
            from_sync: false,
        };
        conversations
            .add_or_update_incoming_message(IncomingMessage {
                sender_identity: ThreemaId::try_from(sender).unwrap(),
                id: MessageId(id),
                overrides: libthreema::model::message::MessageOverrides::default(),
                created_at: 50,
                body,
            })
            .unwrap();
    }

    #[test]
    fn a_received_edit_reaches_a_caption_in_a_chat_and_in_a_group() {
        use libthreema::model::message::{EditMessage, GroupMessageBody, IncomingGroupMessageBody};
        let (_dir, store) = store();
        contact(
            &store,
            MEMBER,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Active,
        );
        let key = group(&store, &["ABCD1234", MEMBER], false);
        let file = json!({"mediaType": "image/jpeg", "size": 1, "caption": "old"}).to_string();
        let direct = store
            .insert_incoming(MEMBER, 1, "file", &file, 1)
            .unwrap()
            .unwrap();
        let in_group = store
            .insert_incoming_group(&key, MEMBER, 2, "file", &file, 1)
            .unwrap()
            .unwrap();
        let store = Rc::new(store);
        let edit = |id, text: &str| EditMessage {
            message_id: MessageId(id),
            text: text.to_owned(),
        };
        deliver(
            &store,
            MEMBER,
            10,
            IncomingMessageBody::Contact(ContactMessageBody::Edit(edit(1, "one"))),
        );
        deliver(
            &store,
            MEMBER,
            11,
            IncomingMessageBody::Group(IncomingGroupMessageBody {
                group_identity: crate::groups::parse_key(&key).unwrap(),
                body: GroupMessageBody::Edit(edit(2, "two")),
            }),
        );
        for (id, caption) in [(direct.id, "one"), (in_group.id, "two")] {
            let edited = store.message(id).unwrap().unwrap();
            let body: serde_json::Value = serde_json::from_str(&edited.body).unwrap();
            assert_eq!(body["caption"], caption);
            assert_eq!(edited.edited_at, Some(50));
        }
    }

    /// Android lets nobody delete a poll: one that came in stays what votes and a close refer to,
    /// from the contact and in a group.
    #[test]
    fn a_received_deletion_applies_to_texts_locations_and_files_only() {
        use libthreema::model::message::{
            DeleteMessage, GroupMessageBody, IncomingGroupMessageBody,
        };
        let (_dir, store) = store();
        contact(
            &store,
            MEMBER,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Active,
        );
        let key = group(&store, &["ABCD1234", MEMBER], false);
        let insert = |conversation: &str, id, kind: &str| {
            if conversation == key {
                store.insert_incoming_group(&key, MEMBER, id, kind, "{}", 1)
            } else {
                store.insert_incoming(conversation, id, kind, "{}", 1)
            }
            .unwrap()
            .unwrap()
            .id
        };
        let cases: Vec<(&str, i64, u64)> = vec![
            ("text", insert(MEMBER, 1, "text"), 1),
            ("location", insert(MEMBER, 2, "location"), 2),
            ("file", insert(MEMBER, 3, "file"), 3),
            ("poll", insert(MEMBER, 4, "poll"), 4),
            ("pollClosed", insert(MEMBER, 5, "pollClosed"), 5),
        ];
        let in_group: Vec<(&str, i64, u64)> = vec![
            ("text", insert(&key, 11, "text"), 11),
            ("poll", insert(&key, 12, "poll"), 12),
        ];
        let store = Rc::new(store);
        for (_, _, id) in &cases {
            deliver(
                &store,
                MEMBER,
                100 + id,
                IncomingMessageBody::Contact(ContactMessageBody::Delete(DeleteMessage {
                    message_id: MessageId(*id),
                })),
            );
        }
        for (_, _, id) in &in_group {
            deliver(
                &store,
                MEMBER,
                100 + id,
                IncomingMessageBody::Group(IncomingGroupMessageBody {
                    group_identity: crate::groups::parse_key(&key).unwrap(),
                    body: GroupMessageBody::Delete(DeleteMessage {
                        message_id: MessageId(*id),
                    }),
                }),
            );
        }
        for (kind, id, _) in cases.iter().chain(&in_group) {
            let stored = store.message(*id).unwrap().unwrap();
            let expected = if crate::messages::delete_applies_to(kind) {
                "deleted"
            } else {
                kind
            };
            assert_eq!(stored.kind, expected);
        }
    }

    #[test]
    fn a_reflected_group_message_asks_nobody_and_reports_no_change() {
        use libthreema::model::message::{GroupMessageBody, IncomingGroupMessageBody, TextMessage};
        let (_dir, store) = store();
        contact(
            &store,
            MEMBER,
            pc::AcquaintanceLevel::GroupOrDeleted,
            pc::ActivityState::Active,
        );
        let key = group(&store, &["ABCD1234", MEMBER], false);
        let store = Rc::new(store);
        let receive = |from_sync: bool, id: u64| {
            let events: EventSink = Rc::default();
            let mut conversations = StoreConversations {
                store: Rc::clone(&store),
                events: Rc::clone(&events),
                user: ThreemaId::try_from("USER0001").unwrap(),
                from_sync,
            };
            conversations
                .add_or_update_incoming_message(IncomingMessage {
                    sender_identity: ThreemaId::try_from(MEMBER).unwrap(),
                    id: MessageId(id),
                    overrides: libthreema::model::message::MessageOverrides::default(),
                    created_at: 1,
                    body: IncomingMessageBody::Group(IncomingGroupMessageBody {
                        group_identity: crate::groups::parse_key(&key).unwrap(),
                        body: GroupMessageBody::Text(TextMessage { text: "hi".into() }),
                    }),
                })
                .unwrap();
            let added = events
                .borrow()
                .iter()
                .any(|event| matches!(event, CoreEvent::MessageAdded { .. }));
            assert!(added);
        };
        receive(true, 1);
        assert!(store.outbox().unwrap().is_empty());
    }

    #[test]
    fn someone_no_longer_in_the_group_gets_only_what_says_so() {
        let (_dir, store) = store();
        let config = Config::from(ConfigEnvironment::Production);
        for id in [MEMBER, OTHER] {
            contact(
                &store,
                id,
                pc::AcquaintanceLevel::Direct,
                pc::ActivityState::Active,
            );
        }
        // OTHER left or was removed after these were queued.
        let key = group(&store, &[MEMBER], false);
        let both = [MEMBER.to_owned(), OTHER.to_owned()];

        store.set_pending_receivers(&key, 1, &both).unwrap();
        assert_eq!(
            group_receivers(&store, &config, &key, 1, false, true).unwrap(),
            vec![MEMBER]
        );
        assert_eq!(store.pending_receivers(&key, 1).unwrap(), vec![MEMBER]);

        // An empty setup, say.
        store.set_pending_receivers(&key, 2, &both).unwrap();
        assert_eq!(
            group_receivers(&store, &config, &key, 2, true, false).unwrap(),
            vec![MEMBER, OTHER]
        );

        // The group is gone: what is left is the farewell, by the key alone.
        store.delete_group(&key).unwrap();
        store.set_pending_receivers(&key, 3, &both).unwrap();
        assert_eq!(
            group_receivers(&store, &config, &key, 3, true, true).unwrap(),
            vec![MEMBER, OTHER]
        );
    }

    #[test]
    fn a_group_change_received_as_leader_is_reflected_and_its_new_members_looked_up() {
        use libthreema::model::message::{
            GroupMessageBody, GroupSetupMessage, IncomingGroupMessageBody,
        };
        let (_dir, store) = store();
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "USER0001".into(),
                client_key: zeroize::Zeroizing::new([9; 32]),
                server_group: "01".into(),
            })
            .unwrap();
        store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        contact(
            &store,
            "ABCD1234",
            pc::AcquaintanceLevel::GroupOrDeleted,
            pc::ActivityState::Active,
        );
        let store = Rc::new(store);
        let events: EventSink = Rc::default();
        let mut conversations = StoreConversations {
            store: Rc::clone(&store),
            events: Rc::clone(&events),
            user: ThreemaId::try_from("USER0001").unwrap(),
            from_sync: false,
        };
        let identity = GroupIdentity {
            creator_identity: ThreemaId::try_from("ABCD1234").unwrap(),
            group_id: 7,
        };
        conversations
            .add_or_update_incoming_message(IncomingMessage {
                sender_identity: identity.creator_identity,
                id: MessageId(1),
                overrides: libthreema::model::message::MessageOverrides::default(),
                created_at: 1,
                body: IncomingMessageBody::Group(IncomingGroupMessageBody {
                    group_identity: identity,
                    body: GroupMessageBody::Setup(GroupSetupMessage {
                        members: vec![
                            ThreemaId::try_from("USER0001").unwrap(),
                            ThreemaId::try_from(OTHER).unwrap(),
                        ],
                    }),
                }),
            })
            .unwrap();
        let events = events.borrow();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, CoreEvent::SyncQueued))
        );
        assert!(events.iter().any(|event| matches!(
            event,
            CoreEvent::UnknownMembers(identities) if identities.as_slice() == [OTHER.to_owned()]
        )));
        assert!(store.next_sync().unwrap().is_some());
    }

    #[test]
    fn control_messages_go_to_contacts_and_to_strangers_who_just_wrote() {
        let (_dir, store) = store();
        let config = config();
        let key = |byte| libthreema::common::keys::PublicKey::from([byte; 32]);
        let to = |id: &str, peer_key| {
            control_receiver(&store, &config, id, peer_key)
                .unwrap()
                .map(|(identity, key)| (identity.to_string(), key))
        };
        contact(
            &store,
            OTHER,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Active,
        );
        contact(
            &store,
            MEMBER,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Invalid,
        );
        // A contact goes by the stored key, whatever the message came with.
        let stored = store.contact(OTHER).unwrap().unwrap().public_key;
        assert!(to(OTHER, Some(key(1))) == Some((OTHER.to_owned(), stored)));
        assert!(to(OTHER, None) == Some((OTHER.to_owned(), stored)));
        // Revoked: nothing.
        assert!(to(MEMBER, Some(key(1))).is_none());
        // A stranger who wrote, and only one who did.
        assert!(to(STRANGER, Some(key(1))) == Some((STRANGER.to_owned(), key(1))));
        assert!(to(STRANGER, None).is_none());
        // Not on the block list, and not with unknown senders blocked.
        store
            .set_setting(
                crate::store::keys::APP_SETTINGS,
                &serde_json::json!({ "blockedContacts": [STRANGER] }).to_string(),
            )
            .unwrap();
        assert!(to(STRANGER, Some(key(1))).is_none());
        store
            .set_setting(
                crate::store::keys::APP_SETTINGS,
                &serde_json::json!({ "blockUnknown": true }).to_string(),
            )
            .unwrap();
        assert!(to(STRANGER, Some(key(1))).is_none());
    }

    #[test]
    fn identity_blocked_steps() {
        let (_dir, store) = store();
        let config = config();
        let blocked = |id: &str| identity_blocked(&store, &config, id).unwrap();
        contact(
            &store,
            MEMBER,
            pc::AcquaintanceLevel::GroupOrDeleted,
            pc::ActivityState::Active,
        );
        contact(
            &store,
            OTHER,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Active,
        );

        // Nothing set: nobody.
        assert!(!blocked(STRANGER) && !blocked(MEMBER) && !blocked(OTHER));

        // The block list, whoever is on it; special contacts can't be.
        settings(
            &store,
            &json!({ "blockedContacts": [OTHER, "*3MAPUSH", "*3MATOKN"] }),
        );
        assert!(blocked(OTHER) && blocked("*3MATOKN"));
        assert!(!blocked("*3MAPUSH") && !blocked(STRANGER));

        // Block unknown: strangers and non-direct contacts, unless they share an active group.
        settings(&store, &json!({ "blockUnknown": true }));
        assert!(blocked(STRANGER) && blocked(MEMBER));
        assert!(!blocked(OTHER), "a direct contact");
        assert!(!blocked("*3MATOKN"), "a predefined contact");
        group(&store, &[MEMBER], true);
        assert!(blocked(MEMBER), "only a group we left");
        group(&store, &[MEMBER], false);
        assert!(!blocked(MEMBER));
    }

    #[test]
    fn group_receivers_lose_the_blocked_unless_exempt_and_always_the_revoked() {
        let (_dir, store) = store();
        let config = config();
        for id in [MEMBER, OTHER, STRANGER] {
            contact(
                &store,
                id,
                pc::AcquaintanceLevel::Direct,
                pc::ActivityState::Active,
            );
        }
        contact(
            &store,
            STRANGER,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Invalid,
        );
        settings(&store, &json!({ "blockedContacts": [OTHER] }));
        let key = group(&store, &[MEMBER, OTHER, STRANGER], false);
        let all = [MEMBER.to_owned(), OTHER.to_owned(), STRANGER.to_owned()];

        // A group-name, say: the blocked member stays in step, the revoked one is gone.
        store.set_pending_receivers(&key, 1, &all).unwrap();
        assert_eq!(
            group_receivers(&store, &config, &key, 1, true, true).unwrap(),
            vec![MEMBER, OTHER]
        );
        assert_eq!(
            store.pending_receivers(&key, 1).unwrap(),
            vec![MEMBER, OTHER]
        );

        // A text queued before the block: off the list, so it doesn't wait for them.
        store.set_pending_receivers(&key, 2, &all).unwrap();
        assert_eq!(
            group_receivers(&store, &config, &key, 2, false, true).unwrap(),
            vec![MEMBER]
        );
        assert_eq!(store.pending_receivers(&key, 2).unwrap(), vec![MEMBER]);

        // Nobody left: the caller counts it as sent.
        settings(&store, &json!({ "blockedContacts": [MEMBER, OTHER] }));
        assert!(
            group_receivers(&store, &config, &key, 2, false, true)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn revoked_and_blocked_receive_nothing_they_are_not_owed() {
        let (_dir, store) = store();
        let config = config();
        contact(
            &store,
            MEMBER,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Active,
        );
        let may = |exempt| may_receive(&store, &config, MEMBER, exempt).unwrap();
        assert!(may(false));

        settings(&store, &json!({ "blockedContacts": [MEMBER] }));
        assert!(!may(false) && may(true), "only what is exempt");

        settings(&store, &json!({}));
        contact(
            &store,
            MEMBER,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Invalid,
        );
        assert!(!may(false) && !may(true), "a revoked ID, nothing at all");
    }

    #[test]
    fn fs_status_lines_only_for_contacts_not_blocked() {
        let (_dir, store) = store();
        let config = config();
        let shows = |id: &str| shows_fs_status(&store, &config, id).unwrap();
        contact(
            &store,
            MEMBER,
            pc::AcquaintanceLevel::Direct,
            pc::ActivityState::Active,
        );
        assert!(shows(MEMBER));
        assert!(!shows(STRANGER), "no contact, no chat");
        settings(&store, &json!({ "blockedContacts": [MEMBER] }));
        assert!(!shows(MEMBER));
        // Known but neither direct nor in a group with us, with "block unknown senders" on.
        contact(
            &store,
            OTHER,
            pc::AcquaintanceLevel::GroupOrDeleted,
            pc::ActivityState::Active,
        );
        assert!(shows(OTHER));
        settings(&store, &json!({ "blockUnknown": true }));
        assert!(!shows(OTHER));
    }

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
