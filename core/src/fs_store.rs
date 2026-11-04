// SPDX-License-Identifier: AGPL-3.0-only
//! Forward security session storage for libthreema ([`FsSessionStore`]) in the encrypted store.
//! Sessions are secret key material: never part of Threema Safe or other backups.

use std::rc::Rc;

use libthreema::{
    common::{
        ThreemaId,
        keys::{ClientKey, PublicKey},
    },
    csp_e2e::fs::{
        Encapsulation, EncapsulationError, FsSessionStore, FsStatus, encapsulate,
        ratchet::Ratchet,
        session::{Session, SessionId, SessionState, Versions},
        terminate_all,
    },
    model::provider::ProviderError,
    protobuf::common::GroupIdentity,
};
use rusqlite::{OptionalExtension as _, Row, params};

use crate::{
    providers::{CoreEvent, EventSink},
    store::{Store, StoreError},
};

/// Schema (store migration 4).
pub(crate) const MIGRATION: &str = "
    CREATE TABLE fs_sessions (
        peer                 TEXT NOT NULL,
        session_id           BLOB NOT NULL,
        my_ephemeral_private BLOB,
        my_ephemeral_public  BLOB NOT NULL,
        version_local        INTEGER,
        version_remote       INTEGER,
        last_outgoing_at     INTEGER NOT NULL,
        my_2dh_key  BLOB, my_2dh_counter  INTEGER,
        my_4dh_key  BLOB, my_4dh_counter  INTEGER,
        peer_2dh_key BLOB, peer_2dh_counter INTEGER,
        peer_4dh_key BLOB, peer_4dh_counter INTEGER,
        PRIMARY KEY (peer, session_id)
    );";

const COLUMNS: &str = "peer, session_id, my_ephemeral_private, my_ephemeral_public, version_local,
    version_remote, last_outgoing_at, my_2dh_key, my_2dh_counter, my_4dh_key, my_4dh_counter,
    peer_2dh_key, peer_2dh_counter, peer_4dh_key, peer_4dh_counter";

fn bad(column: usize) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnType(column, "fs session".into(), rusqlite::types::Type::Blob)
}

/// Read in place: no heap copy of the key to wipe or to forget.
fn key32(row: &Row<'_>, column: usize) -> rusqlite::Result<Option<[u8; 32]>> {
    let Some(bytes) = row
        .get_ref(column)?
        .as_blob_or_null()
        .map_err(|_| bad(column))?
    else {
        return Ok(None);
    };
    <[u8; 32]>::try_from(bytes)
        .map(Some)
        .map_err(|_| bad(column))
}

fn ratchet(row: &Row<'_>, key_column: usize) -> rusqlite::Result<Option<Ratchet>> {
    let Some(chain_key) = key32(row, key_column)? else {
        return Ok(None);
    };
    let counter = row.get::<_, i64>(key_column + 1)?.cast_unsigned();
    Ok(Some(Ratchet { counter, chain_key }))
}

fn ratchet_columns(ratchet: Option<&Ratchet>) -> (Option<&[u8]>, Option<i64>) {
    ratchet
        .map(|ratchet| (&ratchet.chain_key[..], ratchet.counter.cast_signed()))
        .unzip()
}

fn from_row(row: &Row<'_>, my_identity: ThreemaId) -> rusqlite::Result<Session> {
    let peer: String = row.get(0)?;
    let id: Vec<u8> = row.get(1)?;
    let local: Option<u32> = row.get(4)?;
    let remote: Option<u32> = row.get(5)?;
    Ok(Session {
        id: SessionId::try_from(id.as_slice()).map_err(|_| bad(1))?,
        my_identity,
        peer_identity: peer.parse().map_err(|_| bad(0))?,
        my_ephemeral_private_key: key32(row, 2)?,
        my_ephemeral_public_key: key32(row, 3)?.ok_or_else(|| bad(3))?,
        versions: local
            .zip(remote)
            .map(|(local, remote)| Versions { local, remote }),
        last_outgoing_message_at: row.get::<_, i64>(6)?.cast_unsigned(),
        my_ratchet_2dh: ratchet(row, 7)?,
        my_ratchet_4dh: ratchet(row, 9)?,
        peer_ratchet_2dh: ratchet(row, 11)?,
        peer_ratchet_4dh: ratchet(row, 13)?,
    })
}

/// A row that can't be a session, as opposed to a database that can't be read. It is treated as
/// no session and removed, so that the peer's messages are rejected and ours start a new one: left
/// in place, it would stop the whole connection on every login, for everyone.
fn is_damaged(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::InvalidColumnType(..)
            | rusqlite::Error::IntegralValueOutOfRange(..)
            | rusqlite::Error::FromSqlConversionFailure(..)
    )
}

impl Store {
    fn fs_sessions(&self, peer: &str, my_identity: ThreemaId) -> Result<Vec<Session>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {COLUMNS}, rowid FROM fs_sessions WHERE peer = ?1"
        ))?;
        let rows = statement.query_map([peer], |row| {
            Ok((row.get::<_, i64>(15)?, from_row(row, my_identity)))
        })?;
        let mut sessions = Vec::new();
        let mut damaged = Vec::new();
        for row in rows {
            let (rowid, session) = row?;
            match session {
                Ok(session) => sessions.push(session),
                Err(error) if is_damaged(&error) => damaged.push(rowid),
                Err(error) => return Err(error.into()),
            }
        }
        for rowid in damaged {
            tracing::warn!("removing a damaged forward secrecy session");
            self.conn
                .execute("DELETE FROM fs_sessions WHERE rowid = ?1", [rowid])?;
        }
        Ok(sessions)
    }

    /// Everyone we have an FS session with.
    pub fn fs_peers(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT DISTINCT peer FROM fs_sessions ORDER BY peer")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Delete all FS sessions with `peer` (contact deleted).
    pub fn delete_fs_sessions(&self, peer: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM fs_sessions WHERE peer = ?1", [peer])?;
        Ok(())
    }
}

fn storage(error: impl std::fmt::Display) -> ProviderError {
    ProviderError::InvalidState(format!("storage: {error}"))
}

fn is_4dh(session: &Session) -> bool {
    matches!(session.state(), Ok(SessionState::R24 | SessionState::RL44))
}

/// libthreema's FS session store on top of our [`Store`]. Control messages and statuses are
/// passed on to the connection through the event sink.
pub struct StoreFs {
    pub store: Rc<Store>,
    pub my_identity: ThreemaId,
    pub events: EventSink,
}

impl FsSessionStore for StoreFs {
    fn get(&self, peer: ThreemaId, id: SessionId) -> Result<Option<Session>, ProviderError> {
        let session = self.store.conn.query_row(
            &format!("SELECT {COLUMNS} FROM fs_sessions WHERE peer = ?1 AND session_id = ?2"),
            params![peer.to_string(), &id.0[..]],
            |row| from_row(row, self.my_identity),
        );
        if session.as_ref().is_err_and(is_damaged) {
            tracing::warn!("removing a damaged forward secrecy session");
            self.store
                .conn
                .execute(
                    "DELETE FROM fs_sessions WHERE peer = ?1 AND session_id = ?2",
                    params![peer.to_string(), &id.0[..]],
                )
                .map_err(storage)?;
            return Ok(None);
        }
        session.optional().map_err(storage)
    }

    fn get_best(&self, peer: ThreemaId) -> Result<Option<Session>, ProviderError> {
        let mut sessions = self
            .store
            .fs_sessions(&peer.to_string(), self.my_identity)
            .map_err(storage)?;
        // Like Android's SQLDHSessionStore: 4DH sessions first, then the lowest session ID.
        sessions.sort_by_key(|session| (!is_4dh(session), session.id.0));
        Ok(sessions.into_iter().next())
    }

    fn store(&mut self, session: &Session) -> Result<(), ProviderError> {
        // Keys are bound by reference: a copy would stay in freed memory after they ratcheted on.
        let (my_2dh_key, my_2dh_counter) = ratchet_columns(session.my_ratchet_2dh.as_ref());
        let (my_4dh_key, my_4dh_counter) = ratchet_columns(session.my_ratchet_4dh.as_ref());
        let (peer_2dh_key, peer_2dh_counter) = ratchet_columns(session.peer_ratchet_2dh.as_ref());
        let (peer_4dh_key, peer_4dh_counter) = ratchet_columns(session.peer_ratchet_4dh.as_ref());
        let result = self.store.conn.execute(
            &format!(
                "INSERT OR REPLACE INTO fs_sessions ({COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"
            ),
            params![
                session.peer_identity.to_string(),
                &session.id.0[..],
                session
                    .my_ephemeral_private_key
                    .as_ref()
                    .map(|key| &key[..]),
                &session.my_ephemeral_public_key[..],
                session.versions.map(|v| v.local),
                session.versions.map(|v| v.remote),
                session.last_outgoing_message_at.cast_signed(),
                my_2dh_key,
                my_2dh_counter,
                my_4dh_key,
                my_4dh_counter,
                peer_2dh_key,
                peer_2dh_counter,
                peer_4dh_key,
                peer_4dh_counter,
            ],
        );
        result.map(drop).map_err(storage)
    }

    fn delete(&mut self, peer: ThreemaId, id: SessionId) -> Result<(), ProviderError> {
        self.store
            .conn
            .execute(
                "DELETE FROM fs_sessions WHERE peer = ?1 AND session_id = ?2",
                params![peer.to_string(), &id.0[..]],
            )
            .map(drop)
            .map_err(storage)
    }

    fn delete_all_except(
        &mut self,
        peer: ThreemaId,
        keep: Option<SessionId>,
        four_dh_only: bool,
    ) -> Result<usize, ProviderError> {
        let mut deleted = 0;
        for session in self
            .store
            .fs_sessions(&peer.to_string(), self.my_identity)
            .map_err(storage)?
        {
            if Some(session.id) == keep || (four_dh_only && !is_4dh(&session)) {
                continue;
            }
            self.delete(peer, session.id)?;
            deleted += 1;
        }
        Ok(deleted)
    }

    fn enqueue_control(
        &mut self,
        peer: ThreemaId,
        peer_public_key: Option<PublicKey>,
        envelope: Vec<u8>,
    ) -> Result<(), ProviderError> {
        self.events.borrow_mut().push(CoreEvent::FsControl {
            peer: peer.to_string(),
            peer_key: peer_public_key,
            envelope,
        });
        Ok(())
    }

    fn status(&mut self, peer: ThreemaId, status: FsStatus) -> Result<(), ProviderError> {
        self.events.borrow_mut().push(CoreEvent::FsStatus {
            peer: peer.to_string(),
            status,
        });
        Ok(())
    }
}

/// Two parties with our real stores exchange FS payloads through libthreema's incoming message
/// task (no network): exactly the bytes the connection would send.
/// [`fs::encapsulate`] for the connection: a session that can't be used (not one of the states a
/// session can be in) is ended, with a Terminate for the peer, and replaced by a new one, so that
/// one bad session costs a reset line and not this contact's messages. With `true` if that
/// happened. An error now is about this contact alone: it must not stop the connection.
///
/// # Errors
///
/// [`EncapsulationError`] if storing failed, or a new session failed too.
#[expect(clippy::too_many_arguments, reason = "what encapsulating takes")]
pub fn encapsulate_or_replace(
    fs: &mut StoreFs,
    client_key: &ClientKey,
    peer: ThreemaId,
    peer_public_key: &PublicKey,
    inner_type: u8,
    inner_data: &[u8],
    group_identity: Option<GroupIdentity>,
    now_ms: u64,
) -> Result<(Encapsulation, bool), EncapsulationError> {
    let my_identity = fs.my_identity;
    let attempt = |fs: &mut StoreFs| {
        encapsulate(
            fs,
            my_identity,
            client_key,
            peer,
            peer_public_key,
            inner_type,
            inner_data,
            group_identity.clone(),
            now_ms,
        )
    };
    match attempt(fs) {
        Ok(encapsulation) => Ok((encapsulation, false)),
        Err(EncapsulationError::Session(error)) => {
            tracing::warn!(%error, "replacing an unusable forward secrecy session");
            terminate_all(fs, peer)?;
            Ok((attempt(fs)?, true))
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use libthreema::{
        common::{
            ClientInfo, FeatureMask, MessageId, Nonce, ThreemaId,
            config::{Config, ConfigEnvironment, Flavor},
            keys::ClientKey,
        },
        csp::payload::MessageWithMetadataBox,
        csp_e2e::{
            CspE2eContextInit, CspE2eProtocol, CspE2eProtocolContextInit, fs,
            message::task::{
                incoming::IncomingMessageLoop,
                outgoing::{
                    encode_contact_message_body, encode_outgoing_contact_message,
                    encode_outgoing_contact_message_encapsulated,
                    encode_outgoing_fs_control_message,
                },
            },
        },
        model::{
            contact::ContactInit,
            message::{
                ContactMessageBody, MessageOverrides, OutgoingContactMessageBody, OutgoingMessage,
                OutgoingMessageBody, TextMessage,
            },
        },
        protobuf::d2d_sync::{ConversationCategory, ConversationVisibility, contact as pc},
    };

    use super::*;
    use crate::providers::{
        NoShortcuts, StoreContacts, StoreConversations, StoreNonces, StoreSettings,
    };

    struct Party {
        id: ThreemaId,
        key: [u8; 32],
        store: Rc<Store>,
        events: EventSink,
        e2e: CspE2eProtocol,
        _dir: tempfile::TempDir,
    }

    fn contact(id: ThreemaId, key: &ClientKey) -> ContactInit {
        ContactInit {
            identity: id,
            public_key: key.public_key(),
            created_at: 0,
            first_name: None,
            last_name: None,
            nickname: None,
            verification_level: pc::VerificationLevel::Unverified,
            work_verification_level: pc::WorkVerificationLevel::None,
            identity_type: pc::IdentityType::Regular,
            acquaintance_level: pc::AcquaintanceLevel::Direct,
            activity_state: pc::ActivityState::Active,
            feature_mask: FeatureMask(FeatureMask::FORWARD_SECURITY_SUPPORT),
            sync_state: pc::SyncState::Initial,
            work_last_full_sync_at: None,
            work_availability_status: None,
            read_receipt_policy_override: None,
            typing_indicator_policy_override: None,
            notification_trigger_policy_override: None,
            conversation_category: ConversationCategory::Default,
            conversation_visibility: ConversationVisibility::Normal,
        }
    }

    impl Party {
        fn new(id: &str, key: [u8; 32], peer: (&str, [u8; 32])) -> Self {
            Self::build(id, key, peer, false)
        }

        fn build(id: &str, key: [u8; 32], peer: (&str, [u8; 32]), fs_off: bool) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = Rc::new(Store::open(&dir.path().join("t.db"), &[4; 32]).unwrap());
            let id = ThreemaId::try_from(id).unwrap();
            let peer_id = ThreemaId::try_from(peer.0).unwrap();
            store
                .upsert_contact(&contact(peer_id, &ClientKey::from(peer.1)))
                .unwrap();
            let events: EventSink = Rc::new(RefCell::new(Vec::new()));
            let e2e = CspE2eProtocol::new(CspE2eProtocolContextInit {
                client_info: ClientInfo::Libthreema,
                config: std::sync::Arc::new(Config::from(ConfigEnvironment::Production)),
                csp_e2e: CspE2eContextInit {
                    user_identity: id,
                    client_key: ClientKey::from(key),
                    flavor: Flavor::Consumer,
                    nonce_storage: Box::new(RefCell::new(StoreNonces(Rc::clone(&store)))),
                    fs_sessions: Some(Box::new(RefCell::new(StoreFs {
                        store: Rc::clone(&store),
                        my_identity: id,
                        events: Rc::clone(&events),
                    }))),
                    fs_answering_only: fs_off,
                },
                d2x: None,
                shortcut: Box::new(NoShortcuts),
                settings: Box::new(RefCell::new(StoreSettings(Rc::clone(&store)))),
                contacts: Box::new(RefCell::new(StoreContacts {
                    store: Rc::clone(&store),
                    user_identity: id,
                    events: Rc::clone(&events),
                })),
                conversations: Box::new(RefCell::new(StoreConversations {
                    store: Rc::clone(&store),
                    events: Rc::clone(&events),
                    user: id,
                    from_sync: false,
                })),
            });
            Self {
                id,
                key,
                store,
                events,
                e2e,
                _dir: dir,
            }
        }

        fn fs(&self) -> StoreFs {
            StoreFs {
                store: Rc::clone(&self.store),
                my_identity: self.id,
                events: Rc::clone(&self.events),
            }
        }

        /// What the connection sends for a text: optional Init(s), then the message.
        fn send_text(&self, to: &Party, text: &str) -> Vec<MessageWithMetadataBox> {
            let body = ContactMessageBody::Text(TextMessage {
                text: text.to_owned(),
            });
            let (inner_type, inner_data) = encode_contact_message_body(to.id, &body);
            let key = ClientKey::from(self.key);
            let to_key = ClientKey::from(to.key).public_key();
            let encapsulation = fs::encapsulate(
                &mut self.fs(),
                self.id,
                &key,
                to.id,
                &to_key,
                inner_type,
                &inner_data,
                None,
                1,
            )
            .unwrap();
            let mut payloads: Vec<_> = encapsulation
                .prelude
                .iter()
                .map(|control| self.control(to, control))
                .collect();
            let message = OutgoingMessage {
                id: MessageId::random(),
                overrides: MessageOverrides::default(),
                created_at: 1_700_000_000_000,
                body: OutgoingMessageBody::Contact(OutgoingContactMessageBody {
                    receiver_identity: to.id,
                    body,
                }),
            };
            payloads.push(
                encode_outgoing_contact_message_encapsulated(
                    self.id,
                    &key,
                    Some("Nick"),
                    to.id,
                    &to_key,
                    &message,
                    encapsulation
                        .envelope
                        .as_ref()
                        .expect("text is encapsulated"),
                    Nonce::random(),
                )
                .unwrap(),
            );
            payloads
        }

        fn control(&self, to: &Party, envelope: &[u8]) -> MessageWithMetadataBox {
            encode_outgoing_fs_control_message(
                self.id,
                &ClientKey::from(self.key),
                to.id,
                &ClientKey::from(to.key).public_key(),
                envelope,
                1_700_000_000_000,
                Nonce::random(),
            )
            .unwrap()
        }

        /// Process incoming payloads like the connection; returns texts received and control
        /// messages to send back.
        fn receive(
            &mut self,
            from: &Party,
            payloads: Vec<MessageWithMetadataBox>,
        ) -> (Vec<String>, Vec<MessageWithMetadataBox>) {
            let mut texts = Vec::new();
            let mut replies = Vec::new();
            for payload in payloads {
                let mut task = self.e2e.handle_incoming_message(payload);
                let result = match task.poll(self.e2e.context()).unwrap() {
                    IncomingMessageLoop::Done(result) => result,
                    IncomingMessageLoop::Instruction(_) => {
                        panic!("no instruction expected (sender is a contact)")
                    }
                };
                assert!(
                    result.outgoing_message_ack.is_some(),
                    "every message is acknowledged"
                );
                for event in self.events.borrow_mut().drain(..).collect::<Vec<_>>() {
                    match event {
                        CoreEvent::MessageAdded { message, .. } => texts.push(message.body),
                        CoreEvent::FsControl { envelope, .. } => {
                            replies.push(self.control(from, &envelope))
                        }
                        _ => {}
                    }
                }
            }
            (texts, replies)
        }
    }

    /// Alice starts a session with Bob: what she sends, ready for Bob's `receive`.
    fn first_message(alice: &Party, bob: &Party) -> Vec<MessageWithMetadataBox> {
        alice.send_text(bob, "hello")
    }

    fn session_count(party: &Party, peer: &Party) -> usize {
        party
            .store
            .fs_sessions(&peer.id.to_string(), party.id)
            .unwrap()
            .len()
    }

    fn block(party: &Party, peer: &Party) {
        party
            .store
            .set_setting(
                crate::store::keys::APP_SETTINGS,
                &serde_json::json!({ "blockedContacts": [peer.id.to_string()] }).to_string(),
            )
            .unwrap();
    }

    fn make_group_member(party: &Party, peer: &Party) {
        let identity = libthreema::common::GroupIdentity {
            creator_identity: peer.id,
            group_id: 1,
        };
        party
            .store
            .upsert_group(&crate::groups::Group {
                key: crate::groups::key(&identity),
                identity,
                name: String::new(),
                members: vec![peer.id.to_string()],
                left: false,
                created_at: 0,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
    }

    fn pair() -> (Party, Party) {
        let (alice_key, bob_key) = ([1; 32], [2; 32]);
        (
            Party::new("ABCD1234", alice_key, ("EFGH5678", bob_key)),
            Party::new("EFGH5678", bob_key, ("ABCD1234", alice_key)),
        )
    }

    /// Nothing but the acknowledgement: no session, no control message, no status event.
    fn assert_no_trace(receiver: &mut Party, sender: &Party) {
        let payloads = first_message(sender, receiver);
        for payload in payloads {
            let mut task = receiver.e2e.handle_incoming_message(payload);
            let IncomingMessageLoop::Done(result) = task.poll(receiver.e2e.context()).unwrap()
            else {
                panic!("no instruction expected");
            };
            assert!(result.outgoing_message_ack.is_some());
        }
        assert_eq!(session_count(receiver, sender), 0);
        assert!(receiver.events.borrow().is_empty());
    }

    #[test]
    fn a_blocked_contact_gets_no_session_and_no_answer() {
        let (alice, mut bob) = pair();
        block(&bob, &alice);
        assert_no_trace(&mut bob, &alice);
    }

    #[test]
    fn an_unknown_sender_keeps_a_session_with_block_unknown_on_but_gets_no_text_through() {
        let (alice, mut bob) = pair();
        let mut known = bob.store.contact("ABCD1234").unwrap().unwrap();
        known.acquaintance_level = pc::AcquaintanceLevel::GroupOrDeleted;
        bob.store.upsert_contact(&known).unwrap();
        bob.store
            .set_setting(
                crate::store::keys::APP_SETTINGS,
                &serde_json::json!({ "blockUnknown": true }).to_string(),
            )
            .unwrap();
        // Their group-setup is exempt and may be encapsulated, so the session has to exist.
        let (texts, _) = bob.receive(&alice, first_message(&alice, &bob));
        assert_eq!(session_count(&bob, &alice), 1);
        assert!(texts.is_empty());
    }

    #[test]
    fn a_blocked_group_member_keeps_a_session_for_the_exempt_messages() {
        let (alice, mut bob) = pair();
        block(&bob, &alice);
        make_group_member(&bob, &alice);
        let (_, replies) = bob.receive(&alice, first_message(&alice, &bob));
        assert_eq!(session_count(&bob, &alice), 1);
        assert_eq!(replies.len(), 1, "the Accept");
    }

    /// Both sides in RL44, the events drained.
    fn established() -> (Party, Party) {
        let (mut alice, mut bob) = pair();
        let (_, accept) = bob.receive(&alice, first_message(&alice, &bob));
        let _ = alice.receive(&bob, accept);
        let _ = bob.receive(&alice, alice.send_text(&bob, "4DH"));
        let _ = alice.receive(&bob, bob.send_text(&alice, "4DH too"));
        (alice, bob)
    }

    /// What the connection sends with forward secrecy off or for a contact without it.
    fn plain_text(from: &Party, to: &Party) -> MessageWithMetadataBox {
        let message = OutgoingMessage {
            id: MessageId::random(),
            overrides: MessageOverrides::default(),
            created_at: 1_700_000_000_000,
            body: OutgoingMessageBody::Contact(OutgoingContactMessageBody {
                receiver_identity: to.id,
                body: ContactMessageBody::Text(TextMessage {
                    text: "plain".to_owned(),
                }),
            }),
        };
        encode_outgoing_contact_message(
            from.id,
            &ClientKey::from(from.key),
            None,
            to.id,
            &ClientKey::from(to.key).public_key(),
            &message,
            Nonce::random(),
        )
        .unwrap()
    }

    /// The statuses a payload leaves behind, passed through the incoming task.
    fn statuses(receiver: &mut Party, payload: MessageWithMetadataBox) -> Vec<FsStatus> {
        let mut task = receiver.e2e.handle_incoming_message(payload);
        let IncomingMessageLoop::Done(_) = task.poll(receiver.e2e.context()).unwrap() else {
            panic!("no instruction expected");
        };
        receiver
            .events
            .borrow_mut()
            .drain(..)
            .filter_map(|event| match event {
                CoreEvent::FsStatus { status, .. } => Some(status),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_plain_message_on_a_live_session_is_reported_and_still_arrives() {
        let (alice, mut bob) = established();
        let texts = |bob: &Party| -> i64 {
            bob.store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE body = 'plain'",
                    [],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(texts(&bob), 0);
        let plain = plain_text(&alice, &bob);
        let found = statuses(&mut bob, plain);
        assert_eq!(found, vec![FsStatus::MessageWithoutFs]);
        assert_eq!(texts(&bob), 1);

        // Its receipts and typing add no rows, so these don't add a line per receipt.
        let chat = alice.id.to_string();
        assert!(
            !bob.store
                .newest_is_status(&chat, "fsMessageWithoutFs")
                .unwrap()
        );
        bob.store
            .insert_status(&chat, "fsMessageWithoutFs")
            .unwrap();
        assert!(
            bob.store
                .newest_is_status(&chat, "fsMessageWithoutFs")
                .unwrap()
        );
        assert!(!bob.store.newest_is_status(&chat, "fsReset").unwrap());
    }

    #[test]
    fn a_plain_message_is_no_news_without_a_session() {
        let (alice, mut bob) = pair();
        let plain = plain_text(&alice, &bob);
        assert_eq!(statuses(&mut bob, plain), Vec::new());
    }

    #[test]
    fn a_blocked_senders_plain_message_is_not_reported() {
        let (alice, mut bob) = established();
        block(&bob, &alice);
        let plain = plain_text(&alice, &bob);
        assert_eq!(statuses(&mut bob, plain), Vec::new());
    }

    #[test]
    fn with_forward_security_off_a_message_is_rejected_so_that_it_can_be_sent_again() {
        let (alice_key, bob_key) = ([1; 32], [2; 32]);
        let mut alice = Party::new("ABCD1234", alice_key, ("EFGH5678", bob_key));
        let mut bob = Party::build("EFGH5678", bob_key, ("ABCD1234", alice_key), true);

        // A contact with our old feature mask: Init and message, neither of them readable here.
        let (texts, replies) = bob.receive(&alice, first_message(&alice, &bob));
        assert!(texts.is_empty());
        assert_eq!(replies.len(), 1, "only the message gets an answer");
        assert_eq!(session_count(&bob, &alice), 0);

        // It ends her session and tells her connection that the message was rejected.
        assert_eq!(session_count(&alice, &bob), 1);
        let reject = replies.into_iter().next().unwrap();
        let found = statuses(&mut alice, reject);
        assert!(matches!(
            found.as_slice(),
            [FsStatus::MessageRejected {
                session_known: true,
                ..
            }]
        ));
        assert_eq!(session_count(&alice, &bob), 0);

        // Sent again without forward secrecy, it arrives.
        let plain = plain_text(&alice, &bob);
        let (texts, _) = bob.receive(&alice, vec![plain]);
        assert_eq!(texts, vec!["plain".to_owned()]);
    }

    #[test]
    fn session_keys_survive_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Rc::new(Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap());
        let me: ThreemaId = "ABCD1234".parse().unwrap();
        let peer: ThreemaId = "EFGH5678".parse().unwrap();
        let mut fs = StoreFs {
            store: Rc::clone(&store),
            my_identity: me,
            events: EventSink::default(),
        };
        let id = SessionId([7; 16]);
        let session = Session {
            id,
            my_identity: me,
            peer_identity: peer,
            my_ephemeral_private_key: Some([1; 32]),
            my_ephemeral_public_key: [2; 32],
            versions: None,
            last_outgoing_message_at: 5,
            my_ratchet_2dh: Some(Ratchet {
                counter: 3,
                chain_key: [4; 32],
            }),
            my_ratchet_4dh: None,
            peer_ratchet_2dh: None,
            peer_ratchet_4dh: Some(Ratchet {
                counter: 6,
                chain_key: [8; 32],
            }),
        };
        fs.store(&session).unwrap();
        let back = fs.get(peer, id).unwrap().unwrap();
        assert_eq!(back.my_ephemeral_private_key, Some([1; 32]));
        assert_eq!(back.my_ephemeral_public_key, [2; 32]);
        assert_eq!(back.my_ratchet_2dh.as_ref().unwrap().chain_key, [4; 32]);
        assert_eq!(back.my_ratchet_2dh.as_ref().unwrap().counter, 3);
        assert!(back.my_ratchet_4dh.is_none());
        assert_eq!(back.peer_ratchet_4dh.as_ref().unwrap().chain_key, [8; 32]);
        // A key of the wrong size is a damaged session, not a short key.
        store
            .conn
            .execute("UPDATE fs_sessions SET my_2dh_key = x'0404'", [])
            .unwrap();
        assert!(fs.get(peer, id).unwrap().is_none());
    }

    #[test]
    fn a_damaged_session_counts_as_none_and_goes_away() {
        let (alice, bob) = established();
        let chat = bob.id.to_string();
        assert_eq!(session_count(&alice, &bob), 1);
        let good = alice.fs().get_best(bob.id).unwrap().unwrap();
        let store = |id: &[u8; 16]| {
            let mut copy = good.clone();
            copy.id = SessionId(*id);
            alice.fs().store(&copy).unwrap();
        };
        store(&[9; 16]);
        store(&[10; 16]);
        assert_eq!(session_count(&alice, &bob), 3);
        let damage = |id: &[u8]| {
            alice
                .store
                .conn
                .execute(
                    "UPDATE fs_sessions SET my_ephemeral_public = x'01' WHERE session_id = ?1",
                    [id],
                )
                .unwrap();
        };

        // Looking one up by its ID.
        damage(&[9; 16]);
        assert!(
            alice
                .fs()
                .get(bob.id, SessionId([9; 16]))
                .unwrap()
                .is_none()
        );
        // Looking for the best: what is good is still there.
        damage(&[10; 16]);
        let best = alice.fs().get_best(bob.id).unwrap().unwrap();
        assert_eq!(best.id, good.id);
        let left: i64 = alice
            .store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM fs_sessions WHERE peer = ?1",
                [&chat],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(left, 1);
    }

    #[test]
    fn an_unusable_session_is_replaced_and_the_peer_told() {
        let (alice, bob) = pair();
        let peer_key = ClientKey::from(bob.key).public_key();
        let key = ClientKey::from(alice.key);
        let send = || {
            encapsulate_or_replace(
                &mut alice.fs(),
                &key,
                bob.id,
                &peer_key,
                0x01,
                b"hello",
                None,
                1,
            )
            .unwrap()
        };

        // The first message starts a session, nothing to replace.
        let (first, replaced) = send();
        assert!(!replaced);
        assert_eq!(first.prelude.len(), 1);
        let (second, replaced) = send();
        assert!(!replaced);
        assert!(second.prelude.is_empty(), "the session is reused");
        assert!(alice.events.borrow().is_empty());

        // No ratchets at all: not a state a session can be in.
        let old = alice.fs().get_best(bob.id).unwrap().unwrap();
        alice
            .store
            .conn
            .execute(
                "UPDATE fs_sessions SET my_2dh_key = NULL, my_2dh_counter = NULL,
                     my_4dh_key = NULL, my_4dh_counter = NULL,
                     peer_2dh_key = NULL, peer_2dh_counter = NULL,
                     peer_4dh_key = NULL, peer_4dh_counter = NULL",
                [],
            )
            .unwrap();
        let (third, replaced) = send();
        assert!(replaced);
        assert_eq!(third.prelude.len(), 1, "the new session's Init");
        assert!(third.envelope.is_some(), "and the message in it");
        let new = alice.fs().get_best(bob.id).unwrap().unwrap();
        assert_ne!(new.id, old.id);
        assert_eq!(session_count(&alice, &bob), 1);
        let terminates = alice
            .events
            .borrow()
            .iter()
            .filter(|event| matches!(event, CoreEvent::FsControl { .. }))
            .count();
        assert_eq!(terminates, 1, "the old session's Terminate");
    }

    #[test]
    fn conversation_through_libthreema_incoming_task() {
        let alice_key = [0x11; 32];
        let bob_key = [0x22; 32];
        let mut alice = Party::new("000ALICE", alice_key, ("00000BOB", bob_key));
        let mut bob = Party::new("00000BOB", bob_key, ("000ALICE", alice_key));

        // Alice: Init + 2DH message; Bob decrypts and answers with an Accept.
        let first = alice.send_text(&bob, "hello over 2DH");
        assert_eq!(first.len(), 2);
        let (texts, accept) = bob.receive(&alice, first);
        assert_eq!(texts, vec!["hello over 2DH".to_owned()]);
        assert_eq!(accept.len(), 1);

        // A second 2DH message (Accept still in flight): the ratchet commit on ack must have
        // moved past the first message.
        let (texts, _) = bob.receive(&alice, alice.send_text(&bob, "second 2DH"));
        assert_eq!(texts, vec!["second 2DH".to_owned()]);

        // Alice processes the Accept, then both talk 4DH.
        let (_, nothing) = alice.receive(&bob, accept);
        assert!(nothing.is_empty());
        let (texts, _) = bob.receive(&alice, alice.send_text(&bob, "now 4DH"));
        assert_eq!(texts, vec!["now 4DH".to_owned()]);
        let reply = bob.send_text(&alice, "reply 4DH");
        assert_eq!(reply.len(), 1, "Bob reuses the session, no Init");
        let (texts, _) = alice.receive(&bob, reply);
        assert_eq!(texts, vec!["reply 4DH".to_owned()]);

        // Sessions survive in the store with the right states.
        let alice_session = alice.fs().get_best(bob.id).unwrap().unwrap();
        let bob_session = bob.fs().get_best(alice.id).unwrap().unwrap();
        assert_eq!(alice_session.state(), Ok(SessionState::RL44));
        assert_eq!(bob_session.state(), Ok(SessionState::RL44));
        assert_eq!(alice_session.id, bob_session.id);
    }
}
