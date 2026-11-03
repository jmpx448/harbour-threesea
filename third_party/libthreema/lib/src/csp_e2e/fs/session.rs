//! Forward security session (`DHSession` in the Android implementation).
//!
//! Key schedule (verified against Android `DHSession.java`, 6.5.3-1216), with `DH(a, b)` being
//! X25519 followed by HSalsa20 (the NaCl shared secret) and `B2b(key, salt)` BLAKE2b-MAC-256 with
//! personal `3ma-e2e`:
//!
//! - 2DH (initiator → responder only): `B2b(DH(I_CK, R_CK) || DH(I_FSSK, R_CK), "ke-2dh-" + I_ID)`
//! - 4DH: `I = BLAKE2b-512(DH(I_CK, R_CK) || DH(I_FSSK, R_CK) || DH(I_CK, R_FSSK) ||
//!   DH(I_FSSK, R_FSSK))`, then per direction `B2b(I, "ke-4dh-" + <sender ID>)`.
use educe::Educe;
use rand::Rng as _;
use zeroize::{Zeroize as _, ZeroizeOnDrop};

use super::ratchet::{Ratchet, blake2b_256};
use crate::{
    common::{
        ThreemaId,
        keys::{ClientKey, PublicKey},
    },
    crypto::{
        aead::Aead as _,
        blake2b::Blake2bMac512,
        cipher::KeyInit as _,
        digest::{FixedOutput as _, Mac as _},
        salsa20, x25519,
    },
    protobuf::csp_e2e_fs::{Version, VersionRange},
};

/// Versions we support (Android 6.5.3: 1.0 ..= 1.2).
pub const SUPPORTED_VERSION_MIN: Version = Version::V10;
/// Highest version we support.
pub const SUPPORTED_VERSION_MAX: Version = Version::V12;

/// Our supported version range.
#[must_use]
pub fn supported_version_range() -> VersionRange {
    VersionRange {
        min: SUPPORTED_VERSION_MIN as u32,
        max: SUPPORTED_VERSION_MAX as u32,
    }
}

/// A forward security session ID (16 random bytes).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub [u8; 16]);
impl SessionId {
    /// A random session ID.
    #[must_use]
    pub fn random() -> Self {
        let mut id = [0_u8; 16];
        rand::thread_rng().fill(&mut id);
        Self(id)
    }
}
impl core::fmt::Debug for SessionId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&data_encoding::HEXLOWER.encode(&self.0))
    }
}
impl TryFrom<&[u8]> for SessionId {
    type Error = SessionError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self(
            bytes.try_into().map_err(|_| SessionError::InvalidSessionId)?,
        ))
    }
}

/// Session state (Android merges L44 and R44 into RL44).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// We sent an Init: outgoing 2DH, no incoming.
    L20,
    /// We received an Init (only transient; we accept immediately and move to R24).
    R20,
    /// We sent an Accept: incoming 2DH (in-flight messages) and 4DH, outgoing 4DH.
    R24,
    /// Both directions 4DH.
    RL44,
}

/// Errors from session operations.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// The ratchets don't form a valid state.
    #[error("Illegal session state")]
    IllegalState,
    /// Session ID has the wrong length.
    #[error("Invalid session ID")]
    InvalidSessionId,
    /// A public key is invalid or non-contributory.
    #[error("Invalid or non-contributory public key")]
    InvalidKey,
    /// Version ranges don't overlap or are malformed.
    #[error("Unable to negotiate version: {0}")]
    VersionNegotiation(&'static str),
    /// The message does not fit the session (state, versions, DH type).
    #[error("Message rejected: {0}")]
    Reject(String),
    /// No ephemeral private key left (Accept processed twice).
    #[error("Missing ephemeral private key")]
    MissingEphemeralPrivateKey,
    /// Decryption failed.
    #[error("Decryption failed")]
    DecryptionFailed,
}

/// Negotiated 4DH versions: `local` = what we apply to outgoing messages, `remote` = minimum the
/// peer applies to incoming messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Versions {
    /// Local (outgoing applied) version.
    pub local: u32,
    /// Remote (incoming applied) version.
    pub remote: u32,
}

/// Highest supported version within the major version of `version`.
fn supported_version_within(version: u32) -> Option<u32> {
    (version & 0xff00 == Version::V10 as u32 & 0xff00).then_some(SUPPORTED_VERSION_MAX as u32)
}

/// Negotiate a version from our range and the peer's range (`negotiateMajorAndMinorVersion`).
///
/// # Errors
///
/// [`SessionError::VersionNegotiation`] if the ranges are invalid or don't overlap.
pub fn negotiate_version(remote: Option<&VersionRange>) -> Result<u32, SessionError> {
    let local = supported_version_range();
    let remote = match remote {
        // Older clients don't send a range: assume 1.0 ..= 1.0.
        Some(range) if range.min != 0 || range.max != 0 => range.clone(),
        _ => VersionRange {
            min: Version::V10 as u32,
            max: Version::V10 as u32,
        },
    };
    if remote.max < remote.min {
        return Err(SessionError::VersionNegotiation("invalid remote range"));
    }
    if remote.min > local.max || local.min > remote.max {
        return Err(SessionError::VersionNegotiation("ranges don't overlap"));
    }
    let negotiated = local.max.min(remote.max);
    if Version::try_from(i32::try_from(negotiated).unwrap_or(0)).is_err() || negotiated == 0 {
        return Err(SessionError::VersionNegotiation("unknown version"));
    }
    Ok(negotiated)
}

/// Result of validating an incoming message's versions (`processIncomingMessageVersion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IncomingVersions {
    /// Version the peer offered.
    pub offered: u32,
    /// Version the peer applied to this message.
    pub applied: u32,
    /// New 4DH versions to commit (4DH messages only).
    pub pending: Option<Versions>,
}

/// DH type of an encapsulated message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhType {
    /// 2DH (initiator to responder before the Accept arrived).
    TwoDh,
    /// 4DH.
    FourDh,
}

fn client_shared_secret(client_key: &ClientKey, public_key: &PublicKey) -> Result<[u8; 32], SessionError> {
    Ok(client_key
        .derive_fs_shared_secret(public_key)
        .ok_or(SessionError::InvalidKey)?
        .to_bytes())
}

fn shared_secret(secret: &x25519::StaticSecret, public_key: &PublicKey) -> Result<[u8; 32], SessionError> {
    Ok(x25519::SharedSecretHSalsa20::from(
        secret
            .diffie_hellman(&public_key.0)
            .ok_or(SessionError::InvalidKey)?,
    )
    .to_bytes())
}

fn concat_and_wipe(parts: &mut [[u8; 32]]) -> Vec<u8> {
    let mut output = Vec::with_capacity(parts.len().saturating_mul(32));
    for part in parts.iter_mut() {
        output.extend_from_slice(part);
        part.zeroize();
    }
    output
}

/// A forward security session with one peer. All fields are public so that the app can persist
/// them (see [`crate::model::provider::FsSessionStore`]); treat them as secret.
#[derive(Clone, Educe, ZeroizeOnDrop)]
#[educe(Debug)]
pub struct Session {
    /// Session ID.
    #[zeroize(skip)]
    pub id: SessionId,
    /// Our identity.
    #[zeroize(skip)]
    pub my_identity: ThreemaId,
    /// The peer's identity.
    #[zeroize(skip)]
    pub peer_identity: ThreemaId,
    /// Our ephemeral private key; only kept by the initiator until the Accept arrives.
    #[educe(Debug(ignore))]
    pub my_ephemeral_private_key: Option<[u8; 32]>,
    /// Our ephemeral public key.
    #[zeroize(skip)]
    #[educe(Debug(ignore))]
    pub my_ephemeral_public_key: [u8; 32],
    /// Negotiated 4DH versions (absent in L20/R20).
    #[zeroize(skip)]
    pub versions: Option<Versions>,
    /// Unix-ish ms timestamp of the last outgoing (encapsulated) message.
    #[zeroize(skip)]
    pub last_outgoing_message_at: u64,
    /// Our 2DH ratchet (L20 only).
    pub my_ratchet_2dh: Option<Ratchet>,
    /// Our 4DH ratchet.
    pub my_ratchet_4dh: Option<Ratchet>,
    /// The peer's 2DH ratchet (R20/R24).
    pub peer_ratchet_2dh: Option<Ratchet>,
    /// The peer's 4DH ratchet.
    pub peer_ratchet_4dh: Option<Ratchet>,
}

impl Session {
    /// Start a new session as initiator (state L20). Send an Init with
    /// [`Self::my_ephemeral_public_key`] and [`supported_version_range`].
    ///
    /// # Errors
    ///
    /// [`SessionError::InvalidKey`] if the peer's key is non-contributory.
    pub fn new_initiator(
        my_identity: ThreemaId,
        client_key: &ClientKey,
        peer_identity: ThreemaId,
        peer_public_key: &PublicKey,
    ) -> Result<Self, SessionError> {
        let ephemeral = x25519::StaticSecret::random();
        let my_ephemeral_public_key = x25519::PublicKey::from(&ephemeral).to_bytes();
        let mut parts = [
            client_shared_secret(client_key, peer_public_key)?,
            shared_secret(&ephemeral, peer_public_key)?,
        ];
        let mut key = concat_and_wipe(&mut parts);
        let chain_key = blake2b_256(&key, &salt(b"ke-2dh-", my_identity));
        key.zeroize();
        Ok(Self {
            id: SessionId::random(),
            my_identity,
            peer_identity,
            my_ephemeral_private_key: Some(*ephemeral.as_bytes()),
            my_ephemeral_public_key,
            versions: None,
            last_outgoing_message_at: 0,
            my_ratchet_2dh: Some(Ratchet::new(chain_key)),
            my_ratchet_4dh: None,
            peer_ratchet_2dh: None,
            peer_ratchet_4dh: None,
        })
    }

    /// Respond to an Init (results in state R24). Send an Accept with
    /// [`Self::my_ephemeral_public_key`] and [`supported_version_range`].
    ///
    /// # Errors
    ///
    /// Version negotiation failures or invalid keys.
    pub fn new_responder(
        id: SessionId,
        my_identity: ThreemaId,
        client_key: &ClientKey,
        peer_identity: ThreemaId,
        peer_public_key: &PublicKey,
        peer_ephemeral_public_key: &[u8],
        peer_supported_versions: Option<&VersionRange>,
    ) -> Result<Self, SessionError> {
        let negotiated = negotiate_version(peer_supported_versions)?;
        let peer_ephemeral =
            PublicKey::try_from(peer_ephemeral_public_key).map_err(|_| SessionError::InvalidKey)?;
        let ephemeral = x25519::StaticSecret::random();
        let my_ephemeral_public_key = x25519::PublicKey::from(&ephemeral).to_bytes();

        let static_static = client_shared_secret(client_key, peer_public_key)?;
        let static_ephemeral = client_shared_secret(client_key, &peer_ephemeral)?;
        // Peer's 2DH (as the peer computed it: DH(I_CK, R_CK) || DH(I_FSSK, R_CK)).
        let mut parts_2dh = [static_static, static_ephemeral];
        let mut key_2dh = concat_and_wipe(&mut parts_2dh);
        let peer_chain_key_2dh = blake2b_256(&key_2dh, &salt(b"ke-2dh-", peer_identity));
        key_2dh.zeroize();

        // 4DH in initiator order: ss, DH(I_FSSK, R_CK), DH(I_CK, R_FSSK), ee.
        let mut parts_4dh = [
            static_static,
            static_ephemeral,
            shared_secret(&ephemeral, peer_public_key)?,
            shared_secret(&ephemeral, &peer_ephemeral)?,
        ];
        let (my_chain_key, peer_chain_key) = derive_4dh(&mut parts_4dh, my_identity, peer_identity);

        Ok(Self {
            id,
            my_identity,
            peer_identity,
            my_ephemeral_private_key: None,
            my_ephemeral_public_key,
            versions: Some(Versions {
                local: negotiated,
                remote: negotiated,
            }),
            last_outgoing_message_at: 0,
            my_ratchet_2dh: None,
            my_ratchet_4dh: Some(Ratchet::new(my_chain_key)),
            peer_ratchet_2dh: Some(Ratchet::new(peer_chain_key_2dh)),
            peer_ratchet_4dh: Some(Ratchet::new(peer_chain_key)),
        })
    }

    /// Process the peer's Accept (L20 → RL44).
    ///
    /// # Errors
    ///
    /// Negotiation failures, invalid keys or a missing ephemeral key (Accept processed twice).
    pub fn process_accept(
        &mut self,
        client_key: &ClientKey,
        peer_public_key: &PublicKey,
        peer_ephemeral_public_key: &[u8],
        peer_supported_versions: Option<&VersionRange>,
    ) -> Result<(), SessionError> {
        let mut private_key = self
            .my_ephemeral_private_key
            .ok_or(SessionError::MissingEphemeralPrivateKey)?;
        let negotiated = negotiate_version(peer_supported_versions)?;
        let peer_ephemeral =
            PublicKey::try_from(peer_ephemeral_public_key).map_err(|_| SessionError::InvalidKey)?;
        let ephemeral = x25519::StaticSecret::from(private_key);
        private_key.zeroize();

        let mut parts = [
            client_shared_secret(client_key, peer_public_key)?,
            shared_secret(&ephemeral, peer_public_key)?,
            client_shared_secret(client_key, &peer_ephemeral)?,
            shared_secret(&ephemeral, &peer_ephemeral)?,
        ];
        let (my_chain_key, peer_chain_key) = derive_4dh(&mut parts, self.my_identity, self.peer_identity);

        if let Some(key) = self.my_ephemeral_private_key.as_mut() {
            key.zeroize();
        }
        self.my_ephemeral_private_key = None;
        self.versions = Some(Versions {
            local: negotiated,
            remote: negotiated,
        });
        self.my_ratchet_2dh = None;
        self.my_ratchet_4dh = Some(Ratchet::new(my_chain_key));
        self.peer_ratchet_4dh = Some(Ratchet::new(peer_chain_key));
        Ok(())
    }

    /// Current state.
    ///
    /// # Errors
    ///
    /// [`SessionError::IllegalState`] for impossible ratchet combinations.
    pub fn state(&self) -> Result<SessionState, SessionError> {
        match (
            self.my_ratchet_2dh.is_some(),
            self.my_ratchet_4dh.is_some(),
            self.peer_ratchet_2dh.is_some(),
            self.peer_ratchet_4dh.is_some(),
        ) {
            (false, true, false, true) => Ok(SessionState::RL44),
            (false, true, true, true) => Ok(SessionState::R24),
            (true, false, false, false) => Ok(SessionState::L20),
            (false, false, true, false) => Ok(SessionState::R20),
            _ => Err(SessionError::IllegalState),
        }
    }

    /// Version offered in outgoing messages.
    ///
    /// # Errors
    ///
    /// [`SessionError::IllegalState`].
    pub fn outgoing_offered_version(&self) -> Result<u32, SessionError> {
        Ok(match (self.state()?, self.versions) {
            (SessionState::L20 | SessionState::R20, _) | (_, None) => Version::V10 as u32,
            (_, Some(versions)) => supported_version_within(versions.local).unwrap_or(Version::V10 as u32),
        })
    }

    /// Version applied to outgoing messages.
    ///
    /// # Errors
    ///
    /// [`SessionError::IllegalState`].
    pub fn outgoing_applied_version(&self) -> Result<u32, SessionError> {
        Ok(match (self.state()?, self.versions) {
            (SessionState::L20 | SessionState::R20, _) | (_, None) => Version::V10 as u32,
            (_, Some(versions)) => versions.local,
        })
    }

    /// Minimum version the peer applies to incoming messages (for "message without FS" checks).
    ///
    /// # Errors
    ///
    /// [`SessionError::IllegalState`].
    pub fn minimum_incoming_applied_version(&self) -> Result<u32, SessionError> {
        Ok(match (self.state()?, self.versions) {
            (SessionState::L20 | SessionState::R20 | SessionState::R24, _) | (_, None) => Version::V10 as u32,
            (SessionState::RL44, Some(versions)) => versions.remote,
        })
    }

    /// Validate the versions of an incoming encapsulated message (`processIncomingMessageVersion`).
    ///
    /// # Errors
    ///
    /// [`SessionError::Reject`] if the message must be rejected.
    pub fn process_incoming_versions(
        &self,
        dh_type: DhType,
        offered: u32,
        applied: u32,
    ) -> Result<IncomingVersions, SessionError> {
        let v1_0 = Version::V10 as u32;
        let mut offered = if offered == 0 { v1_0 } else { offered };
        let mut applied = if applied == 0 { offered } else { applied };
        // Android clamps 2DH messages to 1.0 (TODO ANDR-2452 upstream).
        if dh_type == DhType::TwoDh {
            offered = v1_0;
            applied = v1_0;
        }
        if applied > offered {
            return Err(SessionError::Reject(format!(
                "applied {applied:#x} > offered {offered:#x}"
            )));
        }
        let state = self.state()?;
        match dh_type {
            DhType::TwoDh => {
                if !matches!(state, SessionState::R20 | SessionState::R24) {
                    return Err(SessionError::Reject(format!(
                        "unexpected 2DH message in {state:?}"
                    )));
                }
                Ok(IncomingVersions {
                    offered,
                    applied,
                    pending: None,
                })
            },
            DhType::FourDh => {
                if !matches!(state, SessionState::R24 | SessionState::RL44) {
                    return Err(SessionError::Reject(format!(
                        "unexpected 4DH message in {state:?}"
                    )));
                }
                let current = self
                    .versions
                    .ok_or_else(|| SessionError::Reject("missing 4DH versions".to_owned()))?;
                let major = |version: u32| version & 0xff00;
                let minor = |version: u32| version & 0x00ff;
                if major(offered) != major(current.local) || minor(offered) < minor(current.local) {
                    return Err(SessionError::Reject(format!(
                        "invalid offered version {offered:#x}"
                    )));
                }
                if major(applied) != major(current.remote) || minor(applied) < minor(current.remote) {
                    return Err(SessionError::Reject(format!(
                        "invalid applied version {applied:#x}"
                    )));
                }
                let supported_offered = supported_version_within(offered)
                    .ok_or_else(|| SessionError::Reject("unsupported offered major version".to_owned()))?;
                let new_local = offered.min(supported_offered);
                let supported_applied = supported_version_within(applied)
                    .ok_or_else(|| SessionError::Reject("unsupported applied major version".to_owned()))?;
                if applied > supported_applied {
                    return Err(SessionError::Reject(format!(
                        "unsupported applied version {applied:#x}"
                    )));
                }
                Ok(IncomingVersions {
                    offered,
                    applied,
                    pending: Some(Versions {
                        local: new_local,
                        remote: applied,
                    }),
                })
            },
        }
    }
}

fn salt(prefix: &[u8], identity: ThreemaId) -> Vec<u8> {
    let mut salt = prefix.to_vec();
    salt.extend_from_slice(identity.as_str().as_bytes());
    salt
}

/// 4DH: parts in initiator order. Returns (my chain key, peer chain key).
fn derive_4dh(
    parts: &mut [[u8; 32]; 4],
    my_identity: ThreemaId,
    peer_identity: ThreemaId,
) -> ([u8; 32], [u8; 32]) {
    let mut input = concat_and_wipe(parts);
    let mut intermediate: [u8; 64] = Blake2bMac512::new_with_salt_and_personal(None, &[], &[])
        .expect("Blake2bMac512 without key must not fail")
        .chain_update(&input)
        .finalize_fixed()
        .into();
    input.zeroize();
    let mine = blake2b_256(&intermediate, &salt(b"ke-4dh-", my_identity));
    let peers = blake2b_256(&intermediate, &salt(b"ke-4dh-", peer_identity));
    intermediate.zeroize();
    (mine, peers)
}

/// Encrypt the inner message (`type || body`, no padding) with a ratchet's current key and a
/// zero nonce. Every key is used exactly once, which is why the ratchet must be turned (and the
/// turn persisted) right after.
pub(crate) fn encrypt_inner(ratchet: &Ratchet, plaintext: &[u8]) -> Vec<u8> {
    let mut key = ratchet.encryption_key();
    let cipher = salsa20::XSalsa20Poly1305::new(&key.into());
    key.zeroize();
    cipher
        .encrypt(&[0_u8; 24].into(), plaintext)
        .expect("XSalsa20Poly1305 encryption must not fail")
}

/// Decrypt an encapsulated inner message with a ratchet's current key and a zero nonce.
pub(crate) fn decrypt_inner(ratchet: &Ratchet, ciphertext: &[u8]) -> Result<Vec<u8>, SessionError> {
    let mut key = ratchet.encryption_key();
    let cipher = salsa20::XSalsa20Poly1305::new(&key.into());
    key.zeroize();
    cipher
        .decrypt(&[0_u8; 24].into(), ciphertext)
        .map_err(|_| SessionError::DecryptionFailed)
}

#[cfg(test)]
mod tests {
    use data_encoding::HEXLOWER;

    use super::*;

    // Fixed identities from Android's `DummyUsers.java`.
    fn alice() -> (ThreemaId, ClientKey) {
        key(
            "000ALICE",
            "6eda2ebb8527ff5bd0e8719602f710c13e162a3be612de0ad2a2ff66f5050630",
        )
    }
    fn bob() -> (ThreemaId, ClientKey) {
        key(
            "00000BOB",
            "533058227925006d86bb8dd88b0442ed73fbc49216b6e94b0870a7761d979eca",
        )
    }
    fn key(id: &str, hex: &str) -> (ThreemaId, ClientKey) {
        let bytes: [u8; 32] = HEXLOWER.decode(hex.as_bytes()).unwrap().try_into().unwrap();
        (ThreemaId::try_from(id).unwrap(), ClientKey::from(bytes))
    }

    #[test]
    fn full_handshake_keys_match() {
        let (alice_id, alice_key) = alice();
        let (bob_id, bob_key) = bob();

        // Alice initiates.
        let mut alice = Session::new_initiator(alice_id, &alice_key, bob_id, &bob_key.public_key()).unwrap();
        assert_eq!(alice.state(), Ok(SessionState::L20));

        // Bob accepts.
        let mut bob = Session::new_responder(
            alice.id,
            bob_id,
            &bob_key,
            alice_id,
            &alice_key.public_key(),
            &alice.my_ephemeral_public_key,
            Some(&supported_version_range()),
        )
        .unwrap();
        assert_eq!(bob.state(), Ok(SessionState::R24));

        // A 2DH message from Alice decrypts at Bob.
        let ciphertext = encrypt_inner(alice.my_ratchet_2dh.as_ref().unwrap(), b"\x01hello");
        let plaintext = decrypt_inner(bob.peer_ratchet_2dh.as_ref().unwrap(), &ciphertext).unwrap();
        assert_eq!(plaintext, b"\x01hello");

        // Alice processes the Accept.
        alice
            .process_accept(
                &alice_key,
                &bob_key.public_key(),
                &bob.my_ephemeral_public_key,
                Some(&supported_version_range()),
            )
            .unwrap();
        assert_eq!(alice.state(), Ok(SessionState::RL44));
        assert!(alice.my_ephemeral_private_key.is_none());
        assert_eq!(alice.outgoing_applied_version(), Ok(Version::V12 as u32));
        // Processing twice fails.
        assert_eq!(
            alice.process_accept(
                &alice_key,
                &bob_key.public_key(),
                &bob.my_ephemeral_public_key,
                None
            ),
            Err(SessionError::MissingEphemeralPrivateKey)
        );

        // 4DH both ways, directions differ.
        let alice_to_bob = alice.my_ratchet_4dh.as_ref().unwrap();
        let bob_to_alice = bob.my_ratchet_4dh.as_ref().unwrap();
        assert_eq!(
            alice_to_bob.chain_key,
            bob.peer_ratchet_4dh.as_ref().unwrap().chain_key
        );
        assert_eq!(
            bob_to_alice.chain_key,
            alice.peer_ratchet_4dh.as_ref().unwrap().chain_key
        );
        assert_ne!(alice_to_bob.chain_key, bob_to_alice.chain_key);

        // After a 4DH message Bob drops the 2DH ratchet.
        bob.peer_ratchet_2dh = None;
        assert_eq!(bob.state(), Ok(SessionState::RL44));
    }

    #[test]
    fn version_negotiation() {
        assert_eq!(negotiate_version(None), Ok(Version::V10 as u32));
        assert_eq!(
            negotiate_version(Some(&VersionRange {
                min: 0x0100,
                max: 0x0101
            })),
            Ok(0x0101)
        );
        assert_eq!(
            negotiate_version(Some(&VersionRange {
                min: 0x0100,
                max: 0x01ff
            })),
            Ok(0x0102)
        );
        assert!(
            negotiate_version(Some(&VersionRange {
                min: 0x0200,
                max: 0x0201
            }))
            .is_err()
        );
        assert!(
            negotiate_version(Some(&VersionRange {
                min: 0x0102,
                max: 0x0100
            }))
            .is_err()
        );
    }

    #[test]
    fn incoming_version_rules() {
        let (alice_id, alice_key) = alice();
        let (bob_id, bob_key) = bob();
        let alice = Session::new_initiator(alice_id, &alice_key, bob_id, &bob_key.public_key()).unwrap();
        let bob = Session::new_responder(
            alice.id,
            bob_id,
            &bob_key,
            alice_id,
            &alice_key.public_key(),
            &alice.my_ephemeral_public_key,
            None,
        )
        .unwrap();
        // 2DH is clamped to 1.0 and allowed in R24.
        assert_eq!(
            bob.process_incoming_versions(DhType::TwoDh, 0x0102, 0x0102)
                .unwrap()
                .applied,
            0x0100
        );
        // 4DH with a valid upgrade offer.
        let versions = bob
            .process_incoming_versions(DhType::FourDh, 0x0102, 0x0100)
            .unwrap();
        assert_eq!(
            versions.pending,
            Some(Versions {
                local: 0x0102,
                remote: 0x0100
            })
        );
        // Applied > offered is rejected.
        assert!(
            bob.process_incoming_versions(DhType::FourDh, 0x0100, 0x0101)
                .is_err()
        );
        // The initiator in L20 must not receive anything.
        assert!(alice.process_incoming_versions(DhType::TwoDh, 0, 0).is_err());
    }
}
