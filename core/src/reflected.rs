// SPDX-License-Identifier: AGPL-3.0-only
//! Reflected D2D envelopes (multi-device, ADR 0011): what the other devices of the group did,
//! applied to our store.
//!
//! Upstream: Android's `IncomingReflectedMessageTask` with `reflectedoutgoingmessage/*`,
//! `reflectedmessageupdate/*` and the incoming message processor. Handled so far: outgoing
//! messages (text, location, file, edit, delete, reaction, read receipts, profile pictures),
//! incoming messages (when another device was leader, with its typing indicators), the sent/read
//! updates, and contact, group, profile and settings sync (`sync.rs`). Distribution lists are not
//! synced (Android doesn't either).

use std::rc::Rc;

use libthreema::{
    common::{MessageId, ThreemaId},
    model::{
        message::{
            ContactMessageBody, DeliveryReceiptType, GroupMessageBody, IncomingMessage,
            IncomingMessageBody, MessageOverrides, ReactionAction,
        },
        provider::ConversationProvider as _,
    },
    protobuf::{
        d2d::{
            self, ConversationId, conversation_id, envelope, incoming_message_update,
            outgoing_message_update,
        },
        d2d_sync::contact::AcquaintanceLevel,
    },
};
use prost::Message as _;
use serde_json::json;

use crate::{
    messages::{Message, OutgoingState},
    providers::{CoreEvent, EventSink, StoreConversations},
    store::{Store, StoreError},
};

/// The length of a chat server message box's nonce.
const NONCE_LEN: usize = 24;

/// How many reflected messages may wait for their sender's contact (at most a frame each, so some
/// 8 MB). The oldest stay. A message beyond this is on the other device only, which is counted, and
/// shown with the linked devices (`multiDevice`).
pub(crate) const REFLECTED_WAITING_MAX: i64 = 1000;

/// A reflected envelope could not be used. It is still acknowledged, as the mediator would
/// otherwise send it again forever, unless the store failed ([`Self::storage_code`]).
#[derive(Debug, thiserror::Error)]
pub enum ReflectedError {
    #[error("decrypt: {0}")]
    Decrypt(String),
    #[error("replayed nonce")]
    Replay,
    #[error("decode: {0}")]
    Decode(String),
    #[error("store: {0}")]
    Store(#[from] StoreError),
    /// A store failure that came through libthreema's provider interface: SQLite's code.
    #[error("store: code {0}")]
    StoreCode(i32),
}

impl ReflectedError {
    /// SQLite's code if the store failed: the mediator is to send the envelope again, as no other
    /// route brings it here. (Not for good if it fails the same way every time:
    /// [`crate::store::fails_every_time`].)
    #[must_use]
    pub fn storage_code(&self) -> Option<i32> {
        match self {
            Self::Store(error) => Some(error.sqlite_code()),
            Self::StoreCode(code) => Some(*code),
            Self::Decrypt(_) | Self::Replay | Self::Decode(_) => None,
        }
    }
}

/// A decrypted reflected envelope seen for the first time.
pub struct Opened {
    pub envelope: d2d::Envelope,
    nonce: [u8; 24],
}

/// What an envelope was about (for logs: never content).
#[must_use]
pub fn content_name(content: Option<&envelope::Content>) -> &'static str {
    match content {
        Some(envelope::Content::OutgoingMessage(_)) => "outgoing-message",
        Some(envelope::Content::OutgoingMessageUpdate(_)) => "outgoing-message-update",
        Some(envelope::Content::IncomingMessage(_)) => "incoming-message",
        Some(envelope::Content::IncomingMessageUpdate(_)) => "incoming-message-update",
        Some(envelope::Content::UserProfileSync(_)) => "user-profile-sync",
        Some(envelope::Content::ContactSync(_)) => "contact-sync",
        Some(envelope::Content::GroupSync(_)) => "group-sync",
        Some(envelope::Content::DistributionListSync(_)) => "distribution-list-sync",
        Some(envelope::Content::SettingsSync(_)) => "settings-sync",
        Some(envelope::Content::MdmParameterSync(_)) => "mdm-parameter-sync",
        None => "empty",
    }
}

/// Our conversation key for a D2D conversation id.
fn conversation_key(conversation: Option<&ConversationId>) -> Option<String> {
    match conversation?.id.as_ref()? {
        conversation_id::Id::Contact(identity) => Some(identity.clone()),
        conversation_id::Id::Group(group) => {
            let creator: ThreemaId = group.creator_identity.parse().ok()?;
            Some(crate::groups::key(&libthreema::common::GroupIdentity {
                group_id: group.group_id,
                creator_identity: creator,
            }))
        }
        conversation_id::Id::DistributionList(_) => None,
    }
}

/// Decrypt, check the nonce, decode. The nonce of an envelope to apply counts once that is done
/// ([`apply_once`]): killed in between, the mediator's next copy must not look like a replay.
pub fn open(
    store: &Store,
    device_group_key: &libthreema::common::keys::DeviceGroupKey,
    data: &[u8],
) -> Result<Opened, ReflectedError> {
    let (nonce, plain) = device_group_key
        .decrypt_envelope(data)
        .map_err(|e| ReflectedError::Decrypt(e.to_string()))?;
    if store.has_d2d_nonce(&nonce)? {
        return Err(ReflectedError::Replay);
    }
    match d2d::Envelope::decode(plain.as_slice()) {
        Ok(envelope) => Ok(Opened { envelope, nonce }),
        Err(error) => {
            store.add_d2d_nonce(&nonce)?;
            Err(ReflectedError::Decode(error.to_string()))
        }
    }
}

/// An envelope is given up on: from now on it counts as seen, and so does what it says came from
/// the chat server.
pub fn note_dropped(
    store: &Store,
    user: ThreemaId,
    device_group_key: &libthreema::common::keys::DeviceGroupKey,
    data: &[u8],
) -> Result<(), ReflectedError> {
    let (nonce, plain) = device_group_key
        .decrypt_envelope(data)
        .map_err(|e| ReflectedError::Decrypt(e.to_string()))?;
    let csp = d2d::Envelope::decode(plain.as_slice())
        .map(|envelope| csp_nonces(user, envelope.content.as_ref()))
        .unwrap_or_default();
    note_seen(store, &nonce, &csp)?;
    Ok(())
}

/// The nonces of the chat server's message boxes an envelope tells of, for this device's replay
/// protection (md-d2d.proto): should it become leader, the server can't hand those out again, and
/// a device joining later gets them along. Not those of messages without replay protection (typing,
/// like the incoming message task). A body that doesn't decode counts as protected.
fn csp_nonces(user: ThreemaId, content: Option<&envelope::Content>) -> Vec<Vec<u8>> {
    let protected = |sender: ThreemaId, message_type: i32, body: &[u8]| {
        IncomingMessageBody::decode_reflected(sender, message_type, body)
            .map_or(true, |body| body.has_replay_protection())
    };
    let nonces: Vec<&Vec<u8>> = match content {
        Some(envelope::Content::IncomingMessage(message)) => {
            let sender = message.sender_identity.parse().unwrap_or(user);
            if !protected(sender, message.r#type, &message.body) || message.nonce.is_empty() {
                return Vec::new();
            }
            vec![&message.nonce]
        }
        Some(envelope::Content::OutgoingMessage(message))
            if protected(user, message.r#type, &message.body) =>
        {
            message.nonces.iter().collect()
        }
        _ => return Vec::new(),
    };
    let wanted = nonces.len();
    let nonces: Vec<Vec<u8>> = nonces
        .into_iter()
        .filter(|nonce| nonce.len() == NONCE_LEN)
        .cloned()
        .collect();
    if nonces.len() != wanted {
        tracing::warn!(
            count = wanted - nonces.len(),
            "reflected nonces of a wrong length left out"
        );
    }
    nonces
}

/// An envelope's nonce and those of the message boxes it tells of count as used.
fn note_seen(store: &Store, envelope_nonce: &[u8], csp: &[Vec<u8>]) -> Result<(), StoreError> {
    store.add_d2d_nonce(envelope_nonce)?;
    let csp: Vec<&[u8]> = csp.iter().map(Vec::as_slice).collect();
    store.add_nonces(&csp)
}

/// Whether a reflected message from someone this device has no contact for is worth keeping until
/// it has one: what makes them a direct contact on a device that leads, and what is said in a group
/// we have, by a member whose lookup is still open. What is left is of no use without a chat to
/// show it in, and someone on the block list doesn't become a contact by trying (ADR 0019).
fn waits_for_sender(
    store: &Store,
    sender: ThreemaId,
    body: &IncomingMessageBody,
) -> Result<bool, StoreError> {
    let wanted = match body {
        IncomingMessageBody::Contact(_) => body.requires_direct_contact(),
        IncomingMessageBody::Group(group) => {
            let control = group.body.uses_creator_container()
                || matches!(
                    group.body,
                    GroupMessageBody::Leave(_)
                        | GroupMessageBody::SyncRequest(_)
                        | GroupMessageBody::CallStart(_)
                );
            !control
                && store
                    .group(&crate::groups::key(&group.group_identity))?
                    .is_some()
        }
    };
    Ok(wanted && !store.is_explicitly_blocked(sender.as_str())?)
}

/// Apply what has waited for a sender who has a contact by now ([`waits_for_sender`]), each in a
/// store step of its own. What they'd be stopped for now (the block list, or "block unknown senders"
/// for a stranger, as on a device that leads: ADR 0019) goes, contact or not, and no lookup is owed
/// for it. Who the directory has said doesn't exist takes it with them; for the rest the lookup
/// stays owed.
pub fn apply_waiting(
    store: &Rc<Store>,
    config: &libthreema::common::config::Config,
    events: &EventSink,
) -> Result<(), StoreError> {
    let waiting = store.reflected_waiting()?;
    if waiting.is_empty() {
        return Ok(());
    }
    let Some(user) = store
        .identity()?
        .and_then(|identity| identity.threema_id.parse::<ThreemaId>().ok())
    else {
        return Ok(());
    };
    let mut owed = Vec::new();
    for (id, sender, message) in waiting {
        if crate::providers::identity_blocked(store, config, &sender)? {
            tracing::warn!("a reflected message's sender is blocked by now, message dropped");
            store.reflected_done(id)?;
            store.member_lookups_answered(&[sender.as_str()])?;
            continue;
        }
        if store.contact(&sender)?.is_none() {
            if store.recently_invalid(&sender) {
                tracing::warn!("a reflected message's sender doesn't exist, message dropped");
                store.reflected_done(id)?;
            } else {
                owed.push(sender);
            }
            continue;
        }
        let announced = events.borrow().len();
        let content = d2d::IncomingMessage::decode(message.as_slice())
            .map(envelope::Content::IncomingMessage)
            .map_err(|error| ReflectedError::Decode(error.to_string()));
        let step = store.step()?;
        let applied = content
            .and_then(|content| {
                let envelope = d2d::Envelope {
                    content: Some(content),
                    ..Default::default()
                };
                apply(store, user, envelope, events)
            })
            .and_then(|()| {
                store.reflected_done(id)?;
                Ok(step.commit()?)
            });
        if let Err(error) = applied {
            events.borrow_mut().truncate(announced);
            tracing::warn!(%error, "a reflected message that waited for its sender failed");
            // Comes again if the store failed; otherwise it won't do any better next time.
            if error.storage_code().is_none() {
                store.reflected_done(id)?;
            }
        }
    }
    store.owe_member_lookups(&owed)
}

/// Apply an opened envelope and note its nonces, in one store step. An envelope this can't use
/// leaves nothing behind but the nonces, and one the store failed on not even those. Events only
/// for what holds.
pub fn apply_once(
    store: &Rc<Store>,
    user: ThreemaId,
    opened: Opened,
    events: &EventSink,
) -> Result<(), ReflectedError> {
    let announced = events.borrow().len();
    let Opened { envelope, nonce } = opened;
    let csp = csp_nonces(user, envelope.content.as_ref());
    let step = store.step()?;
    let applied = apply(store, user, envelope, events).and_then(|()| {
        note_seen(store, &nonce, &csp)?;
        Ok(step.commit()?)
    });
    if let Err(error) = &applied {
        events.borrow_mut().truncate(announced);
        if error.storage_code().is_none() {
            note_seen(store, &nonce, &csp)?;
        }
    }
    applied
}

/// Apply an envelope; events for the UI go to `events`.
pub fn apply(
    store: &Rc<Store>,
    user: ThreemaId,
    envelope: d2d::Envelope,
    events: &EventSink,
) -> Result<(), ReflectedError> {
    match envelope.content {
        Some(envelope::Content::OutgoingMessage(message)) => {
            outgoing_message(store, user, message, events)
        }
        Some(envelope::Content::OutgoingMessageUpdate(update)) => {
            for update in update.updates {
                let Some(conversation) = conversation_key(update.conversation.as_ref()) else {
                    continue;
                };
                if matches!(
                    update.update,
                    Some(outgoing_message_update::update::Update::Sent(_))
                ) {
                    let changed = store.raise_outgoing_state(
                        &conversation,
                        &[update.message_id],
                        OutgoingState::Sent,
                    )?;
                    push_changed(events, changed);
                }
            }
            Ok(())
        }
        Some(envelope::Content::IncomingMessage(message)) => {
            let sender: ThreemaId = message
                .sender_identity
                .parse()
                .map_err(|_| ReflectedError::Decode("sender identity".into()))?;
            let body = IncomingMessageBody::decode_reflected(sender, message.r#type, &message.body)
                .map_err(ReflectedError::Decode)?;
            // Nothing else brings it here and the mediator's queue must go on: it stays here, for
            // as long as the directory takes (`apply_waiting`).
            let Some(mut contact) = store.contact(sender.as_str())? else {
                if !waits_for_sender(store, sender, &body)? {
                    tracing::warn!("reflected incoming message from an unknown contact skipped");
                } else if store.reflected_waiting_count()? >= REFLECTED_WAITING_MAX {
                    let count = store.note_reflected_left_out()?;
                    tracing::warn!(count, "too many reflected messages wait, this one left out");
                } else {
                    // Asked for once per sender: a burst must not be one request per message, let
                    // alone one wait. The note is what later tries go by.
                    let first = !store.has_reflected_waiting(sender.as_str())?;
                    store.keep_reflected(sender.as_str(), &message.encode_to_vec())?;
                    store.owe_member_lookups(&[sender.to_string()])?;
                    if first {
                        events
                            .borrow_mut()
                            .push(CoreEvent::UnknownMembers(vec![sender.to_string()]));
                    }
                }
                return Ok(());
            };
            // Not stored. Whether it is still news is for the connection to say
            // (`Session::handle_reflected`).
            if let IncomingMessageBody::Contact(ContactMessageBody::TypingIndicator(typing)) = &body
            {
                events.borrow_mut().push(CoreEvent::Typing {
                    identity: sender.to_string(),
                    typing: typing.is_typing,
                });
                return Ok(());
            }
            // Someone deleted here, or known from a group, writes while another device leads: what
            // the incoming message task does there. That device tells the others itself.
            // Not someone on the block list (ADR 0019), and not for a group-setup, the one group
            // message that would: a reflected one changes nothing here, a group sync does that.
            if matches!(body, IncomingMessageBody::Contact(_))
                && body.requires_direct_contact()
                && contact.acquaintance_level != AcquaintanceLevel::Direct
                && !store.is_explicitly_blocked(sender.as_str())?
            {
                contact.acquaintance_level = AcquaintanceLevel::Direct;
                store.upsert_contact(&contact)?;
                events.borrow_mut().push(CoreEvent::ContactsChanged);
            }
            let mut conversations = StoreConversations {
                store: Rc::clone(store),
                events: Rc::clone(events),
                user,
                from_sync: true,
            };
            conversations
                .add_or_update_incoming_message(IncomingMessage {
                    sender_identity: sender,
                    id: MessageId(message.message_id),
                    overrides: MessageOverrides::default(),
                    created_at: message.created_at,
                    body,
                })
                .map_err(|error| {
                    let error = error.to_string();
                    crate::providers::storage_code(&error)
                        .map_or(ReflectedError::Decode(error), ReflectedError::StoreCode)
                })?;
            Ok(())
        }
        Some(envelope::Content::IncomingMessageUpdate(update)) => {
            for update in update.updates {
                let Some(conversation) = conversation_key(update.conversation.as_ref()) else {
                    continue;
                };
                if let Some(incoming_message_update::update::Update::Read(read)) = update.update {
                    let changed = store.mark_incoming_read(
                        &conversation,
                        &[update.message_id],
                        i64::try_from(read.at).unwrap_or_default(),
                    )?;
                    // Also when nothing changed: the read receipt the other device sent may
                    // have marked these messages read a moment earlier (user report
                    // 2026-10-28: notifications stayed for chats read on Threema Desktop).
                    seen_elsewhere(store, events, &conversation);
                    push_changed(events, changed);
                }
            }
            Ok(())
        }
        Some(envelope::Content::ContactSync(sync)) => {
            let mut new_events = Vec::new();
            crate::sync::contact_sync(store, sync, &mut new_events)?;
            events.borrow_mut().extend(new_events);
            Ok(())
        }
        Some(envelope::Content::GroupSync(sync)) => {
            let mut new_events = Vec::new();
            crate::sync::group_sync(store, user.as_str(), sync, &mut new_events)?;
            events.borrow_mut().extend(new_events);
            Ok(())
        }
        Some(envelope::Content::SettingsSync(sync)) => {
            let mut new_events = Vec::new();
            crate::sync::settings_sync(store, sync, &mut new_events)?;
            events.borrow_mut().extend(new_events);
            Ok(())
        }
        Some(envelope::Content::UserProfileSync(sync)) => {
            let mut new_events = Vec::new();
            crate::sync::user_profile_sync(store, sync, &mut new_events)?;
            events.borrow_mut().extend(new_events);
            Ok(())
        }
        other => {
            // log-ok: the kind of content, not the content
            tracing::info!(
                content = content_name(other.as_ref()),
                "reflected sync not applied"
            );
            Ok(())
        }
    }
}

/// The chat was read or answered on another device: its notifications here go too (not
/// for a private chat, whose ID the UI must not learn while they are hidden). Harmless when
/// there is none.
fn seen_elsewhere(store: &Store, events: &EventSink, conversation: &str) {
    if !store.is_private(conversation).unwrap_or(true) {
        events.borrow_mut().push(CoreEvent::Call(
            json!({ "event": "conversationRead", "identity": conversation }),
        ));
    }
}

fn push_changed(events: &EventSink, changed: Vec<Message>) {
    if !changed.is_empty() {
        events
            .borrow_mut()
            .push(CoreEvent::MessagesChanged(changed));
    }
}

/// A message another device sent: stored like ours, already sent.
fn outgoing_message(
    store: &Rc<Store>,
    user: ThreemaId,
    message: d2d::OutgoingMessage,
    events: &EventSink,
) -> Result<(), ReflectedError> {
    let Some(conversation) = conversation_key(message.conversation.as_ref()) else {
        return Ok(());
    };
    let body = IncomingMessageBody::decode_reflected(user, message.r#type, &message.body)
        .map_err(ReflectedError::Decode)?;
    let created_at = i64::try_from(message.created_at).unwrap_or_default();
    // Group bodies carry the same content as 1:1 ones.
    let contact_body = match body {
        IncomingMessageBody::Contact(body) => Some(body),
        IncomingMessageBody::Group(group) => match group.body {
            GroupMessageBody::Text(text) => Some(ContactMessageBody::Text(text)),
            GroupMessageBody::Location(location) => Some(ContactMessageBody::Location(location)),
            GroupMessageBody::File(file) => Some(ContactMessageBody::File(file)),
            GroupMessageBody::Reaction(reaction) => Some(ContactMessageBody::Reaction(reaction)),
            GroupMessageBody::LegacyReaction(reaction) => {
                Some(ContactMessageBody::LegacyReaction(reaction))
            }
            GroupMessageBody::PollSetup(setup) => Some(ContactMessageBody::PollSetup(setup)),
            GroupMessageBody::PollVote(vote) => Some(ContactMessageBody::PollVote(vote)),
            GroupMessageBody::Edit(edit) => Some(ContactMessageBody::Edit(edit)),
            GroupMessageBody::Delete(delete) => Some(ContactMessageBody::Delete(delete)),
            // Group control (setup, name, leave, pictures) arrives as a group sync too, and
            // group calls are found by their refresh: nothing to store from the message.
            _ => None,
        },
    };
    let Some(body) = contact_body else {
        tracing::info!("reflected outgoing message of a kind not handled yet");
        return Ok(());
    };
    let (kind, content, file_keys) = match body {
        ContactMessageBody::Text(text) => ("text", text.text, None),
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
            None,
        ),
        ContactMessageBody::File(file) => {
            // Ours, already on the blob server: like a received one (download on demand).
            let mut body = crate::app_media::file_body(&file);
            body["uploadState"] = json!(crate::app_media::STATE_LOCAL);
            (
                "file",
                body.to_string(),
                Some(crate::app_media::file_keys(&file)),
            )
        }
        // What this contact got of our picture from another device needn't go out from here too.
        // A picture only if that device's upload is ours as well: another blob on record would
        // send ours again with every switch between devices.
        ContactMessageBody::SetProfilePicture(picture) => {
            if store
                .picture_upload()?
                .is_some_and(|upload| upload.blob_id.0 == picture.blob_id.0)
            {
                store.picture_sent(&conversation, Some(&picture.blob_id))?;
            }
            return Ok(());
        }
        ContactMessageBody::DeleteProfilePicture(_) => {
            store.picture_sent(&conversation, None)?;
            return Ok(());
        }
        // Read receipts sent elsewhere: those messages are read here too.
        ContactMessageBody::DeliveryReceipt(receipt) => {
            if matches!(receipt.receipt_type, DeliveryReceiptType::Read) {
                let ids: Vec<u64> = receipt.message_ids.iter().map(|id| id.0).collect();
                let changed = store.mark_incoming_read(&conversation, &ids, created_at)?;
                seen_elsewhere(store, events, &conversation);
                push_changed(events, changed);
            }
            return Ok(());
        }
        ContactMessageBody::Reaction(reaction) => {
            let (emoji, apply) = match &reaction.action {
                ReactionAction::Apply(emoji) => (emoji, true),
                ReactionAction::Withdraw(emoji) => (emoji, false),
            };
            if let Some(id) = store.reactable_message(&conversation, reaction.message_id.0)? {
                // Our own reactions have an empty sender.
                let changed = store.set_reaction(id, "", emoji, apply, created_at)?;
                push_changed(events, changed.into_iter().collect());
            }
            return Ok(());
        }
        // A poll created or closed on another device: ours.
        ContactMessageBody::PollSetup(setup) => {
            let (added, changed) = crate::polls::reflect_setup(
                store,
                &conversation,
                user.as_str(),
                message.message_id,
                created_at,
                setup.poll_id,
                &setup.poll,
            )?;
            push_changed(events, added.into_iter().chain(changed).collect());
            return Ok(());
        }
        // Our vote, cast on another device.
        ContactMessageBody::PollVote(vote) => {
            let changed = crate::polls::receive_vote(
                store,
                &conversation,
                user.as_str(),
                vote.creator_identity.as_str(),
                vote.poll_id,
                &vote.choices,
            )?;
            push_changed(events, changed.into_iter().collect());
            return Ok(());
        }
        // A thumbs up/down (for contacts on old apps) from another device: our reaction,
        // replacing our earlier ones like a received legacy reaction does.
        ContactMessageBody::LegacyReaction(reaction) => {
            let emoji = crate::reactions::legacy_emoji(reaction.reaction_type);
            let mut changed = Vec::new();
            for message_id in &reaction.message_ids {
                if let Some(id) = store.reactable_message(&conversation, message_id.0)? {
                    changed.extend(store.replace_reactions(id, "", emoji, created_at)?);
                }
            }
            push_changed(events, changed);
            return Ok(());
        }
        ContactMessageBody::Edit(edit) => {
            if let Some(original) = store
                .message_by_id(&conversation, true, edit.message_id.0)?
                .filter(|original| crate::messages::edit_applies_to(&original.kind))
            {
                let changed = store.edit_message(original.id, &edit.text, Some(created_at))?;
                push_changed(events, changed.into_iter().collect());
            }
            return Ok(());
        }
        ContactMessageBody::Delete(delete) => {
            if let Some(original) = store
                .message_by_id(&conversation, true, delete.message_id.0)?
                .filter(|original| crate::messages::delete_applies_to(&original.kind))
            {
                let changed = store.mark_deleted(original.id, created_at)?;
                push_changed(events, changed.into_iter().collect());
            }
            return Ok(());
        }
        _ => {
            tracing::info!("reflected outgoing message of a kind not handled yet");
            return Ok(());
        }
    };
    if let Some(stored) = store.insert_reflected_outgoing(
        &conversation,
        message.message_id,
        kind,
        &content,
        created_at,
    )? {
        if let Some(keys) = file_keys {
            store.set_file_keys(stored.id, &keys)?;
        }
        events
            .borrow_mut()
            .push(CoreEvent::MessagesChanged(vec![stored]));
        // Answered on another device: the chat has been seen there.
        seen_elsewhere(store, events, &conversation);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn read_update(conversation: &str, message_id: u64) -> d2d::Envelope {
        #[expect(deprecated, reason = "Padding is still part of the envelope")]
        d2d::Envelope {
            padding: Vec::new(),
            device_id: 0,
            protocol_version: 0,
            content: Some(envelope::Content::IncomingMessageUpdate(
                d2d::IncomingMessageUpdate {
                    updates: vec![incoming_message_update::Update {
                        conversation: Some(ConversationId {
                            id: Some(conversation_id::Id::Contact(conversation.to_owned())),
                        }),
                        message_id,
                        update: Some(incoming_message_update::update::Update::Read(
                            incoming_message_update::Read { at: 5 },
                        )),
                    }],
                },
            )),
        }
    }

    fn clears(events: &EventSink, conversation: &str) -> bool {
        events.borrow().iter().any(|event| {
            matches!(event, CoreEvent::Call(json)
                if json["event"] == "conversationRead" && json["identity"] == conversation)
        })
    }

    fn reflected_outgoing(
        conversation: &str,
        message_id: u64,
        body: ContactMessageBody,
    ) -> d2d::Envelope {
        use libthreema::{
            csp_e2e::message::task::outgoing::encode_message_body,
            model::message::{OutgoingContactMessageBody, OutgoingMessageBody},
        };
        let (message_type, data) =
            encode_message_body(&OutgoingMessageBody::Contact(OutgoingContactMessageBody {
                receiver_identity: conversation.parse().unwrap(),
                body,
            }));
        #[expect(deprecated, reason = "Padding is still part of the envelope")]
        d2d::Envelope {
            padding: Vec::new(),
            device_id: 0,
            protocol_version: 0,
            content: Some(envelope::Content::OutgoingMessage(d2d::OutgoingMessage {
                conversation: Some(ConversationId {
                    id: Some(conversation_id::Id::Contact(conversation.to_owned())),
                }),
                message_id,
                thread_message_id: None,
                created_at: 10,
                r#type: i32::from(message_type),
                body: data,
                nonces: Vec::new(),
            })),
        }
    }

    #[test]
    fn polls_from_another_device_are_ours() {
        use libthreema::model::message::{PollSetupMessage, PollVoteMessage};
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        store
            .upsert_contact(&crate::contacts::test_contact("B0B0B0B0"))
            .unwrap();
        let user: ThreemaId = "ABCD1234".parse().unwrap();
        let poll = crate::polls::new_poll(
            0x77,
            "ABCD1234",
            "Lunch?",
            &["Pizza".into(), "Sushi".into()],
            false,
            true,
        )
        .unwrap();
        let events: EventSink = Rc::new(RefCell::new(Vec::new()));

        // Created on the Desktop: stored as our sent poll, registered.
        let setup = ContactMessageBody::PollSetup(PollSetupMessage {
            poll_id: 0x77,
            poll: crate::polls::setup_json(&poll),
        });
        apply(
            &store,
            user,
            reflected_outgoing("B0B0B0B0", 1, setup),
            &events,
        )
        .unwrap();
        let stored = store
            .poll_message("B0B0B0B0", "ABCD1234", 0x77)
            .unwrap()
            .unwrap();
        assert!(stored.outgoing);
        assert_eq!(stored.kind, "poll");

        // Our vote cast on the Desktop lands in it.
        let vote = ContactMessageBody::PollVote(PollVoteMessage {
            creator_identity: user,
            poll_id: 0x77,
            choices: crate::polls::vote_json(&poll, &[1]),
        });
        apply(
            &store,
            user,
            reflected_outgoing("B0B0B0B0", 2, vote),
            &events,
        )
        .unwrap();
        let voted: serde_json::Value =
            serde_json::from_str(&store.message(stored.id).unwrap().unwrap().body).unwrap();
        assert_eq!(voted["votes"]["ABCD1234"], serde_json::json!([1]));

        // Closed on the Desktop: the poll closes, the results come as a message of their own.
        let mut closed = voted.clone();
        closed["state"] = serde_json::json!(1);
        let close = ContactMessageBody::PollSetup(PollSetupMessage {
            poll_id: 0x77,
            poll: crate::polls::setup_json(&closed),
        });
        apply(
            &store,
            user,
            reflected_outgoing("B0B0B0B0", 3, close),
            &events,
        )
        .unwrap();
        let rows = store.conversation_messages("B0B0B0B0", 10).unwrap();
        assert!(
            rows.iter()
                .any(|m| m.kind == crate::polls::KIND_CLOSED && m.outgoing)
        );
    }

    /// An envelope with a text, encrypted for the device group.
    fn sealed(key: &libthreema::common::keys::DeviceGroupKey, id: u64) -> (Vec<u8>, d2d::Envelope) {
        let body =
            ContactMessageBody::Text(libthreema::model::message::TextMessage { text: "hi".into() });
        let envelope = reflected_outgoing("B0B0B0B0", id, body);
        let (_, sealed) = key.encrypt_envelope(&envelope.encode_to_vec()).unwrap();
        (sealed, envelope)
    }

    /// The store failing leaves nothing that would make the mediator's next copy a replay, and
    /// that copy is applied. From then on it is one.
    #[test]
    fn a_nonce_counts_only_once_its_envelope_is_applied() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let key = libthreema::common::keys::DeviceGroupKey::from([7; 32]);
        let user: ThreemaId = "ABCD1234".parse().unwrap();
        let events: EventSink = Rc::default();
        let (data, _) = sealed(&key, 5);

        store.conn.execute_batch("PRAGMA query_only = ON").unwrap();
        let opened = open(&store, &key, &data).unwrap();
        let error = apply_once(&store, user, opened, &events).unwrap_err();
        assert!(error.storage_code().is_some(), "{error}");
        assert!(events.borrow().is_empty());
        assert!(store.conn.is_autocommit());

        store.conn.execute_batch("PRAGMA query_only = OFF").unwrap();
        let opened = open(&store, &key, &data).unwrap();
        apply_once(&store, user, opened, &events).unwrap();
        assert!(store.message_by_id("B0B0B0B0", true, 5).unwrap().is_some());
        assert!(!events.borrow().is_empty());

        assert!(matches!(
            open(&store, &key, &data),
            Err(ReflectedError::Replay)
        ));
    }

    /// A reflected incoming message passes the provider interface, which only keeps text.
    #[test]
    fn a_store_failure_on_a_reflected_message_stays_one() {
        use libthreema::{
            csp_e2e::message::task::outgoing::encode_message_body,
            model::message::{OutgoingContactMessageBody, OutgoingMessageBody},
        };
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let mut contact = crate::contacts::test_contact("B0B0B0B0");
        contact.acquaintance_level =
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::Direct;
        store.upsert_contact(&contact).unwrap();
        let user: ThreemaId = "ABCD1234".parse().unwrap();
        let (message_type, data) =
            encode_message_body(&OutgoingMessageBody::Contact(OutgoingContactMessageBody {
                receiver_identity: user,
                body: ContactMessageBody::Text(libthreema::model::message::TextMessage {
                    text: "hi".into(),
                }),
            }));
        let envelope = d2d::Envelope {
            content: Some(envelope::Content::IncomingMessage(d2d::IncomingMessage {
                sender_identity: "B0B0B0B0".into(),
                message_id: 5,
                created_at: 1,
                r#type: i32::from(message_type),
                body: data,
                nonce: vec![1; 24],
            })),
            ..Default::default()
        };
        let events: EventSink = Rc::default();
        store.conn.execute_batch("PRAGMA query_only = ON").unwrap();
        let error = apply(&store, user, envelope, &events).unwrap_err();
        assert!(error.storage_code().is_some(), "{error}");
    }

    /// What can't be used is acknowledged and counts as seen.
    #[test]
    fn an_unusable_envelope_is_dropped_and_leaves_its_nonce() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let key = libthreema::common::keys::DeviceGroupKey::from([7; 32]);

        let (_, garbage) = key.encrypt_envelope(&[0xff; 8]).unwrap();
        let error = open(&store, &key, &garbage).map(|_| ()).unwrap_err();
        assert!(matches!(error, ReflectedError::Decode(_)), "{error}");
        assert!(error.storage_code().is_none());
        assert!(matches!(
            open(&store, &key, &garbage),
            Err(ReflectedError::Replay)
        ));
    }

    /// A caption edited on another device changes here too.
    #[test]
    fn a_caption_edited_elsewhere_is_edited_here() {
        use libthreema::model::message::EditMessage;
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let file = serde_json::json!({"size": 1, "caption": "old"}).to_string();
        let stored = store.insert_outgoing("B0B0B0B0", 5, "file", &file).unwrap();
        let events: EventSink = Rc::new(RefCell::new(Vec::new()));
        let edit = ContactMessageBody::Edit(EditMessage {
            message_id: libthreema::common::MessageId(5),
            text: "new".into(),
        });
        apply(
            &store,
            "ABCD1234".parse().unwrap(),
            reflected_outgoing("B0B0B0B0", 6, edit),
            &events,
        )
        .unwrap();
        let edited = store.message(stored.id).unwrap().unwrap();
        let body: serde_json::Value = serde_json::from_str(&edited.body).unwrap();
        assert_eq!(body["caption"], "new");
        assert!(edited.edited_at.is_some());
    }

    #[test]
    fn read_elsewhere_clears_notifications_even_when_already_read() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        store
            .upsert_contact(&crate::contacts::test_contact("B0B0B0B0"))
            .unwrap();
        store
            .insert_incoming("B0B0B0B0", 7, "text", "hi", 1)
            .unwrap();
        let user: ThreemaId = "ABCD1234".parse().unwrap();

        // First the read update while the message is still unread…
        let events: EventSink = Rc::new(RefCell::new(Vec::new()));
        apply(&store, user, read_update("B0B0B0B0", 7), &events).unwrap();
        assert!(clears(&events, "B0B0B0B0"));
        // …and again once it is read already (the other device's read receipt came first):
        // the notification must still go (user report 2026-10-28).
        let events: EventSink = Rc::new(RefCell::new(Vec::new()));
        apply(&store, user, read_update("B0B0B0B0", 7), &events).unwrap();
        assert!(clears(&events, "B0B0B0B0"));

        // Private chats stay unnamed to the UI while they are hidden.
        store.set_private("B0B0B0B0", true).unwrap();
        let events: EventSink = Rc::new(RefCell::new(Vec::new()));
        apply(&store, user, read_update("B0B0B0B0", 7), &events).unwrap();
        assert!(!clears(&events, "B0B0B0B0"));
    }

    /// A reflected incoming message, as it would come from the leader.
    fn reflected_incoming(
        sender: &str,
        message_id: u64,
        nonce: Vec<u8>,
        body: ContactMessageBody,
    ) -> d2d::Envelope {
        use libthreema::{
            csp_e2e::message::task::outgoing::encode_message_body,
            model::message::{OutgoingContactMessageBody, OutgoingMessageBody},
        };
        let (message_type, data) =
            encode_message_body(&OutgoingMessageBody::Contact(OutgoingContactMessageBody {
                receiver_identity: "ABCD1234".parse().unwrap(),
                body,
            }));
        d2d::Envelope {
            content: Some(envelope::Content::IncomingMessage(d2d::IncomingMessage {
                sender_identity: sender.into(),
                message_id,
                created_at: 1,
                r#type: i32::from(message_type),
                body: data,
                nonce,
            })),
            ..Default::default()
        }
    }

    fn text_body() -> ContactMessageBody {
        ContactMessageBody::Text(libthreema::model::message::TextMessage { text: "hi".into() })
    }

    /// Run an envelope through what a follower does with one.
    fn apply_sealed(store: &Rc<Store>, envelope: &d2d::Envelope, events: &EventSink) {
        let key = libthreema::common::keys::DeviceGroupKey::from([7; 32]);
        let (_, sealed) = key.encrypt_envelope(&envelope.encode_to_vec()).unwrap();
        let opened = open(store, &key, &sealed).unwrap();
        let _ = apply_once(store, "ABCD1234".parse().unwrap(), opened, events);
    }

    /// What another device received or sent while it led can't be replayed here once this device
    /// leads, whatever became of the envelope.
    #[test]
    fn the_nonces_a_reflection_tells_of_count_as_used() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        store
            .upsert_contact(&crate::contacts::test_contact("B0B0B0B0"))
            .unwrap();
        let events: EventSink = Rc::default();

        // Received elsewhere.
        let incoming = reflected_incoming("B0B0B0B0", 1, vec![1; 24], text_body());
        apply_sealed(&store, &incoming, &events);
        assert!(store.has_nonce(&[1; 24]).unwrap());

        // Sent elsewhere, to a group's members; something of a wrong length stays out.
        let mut outgoing = reflected_outgoing("B0B0B0B0", 2, text_body());
        if let Some(envelope::Content::OutgoingMessage(message)) = &mut outgoing.content {
            message.nonces = vec![vec![2; 24], vec![3; 24], vec![4; 5]];
        }
        apply_sealed(&store, &outgoing, &events);
        assert!(store.has_nonce(&[2; 24]).unwrap());
        assert!(store.has_nonce(&[3; 24]).unwrap());
        assert!(!store.has_nonce(&[4; 5]).unwrap());

        // From someone this device doesn't know.
        let stranger = reflected_incoming("C0C0C0C0", 3, vec![5; 24], text_body());
        apply_sealed(&store, &stranger, &events);
        assert!(store.has_nonce(&[5; 24]).unwrap());

        // What can't be used: the other device has received it all the same.
        let mut broken = reflected_incoming("B0B0B0B0", 4, vec![6; 24], text_body());
        if let Some(envelope::Content::IncomingMessage(message)) = &mut broken.content {
            message.r#type = 0xfe;
        }
        apply_sealed(&store, &broken, &events);
        assert!(store.has_nonce(&[6; 24]).unwrap());

        // Without replay protection there is nothing to keep.
        let typing = reflected_incoming("B0B0B0B0", 5, vec![7; 24], typing_body());
        apply_sealed(&store, &typing, &events);
        assert!(!store.has_nonce(&[7; 24]).unwrap());
    }

    /// One the store failed on comes again and leaves nothing, so that next copy isn't a replay.
    #[test]
    fn nothing_counts_as_used_when_the_store_failed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER refuse BEFORE INSERT ON d2d_nonces
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        let events: EventSink = Rc::default();
        let mut outgoing = reflected_outgoing("B0B0B0B0", 2, text_body());
        if let Some(envelope::Content::OutgoingMessage(message)) = &mut outgoing.content {
            message.nonces = vec![vec![2; 24]];
        }
        apply_sealed(&store, &outgoing, &events);
        assert!(!store.has_nonce(&[2; 24]).unwrap());
    }

    fn typing_body() -> ContactMessageBody {
        ContactMessageBody::TypingIndicator(libthreema::model::message::TypingIndicatorMessage {
            is_typing: true,
        })
    }

    /// Typing reaches this device from a leading one; of someone unknown it means nothing.
    #[test]
    fn typing_on_another_device_shows_here() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        store
            .upsert_contact(&crate::contacts::test_contact("B0B0B0B0"))
            .unwrap();
        let user: ThreemaId = "ABCD1234".parse().unwrap();
        let events: EventSink = Rc::default();
        let typing = reflected_incoming("B0B0B0B0", 1, Vec::new(), typing_body());
        apply(&store, user, typing, &events).unwrap();
        assert!(matches!(
            events.borrow().as_slice(),
            [CoreEvent::Typing { identity, typing: true }] if identity == "B0B0B0B0"
        ));
        assert!(
            store
                .conversation_messages("B0B0B0B0", 1)
                .unwrap()
                .is_empty()
        );

        let events: EventSink = Rc::default();
        let typing = reflected_incoming("C0C0C0C0", 2, Vec::new(), typing_body());
        apply(&store, user, typing, &events).unwrap();
        assert!(events.borrow().is_empty());
    }

    /// Someone deleted here writes while another device leads: back as a contact, as on the
    /// leader, and the message is there. What makes nobody a contact leaves them as they are.
    #[test]
    fn a_contact_deleted_here_who_writes_elsewhere_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let mut contact = crate::contacts::test_contact("B0B0B0B0");
        contact.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
        store.upsert_contact(&contact).unwrap();
        let user: ThreemaId = "ABCD1234".parse().unwrap();
        let events: EventSink = Rc::default();
        let level = || {
            store
                .contact("B0B0B0B0")
                .unwrap()
                .unwrap()
                .acquaintance_level
        };

        let typing = reflected_incoming("B0B0B0B0", 1, Vec::new(), typing_body());
        apply(&store, user, typing, &events).unwrap();
        assert_eq!(level(), AcquaintanceLevel::GroupOrDeleted);

        let text = reflected_incoming("B0B0B0B0", 2, vec![1; 24], text_body());
        apply(&store, user, text, &events).unwrap();
        assert_eq!(level(), AcquaintanceLevel::Direct);
        assert!(
            events
                .borrow()
                .iter()
                .any(|event| matches!(event, CoreEvent::ContactsChanged))
        );
        assert_eq!(store.conversation_messages("B0B0B0B0", 5).unwrap().len(), 1);
    }

    /// A store failure takes the raised level back with the message.
    #[test]
    fn the_level_is_raised_only_with_the_message() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let mut contact = crate::contacts::test_contact("B0B0B0B0");
        contact.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
        store.upsert_contact(&contact).unwrap();
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER refuse BEFORE INSERT ON d2d_nonces
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        let events: EventSink = Rc::default();
        let text = reflected_incoming("B0B0B0B0", 2, vec![1; 24], text_body());
        apply_sealed(&store, &text, &events);
        assert_eq!(
            store
                .contact("B0B0B0B0")
                .unwrap()
                .unwrap()
                .acquaintance_level,
            AcquaintanceLevel::GroupOrDeleted
        );
        assert!(events.borrow().is_empty());
    }

    /// A removal of our picture sent from another device needn't be repeated from here.
    #[test]
    fn a_picture_removal_from_another_device_is_not_repeated() {
        use libthreema::model::message::DeleteProfilePictureMessage;

        use crate::profile_pictures::{Distribution, Share};
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        store.set_picture_share(Share::Everyone).unwrap();
        assert!(matches!(
            store.picture_distribution("B0B0B0B0", false).unwrap(),
            Distribution::Delete
        ));
        let events: EventSink = Rc::default();
        let removal = ContactMessageBody::DeleteProfilePicture(DeleteProfilePictureMessage);
        apply(
            &store,
            "ABCD1234".parse().unwrap(),
            reflected_outgoing("B0B0B0B0", 1, removal),
            &events,
        )
        .unwrap();
        assert!(matches!(
            store.picture_distribution("B0B0B0B0", false).unwrap(),
            Distribution::None
        ));
    }

    /// A reflected message as the given group body.
    fn reflected_group_incoming(
        sender: &str,
        message_id: u64,
        group: libthreema::common::GroupIdentity,
        body: GroupMessageBody,
    ) -> d2d::Envelope {
        use libthreema::{
            csp_e2e::message::task::outgoing::encode_message_body,
            model::message::{OutgoingGroupMessageBody, OutgoingMessageBody},
        };
        let (message_type, data) =
            encode_message_body(&OutgoingMessageBody::Group(OutgoingGroupMessageBody {
                group_identity: group,
                body,
            }));
        d2d::Envelope {
            content: Some(envelope::Content::IncomingMessage(d2d::IncomingMessage {
                sender_identity: sender.into(),
                message_id,
                created_at: 1,
                r#type: i32::from(message_type),
                body: data,
                nonce: Vec::new(),
            })),
            ..Default::default()
        }
    }

    fn block(store: &Store, identity: &str) {
        store
            .set_setting(
                crate::store::keys::APP_SETTINGS,
                &json!({ "blockedContacts": [identity] }).to_string(),
            )
            .unwrap();
    }

    /// Someone blocked and deleted stays out of the contact list, whatever another device reflects
    /// (ADR 0019): a group-setup, which is exempt from blocking, or a text sent before the block
    /// reached the other device.
    #[test]
    fn a_blocked_contact_deleted_here_does_not_come_back() {
        use libthreema::model::message::GroupSetupMessage;
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let mut contact = crate::contacts::test_contact("B0B0B0B0");
        contact.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
        store.upsert_contact(&contact).unwrap();
        block(&store, "B0B0B0B0");
        let user: ThreemaId = "ABCD1234".parse().unwrap();
        let events: EventSink = Rc::default();
        let level = || {
            store
                .contact("B0B0B0B0")
                .unwrap()
                .unwrap()
                .acquaintance_level
        };
        let group = libthreema::common::GroupIdentity {
            group_id: 9,
            creator_identity: "B0B0B0B0".parse().unwrap(),
        };
        let setup = reflected_group_incoming(
            "B0B0B0B0",
            1,
            group,
            GroupMessageBody::Setup(GroupSetupMessage {
                members: vec![user],
            }),
        );
        apply(&store, user, setup, &events).unwrap();
        assert_eq!(level(), AcquaintanceLevel::GroupOrDeleted);

        let text = reflected_incoming("B0B0B0B0", 2, Vec::new(), text_body());
        apply(&store, user, text, &events).unwrap();
        assert_eq!(level(), AcquaintanceLevel::GroupOrDeleted);
        assert!(
            store
                .conversation_messages("B0B0B0B0", 5)
                .unwrap()
                .is_empty()
        );
        assert!(
            !events
                .borrow()
                .iter()
                .any(|event| matches!(event, CoreEvent::ContactsChanged))
        );
    }

    /// A reflected group-setup leaves an unblocked contact's level too: the group sync decides.
    #[test]
    fn a_reflected_group_setup_makes_nobody_a_contact() {
        use libthreema::model::message::GroupSetupMessage;
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let mut contact = crate::contacts::test_contact("B0B0B0B0");
        contact.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
        store.upsert_contact(&contact).unwrap();
        let user: ThreemaId = "ABCD1234".parse().unwrap();
        let group = libthreema::common::GroupIdentity {
            group_id: 9,
            creator_identity: "B0B0B0B0".parse().unwrap(),
        };
        let setup = reflected_group_incoming(
            "B0B0B0B0",
            1,
            group,
            GroupMessageBody::Setup(GroupSetupMessage {
                members: vec![user],
            }),
        );
        apply(&store, user, setup, &Rc::default()).unwrap();
        assert_eq!(
            store
                .contact("B0B0B0B0")
                .unwrap()
                .unwrap()
                .acquaintance_level,
            AcquaintanceLevel::GroupOrDeleted
        );
    }

    fn test_config() -> libthreema::common::config::Config {
        libthreema::common::config::Config::from(
            libthreema::common::config::ConfigEnvironment::Production,
        )
    }

    fn stranger_text(message_id: u64, nonce: Vec<u8>) -> d2d::Envelope {
        reflected_incoming("C0C0C0C0", message_id, nonce, text_body())
    }

    fn stranger() -> libthreema::model::contact::Contact {
        crate::contacts::test_contact("C0C0C0C0")
    }

    /// A message from someone this device doesn't know yet: acknowledged and kept, and there once
    /// they have a contact, however long the directory takes, as a direct one like on the leader.
    #[test]
    fn a_reflected_message_waits_for_its_sender_and_loses_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([9; 32]),
                server_group: "01".into(),
            })
            .unwrap();
        let events: EventSink = Rc::default();
        apply_sealed(&store, &stranger_text(1, vec![1; 24]), &events);
        assert!(store.has_nonce(&[1; 24]).unwrap());
        assert_eq!(store.reflected_waiting().unwrap().len(), 1);
        assert_eq!(store.owed_member_lookups().unwrap(), ["C0C0C0C0"]);
        assert!(events.borrow().iter().any(|event| matches!(
            event,
            CoreEvent::UnknownMembers(identities) if identities.as_slice() == ["C0C0C0C0"]
        )));

        // No contact yet: it stays, and a lookup no longer noted as owed is owed again.
        store.member_lookups_answered(&["C0C0C0C0"]).unwrap();
        apply_waiting(&store, &test_config(), &events).unwrap();
        assert_eq!(store.reflected_waiting().unwrap().len(), 1);
        assert_eq!(store.owed_member_lookups().unwrap(), ["C0C0C0C0"]);

        let mut contact = stranger();
        contact.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
        store.upsert_contact(&contact).unwrap();
        let events: EventSink = Rc::default();
        apply_waiting(&store, &test_config(), &events).unwrap();
        assert!(store.reflected_waiting().unwrap().is_empty());
        let stored = store.conversation_messages("C0C0C0C0", 5).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].body, "hi");
        assert_eq!(
            store
                .contact("C0C0C0C0")
                .unwrap()
                .unwrap()
                .acquaintance_level,
            AcquaintanceLevel::Direct
        );
        assert!(
            events
                .borrow()
                .iter()
                .any(|event| matches!(event, CoreEvent::MessageAdded { .. }))
        );
    }

    /// Nothing waits for an ID the directory says doesn't exist.
    #[test]
    fn a_reflected_message_from_an_invalid_id_goes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([9; 32]),
                server_group: "01".into(),
            })
            .unwrap();
        let events: EventSink = Rc::default();
        apply_sealed(&store, &stranger_text(1, vec![1; 24]), &events);
        store.note_invalid_ids(&["C0C0C0C0".to_owned()]);
        apply_waiting(&store, &test_config(), &events).unwrap();
        assert!(store.reflected_waiting().unwrap().is_empty());
    }

    /// A store failure while applying what waited leaves it waiting, with nothing announced.
    #[test]
    fn a_waiting_message_the_store_failed_on_waits_on() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([9; 32]),
                server_group: "01".into(),
            })
            .unwrap();
        apply_sealed(&store, &stranger_text(1, vec![1; 24]), &Rc::default());
        store.upsert_contact(&stranger()).unwrap();
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER refuse BEFORE DELETE ON reflected_waiting
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        let events: EventSink = Rc::default();
        apply_waiting(&store, &test_config(), &events).unwrap();
        assert_eq!(store.reflected_waiting().unwrap().len(), 1);
        assert!(events.borrow().is_empty());
        assert!(
            store
                .conversation_messages("C0C0C0C0", 5)
                .unwrap()
                .is_empty()
        );
    }

    /// A stranger on the block list doesn't become a contact by trying (ADR 0019): no lookup.
    #[test]
    fn a_blocked_stranger_is_not_looked_up() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        block(&store, "C0C0C0C0");
        let events: EventSink = Rc::default();
        apply_sealed(&store, &stranger_text(1, vec![1; 24]), &events);
        assert!(store.reflected_waiting().unwrap().is_empty());
        assert!(store.owed_member_lookups().unwrap().is_empty());
        assert!(events.borrow().is_empty());
        assert!(store.has_nonce(&[1; 24]).unwrap());
    }

    /// What is kept: what makes a contact and what is said in a group we have. Not typing, group
    /// control, or a group we don't have.
    #[test]
    fn only_what_needs_its_sender_waits() {
        use libthreema::model::message::{GroupLeaveMessage, TextMessage};
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let user: ThreemaId = "ABCD1234".parse().unwrap();
        let waiting = |envelope: d2d::Envelope| {
            let before = store.reflected_waiting().unwrap().len();
            apply(&store, user, envelope, &Rc::default()).unwrap();
            store.reflected_waiting().unwrap().len() > before
        };
        let identity = |group_id| libthreema::common::GroupIdentity {
            group_id,
            creator_identity: "B0B0B0B0".parse().unwrap(),
        };
        store
            .upsert_group(&crate::groups::Group {
                key: crate::groups::key(&identity(9)),
                identity: identity(9),
                name: "Lunch".into(),
                members: vec!["ABCD1234".into()],
                left: false,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        let text = || GroupMessageBody::Text(TextMessage { text: "hi".into() });
        assert!(!waiting(reflected_incoming(
            "C0C0C0C0",
            1,
            Vec::new(),
            typing_body()
        )));
        assert!(!waiting(reflected_group_incoming(
            "C0C0C0C0",
            2,
            identity(9),
            GroupMessageBody::Leave(GroupLeaveMessage),
        )));
        assert!(!waiting(reflected_group_incoming(
            "C0C0C0C0",
            3,
            identity(8),
            text()
        )));
        assert!(waiting(reflected_group_incoming(
            "C0C0C0C0",
            4,
            identity(9),
            text()
        )));
        assert!(waiting(stranger_text(5, Vec::new())));
    }

    /// Another device's upload doesn't replace what this contact has from ours on record, which
    /// would send it again; our own upload counts.
    #[test]
    fn only_our_own_upload_counts_as_sent_from_another_device() {
        use libthreema::{common::BlobId, model::message::SetProfilePictureMessage};

        use crate::profile_pictures::{Distribution, Share, Upload};
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        store.set_picture_share(Share::Everyone).unwrap();
        let ours = Upload {
            blob_id: BlobId([1; 16]),
            key: [2; 32],
            size: 1,
            uploaded_at: crate::messages::now_ms(),
        };
        store.set_picture_upload(Some(&ours)).unwrap();
        store.picture_sent("B0B0B0B0", Some(&ours.blob_id)).unwrap();
        let reflect = |to: &str, blob: [u8; 16]| {
            let set = ContactMessageBody::SetProfilePicture(SetProfilePictureMessage {
                blob_id: BlobId(blob),
                size: 1,
                key: [3; 32],
            });
            apply(
                &store,
                "ABCD1234".parse().unwrap(),
                reflected_outgoing(to, 1, set),
                &Rc::default(),
            )
            .unwrap();
        };
        let has_it = |to: &str| {
            matches!(
                store.picture_distribution(to, true).unwrap(),
                Distribution::None
            )
        };

        reflect("B0B0B0B0", [9; 16]);
        assert!(has_it("B0B0B0B0"));

        assert!(!has_it("C0C0C0C0"));
        reflect("C0C0C0C0", [1; 16]);
        assert!(has_it("C0C0C0C0"));
    }

    /// At the bound the oldest stay, the new one is counted for the user to see, and the mediator's
    /// queue goes on: its nonces are noted, and no lookup is owed for it.
    #[test]
    fn what_doesnt_fit_among_the_waiting_is_counted() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let step = store.step().unwrap();
        for _ in 0..REFLECTED_WAITING_MAX {
            store.keep_reflected("D0D0D0D0", b"kept").unwrap();
        }
        step.commit().unwrap();
        assert_eq!(store.reflected_left_out().unwrap(), 0);

        let events: EventSink = Rc::default();
        apply_sealed(&store, &stranger_text(1, vec![1; 24]), &events);
        assert_eq!(
            store.reflected_waiting_count().unwrap(),
            REFLECTED_WAITING_MAX
        );
        assert_eq!(store.reflected_left_out().unwrap(), 1);
        assert!(store.has_nonce(&[1; 24]).unwrap());
        assert!(store.owed_member_lookups().unwrap().is_empty());
        assert!(events.borrow().is_empty());

        // The count holds on; room again, and the next one waits.
        let (first, _, _) = store.reflected_waiting().unwrap().remove(0);
        store.reflected_done(first).unwrap();
        apply_sealed(&store, &stranger_text(2, vec![2; 24]), &events);
        assert_eq!(
            store.reflected_waiting_count().unwrap(),
            REFLECTED_WAITING_MAX
        );
        assert_eq!(store.reflected_left_out().unwrap(), 1);
    }

    /// A store failure doesn't count one: the envelope comes again.
    #[test]
    fn a_count_of_the_left_out_goes_back_with_its_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let step = store.step().unwrap();
        for _ in 0..REFLECTED_WAITING_MAX {
            store.keep_reflected("D0D0D0D0", b"kept").unwrap();
        }
        step.commit().unwrap();
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER refuse BEFORE INSERT ON d2d_nonces
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        apply_sealed(&store, &stranger_text(1, vec![1; 24]), &Rc::default());
        assert_eq!(store.reflected_left_out().unwrap(), 0);
    }

    fn with_identity() -> (tempfile::TempDir, Rc<Store>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([9; 32]),
                server_group: "01".into(),
            })
            .unwrap();
        (dir, store)
    }

    /// Blocked while it waited, and by now a direct contact, whom nothing else would stop: it goes,
    /// with no message, no notification and no lookup left (ADR 0019).
    #[test]
    fn what_waited_from_someone_blocked_since_goes() {
        let (_dir, store) = with_identity();
        apply_sealed(&store, &stranger_text(1, vec![1; 24]), &Rc::default());
        let mut contact = stranger();
        contact.acquaintance_level = AcquaintanceLevel::Direct;
        store.upsert_contact(&contact).unwrap();
        block(&store, "C0C0C0C0");
        let events: EventSink = Rc::default();
        apply_waiting(&store, &test_config(), &events).unwrap();
        assert!(store.reflected_waiting().unwrap().is_empty());
        assert!(
            store
                .conversation_messages("C0C0C0C0", 5)
                .unwrap()
                .is_empty()
        );
        assert!(store.owed_member_lookups().unwrap().is_empty());
        assert!(events.borrow().is_empty());
    }

    /// "Block unknown senders" switched on while a stranger's message waited: no new chat. A
    /// direct contact isn't a stranger.
    #[test]
    fn what_waited_from_a_stranger_goes_under_block_unknown() {
        let (_dir, store) = with_identity();
        apply_sealed(&store, &stranger_text(1, vec![1; 24]), &Rc::default());
        let mut contact = stranger();
        contact.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
        store.upsert_contact(&contact).unwrap();
        store
            .set_setting(
                crate::store::keys::APP_SETTINGS,
                &json!({ "blockUnknown": true }).to_string(),
            )
            .unwrap();
        apply_waiting(&store, &test_config(), &Rc::default()).unwrap();
        assert!(store.reflected_waiting().unwrap().is_empty());
        assert!(
            store
                .conversation_messages("C0C0C0C0", 5)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .contact("C0C0C0C0")
                .unwrap()
                .unwrap()
                .acquaintance_level,
            AcquaintanceLevel::GroupOrDeleted
        );

        contact.acquaintance_level = AcquaintanceLevel::Direct;
        store.upsert_contact(&contact).unwrap();
        apply_sealed(&store, &stranger_text(2, vec![2; 24]), &Rc::default());
        assert_eq!(store.conversation_messages("C0C0C0C0", 5).unwrap().len(), 1);
    }

    /// More from the same stranger: kept, with no further lookup to wait for.
    #[test]
    fn only_a_senders_first_waiting_message_asks_for_them() {
        let (_dir, store) = with_identity();
        let asked = |id| {
            let events: EventSink = Rc::default();
            apply_sealed(
                &store,
                &stranger_text(id, vec![u8::try_from(id).unwrap(); 24]),
                &events,
            );
            events
                .borrow()
                .iter()
                .any(|event| matches!(event, CoreEvent::UnknownMembers(_)))
        };
        assert!(asked(1));
        assert!(!asked(2));
        assert_eq!(store.reflected_waiting().unwrap().len(), 2);
        assert_eq!(store.owed_member_lookups().unwrap(), ["C0C0C0C0"]);
    }
}
