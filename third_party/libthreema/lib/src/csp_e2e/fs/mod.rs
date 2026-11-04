//! Forward security (PFS) for 1:1 end-to-end encrypted messages, see
//! `threema-protocols/src/csp-e2e-fs.proto`. Behaviour follows the Android implementation
//! (`ch.threema.domain.fs`, `ForwardSecurityMessageProcessor`), which defines interoperability.
//!
//! - Incoming: the incoming message task hands `0xa0` envelopes to [`process_incoming`]. Control
//!   messages (Init/Accept/Reject/Terminate) are handled completely; their replies are queued via
//!   [`FsSessionStore::enqueue_control`]. Encapsulated messages are decrypted and the inner message
//!   continues through the normal receive steps; the peer ratchet is turned when the message is
//!   acknowledged ([`commit_peer_ratchet`], the spec's `fs-commit-fn`), so that an interrupted
//!   message can be decrypted again after redelivery.
//! - Outgoing: [`encapsulate`] wraps an outgoing message (creating a session with an `Init` first
//!   if needed). The turned ratchet is stored before the bytes are returned: the inner encryption
//!   uses a zero nonce, so a key must never be used twice.
pub mod ratchet;
pub mod session;

use prost::Message as _;
use tracing::{debug, info, warn};

use self::session::{DhType, Session, SessionError, SessionId, SessionState, supported_version_range};
use crate::{
    common::{
        self, ThreemaId,
        keys::{ClientKey, PublicKey},
    },
    model::provider::ProviderError,
    protobuf::{
        common::GroupIdentity,
        csp_e2e_fs::{
            Accept, Encapsulated, Envelope, Init, Reject, Terminate, Version, encapsulated, envelope, reject,
            terminate,
        },
    },
};

/// Message type of the FS envelope container.
pub const ENVELOPE_MESSAGE_TYPE: u8 = 0xa0;
/// Message type of the `empty` message (used to announce version upgrades and keep sessions alive).
pub const EMPTY_MESSAGE_TYPE: u8 = 0xfc;

/// Minimum FS version required to encapsulate a message type, or `None` if it is never
/// encapsulated (`getMinimumRequiredForwardSecurityVersion` in the Android message classes).
#[must_use]
pub fn minimum_version_for(message_type: u8) -> Option<u32> {
    let v1_0 = Version::V10 as u32;
    let v1_1 = Version::V11 as u32;
    let v1_2 = Version::V12 as u32;
    match message_type {
        // Text, location, file, poll setup/vote, empty
        0x01 | 0x10 | 0x17 | 0x15 | 0x16 | EMPTY_MESSAGE_TYPE => Some(v1_0),
        // Delivery receipt, typing, reaction, edit, delete, profile pictures, calls
        0x80 | 0x90 | 0x82 | 0x91 | 0x92 | 0x18 | 0x19 | 0x1a | 0x60..=0x64 => Some(v1_1),
        // Group messages, work sync delta
        0x41..=0x46 | 0x4a..=0x4c | 0x4f | 0x50..=0x54 | 0x81 | 0x83 | 0x93 | 0x94 | 0xfd => Some(v1_2),
        // Web session resume, the envelope itself, unknown types
        _ => None,
    }
}

/// User-visible FS events (Android's `ForwardSecurityStatusSender` statuses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsStatus {
    /// A session with a version ≥ 1.1 has been established (or upgraded to it).
    Established,
    /// The existing session was replaced (the peer reset it or started a new one).
    Reset,
    /// The peer rejected one of our messages; it may need to be sent again.
    MessageRejected {
        /// ID of the rejected (outer) message.
        message_id: u64,
        /// The group the peer says the message was for, if it named a valid one.
        group_identity: Option<common::GroupIdentity>,
        /// Whether the reject ended a session we had. Every message sent in a session the peer
        /// lost is rejected on its own; only the first of them finds the session still there.
        session_known: bool,
    },
    /// The peer ended a session, or said it has forward security disabled for one we don't have
    /// (the only unknown session this is reported for: the sender's feature mask is out of date).
    Terminated {
        /// Whether it ended a session we had.
        session_known: bool,
    },
    /// A message arrived without forward security although the session with its sender covers
    /// that message type: the sender dropped it, or someone else holds its long-term key.
    MessageWithoutFs,
}

/// Storage of FS sessions (implemented by the app, persisted encrypted, never backed up) plus the
/// side effects of processing FS control messages.
pub trait FsSessionStore {
    /// The session with `id` for `peer`, if any.
    ///
    /// # Errors
    ///
    /// Storage failures.
    fn get(&self, peer: ThreemaId, id: SessionId) -> Result<Option<Session>, ProviderError>;

    /// The best session for `peer`: one in a 4DH state first, then the lowest session ID.
    ///
    /// # Errors
    ///
    /// Storage failures.
    fn get_best(&self, peer: ThreemaId) -> Result<Option<Session>, ProviderError>;

    /// Insert or replace `session`.
    ///
    /// # Errors
    ///
    /// Storage failures.
    fn store(&mut self, session: &Session) -> Result<(), ProviderError>;

    /// Delete one session.
    ///
    /// # Errors
    ///
    /// Storage failures.
    fn delete(&mut self, peer: ThreemaId, id: SessionId) -> Result<(), ProviderError>;

    /// Delete all sessions of `peer` except `keep`; with `four_dh_only` only sessions in a 4DH
    /// state (R24/RL44). Returns how many were deleted.
    ///
    /// # Errors
    ///
    /// Storage failures.
    fn delete_all_except(
        &mut self,
        peer: ThreemaId,
        keep: Option<SessionId>,
        four_dh_only: bool,
    ) -> Result<usize, ProviderError>;

    /// Queue an FS control envelope (encoded [`Envelope`]) to be sent to `peer` as a `0xa0`
    /// message with a new random message ID, no flags and no nickname. `peer_public_key` is known
    /// when this answers a message of `peer`, who may not be a contact yet: an Init comes before
    /// the message that creates the contact.
    ///
    /// # Errors
    ///
    /// Storage failures.
    fn enqueue_control(
        &mut self,
        peer: ThreemaId,
        peer_public_key: Option<PublicKey>,
        envelope: Vec<u8>,
    ) -> Result<(), ProviderError>;

    /// Report a user-visible event.
    ///
    /// # Errors
    ///
    /// Storage failures.
    fn status(&mut self, peer: ThreemaId, status: FsStatus) -> Result<(), ProviderError>;
}

/// FS state of the CSP E2E context.
pub struct FsContext {
    /// Session storage.
    pub(crate) store: Box<core::cell::RefCell<dyn FsSessionStore>>,
    /// Peer ratchet to turn once the current incoming message is acknowledged.
    pub(crate) pending_commit: Option<PeerRatchetCommit>,
}

impl FsContext {
    /// Wrap a session store.
    #[must_use]
    pub fn new(store: Box<core::cell::RefCell<dyn FsSessionStore>>) -> Self {
        Self {
            store,
            pending_commit: None,
        }
    }
}

/// Identifies the peer ratchet used for an incoming encapsulated message.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PeerRatchetCommit {
    peer: ThreemaId,
    session_id: SessionId,
    dh_type: DhType,
}

/// Result of processing an incoming envelope.
#[derive(Debug)]
pub(crate) enum IncomingFsOutcome {
    /// Decrypted: continue processing the inner message (`type`, `data`).
    Inner { inner_type: u8, data: Vec<u8> },
    /// A control message or a rejected/undecryptable message: acknowledge and discard.
    Handled,
}

fn envelope_bytes(session_id: SessionId, content: envelope::Content) -> Vec<u8> {
    Envelope {
        session_id: session_id.0.to_vec(),
        content: Some(content),
    }
    .encode_to_vec()
}

fn send_reject(
    store: &mut dyn FsSessionStore,
    peer: ThreemaId,
    peer_public_key: PublicKey,
    session_id: SessionId,
    message_id: u64,
    group_identity: Option<GroupIdentity>,
    cause: reject::Cause,
) -> Result<(), ProviderError> {
    store.enqueue_control(
        peer,
        Some(peer_public_key),
        envelope_bytes(
            session_id,
            envelope::Content::Reject(Reject {
                message_id,
                group_identity,
                cause: cause as i32,
            }),
        ),
    )
}

fn send_terminate(
    store: &mut dyn FsSessionStore,
    peer: ThreemaId,
    peer_public_key: Option<PublicKey>,
    session_id: SessionId,
    cause: terminate::Cause,
) -> Result<(), ProviderError> {
    store.enqueue_control(
        peer,
        peer_public_key,
        envelope_bytes(
            session_id,
            envelope::Content::Terminate(Terminate { cause: cause as i32 }),
        ),
    )
}

/// Process an incoming FS envelope from `peer` (already E2E-decrypted).
pub(crate) fn process_incoming(
    fs: &mut FsContext,
    my_identity: ThreemaId,
    client_key: &ClientKey,
    peer: ThreemaId,
    peer_public_key: &PublicKey,
    outer_message_id: u64,
    envelope_data: &[u8],
) -> Result<IncomingFsOutcome, ProviderError> {
    fs.pending_commit = None;
    let Ok(envelope) = Envelope::decode(envelope_data) else {
        warn!("Discarding undecodable FS envelope");
        return Ok(IncomingFsOutcome::Handled);
    };
    let Ok(session_id) = SessionId::try_from(envelope.session_id.as_slice()) else {
        warn!("Discarding FS envelope with invalid session ID");
        return Ok(IncomingFsOutcome::Handled);
    };
    let mut store = fs.store.borrow_mut();
    let store = &mut *store;
    match envelope.content {
        Some(envelope::Content::Init(init)) => {
            handle_init(
                store,
                my_identity,
                client_key,
                peer,
                peer_public_key,
                session_id,
                &init,
            )?;
            Ok(IncomingFsOutcome::Handled)
        },
        Some(envelope::Content::Accept(accept)) => {
            handle_accept(store, client_key, peer, peer_public_key, session_id, &accept)?;
            Ok(IncomingFsOutcome::Handled)
        },
        Some(envelope::Content::Reject(reject)) => {
            info!(?session_id, cause = reject.cause, "FS reject received");
            let session_known = store.get(peer, session_id)?.is_some();
            if session_known {
                store.delete(peer, session_id)?;
            }
            store.status(
                peer,
                FsStatus::MessageRejected {
                    message_id: reject.message_id,
                    group_identity: reject
                        .group_identity
                        .as_ref()
                        .and_then(|group_identity| common::GroupIdentity::try_from(group_identity).ok()),
                    session_known,
                },
            )?;
            Ok(IncomingFsOutcome::Handled)
        },
        Some(envelope::Content::Terminate(terminate)) => {
            info!(?session_id, cause = terminate.cause, "FS terminate received");
            let session_known = store.get(peer, session_id)?.is_some();
            if session_known {
                store.delete(peer, session_id)?;
            }
            if session_known || terminate.cause == terminate::Cause::DisabledByLocal as i32 {
                store.status(peer, FsStatus::Terminated { session_known })?;
            }
            Ok(IncomingFsOutcome::Handled)
        },
        Some(envelope::Content::Encapsulated(encapsulated)) => {
            let outcome = handle_encapsulated(
                store,
                peer,
                *peer_public_key,
                session_id,
                outer_message_id,
                &encapsulated,
            )?;
            if let IncomingFsOutcome::Inner { .. } = outcome {
                fs.pending_commit = Some(PeerRatchetCommit {
                    peer,
                    session_id,
                    dh_type: dh_type_of(&encapsulated),
                });
            }
            Ok(outcome)
        },
        None => {
            warn!("Discarding FS envelope without content");
            Ok(IncomingFsOutcome::Handled)
        },
    }
}

fn handle_init(
    store: &mut dyn FsSessionStore,
    my_identity: ThreemaId,
    client_key: &ClientKey,
    peer: ThreemaId,
    peer_public_key: &PublicKey,
    session_id: SessionId,
    init: &Init,
) -> Result<(), ProviderError> {
    if store.get(peer, session_id)?.is_some() {
        debug!(?session_id, "Ignoring repeated FS init");
        return Ok(());
    }
    // A new Init replaces existing 4DH sessions (2DH ones are kept for races).
    let preempted = store.delete_all_except(peer, None, true)?;
    let session = match Session::new_responder(
        session_id,
        my_identity,
        client_key,
        peer,
        peer_public_key,
        &init.fssk,
        init.supported_version.as_ref(),
    ) {
        Ok(session) => session,
        Err(error) => {
            warn!(%error, "Cannot accept FS init");
            return send_terminate(
                store,
                peer,
                Some(*peer_public_key),
                session_id,
                terminate::Cause::Reset,
            );
        },
    };
    store.store(&session)?;
    store.enqueue_control(
        peer,
        Some(*peer_public_key),
        envelope_bytes(
            session_id,
            envelope::Content::Accept(Accept {
                supported_version: Some(supported_version_range()),
                fssk: session.my_ephemeral_public_key.to_vec(),
            }),
        ),
    )?;
    info!(?session_id, "FS session accepted");
    if preempted > 0 {
        store.status(peer, FsStatus::Reset)?;
    }
    if session.outgoing_applied_version().unwrap_or(0) >= Version::V11 as u32 {
        store.status(peer, FsStatus::Established)?;
    }
    Ok(())
}

fn handle_accept(
    store: &mut dyn FsSessionStore,
    client_key: &ClientKey,
    peer: ThreemaId,
    peer_public_key: &PublicKey,
    session_id: SessionId,
    accept: &Accept,
) -> Result<(), ProviderError> {
    let Some(mut session) = store.get(peer, session_id)? else {
        warn!(?session_id, "FS accept for unknown session");
        return send_terminate(
            store,
            peer,
            Some(*peer_public_key),
            session_id,
            terminate::Cause::UnknownSession,
        );
    };
    match session.process_accept(
        client_key,
        peer_public_key,
        &accept.fssk,
        accept.supported_version.as_ref(),
    ) {
        Ok(()) => {
            store.store(&session)?;
            info!(?session_id, "FS session established (4DH)");
            if session.outgoing_applied_version().unwrap_or(0) >= Version::V11 as u32 {
                store.status(peer, FsStatus::Established)?;
            }
            Ok(())
        },
        Err(error) => {
            warn!(%error, "Cannot process FS accept");
            store.delete(peer, session_id)?;
            send_terminate(
                store,
                peer,
                Some(*peer_public_key),
                session_id,
                terminate::Cause::Reset,
            )
        },
    }
}

fn dh_type_of(encapsulated: &Encapsulated) -> DhType {
    if encapsulated.dh_type == encapsulated::DhType::Fourdh as i32 {
        DhType::FourDh
    } else {
        DhType::TwoDh
    }
}

fn handle_encapsulated(
    store: &mut dyn FsSessionStore,
    peer: ThreemaId,
    peer_public_key: PublicKey,
    session_id: SessionId,
    outer_message_id: u64,
    encapsulated: &Encapsulated,
) -> Result<IncomingFsOutcome, ProviderError> {
    let Some(mut session) = store.get(peer, session_id)? else {
        warn!(?session_id, "FS message for unknown session, rejecting");
        send_reject(
            store,
            peer,
            peer_public_key,
            session_id,
            outer_message_id,
            encapsulated.group_identity.clone(),
            reject::Cause::UnknownSession,
        )?;
        return Ok(IncomingFsOutcome::Handled);
    };
    let dh_type = dh_type_of(encapsulated);
    let reject_and_delete =
        |store: &mut dyn FsSessionStore, reason: &str| -> Result<IncomingFsOutcome, ProviderError> {
            // log-ok: `reason` is one of this file's fixed texts
            warn!(?session_id, reason, "Rejecting FS message");
            store.delete(peer, session_id)?;
            send_reject(
                store,
                peer,
                peer_public_key,
                session_id,
                outer_message_id,
                encapsulated.group_identity.clone(),
                reject::Cause::StateMismatch,
            )?;
            store.status(peer, FsStatus::Reset)?;
            Ok(IncomingFsOutcome::Handled)
        };
    let versions = match session.process_incoming_versions(
        dh_type,
        encapsulated.offered_version,
        encapsulated.applied_version,
    ) {
        Ok(versions) => versions,
        Err(error) => return reject_and_delete(store, &error.to_string()),
    };
    let previous_state = session.state();
    let ratchet = match dh_type {
        DhType::TwoDh => session.peer_ratchet_2dh.as_mut(),
        DhType::FourDh => session.peer_ratchet_4dh.as_mut(),
    };
    let Some(ratchet) = ratchet else {
        return reject_and_delete(store, "no ratchet for DH type");
    };
    if let Err(error) = ratchet.turn_until(encapsulated.counter) {
        // Replayed or too far ahead: discard without reject (like Android).
        warn!(%error, "Discarding FS message with unusable counter");
        return Ok(IncomingFsOutcome::Handled);
    }
    let plaintext = match session::decrypt_inner(ratchet, &encapsulated.encrypted_inner) {
        Ok(plaintext) => plaintext,
        Err(SessionError::DecryptionFailed) => {
            return reject_and_delete(store, "decryption failed");
        },
        Err(error) => return reject_and_delete(store, &error.to_string()),
    };
    let Some((&inner_type, data)) = plaintext.split_first() else {
        warn!("Discarding empty FS inner message");
        return Ok(IncomingFsOutcome::Handled);
    };
    if inner_type == ENVELOPE_MESSAGE_TYPE {
        warn!("Discarding nested FS envelope");
        return Ok(IncomingFsOutcome::Handled);
    }
    if minimum_version_for(inner_type).is_none_or(|required| required > versions.applied) {
        warn!(
            inner_type,
            applied = versions.applied,
            "Discarding inner message not allowed at this version"
        );
        return Ok(IncomingFsOutcome::Handled);
    }

    // Commit what we learned (ratchet not yet turned past this message, see module docs).
    if let Some(pending) = versions.pending {
        session.versions = Some(pending);
    }
    if dh_type == DhType::FourDh {
        session.peer_ratchet_2dh = None;
    }
    store.store(&session)?;
    if dh_type == DhType::FourDh && previous_state == Ok(SessionState::R24) {
        info!(?session_id, "FS session fully established (first 4DH message)");
        // This session works both ways now; others for this peer are obsolete.
        let _ = store.delete_all_except(peer, Some(session_id), false)?;
    }
    Ok(IncomingFsOutcome::Inner {
        inner_type,
        data: data.to_vec(),
    })
}

/// A message arrived from `peer` outside of an envelope: report it if the best session with `peer`
/// would have covered `message_type` (Android's `warnIfMessageWithoutForwardSecurityReceived`).
pub(crate) fn check_message_without_fs(
    fs: &FsContext,
    peer: ThreemaId,
    message_type: u8,
) -> Result<(), ProviderError> {
    let Some(required) = minimum_version_for(message_type) else {
        return Ok(());
    };
    let mut store = fs.store.borrow_mut();
    let Some(session) = store.get_best(peer)? else {
        return Ok(());
    };
    if session
        .minimum_incoming_applied_version()
        .is_ok_and(|minimum| required <= minimum)
    {
        warn!(
            message_type,
            "Message without forward security on an active session"
        );
        store.status(peer, FsStatus::MessageWithoutFs)?;
    }
    Ok(())
}

/// Turn the peer ratchet past the acknowledged incoming message and store it.
pub(crate) fn commit_peer_ratchet(fs: &mut FsContext) -> Result<(), ProviderError> {
    let Some(commit) = fs.pending_commit.take() else {
        return Ok(());
    };
    let mut store = fs.store.borrow_mut();
    let Some(mut session) = store.get(commit.peer, commit.session_id)? else {
        return Ok(());
    };
    let ratchet = match commit.dh_type {
        DhType::TwoDh => session.peer_ratchet_2dh.as_mut(),
        DhType::FourDh => session.peer_ratchet_4dh.as_mut(),
    };
    if let Some(ratchet) = ratchet {
        ratchet.turn();
        store.store(&session)?;
    }
    Ok(())
}

/// Error when encapsulating.
#[derive(Debug, thiserror::Error)]
pub enum EncapsulationError {
    /// Storage failed.
    #[error("Storage: {0}")]
    Storage(#[from] ProviderError),
    /// The session could not be created or is in an illegal state.
    #[error("Session: {0}")]
    Session(#[from] SessionError),
}

/// Result of [`encapsulate`].
#[derive(Debug)]
pub struct Encapsulation {
    /// Control envelopes to send first, each as its own `0xa0` message (new random message ID, no
    /// flags, no nickname): an `Init` when a new session was created.
    pub prelude: Vec<Vec<u8>>,
    /// The envelope to send in place of the message (same message ID, flags, created-at and
    /// nickname as the plain message would have, type `0xa0`); `None` to send it unencapsulated
    /// (type not eligible at the session's version).
    pub envelope: Option<Vec<u8>>,
}

/// Encapsulate an outgoing 1:1 message (`inner_type`, `inner_data` without padding) for `peer`.
/// The caller must have checked that the peer supports forward security (feature mask).
/// `group_identity` is the group of a group message: required for those by the protocol, so that
/// a peer who can't decrypt it can say which message of which group it rejects; `None` for 1:1.
///
/// # Errors
///
/// [`EncapsulationError`] on storage or session failures.
pub fn encapsulate(
    store: &mut dyn FsSessionStore,
    my_identity: ThreemaId,
    client_key: &ClientKey,
    peer: ThreemaId,
    peer_public_key: &PublicKey,
    inner_type: u8,
    inner_data: &[u8],
    group_identity: Option<GroupIdentity>,
    now_ms: u64,
) -> Result<Encapsulation, EncapsulationError> {
    let mut prelude = Vec::new();
    let mut session = if let Some(session) = store.get_best(peer)? {
        session
    } else {
        let session = Session::new_initiator(my_identity, client_key, peer, peer_public_key)?;
        store.store(&session)?;
        info!(session_id = ?session.id, "FS session initiated");
        prelude.push(envelope_bytes(
            session.id,
            envelope::Content::Init(Init {
                supported_version: Some(supported_version_range()),
                fssk: session.my_ephemeral_public_key.to_vec(),
            }),
        ));
        session
    };

    let applied = session.outgoing_applied_version()?;
    if minimum_version_for(inner_type).is_none_or(|required| required > applied) {
        return Ok(Encapsulation {
            prelude,
            envelope: None,
        });
    }
    let offered = session.outgoing_offered_version()?;
    let (dh_type, ratchet) = match session.state()? {
        SessionState::L20 => (DhType::TwoDh, session.my_ratchet_2dh.as_mut()),
        SessionState::R24 | SessionState::RL44 => (DhType::FourDh, session.my_ratchet_4dh.as_mut()),
        SessionState::R20 => (DhType::FourDh, None),
    };
    let ratchet = ratchet.ok_or(SessionError::IllegalState)?;
    let mut plaintext = Vec::with_capacity(inner_data.len().saturating_add(1));
    plaintext.push(inner_type);
    plaintext.extend_from_slice(inner_data);
    let encrypted_inner = session::encrypt_inner(ratchet, &plaintext);
    let counter = ratchet.counter;
    ratchet.turn();
    session.last_outgoing_message_at = now_ms;
    // Persist the turn before the bytes leave: the key must never be used again.
    store.store(&session)?;

    Ok(Encapsulation {
        prelude,
        envelope: Some(envelope_bytes(
            session.id,
            envelope::Content::Encapsulated(Encapsulated {
                dh_type: match dh_type {
                    DhType::TwoDh => encapsulated::DhType::Twodh,
                    DhType::FourDh => encapsulated::DhType::Fourdh,
                } as i32,
                counter,
                offered_version: offered,
                applied_version: applied,
                group_identity,
                encrypted_inner,
            }),
        )),
    })
}

/// Terminate all sessions with `peer` (e.g. when the contact is deleted) and queue the
/// `Terminate` messages.
///
/// # Errors
///
/// Storage failures.
pub fn terminate_all(store: &mut dyn FsSessionStore, peer: ThreemaId) -> Result<(), ProviderError> {
    terminate_all_with_cause(store, peer, terminate::Cause::Reset)
}

/// Terminate all sessions with `peer` for the given `cause` (e.g. `DisabledByLocal` when
/// multi-device turns forward security off) and queue the `Terminate` messages.
///
/// # Errors
///
/// Storage failures.
pub fn terminate_all_with_cause(
    store: &mut dyn FsSessionStore,
    peer: ThreemaId,
    cause: terminate::Cause,
) -> Result<(), ProviderError> {
    while let Some(session) = store.get_best(peer)? {
        store.delete(peer, session.id)?;
        send_terminate(store, peer, None, session.id, cause)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
