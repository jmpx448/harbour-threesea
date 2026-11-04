// SPDX-License-Identifier: AGPL-3.0-only
//! The handshake between two participants of a group call and their encrypted messages
//! afterwards (`group-call.proto` "Participant to Participant Flow", Android's `P2PHandshake`),
//! sans-I/O: [`Peer`] takes the relayed bytes from the other participant and gives back what to
//! relay to it. Only the normal (non-guest) handshake: calls scoped to a group allow no guests.
//!
//! - Hello: XSalsa20-Poly1305 under GCHK with a random nonce in front.
//! - Auth: inside, XSalsa20-Poly1305 under GCNHAK (from both client keys) with a random inner
//!   nonce; outside, under the ephemeral keys (PCK) with `PCCK || PCSN` as nonce.
//! - Afterwards: `Envelope`s under the ephemeral keys with `PCCK || PCSN`.

use crypto_secretbox::{
    KeyInit as _, XSalsa20Poly1305,
    aead::{Aead as _, AeadCore as _, OsRng},
};
use libthreema::{
    common::keys::{ClientKey, PublicKey},
    protobuf::group_call::participant_to_participant::{
        Envelope, MediaKey as ProtoMediaKey,
        handshake::{Auth, AuthEnvelope, Hello, HelloEnvelope, auth_envelope, hello_envelope},
    },
};
use prost::Message as _;
use zeroize::Zeroizing;

use crate::group_calls::blake2b;

const COOKIE_LEN: usize = 16;
const NONCE_LEN: usize = 24;

/// A participant's media key (PCMK) state: what it encrypts its media frames with.
#[derive(Clone, PartialEq, Eq)]
pub struct MediaKey {
    pub epoch: u8,
    pub ratchet: u8,
    pub pcmk: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for MediaKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaKey")
            .field("epoch", &self.epoch)
            .field("ratchet", &self.ratchet)
            .finish_non_exhaustive()
    }
}

impl MediaKey {
    /// A new random key (when joining, or after someone left: `epoch` one up).
    #[must_use]
    pub fn random(epoch: u8) -> Self {
        Self {
            epoch,
            ratchet: 0,
            pcmk: Zeroizing::new(rand::random()),
        }
    }

    /// `PCMK' = BLAKE2b(key=PCMK, salt="m'", personal='3ma-call')`, when someone joined; `None`
    /// after 255 rounds (the call has to end then).
    #[must_use]
    pub fn ratcheted(&self) -> Option<Self> {
        Some(Self {
            epoch: self.epoch,
            ratchet: self.ratchet.checked_add(1)?,
            pcmk: Zeroizing::new(blake2b(self.pcmk.as_slice(), b"m'", &[])),
        })
    }

    fn to_proto(&self) -> ProtoMediaKey {
        ProtoMediaKey {
            epoch: u32::from(self.epoch),
            ratchet_counter: u32::from(self.ratchet),
            pcmk: self.pcmk.to_vec(),
        }
    }

    fn from_proto(key: &ProtoMediaKey) -> Option<Self> {
        Some(Self {
            epoch: u8::try_from(key.epoch).ok()?,
            ratchet: u8::try_from(key.ratchet_counter).ok()?,
            pcmk: Zeroizing::new(key.pcmk.as_slice().try_into().ok()?),
        })
    }
}

/// The `rekey` envelope announcing our next media key.
#[must_use]
pub fn rekey(key: &MediaKey) -> Envelope {
    use libthreema::protobuf::group_call::participant_to_participant::envelope::Content;
    Envelope {
        content: Some(Content::Rekey(key.to_proto())),
        ..Default::default()
    }
}

/// A media key from a rekey message (`None`: invalid).
#[must_use]
pub fn media_key(key: &ProtoMediaKey) -> Option<MediaKey> {
    MediaKey::from_proto(key)
}

/// What we bring to every handshake.
pub struct Local<'a> {
    pub identity: &'a str,
    pub nickname: &'a str,
    pub client_key: &'a ClientKey,
    pub gckh: &'a [u8; 32],
    pub gchk: &'a [u8; 32],
    /// Our applied media key, then a pending one (if any): sent in `Auth`.
    pub media_keys: &'a [MediaKey],
}

/// What the app tells us about the other participant's claimed identity.
pub struct Known {
    /// Its long-term public key (the contact's, or ours for our own other device).
    pub public_key: PublicKey,
    /// A member of the call's group (or the user).
    pub member: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// We joined and sent Hello; the existing participant's Hello comes next.
    AwaitExistingHello,
    /// It joined after us: its Hello comes first.
    AwaitNewcomerHello,
    AwaitAuth,
    Done,
}

/// What came out of a received message.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// Relay these bytes (an `OuterEnvelope.encrypted_data`) to the participant.
    Send(Vec<u8>),
    /// The handshake is complete: who it is and the keys of its media.
    Authenticated {
        identity: String,
        nickname: String,
        media_keys: Vec<MediaKey>,
    },
    /// A message after the handshake.
    Message(Envelope),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HandshakeError {
    #[error("undecryptable or invalid {0}")]
    Invalid(&'static str),
    #[error("{0} in state {1:?}")]
    Unexpected(&'static str, State),
    #[error("identity not allowed in this call")]
    NotMember,
}

struct Remote {
    pck: [u8; 32],
    pcck: [u8; COOKIE_LEN],
    /// The next sequence number expected from it.
    pcsn: u64,
    /// `X25519HSalsa20(our PCK, its PCK)`.
    key: Zeroizing<[u8; 32]>,
    /// GCNHAK: from both client keys and GCKH.
    gcnhak: Zeroizing<[u8; 32]>,
    identity: String,
    nickname: String,
}

/// One other participant: the handshake with it and the messages after.
pub struct Peer {
    pub id: u32,
    state: State,
    pck: ClientKey,
    pcck: [u8; COOKIE_LEN],
    /// Our next sequence number towards it (starts at 1).
    pcsn: u64,
    remote: Option<Remote>,
}

/// The 32 bytes of a public key (libthreema shows them only as hex).
pub(crate) fn public_bytes(key: &PublicKey) -> [u8; 32] {
    let mut bytes = [0; 32];
    let hex = key.to_string();
    if let Ok(decoded) = data_encoding::HEXLOWER.decode(hex.as_bytes())
        && decoded.len() == 32
    {
        bytes.copy_from_slice(&decoded);
    }
    bytes
}

fn cipher(key: &[u8; 32]) -> XSalsa20Poly1305 {
    XSalsa20Poly1305::new(key.into())
}

/// A random amount of protobuf padding (the fields say: random, ignored by the receiver).
fn padding() -> Vec<u8> {
    vec![0; usize::from(rand::random::<u8>() % 32)]
}

fn nonce(cookie: &[u8; COOKIE_LEN], sequence: u64) -> [u8; NONCE_LEN] {
    let mut nonce = [0; NONCE_LEN];
    nonce[..COOKIE_LEN].copy_from_slice(cookie);
    nonce[COOKIE_LEN..].copy_from_slice(&sequence.to_le_bytes());
    nonce
}

impl Peer {
    /// We just joined: the handshake starts with our Hello to `id`.
    pub fn joined(id: u32, local: &Local<'_>) -> (Self, Vec<u8>) {
        let peer = Self::new(id, State::AwaitExistingHello);
        let hello = peer.hello(local);
        (peer, hello)
    }

    /// `id` joined after us: its Hello comes first.
    #[must_use]
    pub fn newcomer(id: u32) -> Self {
        Self::new(id, State::AwaitNewcomerHello)
    }

    fn new(id: u32, state: State) -> Self {
        Self {
            id,
            state,
            pck: ClientKey::random(),
            pcck: rand::random(),
            pcsn: 1,
            remote: None,
        }
    }

    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    /// The other participant's identity and nickname once authenticated.
    #[must_use]
    /// A participant the handshake is done with, whose keys don't matter to the test.
    #[cfg(test)]
    pub(crate) fn authenticated_for_test(id: u32, identity: &str) -> Self {
        let mut peer = Self::new(id, State::Done);
        peer.remote = Some(Remote {
            pck: [0; 32],
            pcck: [0; COOKIE_LEN],
            pcsn: 1,
            key: Zeroizing::new([0; 32]),
            gcnhak: Zeroizing::new([0; 32]),
            identity: identity.to_owned(),
            nickname: String::new(),
        });
        peer
    }

    pub fn identity(&self) -> Option<(&str, &str)> {
        if self.state != State::Done {
            return None;
        }
        self.remote
            .as_ref()
            .map(|remote| (remote.identity.as_str(), remote.nickname.as_str()))
    }

    fn hello(&self, local: &Local<'_>) -> Vec<u8> {
        #[expect(
            deprecated,
            reason = "padding as the protocol describes it, random length"
        )]
        let envelope = HelloEnvelope {
            padding: padding(),
            content: Some(hello_envelope::Content::Hello(Hello {
                identity: local.identity.to_owned(),
                nickname: local.nickname.to_owned(),
                pck: public_bytes(&self.pck.public_key()).to_vec(),
                pcck: self.pcck.to_vec(),
            })),
        };
        let nonce = XSalsa20Poly1305::generate_nonce(&mut OsRng);
        let sealed = cipher(local.gchk)
            .encrypt(&nonce, envelope.encode_to_vec().as_slice())
            .expect("encrypting a hello");
        [nonce.as_slice(), &sealed].concat()
    }

    /// Seal for the other participant with our ephemeral key and the next `PCCK || PCSN`.
    fn seal(&mut self, plaintext: &[u8]) -> Option<Vec<u8>> {
        let key = self.remote.as_ref()?.key.clone();
        let nonce = nonce(&self.pcck, self.pcsn);
        self.pcsn += 1;
        cipher(&key).encrypt((&nonce).into(), plaintext).ok()
    }

    /// Open from the other participant with its `PCCK` and the expected `PCSN`.
    fn open(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let remote = self.remote.as_mut()?;
        let nonce = nonce(&remote.pcck, remote.pcsn);
        let plaintext = cipher(&remote.key).decrypt((&nonce).into(), data).ok()?;
        remote.pcsn += 1;
        Some(plaintext)
    }

    fn auth(&mut self, local: &Local<'_>) -> Option<Vec<u8>> {
        let remote = self.remote.as_ref()?;
        #[expect(
            deprecated,
            reason = "padding as the protocol describes it, random length"
        )]
        let envelope = AuthEnvelope {
            padding: padding(),
            content: Some(auth_envelope::Content::Auth(Auth {
                // The other side's own values from its Hello, repeated.
                pck: remote.pck.to_vec(),
                pcck: remote.pcck.to_vec(),
                media_keys: local.media_keys.iter().map(MediaKey::to_proto).collect(),
            })),
        };
        let inner_nonce = XSalsa20Poly1305::generate_nonce(&mut OsRng);
        let inner = cipher(&remote.gcnhak)
            .encrypt(&inner_nonce, envelope.encode_to_vec().as_slice())
            .ok()?;
        self.seal(&[inner_nonce.as_slice(), &inner].concat())
    }

    /// A message relayed from this participant (`OuterEnvelope.encrypted_data`); `lookup` tells
    /// about a claimed identity (`None`: unknown, refused).
    pub fn receive(
        &mut self,
        data: &[u8],
        local: &Local<'_>,
        lookup: &dyn Fn(&str) -> Option<Known>,
    ) -> Result<Vec<Event>, HandshakeError> {
        match self.state {
            State::AwaitExistingHello | State::AwaitNewcomerHello => {
                self.receive_hello(data, local, lookup)
            }
            State::AwaitAuth => self.receive_auth(data),
            State::Done => {
                let plaintext = self.open(data).ok_or(HandshakeError::Invalid("message"))?;
                let envelope = Envelope::decode(plaintext.as_slice())
                    .map_err(|_| HandshakeError::Invalid("message"))?;
                Ok(vec![Event::Message(envelope)])
            }
        }
    }

    fn receive_hello(
        &mut self,
        data: &[u8],
        local: &Local<'_>,
        lookup: &dyn Fn(&str) -> Option<Known>,
    ) -> Result<Vec<Event>, HandshakeError> {
        if data.len() < NONCE_LEN {
            return Err(HandshakeError::Invalid("hello"));
        }
        let (nonce, sealed) = data.split_at(NONCE_LEN);
        let plaintext = cipher(local.gchk)
            .decrypt(nonce.into(), sealed)
            .map_err(|_| HandshakeError::Invalid("hello"))?;
        let Some(hello_envelope::Content::Hello(hello)) =
            HelloEnvelope::decode(plaintext.as_slice())
                .map_err(|_| HandshakeError::Invalid("hello"))?
                .content
        else {
            // A guest hello: no guests in calls scoped to a group.
            return Err(HandshakeError::Invalid("hello (guest)"));
        };
        let pck: [u8; 32] = hello
            .pck
            .as_slice()
            .try_into()
            .map_err(|_| HandshakeError::Invalid("hello"))?;
        let pcck: [u8; COOKIE_LEN] = hello
            .pcck
            .as_slice()
            .try_into()
            .map_err(|_| HandshakeError::Invalid("hello"))?;
        // Our own values reflected: a replay.
        if pck == public_bytes(&self.pck.public_key()) || pcck == self.pcck {
            return Err(HandshakeError::Invalid("hello (reflected)"));
        }
        let known = lookup(&hello.identity).ok_or(HandshakeError::NotMember)?;
        if !known.member {
            return Err(HandshakeError::NotMember);
        }
        let key = self
            .pck
            .derive_shared_secret(&PublicKey::from(pck))
            .ok_or(HandshakeError::Invalid("hello (key)"))?;
        let shared = local
            .client_key
            .derive_shared_secret(&known.public_key)
            .ok_or(HandshakeError::Invalid("hello (identity key)"))?;
        let gcnhak = Zeroizing::new(blake2b(shared.as_slice(), b"nha", &[local.gckh]));
        self.remote = Some(Remote {
            pck,
            pcck,
            pcsn: 1,
            key,
            gcnhak,
            identity: hello.identity,
            nickname: hello.nickname,
        });
        let mut events = Vec::new();
        // A newcomer gets our Hello first, then our Auth.
        if self.state == State::AwaitNewcomerHello {
            events.push(Event::Send(self.hello(local)));
        }
        events.push(Event::Send(
            self.auth(local)
                .ok_or(HandshakeError::Invalid("auth (create)"))?,
        ));
        self.state = State::AwaitAuth;
        Ok(events)
    }

    fn receive_auth(&mut self, data: &[u8]) -> Result<Vec<Event>, HandshakeError> {
        let outer = self.open(data).ok_or(HandshakeError::Invalid("auth"))?;
        if outer.len() < NONCE_LEN {
            return Err(HandshakeError::Invalid("auth"));
        }
        let remote = self
            .remote
            .as_ref()
            .ok_or(HandshakeError::Unexpected("auth", self.state))?;
        let (inner_nonce, inner) = outer.split_at(NONCE_LEN);
        let plaintext = cipher(&remote.gcnhak)
            .decrypt(inner_nonce.into(), inner)
            .map_err(|_| HandshakeError::Invalid("auth (identity)"))?;
        let Some(auth_envelope::Content::Auth(auth)) = AuthEnvelope::decode(plaintext.as_slice())
            .map_err(|_| HandshakeError::Invalid("auth"))?
            .content
        else {
            return Err(HandshakeError::Invalid("auth (guest)"));
        };
        // Our own Hello's values, repeated (no replay of an older handshake).
        if auth.pck != public_bytes(&self.pck.public_key()) || auth.pcck != self.pcck {
            return Err(HandshakeError::Invalid("auth (repeated values)"));
        }
        let media_keys: Vec<MediaKey> = auth
            .media_keys
            .iter()
            .filter_map(MediaKey::from_proto)
            .collect();
        if media_keys.is_empty() {
            return Err(HandshakeError::Invalid("auth (no media key)"));
        }
        self.state = State::Done;
        Ok(vec![Event::Authenticated {
            identity: remote.identity.clone(),
            nickname: remote.nickname.clone(),
            media_keys,
        }])
    }

    /// An `Envelope` for the participant after the handshake (rekey, capture state).
    pub fn envelope(&mut self, mut envelope: Envelope) -> Option<Vec<u8>> {
        if self.state != State::Done {
            return None;
        }
        #[expect(
            deprecated,
            reason = "padding as the protocol describes it, random length"
        )]
        {
            envelope.padding = padding();
        }
        self.seal(&envelope.encode_to_vec())
    }
}

#[cfg(test)]
mod tests {
    use libthreema::protobuf::group_call::participant_to_participant::envelope::Content;

    use super::*;

    struct Side {
        identity: &'static str,
        client_key: ClientKey,
        media: Vec<MediaKey>,
    }

    fn local<'a>(side: &'a Side, gckh: &'a [u8; 32], gchk: &'a [u8; 32]) -> Local<'a> {
        Local {
            identity: side.identity,
            nickname: "Nick",
            client_key: &side.client_key,
            gckh,
            gchk,
            media_keys: &side.media,
        }
    }

    fn sends(events: &[Event]) -> Vec<Vec<u8>> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Send(data) => Some(data.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn newcomer_and_existing_participant_authenticate() {
        let (gckh, gchk) = ([1; 32], [2; 32]);
        let newcomer = Side {
            identity: "NEWCOMER",
            client_key: ClientKey::random(),
            media: vec![MediaKey::random(0)],
        };
        let existing = Side {
            identity: "EXISTING",
            client_key: ClientKey::random(),
            media: vec![MediaKey::random(3)],
        };
        let (newcomer_pub, existing_pub) = (
            newcomer.client_key.public_key(),
            existing.client_key.public_key(),
        );
        let lookup_for_newcomer = move |identity: &str| {
            (identity == "EXISTING").then_some(Known {
                public_key: existing_pub,
                member: true,
            })
        };
        let lookup_for_existing = move |identity: &str| {
            (identity == "NEWCOMER").then_some(Known {
                public_key: newcomer_pub,
                member: true,
            })
        };

        // The newcomer says Hello; the existing participant answers Hello + Auth.
        let (mut at_newcomer, hello) = Peer::joined(7, &local(&newcomer, &gckh, &gchk));
        let mut at_existing = Peer::newcomer(9);
        let replies = sends(
            &at_existing
                .receive(
                    &hello,
                    &local(&existing, &gckh, &gchk),
                    &lookup_for_existing,
                )
                .unwrap(),
        );
        assert_eq!(replies.len(), 2);
        assert_eq!(at_existing.state(), State::AwaitAuth);

        // The newcomer: Hello → its Auth; Auth → authenticated.
        let auth = sends(
            &at_newcomer
                .receive(
                    &replies[0],
                    &local(&newcomer, &gckh, &gchk),
                    &lookup_for_newcomer,
                )
                .unwrap(),
        );
        assert_eq!(auth.len(), 1);
        let events = at_newcomer
            .receive(
                &replies[1],
                &local(&newcomer, &gckh, &gchk),
                &lookup_for_newcomer,
            )
            .unwrap();
        assert!(
            matches!(&events[..], [Event::Authenticated { identity, media_keys, .. }]
            if identity == "EXISTING" && media_keys[0] == existing.media[0])
        );

        // The existing participant takes the newcomer's Auth.
        let events = at_existing
            .receive(
                &auth[0],
                &local(&existing, &gckh, &gchk),
                &lookup_for_existing,
            )
            .unwrap();
        assert!(
            matches!(&events[..], [Event::Authenticated { identity, media_keys, .. }]
            if identity == "NEWCOMER" && media_keys[0] == newcomer.media[0])
        );
        assert_eq!(at_existing.identity(), Some(("NEWCOMER", "Nick")));

        // Afterwards: envelopes both ways, in order.
        let rekey = MediaKey::random(1);
        let sealed = at_newcomer
            .envelope(Envelope {
                content: Some(Content::Rekey(rekey.to_proto())),
                ..Default::default()
            })
            .unwrap();
        let events = at_existing
            .receive(
                &sealed,
                &local(&existing, &gckh, &gchk),
                &lookup_for_existing,
            )
            .unwrap();
        assert!(
            matches!(&events[..], [Event::Message(Envelope { content: Some(Content::Rekey(key)), .. })]
            if MediaKey::from_proto(key) == Some(rekey.clone()))
        );
        // The same message again (replayed by the SFU): refused.
        assert!(
            at_existing
                .receive(
                    &sealed,
                    &local(&existing, &gckh, &gchk),
                    &lookup_for_existing
                )
                .is_err()
        );
    }

    #[test]
    fn strangers_and_wrong_keys_are_refused() {
        let (gckh, gchk) = ([1; 32], [2; 32]);
        let stranger = Side {
            identity: "STRANGER",
            client_key: ClientKey::random(),
            media: vec![MediaKey::random(0)],
        };
        let me = Side {
            identity: "MEMEMEME",
            client_key: ClientKey::random(),
            media: vec![MediaKey::random(0)],
        };
        let (_, hello) = Peer::joined(1, &local(&stranger, &gckh, &gchk));
        let mut peer = Peer::newcomer(1);
        let not_member = |_: &str| {
            Some(Known {
                public_key: ClientKey::random().public_key(),
                member: false,
            })
        };
        assert_eq!(
            peer.receive(&hello, &local(&me, &gckh, &gchk), &not_member)
                .unwrap_err(),
            HandshakeError::NotMember
        );
        // Another call's GCHK: undecryptable.
        let member = |_: &str| {
            Some(Known {
                public_key: ClientKey::random().public_key(),
                member: true,
            })
        };
        assert!(
            peer.receive(&hello, &local(&me, &gckh, &[3; 32]), &member)
                .is_err()
        );
    }

    #[test]
    fn auth_from_a_wrong_identity_key_fails() {
        // The Hello claims an identity whose real key we know differently: its Auth can't open.
        let (gckh, gchk) = ([1; 32], [2; 32]);
        let newcomer = Side {
            identity: "NEWCOMER",
            client_key: ClientKey::random(),
            media: vec![MediaKey::random(0)],
        };
        let existing = Side {
            identity: "EXISTING",
            client_key: ClientKey::random(),
            media: vec![MediaKey::random(0)],
        };
        let wrong = ClientKey::random().public_key();
        let newcomer_pub = newcomer.client_key.public_key();
        let (mut at_newcomer, hello) = Peer::joined(7, &local(&newcomer, &gckh, &gchk));
        let mut at_existing = Peer::newcomer(9);
        let replies = sends(
            &at_existing
                .receive(&hello, &local(&existing, &gckh, &gchk), &move |_| {
                    Some(Known {
                        public_key: newcomer_pub,
                        member: true,
                    })
                })
                .unwrap(),
        );
        let lookup_wrong = move |_: &str| {
            Some(Known {
                public_key: wrong,
                member: true,
            })
        };
        at_newcomer
            .receive(&replies[0], &local(&newcomer, &gckh, &gchk), &lookup_wrong)
            .unwrap();
        assert_eq!(
            at_newcomer
                .receive(&replies[1], &local(&newcomer, &gckh, &gchk), &lookup_wrong)
                .unwrap_err(),
            HandshakeError::Invalid("auth (identity)")
        );
    }

    #[test]
    fn media_key_ratchet() {
        let key = MediaKey::random(2);
        let next = key.ratcheted().unwrap();
        assert_eq!((next.epoch, next.ratchet), (2, 1));
        assert_ne!(next.pcmk, key.pcmk);
        let last = MediaKey {
            ratchet: 255,
            ..key
        };
        assert!(last.ratcheted().is_none());
    }
}
