// SPDX-License-Identifier: AGPL-3.0-only
//! Forward security session storage for libthreema ([`FsSessionStore`]) in the encrypted store.
//! Sessions are secret key material: never part of Threema Safe or other backups.

use std::rc::Rc;

use libthreema::{
    common::ThreemaId,
    csp_e2e::fs::{
        FsSessionStore, FsStatus,
        ratchet::Ratchet,
        session::{Session, SessionId, SessionState, Versions},
    },
    model::provider::ProviderError,
};
use rusqlite::{OptionalExtension as _, Row, params};
use zeroize::Zeroize as _;

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

fn key32(row: &Row<'_>, column: usize) -> rusqlite::Result<Option<[u8; 32]>> {
    let Some(mut bytes) = row.get::<_, Option<Vec<u8>>>(column)? else {
        return Ok(None);
    };
    let key = <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| bad(column));
    bytes.zeroize();
    key.map(Some)
}

fn ratchet(row: &Row<'_>, key_column: usize) -> rusqlite::Result<Option<Ratchet>> {
    let Some(chain_key) = key32(row, key_column)? else {
        return Ok(None);
    };
    let counter = row.get::<_, i64>(key_column + 1)?.cast_unsigned();
    Ok(Some(Ratchet { counter, chain_key }))
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

impl Store {
    fn fs_sessions(&self, peer: &str, my_identity: ThreemaId) -> Result<Vec<Session>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM fs_sessions WHERE peer = ?1"
        ))?;
        let rows = statement.query_map([peer], |row| from_row(row, my_identity))?;
        Ok(rows.collect::<Result<_, _>>()?)
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
        self.store
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM fs_sessions WHERE peer = ?1 AND session_id = ?2"),
                params![peer.to_string(), &id.0[..]],
                |row| from_row(row, self.my_identity),
            )
            .optional()
            .map_err(storage)
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
        let ratchet = |r: &Option<Ratchet>| {
            r.as_ref()
                .map(|r| (r.chain_key.to_vec(), r.counter.cast_signed()))
                .unzip()
        };
        let (my_2dh_key, my_2dh_counter) = ratchet(&session.my_ratchet_2dh);
        let (my_4dh_key, my_4dh_counter) = ratchet(&session.my_ratchet_4dh);
        let (peer_2dh_key, peer_2dh_counter) = ratchet(&session.peer_ratchet_2dh);
        let (peer_4dh_key, peer_4dh_counter) = ratchet(&session.peer_ratchet_4dh);
        let result = self.store.conn.execute(
            &format!(
                "INSERT OR REPLACE INTO fs_sessions ({COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"
            ),
            params![
                session.peer_identity.to_string(),
                &session.id.0[..],
                session.my_ephemeral_private_key.map(|key| key.to_vec()),
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

    fn enqueue_control(&mut self, peer: ThreemaId, envelope: Vec<u8>) -> Result<(), ProviderError> {
        self.events.borrow_mut().push(CoreEvent::FsControl {
            peer: peer.to_string(),
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
                    encode_contact_message_body, encode_outgoing_contact_message_encapsulated,
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
