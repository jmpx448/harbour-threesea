// SPDX-License-Identifier: AGPL-3.0-only
//! A SaltyRTC responder for Threema Web (ADR 0013, docs/research/threema-web.md §1–2), sans-I/O:
//! WebSocket frames in, frames to send and events out. The browser is the initiator; we offer
//! only the relayed-data task, so after the handshake every payload goes NaCl-boxed with the
//! session keys through the signalling server.
//!
//! Every frame is a 24-byte nonce (cookie 16, source, destination, overflow u16, sequence u32;
//! big endian) and a msgpack map, encrypted (NaCl box or secretbox) except for 'server-hello'
//! and 'client-hello' (saltyrtc-meta Protocol.md).

use crypto_secretbox::{KeyInit as _, XSalsa20Poly1305, aead::Aead as _};
use libthreema::common::keys::{ClientKey, PublicKey};
use rmpv::Value;
use zeroize::Zeroizing;

use crate::group_call_handshake::public_bytes;

/// The WebSocket subprotocol.
pub const SUBPROTOCOL: &str = "v1.saltyrtc.org";
/// The only task we offer.
pub const TASK: &str = "v0.relayed-data.tasks.saltyrtc.org";
/// How often the server should ping us (seconds; Android asks for 60).
const PING_INTERVAL: u64 = 60;

const SERVER: u8 = 0x00;
const INITIATOR: u8 = 0x01;
/// The connection ID's nonce (docs/research/threema-web.md §5).
const CONNECTION_ID_NONCE: &[u8; 24] = b"connectionidconnectionid";

/// Close codes (Protocol.md "Close Code Enumeration").
pub const CLOSE_GOING_AWAY: u16 = 1001;
pub const CLOSE_PROTOCOL_ERROR: u16 = 3001;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// Close the WebSocket with 3001 (the peer or server broke the protocol).
    #[error("protocol error: {0}")]
    Protocol(&'static str),
}

/// What the caller has to do.
#[derive(Debug, PartialEq)]
pub enum Event {
    /// Send this frame (binary WebSocket message).
    Send(Vec<u8>),
    /// The browser is authenticated, the relayed-data task runs.
    PeerConnected,
    /// A 'data' message's payload.
    Data(Value),
    /// The browser closed the session (close code).
    PeerClosed(u16),
    /// The browser left the server (it may come back as a new initiator).
    PeerGone,
}

/// A frame's nonce.
#[derive(Debug, Clone, Copy)]
struct Nonce {
    cookie: [u8; 16],
    source: u8,
    destination: u8,
    /// Overflow and sequence number as one 48-bit number.
    csn: u64,
}

impl Nonce {
    fn parse(frame: &[u8]) -> Option<(Self, [u8; 24], &[u8])> {
        if frame.len() <= 24 {
            return None;
        }
        let (head, data) = frame.split_at(24);
        let bytes: [u8; 24] = head.try_into().ok()?;
        let mut cookie = [0; 16];
        cookie.copy_from_slice(&bytes[..16]);
        let mut csn = [0; 8];
        csn[2..].copy_from_slice(&bytes[18..24]);
        Some((
            Self {
                cookie,
                source: bytes[16],
                destination: bytes[17],
                csn: u64::from_be_bytes(csn),
            },
            bytes,
            data,
        ))
    }

    fn bytes(&self) -> [u8; 24] {
        let mut bytes = [0; 24];
        bytes[..16].copy_from_slice(&self.cookie);
        bytes[16] = self.source;
        bytes[17] = self.destination;
        bytes[18..].copy_from_slice(&self.csn.to_be_bytes()[2..]);
        bytes
    }
}

/// Our cookie and next combined sequence number towards one peer.
struct Outgoing {
    cookie: [u8; 16],
    csn: u64,
}

impl Outgoing {
    fn new() -> Self {
        Self {
            cookie: rand::random(),
            csn: u64::from(rand::random::<u32>()),
        }
    }

    fn next(&mut self) -> Result<u64, Error> {
        let csn = self.csn;
        self.csn += 1;
        if self.csn >= 1 << 48 {
            return Err(Error::Protocol("sequence number exhausted"));
        }
        Ok(csn)
    }
}

/// The cookie and last combined sequence number seen from one peer.
struct Incoming {
    cookie: [u8; 16],
    csn: u64,
}

/// The receive checks of Protocol.md "Receiving a Signalling Message".
fn check_incoming(
    seen: &mut Option<Incoming>,
    own_cookie: &[u8; 16],
    nonce: &Nonce,
) -> Result<(), Error> {
    match seen {
        None => {
            if nonce.csn >> 32 != 0 {
                return Err(Error::Protocol("overflow number not 0"));
            }
            if nonce.cookie == *own_cookie {
                return Err(Error::Protocol("peer uses our cookie"));
            }
            *seen = Some(Incoming {
                cookie: nonce.cookie,
                csn: nonce.csn,
            });
        }
        Some(last) => {
            if last.cookie != nonce.cookie {
                return Err(Error::Protocol("cookie changed"));
            }
            if nonce.csn != last.csn + 1 {
                return Err(Error::Protocol("sequence number not incremented"));
            }
            last.csn = nonce.csn;
        }
    }
    Ok(())
}

/// NaCl box (`crypto_box`): XSalsa20-Poly1305 under the X25519-HSalsa20 shared key.
fn box_key(secret: &ClientKey, public: &PublicKey) -> Result<Zeroizing<[u8; 32]>, Error> {
    secret
        .derive_shared_secret(public)
        .ok_or(Error::Protocol("non-contributory key"))
}

fn seal(key: &[u8; 32], nonce: &[u8; 24], plain: &[u8]) -> Vec<u8> {
    XSalsa20Poly1305::new(key.into())
        .encrypt(nonce.into(), plain)
        .expect("XSalsa20-Poly1305 encryption does not fail")
}

fn open(
    key: &[u8; 32],
    nonce: &[u8; 24],
    sealed: &[u8],
    what: &'static str,
) -> Result<Vec<u8>, Error> {
    XSalsa20Poly1305::new(key.into())
        .decrypt(nonce.into(), sealed)
        .map_err(|_| Error::Protocol(what))
}

/// A msgpack map with string keys.
fn map(fields: Vec<(&str, Value)>) -> Vec<u8> {
    let value = Value::Map(
        fields
            .into_iter()
            .map(|(key, value)| (Value::from(key), value))
            .collect(),
    );
    let mut out = Vec::new();
    rmpv::encode::write_value(&mut out, &value).expect("writing to a Vec does not fail");
    out
}

fn decode(bytes: &[u8], what: &'static str) -> Result<Value, Error> {
    // SaltyRTC's own messages are small: a tight budget before decoding (deep audit D03).
    if !super::budget::within(bytes, 10_000, 16) {
        return Err(Error::Protocol(what));
    }
    rmpv::decode::read_value(&mut &bytes[..]).map_err(|_| Error::Protocol(what))
}

/// A field of a msgpack map.
pub(crate) fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    value
        .as_map()?
        .iter()
        .find(|(key, _)| key.as_str() == Some(name))
        .map(|(_, value)| value)
}

fn message_type(value: &Value) -> Option<&str> {
    field(value, "type")?.as_str()
}

fn key_field(value: &Value) -> Result<[u8; 32], Error> {
    field(value, "key")
        .and_then(Value::as_slice)
        .and_then(|key| key.try_into().ok())
        .ok_or(Error::Protocol("invalid key"))
}

fn cookie_field(value: &Value) -> Option<[u8; 16]> {
    field(value, "your_cookie")?.as_slice()?.try_into().ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    ServerHello,
    ServerAuth,
    /// Authenticated towards the server; the browser handshake, if any, is in `peer`.
    Server,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerPhase {
    /// Our 'key' (after 'token') is out; waiting for the browser's.
    Key,
    /// Our 'auth' is out; waiting for the browser's.
    Auth,
    /// The relayed-data task runs.
    Task,
}

/// The browser on this connection.
struct Peer {
    out: Outgoing,
    seen: Option<Incoming>,
    /// Our session key pair towards it.
    session: ClientKey,
    /// Its session key and the box key with it.
    session_key: Option<Zeroizing<[u8; 32]>>,
    phase: PeerPhase,
}

/// One connection of a Threema Web session as responder.
pub struct Responder {
    /// Our permanent key pair (stored with the session).
    permanent: ClientKey,
    /// The browser's permanent public key (also the signalling path).
    initiator: PublicKey,
    /// The QR code's auth token, until the browser knows our permanent key.
    token: Option<Zeroizing<[u8; 32]>>,
    /// The server's permanent public key, if known: its 'server-auth' must be signed with it.
    server_permanent: Option<PublicKey>,
    phase: Phase,
    address: u8,
    server_out: Outgoing,
    server_seen: Option<Incoming>,
    server_session: Option<PublicKey>,
    peer: Option<Peer>,
}

impl Responder {
    /// `token`: the auth token of a new pairing (`None` once the browser trusts our key).
    #[must_use]
    pub fn new(
        permanent: ClientKey,
        initiator: PublicKey,
        token: Option<[u8; 32]>,
        server_permanent: Option<PublicKey>,
    ) -> Self {
        Self {
            permanent,
            initiator,
            token: token.map(Zeroizing::new),
            server_permanent,
            phase: Phase::ServerHello,
            address: 0,
            server_out: Outgoing::new(),
            server_seen: None,
            server_session: None,
            peer: None,
        }
    }

    /// The signalling path: the browser's permanent public key in lowercase hex.
    #[must_use]
    pub fn path(&self) -> String {
        data_encoding::HEXLOWER.encode(&public_bytes(&self.initiator))
    }

    /// The task runs: 'data' can be sent.
    #[must_use]
    pub fn connected(&self) -> bool {
        self.peer
            .as_ref()
            .is_some_and(|peer| peer.phase == PeerPhase::Task)
    }

    /// Both sides' connection ID: the MAC of an empty NaCl box with the session keys under a
    /// fixed nonce (docs/research/threema-web.md §5).
    #[must_use]
    pub fn connection_id(&self) -> Option<[u8; 16]> {
        let key = self.peer.as_ref()?.session_key.as_ref()?;
        seal(key, CONNECTION_ID_NONCE, &[]).try_into().ok()
    }

    /// A frame from the server.
    ///
    /// # Errors
    /// A protocol error: close the WebSocket with [`CLOSE_PROTOCOL_ERROR`].
    pub fn receive(&mut self, frame: &[u8]) -> Result<Vec<Event>, Error> {
        let (nonce, nonce_bytes, data) =
            Nonce::parse(frame).ok_or(Error::Protocol("short frame"))?;
        // Our address: the first message addressed to something else than 0x00.
        if self.address == 0 && nonce.destination != 0 {
            if nonce.destination < 0x02 {
                return Err(Error::Protocol("not a responder address"));
            }
            self.address = nonce.destination;
        }
        if nonce.destination != self.address {
            return Err(Error::Protocol("not addressed to us"));
        }
        match nonce.source {
            SERVER => self.receive_from_server(&nonce, &nonce_bytes, data),
            INITIATOR if self.phase == Phase::Server => {
                self.receive_from_initiator(&nonce, &nonce_bytes, data)
            }
            _ => {
                tracing::warn!("Threema Web: message from an unexpected address dropped");
                Ok(Vec::new())
            }
        }
    }

    fn receive_from_server(
        &mut self,
        nonce: &Nonce,
        nonce_bytes: &[u8; 24],
        data: &[u8],
    ) -> Result<Vec<Event>, Error> {
        check_incoming(&mut self.server_seen, &self.server_out.cookie, nonce)?;
        match self.phase {
            Phase::ServerHello => {
                let hello = decode(data, "server-hello")?;
                if message_type(&hello) != Some("server-hello") {
                    return Err(Error::Protocol("expected server-hello"));
                }
                let key = PublicKey::from(key_field(&hello)?);
                if self
                    .server_permanent
                    .as_ref()
                    .is_some_and(|permanent| public_bytes(permanent) == public_bytes(&key))
                {
                    return Err(Error::Protocol("server session key is its permanent key"));
                }
                let mut events = vec![Event::Send(self.send_to_server(
                    &map(vec![
                        ("type", "client-hello".into()),
                        (
                            "key",
                            Value::Binary(public_bytes(&self.permanent.public_key()).to_vec()),
                        ),
                    ]),
                    None,
                )?)];
                let mut auth = vec![
                    ("type", Value::from("client-auth")),
                    ("your_cookie", Value::Binary(nonce.cookie.to_vec())),
                    ("subprotocols", Value::Array(vec![SUBPROTOCOL.into()])),
                    ("ping_interval", PING_INTERVAL.into()),
                ];
                if let Some(permanent) = &self.server_permanent {
                    auth.push(("your_key", Value::Binary(public_bytes(permanent).to_vec())));
                }
                let box_key = box_key(&self.permanent, &key)?;
                events.push(Event::Send(
                    self.send_to_server(&map(auth), Some(&box_key))?,
                ));
                self.server_session = Some(key);
                self.phase = Phase::ServerAuth;
                Ok(events)
            }
            Phase::ServerAuth => {
                let session = self
                    .server_session
                    .as_ref()
                    .ok_or(Error::Protocol("no server key"))?;
                let key = box_key(&self.permanent, session)?;
                let auth = decode(
                    &open(&key, nonce_bytes, data, "server-auth")?,
                    "server-auth",
                )?;
                if message_type(&auth) != Some("server-auth") || self.address == 0 {
                    return Err(Error::Protocol("expected server-auth"));
                }
                if cookie_field(&auth) != Some(self.server_out.cookie) {
                    return Err(Error::Protocol("server-auth cookie"));
                }
                match (&self.server_permanent, field(&auth, "signed_keys")) {
                    (Some(permanent), Some(signed)) => {
                        let signed = signed.as_slice().ok_or(Error::Protocol("signed_keys"))?;
                        let key = box_key(&self.permanent, permanent)?;
                        let keys = open(&key, nonce_bytes, signed, "signed_keys")?;
                        let mut expected = public_bytes(session).to_vec();
                        expected.extend_from_slice(&public_bytes(&self.permanent.public_key()));
                        if keys != expected {
                            return Err(Error::Protocol("signed_keys mismatch"));
                        }
                    }
                    (Some(_), None) => return Err(Error::Protocol("server-auth not signed")),
                    (None, _) => tracing::warn!("Threema Web: server's permanent key unknown"),
                }
                self.phase = Phase::Server;
                match field(&auth, "initiator_connected").and_then(Value::as_bool) {
                    Some(true) => self.start_peer(),
                    Some(false) => Ok(Vec::new()),
                    None => Err(Error::Protocol("initiator_connected")),
                }
            }
            Phase::Server => {
                let session = self
                    .server_session
                    .as_ref()
                    .ok_or(Error::Protocol("no server key"))?;
                let key = box_key(&self.permanent, session)?;
                let message = decode(
                    &open(&key, nonce_bytes, data, "server message")?,
                    "server message",
                )?;
                match message_type(&message) {
                    Some("new-initiator") => self.start_peer(),
                    Some("disconnected" | "send-error") => {
                        self.peer = None;
                        Ok(vec![Event::PeerGone])
                    }
                    _ => Err(Error::Protocol("unexpected server message")),
                }
            }
        }
    }

    /// A (new) browser on the path: 'token' (on a new pairing), then 'key'.
    fn start_peer(&mut self) -> Result<Vec<Event>, Error> {
        let mut peer = Peer {
            out: Outgoing::new(),
            seen: None,
            session: ClientKey::random(),
            session_key: None,
            phase: PeerPhase::Key,
        };
        let mut events = Vec::new();
        if let Some(token) = &self.token {
            let token_message = map(vec![
                ("type", "token".into()),
                (
                    "key",
                    Value::Binary(public_bytes(&self.permanent.public_key()).to_vec()),
                ),
            ]);
            let nonce = self.next_peer_nonce(&mut peer)?;
            let mut frame = nonce.to_vec();
            frame.extend(seal(token, &nonce, &token_message));
            events.push(Event::Send(frame));
        }
        let key_message = map(vec![
            ("type", "key".into()),
            (
                "key",
                Value::Binary(public_bytes(&peer.session.public_key()).to_vec()),
            ),
        ]);
        let key = box_key(&self.permanent, &self.initiator)?;
        let nonce = self.next_peer_nonce(&mut peer)?;
        let mut frame = nonce.to_vec();
        frame.extend(seal(&key, &nonce, &key_message));
        events.push(Event::Send(frame));
        self.peer = Some(peer);
        Ok(events)
    }

    fn receive_from_initiator(
        &mut self,
        nonce: &Nonce,
        nonce_bytes: &[u8; 24],
        data: &[u8],
    ) -> Result<Vec<Event>, Error> {
        let permanent_key = box_key(&self.permanent, &self.initiator)?;
        let Some(mut peer) = self.peer.take() else {
            tracing::warn!("Threema Web: browser message before its handshake dropped");
            return Ok(Vec::new());
        };
        check_incoming(&mut peer.seen, &peer.out.cookie, nonce)?;
        let mut events = Vec::new();
        match peer.phase {
            PeerPhase::Key => {
                let message = decode(&open(&permanent_key, nonce_bytes, data, "key")?, "key")?;
                if message_type(&message) != Some("key") {
                    return Err(Error::Protocol("expected key"));
                }
                let session = key_field(&message)?;
                if session == public_bytes(&self.initiator) {
                    return Err(Error::Protocol("session key is the permanent key"));
                }
                let session_key = box_key(&peer.session, &PublicKey::from(session))?;
                let cookie = peer
                    .seen
                    .as_ref()
                    .map(|seen| seen.cookie)
                    .unwrap_or_default();
                let auth = map(vec![
                    ("type", "auth".into()),
                    ("your_cookie", Value::Binary(cookie.to_vec())),
                    ("tasks", Value::Array(vec![TASK.into()])),
                    ("data", Value::Map(vec![(TASK.into(), Value::Nil)])),
                ]);
                let nonce = self.next_peer_nonce(&mut peer)?;
                let mut frame = nonce.to_vec();
                frame.extend(seal(&session_key, &nonce, &auth));
                events.push(Event::Send(frame));
                peer.session_key = Some(session_key);
                peer.phase = PeerPhase::Auth;
            }
            PeerPhase::Auth => {
                let key = peer
                    .session_key
                    .as_ref()
                    .ok_or(Error::Protocol("no session key"))?;
                let auth = decode(&open(key, nonce_bytes, data, "auth")?, "auth")?;
                if message_type(&auth) != Some("auth") {
                    return Err(Error::Protocol("expected auth"));
                }
                if cookie_field(&auth) != Some(peer.out.cookie) {
                    return Err(Error::Protocol("auth cookie"));
                }
                if field(&auth, "task").and_then(Value::as_str) != Some(TASK) {
                    return Err(Error::Protocol("unexpected task"));
                }
                if field(&auth, "data").and_then(Value::as_map).is_none() {
                    return Err(Error::Protocol("auth data"));
                }
                // The browser knows our permanent key now.
                self.token = None;
                peer.phase = PeerPhase::Task;
                events.push(Event::PeerConnected);
            }
            PeerPhase::Task => {
                let key = peer
                    .session_key
                    .as_ref()
                    .ok_or(Error::Protocol("no session key"))?;
                let message = decode(
                    &open(key, nonce_bytes, data, "task message")?,
                    "task message",
                )?;
                match message_type(&message) {
                    Some("data") => {
                        let payload =
                            field(&message, "p").ok_or(Error::Protocol("data without p"))?;
                        events.push(Event::Data(payload.clone()));
                    }
                    Some("close") => {
                        let reason = field(&message, "reason")
                            .and_then(Value::as_u64)
                            .and_then(|reason| u16::try_from(reason).ok())
                            .unwrap_or(CLOSE_GOING_AWAY);
                        events.push(Event::PeerClosed(reason));
                        return Ok(events);
                    }
                    Some("application") => {}
                    _ => return Err(Error::Protocol("unexpected task message")),
                }
            }
        }
        self.peer = Some(peer);
        Ok(events)
    }

    /// A 'data' message to the browser.
    ///
    /// # Errors
    /// Not connected, or the sequence numbers ran out.
    pub fn send_data(&mut self, payload: Value) -> Result<Vec<u8>, Error> {
        self.send_to_peer(&map(vec![("type", "data".into()), ("p", payload)]))
    }

    /// A 'close' message to the browser (then close the WebSocket with [`CLOSE_GOING_AWAY`]).
    pub fn close(&mut self, reason: u16) -> Option<Vec<u8>> {
        let frame = self
            .send_to_peer(&map(vec![
                ("type", "close".into()),
                ("reason", reason.into()),
            ]))
            .ok();
        self.peer = None;
        frame
    }

    fn send_to_peer(&mut self, message: &[u8]) -> Result<Vec<u8>, Error> {
        let mut peer = self.peer.take().ok_or(Error::Protocol("not connected"))?;
        let result = (|| {
            if peer.phase != PeerPhase::Task {
                return Err(Error::Protocol("not connected"));
            }
            let nonce = self.next_peer_nonce(&mut peer)?;
            let key = peer
                .session_key
                .as_ref()
                .ok_or(Error::Protocol("no session key"))?;
            let mut frame = nonce.to_vec();
            frame.extend(seal(key, &nonce, message));
            Ok(frame)
        })();
        self.peer = Some(peer);
        result
    }

    fn next_peer_nonce(&self, peer: &mut Peer) -> Result<[u8; 24], Error> {
        Ok(Nonce {
            cookie: peer.out.cookie,
            source: self.address,
            destination: INITIATOR,
            csn: peer.out.next()?,
        }
        .bytes())
    }

    /// A message to the server, boxed with `key` (`None`: 'client-hello', unencrypted).
    fn send_to_server(&mut self, message: &[u8], key: Option<&[u8; 32]>) -> Result<Vec<u8>, Error> {
        let nonce = Nonce {
            cookie: self.server_out.cookie,
            source: self.address,
            destination: SERVER,
            csn: self.server_out.next()?,
        }
        .bytes();
        let mut frame = nonce.to_vec();
        match key {
            Some(key) => frame.extend(seal(key, &nonce, message)),
            None => frame.extend_from_slice(message),
        }
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The other end of a frame: who sent it and the plaintext map.
    fn read(frame: &[u8], key: Option<&[u8; 32]>) -> (Nonce, Value) {
        let (nonce, bytes, data) = Nonce::parse(frame).unwrap();
        let plain = match key {
            Some(key) => open(key, &bytes, data, "test").unwrap(),
            None => data.to_vec(),
        };
        (nonce, decode(&plain, "test").unwrap())
    }

    fn frame(nonce: &Nonce, message: &[u8], key: Option<&[u8; 32]>) -> Vec<u8> {
        let bytes = nonce.bytes();
        let mut frame = bytes.to_vec();
        match key {
            Some(key) => frame.extend(seal(key, &bytes, message)),
            None => frame.extend_from_slice(message),
        }
        frame
    }

    fn sent(events: &[Event]) -> Vec<Vec<u8>> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Send(frame) => Some(frame.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn new_pairing_handshake_and_data() {
        let server_permanent = ClientKey::random();
        let server_session = ClientKey::random();
        let browser = ClientKey::random();
        let browser_session = ClientKey::random();
        let ours = ClientKey::random();
        let token: [u8; 32] = rand::random();
        let mut responder = Responder::new(
            ClientKey::from(*ours.as_bytes()),
            browser.public_key(),
            Some(token),
            Some(server_permanent.public_key()),
        );
        assert_eq!(responder.path().len(), 64);

        // Server → us: server-hello.
        let server_cookie: [u8; 16] = rand::random();
        let mut server_csn = 1000;
        let mut server_nonce = |destination| {
            server_csn += 1;
            Nonce {
                cookie: server_cookie,
                source: SERVER,
                destination,
                csn: server_csn,
            }
        };
        let hello = map(vec![
            ("type", "server-hello".into()),
            (
                "key",
                Value::Binary(public_bytes(&server_session.public_key()).to_vec()),
            ),
        ]);
        let events = responder
            .receive(&frame(&server_nonce(0), &hello, None))
            .unwrap();
        let frames = sent(&events);
        assert_eq!(frames.len(), 2);
        let (_, client_hello) = read(&frames[0], None);
        assert_eq!(message_type(&client_hello), Some("client-hello"));
        assert_eq!(
            key_field(&client_hello).unwrap(),
            public_bytes(&ours.public_key())
        );
        let to_us = box_key(&server_session, &ours.public_key()).unwrap();
        let (auth_nonce, client_auth) = read(&frames[1], Some(&to_us));
        assert_eq!(message_type(&client_auth), Some("client-auth"));
        assert_eq!(cookie_field(&client_auth), Some(server_cookie));

        // Server → us: server-auth, signed, assigning address 0x05, initiator present.
        let nonce = server_nonce(0x05);
        let mut signed = public_bytes(&server_session.public_key()).to_vec();
        signed.extend_from_slice(&public_bytes(&ours.public_key()));
        let signing_key = box_key(&server_permanent, &ours.public_key()).unwrap();
        let signed_keys = seal(&signing_key, &nonce.bytes(), &signed);
        let auth = map(vec![
            ("type", "server-auth".into()),
            ("your_cookie", Value::Binary(auth_nonce.cookie.to_vec())),
            ("signed_keys", Value::Binary(signed_keys)),
            ("initiator_connected", true.into()),
        ]);
        let frames = sent(
            &responder
                .receive(&frame(&nonce, &auth, Some(&to_us)))
                .unwrap(),
        );
        // 'token' (with the auth token), then 'key'.
        assert_eq!(frames.len(), 2);
        let (token_nonce, token_message) = read(&frames[0], Some(&token));
        assert_eq!(
            (token_nonce.source, token_nonce.destination),
            (0x05, INITIATOR)
        );
        assert_eq!(
            key_field(&token_message).unwrap(),
            public_bytes(&ours.public_key())
        );
        let permanent = box_key(&browser, &ours.public_key()).unwrap();
        let (_, key_message) = read(&frames[1], Some(&permanent));
        let our_session = PublicKey::from(key_field(&key_message).unwrap());

        // Browser → us: its key; we answer with auth offering the relayed-data task.
        let browser_cookie: [u8; 16] = rand::random();
        let mut browser_csn = 7;
        let mut browser_nonce = || {
            browser_csn += 1;
            Nonce {
                cookie: browser_cookie,
                source: INITIATOR,
                destination: 0x05,
                csn: browser_csn,
            }
        };
        let key = map(vec![
            ("type", "key".into()),
            (
                "key",
                Value::Binary(public_bytes(&browser_session.public_key()).to_vec()),
            ),
        ]);
        let frames = sent(
            &responder
                .receive(&frame(&browser_nonce(), &key, Some(&permanent)))
                .unwrap(),
        );
        let session = box_key(&browser_session, &our_session).unwrap();
        let (_, our_auth) = read(&frames[0], Some(&session));
        assert_eq!(cookie_field(&our_auth), Some(browser_cookie));
        assert_eq!(
            field(&our_auth, "tasks").unwrap().as_array().unwrap()[0].as_str(),
            Some(TASK)
        );

        // Browser → us: auth choosing the task.
        let auth = map(vec![
            ("type", "auth".into()),
            ("your_cookie", Value::Binary(token_nonce.cookie.to_vec())),
            ("task", TASK.into()),
            ("data", Value::Map(vec![(TASK.into(), Value::Nil)])),
        ]);
        let events = responder
            .receive(&frame(&browser_nonce(), &auth, Some(&session)))
            .unwrap();
        assert_eq!(events, vec![Event::PeerConnected]);
        assert!(responder.connected());
        // Both sides derive the same connection ID.
        let browser_id: [u8; 16] = seal(&session, CONNECTION_ID_NONCE, &[]).try_into().unwrap();
        assert_eq!(responder.connection_id(), Some(browser_id));

        // Data both ways.
        let data = map(vec![
            ("type", "data".into()),
            ("p", Value::Binary(vec![1, 2, 3])),
        ]);
        let events = responder
            .receive(&frame(&browser_nonce(), &data, Some(&session)))
            .unwrap();
        assert_eq!(events, vec![Event::Data(Value::Binary(vec![1, 2, 3]))]);
        let out = responder.send_data(Value::Binary(vec![4])).unwrap();
        let (_, message) = read(&out, Some(&session));
        assert_eq!(field(&message, "p"), Some(&Value::Binary(vec![4])));

        // A replayed frame is refused.
        let replay = frame(
            &Nonce {
                cookie: browser_cookie,
                source: INITIATOR,
                destination: 0x05,
                csn: 8,
            },
            &data,
            Some(&session),
        );
        assert!(responder.receive(&replay).is_err());
    }

    #[test]
    fn unsigned_server_auth_is_refused_when_the_key_is_known() {
        let server_session = ClientKey::random();
        let ours = ClientKey::random();
        let mut responder = Responder::new(
            ClientKey::from(*ours.as_bytes()),
            ClientKey::random().public_key(),
            None,
            Some(ClientKey::random().public_key()),
        );
        let cookie: [u8; 16] = rand::random();
        let hello = map(vec![
            ("type", "server-hello".into()),
            (
                "key",
                Value::Binary(public_bytes(&server_session.public_key()).to_vec()),
            ),
        ]);
        let events = responder
            .receive(&frame(
                &Nonce {
                    cookie,
                    source: 0,
                    destination: 0,
                    csn: 1,
                },
                &hello,
                None,
            ))
            .unwrap();
        let to_us = box_key(&server_session, &ours.public_key()).unwrap();
        let (auth_nonce, _) = read(&sent(&events)[1], Some(&to_us));
        let auth = map(vec![
            ("type", "server-auth".into()),
            ("your_cookie", Value::Binary(auth_nonce.cookie.to_vec())),
            ("initiator_connected", false.into()),
        ]);
        let nonce = Nonce {
            cookie,
            source: 0,
            destination: 2,
            csn: 2,
        };
        assert_eq!(
            responder.receive(&frame(&nonce, &auth, Some(&to_us))),
            Err(Error::Protocol("server-auth not signed"))
        );
    }
}
