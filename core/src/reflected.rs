// SPDX-License-Identifier: AGPL-3.0-only
//! Reflected D2D envelopes (multi-device, ADR 0011): what the other devices of the group did,
//! applied to our store.
//!
//! Upstream: Android's `IncomingReflectedMessageTask` with `reflectedoutgoingmessage/*`,
//! `reflectedmessageupdate/*` and the incoming message processor. Handled so far: outgoing
//! messages (text, location, file, edit, delete, reaction, read receipts), incoming messages
//! (when another device was leader), the sent/read updates, and contact, group, profile and
//! settings sync (`sync.rs`). Distribution lists are not synced (Android doesn't either).

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
    protobuf::d2d::{
        self, ConversationId, conversation_id, envelope, incoming_message_update,
        outgoing_message_update,
    },
};
use prost::Message as _;
use serde_json::json;

use crate::{
    messages::{Message, OutgoingState},
    providers::{CoreEvent, EventSink, StoreConversations},
    store::{Store, StoreError},
};

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
    /// SQLite's code if the store failed and this device still needs the envelope: then the
    /// mediator is to send it again, as no other route brings it here. A broken rule of ours
    /// (`SQLITE_CONSTRAINT`) would fail the same way every time, and hold up all that follows.
    #[must_use]
    pub fn storage_code(&self) -> Option<i32> {
        const SQLITE_CONSTRAINT: i32 = 19;
        let code = match self {
            Self::Store(error) => error.sqlite_code(),
            Self::StoreCode(code) => *code,
            Self::Decrypt(_) | Self::Replay | Self::Decode(_) => return None,
        };
        (code & 0xff != SQLITE_CONSTRAINT).then_some(code)
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

/// Apply an opened envelope and note its nonce, in one store step. An envelope this can't use
/// leaves nothing behind but the nonce, and one the store failed on not even that. Events only for
/// what holds.
pub fn apply_once(
    store: &Rc<Store>,
    user: ThreemaId,
    opened: Opened,
    events: &EventSink,
) -> Result<(), ReflectedError> {
    let announced = events.borrow().len();
    let Opened { envelope, nonce } = opened;
    let step = store.step()?;
    let applied = apply(store, user, envelope, events).and_then(|()| {
        store.add_d2d_nonce(&nonce)?;
        Ok(step.commit()?)
    });
    if let Err(error) = &applied {
        events.borrow_mut().truncate(announced);
        if error.storage_code().is_none() {
            store.add_d2d_nonce(&nonce)?;
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
            if store.contact(sender.as_str())?.is_none() {
                // The contact comes with a contact sync, which isn't applied yet.
                tracing::warn!("reflected incoming message from an unknown contact skipped");
                return Ok(());
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
        Some(envelope::Content::SettingsSync(sync)) => Ok(crate::sync::settings_sync(store, sync)?),
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

    /// What can't be used is acknowledged and counts as seen; a broken rule of ours is that too,
    /// or it would come back at every login and hold up what follows.
    #[test]
    fn an_unusable_envelope_is_dropped_and_leaves_its_nonce() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let key = libthreema::common::keys::DeviceGroupKey::from([7; 32]);
        let user: ThreemaId = "ABCD1234".parse().unwrap();
        let events: EventSink = Rc::default();

        let (_, garbage) = key.encrypt_envelope(&[0xff; 8]).unwrap();
        let error = open(&store, &key, &garbage).map(|_| ()).unwrap_err();
        assert!(matches!(error, ReflectedError::Decode(_)), "{error}");
        assert!(error.storage_code().is_none());
        assert!(matches!(
            open(&store, &key, &garbage),
            Err(ReflectedError::Replay)
        ));

        store
            .conn
            .execute_batch(
                "CREATE TRIGGER refuse BEFORE INSERT ON messages
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        let (data, _) = sealed(&key, 5);
        let opened = open(&store, &key, &data).unwrap();
        let error = apply_once(&store, user, opened, &events).unwrap_err();
        assert!(error.storage_code().is_none(), "{error}");
        assert!(matches!(
            open(&store, &key, &data),
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
}
