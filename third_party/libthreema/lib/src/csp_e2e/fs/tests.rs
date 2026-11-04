use std::{cell::RefCell, collections::HashMap, rc::Rc};

use data_encoding::HEXLOWER;

use super::*;

#[derive(Default)]
struct MemoryData {
    sessions: HashMap<(ThreemaId, [u8; 16]), Session>,
    controls: Vec<(ThreemaId, Vec<u8>)>,
    statuses: Vec<(ThreemaId, FsStatus)>,
}

/// Store handle; the test keeps a second handle to inspect the data.
#[derive(Clone, Default)]
struct MemoryStore(Rc<RefCell<MemoryData>>);

impl FsSessionStore for MemoryStore {
    fn get(&self, peer: ThreemaId, id: SessionId) -> Result<Option<Session>, ProviderError> {
        Ok(self.0.borrow().sessions.get(&(peer, id.0)).cloned())
    }

    fn get_best(&self, peer: ThreemaId) -> Result<Option<Session>, ProviderError> {
        let data = self.0.borrow();
        let mut candidates: Vec<&Session> = data
            .sessions
            .values()
            .filter(|session| session.peer_identity == peer)
            .collect();
        candidates.sort_by_key(|session| {
            (
                !matches!(session.state(), Ok(SessionState::RL44 | SessionState::R24)),
                session.id.0,
            )
        });
        Ok(candidates.first().map(|session| (*session).clone()))
    }

    fn store(&mut self, session: &Session) -> Result<(), ProviderError> {
        let _ = self
            .0
            .borrow_mut()
            .sessions
            .insert((session.peer_identity, session.id.0), session.clone());
        Ok(())
    }

    fn delete(&mut self, peer: ThreemaId, id: SessionId) -> Result<(), ProviderError> {
        let _ = self.0.borrow_mut().sessions.remove(&(peer, id.0));
        Ok(())
    }

    fn delete_all_except(
        &mut self,
        peer: ThreemaId,
        keep: Option<SessionId>,
        four_dh_only: bool,
    ) -> Result<usize, ProviderError> {
        let mut data = self.0.borrow_mut();
        let before = data.sessions.len();
        data.sessions.retain(|(session_peer, id), session| {
            *session_peer != peer
                || Some(SessionId(*id)) == keep
                || (four_dh_only && !matches!(session.state(), Ok(SessionState::RL44 | SessionState::R24)))
        });
        Ok(before.saturating_sub(data.sessions.len()))
    }

    fn enqueue_control(&mut self, peer: ThreemaId, envelope: Vec<u8>) -> Result<(), ProviderError> {
        self.0.borrow_mut().controls.push((peer, envelope));
        Ok(())
    }

    fn status(&mut self, peer: ThreemaId, status: FsStatus) -> Result<(), ProviderError> {
        self.0.borrow_mut().statuses.push((peer, status));
        Ok(())
    }
}

struct Party {
    id: ThreemaId,
    key: ClientKey,
    fs: FsContext,
    data: MemoryStore,
}

impl Party {
    fn new(id: &str, hex: &str) -> Self {
        let bytes: [u8; 32] = HEXLOWER.decode(hex.as_bytes()).unwrap().try_into().unwrap();
        let data = MemoryStore::default();
        Self {
            id: ThreemaId::try_from(id).unwrap(),
            key: ClientKey::from(bytes),
            fs: FsContext::new(Box::new(RefCell::new(data.clone()))),
            data,
        }
    }

    fn with_store<R>(&self, f: impl FnOnce(&mut MemoryData) -> R) -> R {
        f(&mut self.data.0.borrow_mut())
    }

    fn send(&mut self, to: &Party, inner_type: u8, data: &[u8]) -> Encapsulation {
        self.send_in_group(to, inner_type, data, None)
    }

    fn send_in_group(
        &mut self,
        to: &Party,
        inner_type: u8,
        data: &[u8],
        group_identity: Option<GroupIdentity>,
    ) -> Encapsulation {
        let mut store = self.fs.store.borrow_mut();
        encapsulate(
            &mut *store,
            self.id,
            &self.key,
            to.id,
            &to.key.public_key(),
            inner_type,
            data,
            group_identity,
            1,
        )
        .unwrap()
    }

    fn receive(&mut self, from: &Party, envelope: &[u8], message_id: u64) -> IncomingFsOutcome {
        let outcome = process_incoming(
            &mut self.fs,
            self.id,
            &self.key,
            from.id,
            &from.key.public_key(),
            message_id,
            envelope,
        )
        .unwrap();
        // Acknowledge.
        commit_peer_ratchet(&mut self.fs).unwrap();
        outcome
    }

    fn take_controls(&self) -> Vec<Vec<u8>> {
        self.with_store(|store| store.controls.drain(..).map(|(_, envelope)| envelope).collect())
    }
}

fn alice() -> Party {
    Party::new(
        "000ALICE",
        "6eda2ebb8527ff5bd0e8719602f710c13e162a3be612de0ad2a2ff66f5050630",
    )
}
fn bob() -> Party {
    Party::new(
        "00000BOB",
        "533058227925006d86bb8dd88b0442ed73fbc49216b6e94b0870a7761d979eca",
    )
}

fn inner(outcome: IncomingFsOutcome) -> (u8, Vec<u8>) {
    match outcome {
        IncomingFsOutcome::Inner { inner_type, data } => (inner_type, data),
        IncomingFsOutcome::Handled => panic!("expected an inner message"),
    }
}

#[test]
fn full_conversation() {
    let mut alice = alice();
    let mut bob = bob();

    // Alice's first message: Init + 2DH encapsulated text.
    let first = alice.send(&bob, 0x01, b"hello bob");
    assert_eq!(first.prelude.len(), 1);
    let init = &first.prelude[0];
    assert!(matches!(bob.receive(&alice, init, 1), IncomingFsOutcome::Handled));
    let accept = bob.take_controls();
    assert_eq!(accept.len(), 1, "Bob answers with an Accept");
    assert_eq!(
        inner(bob.receive(&alice, first.envelope.as_ref().unwrap(), 2)),
        (0x01, b"hello bob".to_vec())
    );

    // A second 2DH message before the Accept arrives.
    let second = alice.send(&bob, 0x01, b"still 2dh");
    assert!(second.prelude.is_empty());
    assert_eq!(
        inner(bob.receive(&alice, second.envelope.as_ref().unwrap(), 3)).1,
        b"still 2dh"
    );
    // Replaying it fails (counter too low): discarded, no reject.
    assert!(matches!(
        bob.receive(&alice, second.envelope.as_ref().unwrap(), 3),
        IncomingFsOutcome::Handled
    ));
    assert!(bob.take_controls().is_empty());

    // Alice gets the Accept: 4DH from now on, at version 1.2.
    assert!(matches!(
        alice.receive(&bob, &accept[0], 4),
        IncomingFsOutcome::Handled
    ));
    assert!(alice.with_store(|store| store.statuses.contains(&(bob.id, FsStatus::Established))));
    let third = alice.send(&bob, 0x80, b"\x01receipt-ids");
    assert!(
        third.envelope.is_some(),
        "delivery receipts are encapsulated from 1.1 on"
    );
    assert_eq!(
        inner(bob.receive(&alice, third.envelope.as_ref().unwrap(), 5)).0,
        0x80
    );

    // Bob answers with 4DH.
    let reply = bob.send(&alice, 0x01, b"hi alice");
    assert!(reply.prelude.is_empty(), "Bob reuses the session");
    assert_eq!(
        inner(alice.receive(&bob, reply.envelope.as_ref().unwrap(), 6)).1,
        b"hi alice"
    );

    // Both sides end in RL44.
    let best = |party: &Party, peer: ThreemaId| party.data.get_best(peer).unwrap().unwrap();
    assert_eq!(best(&alice, bob.id).state(), Ok(SessionState::RL44));
    assert_eq!(best(&bob, alice.id).state(), Ok(SessionState::RL44));
}

#[test]
fn not_eligible_types_are_not_encapsulated_in_2dh() {
    let mut alice = alice();
    let bob = bob();
    // Delivery receipts need 1.1, a new session applies 1.0: only the Init goes out.
    let result = alice.send(&bob, 0x80, b"\x01ids");
    assert_eq!(result.prelude.len(), 1);
    assert!(result.envelope.is_none());
    // Web session resume is never encapsulated.
    assert!(alice.send(&bob, 0xfe, b"").envelope.is_none());
}

#[test]
fn unknown_session_is_rejected() {
    let mut alice = alice();
    let mut bob = bob();
    let first = alice.send(&bob, 0x01, b"lost init");
    // Bob never saw the Init.
    assert!(matches!(
        bob.receive(&alice, first.envelope.as_ref().unwrap(), 42),
        IncomingFsOutcome::Handled
    ));
    let controls = bob.take_controls();
    assert_eq!(controls.len(), 1);
    let envelope = Envelope::decode(controls[0].as_slice()).unwrap();
    let Some(envelope::Content::Reject(reject)) = envelope.content else {
        panic!("expected a reject")
    };
    assert_eq!(reject.message_id, 42);
    assert_eq!(reject.cause, reject::Cause::UnknownSession as i32);

    // Alice drops the session and reports the rejected message.
    assert!(matches!(
        alice.receive(&bob, &controls[0], 43),
        IncomingFsOutcome::Handled
    ));
    assert!(alice.with_store(|store| store.sessions.is_empty()));
    assert!(alice.with_store(|store| {
        store.statuses.contains(&(
            bob.id,
            FsStatus::MessageRejected {
                message_id: 42,
                group_identity: None,
                session_known: true,
            },
        ))
    }));
}

fn group_identity() -> GroupIdentity {
    GroupIdentity {
        group_id: 7,
        creator_identity: "00000BOB".to_owned(),
    }
}

fn encapsulated_of(envelope: &[u8]) -> Encapsulated {
    match Envelope::decode(envelope).unwrap().content {
        Some(envelope::Content::Encapsulated(encapsulated)) => encapsulated,
        other => panic!("expected an encapsulated message, got {other:?}"),
    }
}

fn reject_of(envelope: &[u8]) -> Reject {
    match Envelope::decode(envelope).unwrap().content {
        Some(envelope::Content::Reject(reject)) => reject,
        other => panic!("expected a reject, got {other:?}"),
    }
}

#[test]
fn a_group_message_names_its_group_and_a_contact_message_does_not() {
    let mut alice = alice();
    let bob = bob();
    let text = alice.send(&bob, 0x01, b"to bob");
    assert_eq!(
        encapsulated_of(text.envelope.as_ref().unwrap()).group_identity,
        None
    );
    let group_text = alice.send_in_group(&bob, 0x01, b"to the group", Some(group_identity()));
    assert_eq!(
        encapsulated_of(group_text.envelope.as_ref().unwrap()).group_identity,
        Some(group_identity())
    );
}

#[test]
fn a_rejected_group_message_names_its_group_in_the_reject() {
    let mut alice = alice();
    let mut bob = bob();
    let first = alice.send_in_group(&bob, 0x01, b"to the group", Some(group_identity()));

    // Unknown session: Bob never saw the Init.
    let _ = bob.receive(&alice, first.envelope.as_ref().unwrap(), 42);
    let controls = bob.take_controls();
    let reject = reject_of(&controls[0]);
    assert_eq!(reject.cause, reject::Cause::UnknownSession as i32);
    assert_eq!(reject.group_identity, Some(group_identity()));

    // State mismatch: a session exists, but the message doesn't decrypt.
    let _ = bob.receive(&alice, &first.prelude[0], 43);
    let _ = bob.take_controls();
    let mut envelope = Envelope::decode(first.envelope.as_ref().unwrap().as_slice()).unwrap();
    if let Some(envelope::Content::Encapsulated(encapsulated)) = envelope.content.as_mut() {
        encapsulated.encrypted_inner[0] ^= 1;
    }
    let _ = bob.receive(&alice, &envelope.encode_to_vec(), 44);
    let controls = bob.take_controls();
    let reject = reject_of(&controls[0]);
    assert_eq!(reject.cause, reject::Cause::StateMismatch as i32);
    assert_eq!(reject.group_identity, Some(group_identity()));
}

fn envelope_of(session_id: [u8; 16], content: envelope::Content) -> Vec<u8> {
    envelope_bytes(SessionId(session_id), content)
}

#[test]
fn a_reject_says_whether_it_ended_a_session_and_which_group_it_is_about() {
    let mut alice = alice();
    let bob = bob();
    let first = alice.send(&bob, 0x01, b"one");
    let session_id = Envelope::decode(first.prelude[0].as_slice()).unwrap().session_id;
    let session_id: [u8; 16] = session_id.try_into().unwrap();
    let rejected = |message_id, group_identity| {
        envelope_of(
            session_id,
            envelope::Content::Reject(Reject {
                message_id,
                group_identity,
                cause: reject::Cause::UnknownSession as i32,
            }),
        )
    };

    // Several messages in flight on a session the peer lost: only the first finds it still there.
    let _ = alice.receive(&bob, &rejected(1, None), 10);
    let _ = alice.receive(&bob, &rejected(2, Some(group_identity())), 11);
    // A group that isn't one doesn't count as named.
    let _ = alice.receive(
        &bob,
        &rejected(
            3,
            Some(GroupIdentity {
                group_id: 0,
                creator_identity: String::new(),
            }),
        ),
        12,
    );
    let statuses = alice.with_store(|store| {
        store
            .statuses
            .drain(..)
            .map(|(_, status)| status)
            .collect::<Vec<_>>()
    });
    assert_eq!(
        statuses,
        vec![
            FsStatus::MessageRejected {
                message_id: 1,
                group_identity: None,
                session_known: true,
            },
            FsStatus::MessageRejected {
                message_id: 2,
                group_identity: Some(common::GroupIdentity {
                    group_id: 7,
                    creator_identity: ThreemaId::try_from("00000BOB").unwrap(),
                }),
                session_known: false,
            },
            FsStatus::MessageRejected {
                message_id: 3,
                group_identity: None,
                session_known: false,
            },
        ]
    );
}

#[test]
fn a_terminate_is_reported_for_a_known_session_or_when_the_peer_disabled_forward_security() {
    let mut alice = alice();
    let bob = bob();
    let first = alice.send(&bob, 0x01, b"one");
    let session_id: [u8; 16] = Envelope::decode(first.prelude[0].as_slice())
        .unwrap()
        .session_id
        .try_into()
        .unwrap();
    let terminated = |cause: terminate::Cause| {
        envelope_of(
            session_id,
            envelope::Content::Terminate(Terminate { cause: cause as i32 }),
        )
    };

    let _ = alice.receive(&bob, &terminated(terminate::Cause::Reset), 10);
    // The session is gone now: a repeat is nothing to report ...
    let _ = alice.receive(&bob, &terminated(terminate::Cause::Reset), 11);
    // ... unless it says that the peer has no forward security any more.
    let _ = alice.receive(&bob, &terminated(terminate::Cause::DisabledByLocal), 12);
    let statuses = alice.with_store(|store| {
        store
            .statuses
            .drain(..)
            .map(|(_, status)| status)
            .collect::<Vec<_>>()
    });
    assert_eq!(
        statuses,
        vec![
            FsStatus::Terminated { session_known: true },
            FsStatus::Terminated { session_known: false },
        ]
    );
}

#[test]
fn a_message_without_forward_security_is_reported_if_the_session_covers_its_type() {
    let mut alice = alice();
    let mut bob = bob();
    let reports = |party: &Party| {
        party.with_store(|store| {
            store
                .statuses
                .drain(..)
                .filter(|(_, status)| *status == FsStatus::MessageWithoutFs)
                .count()
        })
    };

    // No session: nothing to compare with.
    check_message_without_fs(&bob.fs, alice.id, 0x01).unwrap();
    assert_eq!(reports(&bob), 0);

    // Bring the session to RL44 on Bob's side, at version 1.2.
    let first = alice.send(&bob, 0x01, b"hello");
    let _ = bob.receive(&alice, &first.prelude[0], 1);
    let accept = bob.take_controls();

    // While it is R24, only what version 1.0 covers counts.
    check_message_without_fs(&bob.fs, alice.id, 0x80).unwrap();
    assert_eq!(reports(&bob), 0, "receipts aren't covered at 1.0");
    check_message_without_fs(&bob.fs, alice.id, 0x01).unwrap();
    assert_eq!(reports(&bob), 1, "texts are");

    let _ = alice.receive(&bob, &accept[0], 2);
    let later = alice.send(&bob, 0x01, b"4dh");
    let _ = bob.receive(&alice, later.envelope.as_ref().unwrap(), 3);
    check_message_without_fs(&bob.fs, alice.id, 0x80).unwrap();
    assert_eq!(reports(&bob), 1, "receipts are covered now");
    check_message_without_fs(&bob.fs, alice.id, 0x41).unwrap();
    assert_eq!(reports(&bob), 1, "so are group messages");

    // Never encapsulated, so never missing it.
    check_message_without_fs(&bob.fs, alice.id, 0xfe).unwrap();
    assert_eq!(reports(&bob), 0);
}

#[test]
fn tampered_message_is_rejected_and_session_dropped() {
    let mut alice = alice();
    let mut bob = bob();
    let first = alice.send(&bob, 0x01, b"hello");
    let _ = bob.receive(&alice, &first.prelude[0], 1);
    let _ = bob.take_controls();
    let mut envelope = Envelope::decode(first.envelope.as_ref().unwrap().as_slice()).unwrap();
    if let Some(envelope::Content::Encapsulated(encapsulated)) = envelope.content.as_mut() {
        encapsulated.encrypted_inner[0] ^= 1;
    }
    assert!(matches!(
        bob.receive(&alice, &envelope.encode_to_vec(), 2),
        IncomingFsOutcome::Handled
    ));
    assert_eq!(bob.take_controls().len(), 1, "state mismatch reject");
    assert!(bob.with_store(|store| store.sessions.is_empty()));
}
