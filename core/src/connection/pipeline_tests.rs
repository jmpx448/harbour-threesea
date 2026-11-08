// SPDX-License-Identifier: AGPL-3.0-only
//! The send and receive pipeline against a fake chat server: the real login, `Session::run` and
//! libthreema's CSP on our side, the server's half of the protocol in the test.
//!
//! Nothing here goes by the clock or a socket. A write reaches the server inside the call, so what
//! the server notes about the store is how the store was when those bytes went out, whatever the
//! machine is doing. The tests wait for what the server has seen, and the clock is paused: tokio
//! only moves it when nothing can run any more, so `STUCK` is reached at once if a test hangs, and
//! never because the machine is slow.

use std::collections::HashSet;

use crypto_secretbox::{KeyInit as _, XSalsa20Poly1305, aead::Aead as _};
use libthreema::common::keys::ClientKey;
use tokio::sync::Notify;

use super::{sync_order_tests::app, *};
use crate::{app::App, reactions::OUTBOX_DELIVERY_RECEIPT, transport::ScriptedWire};

const ALICE: &str = "ABCD1234";
const BOB: &str = "BCDE2345";
const CAROL: &str = "CDEF3456";
const GROUP: &str = "g:USER0001:0000000000000001";

/// Far beyond any wait in the connection. See the top of this file: no real time passes.
const STUCK: Duration = Duration::from_secs(24 * 3600);

const COOKIE_LENGTH: usize = 16;
const KEY_LENGTH: usize = 32;
const PAYLOAD_HEADER: [u8; 3] = [0; 3];
/// The start of a message-with-metadata-box up to the length of its metadata.
const METADATA_LENGTH_AT: usize = 8 + 8 + 8 + 4 + 1 + 1;
const FLAGS_AT: usize = 8 + 8 + 8 + 4;
const NICKNAME_LENGTH: usize = 32;

/// CSP's payload types, as on the wire.
mod from_client {
    pub const ECHO_REQUEST: u8 = 0x00;
    pub const MESSAGE: u8 = 0x01;
    pub const ECHO_RESPONSE: u8 = 0x80;
    pub const ACK: u8 = 0x82;
}
mod to_client {
    pub const ECHO_REQUEST: u8 = 0x00;
    pub const MESSAGE: u8 = 0x02;
    pub const ECHO_RESPONSE: u8 = 0x80;
    pub const ACK: u8 = 0x81;
    pub const QUEUE_SEND_COMPLETE: u8 = 0xd0;
    pub const CLOSE_ERROR: u8 = 0xe0;
}

/// What the server got, and how the store was as it went out.
#[derive(Clone, Debug)]
enum Seen {
    Message {
        receiver: String,
        id: u64,
        nonce: Vec<u8>,
        nonce_on_record: bool,
        store_committed: bool,
        state_when_sent: Option<i64>,
    },
    Ack {
        sender: String,
        id: u64,
        stored: bool,
        store_committed: bool,
    },
    EchoResponse(Vec<u8>),
    Other,
}

fn id_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn identity_at(bytes: &[u8], at: usize) -> String {
    String::from_utf8(bytes[at..at + 8].to_vec()).unwrap()
}

/// The server's half of a connection.
struct ServerState {
    store: Rc<Store>,
    /// The first write is the client hello, the second the login: a write is one frame.
    writes: u32,
    permanent_key: ClientKey,
    temporary_key: ClientKey,
    client_cookie: [u8; COOKIE_LENGTH],
    server_cookie: [u8; COOKIE_LENGTH],
    cipher: Option<XSalsa20Poly1305>,
    client_sequence: u64,
    server_sequence: u64,
    seen: Vec<Seen>,
    /// The connection breaks at the first write of what this picks, and what it wrote is lost.
    breaks_at: Option<fn(&Seen) -> bool>,
    broken: bool,
    session_ended: bool,
    next_echo: u32,
    inbound: mpsc::UnboundedSender<Result<Vec<u8>, TransportError>>,
    changed: Rc<Notify>,
}

impl ServerState {
    fn nonce(cookie: &[u8; COOKIE_LENGTH], sequence: &mut u64) -> [u8; 24] {
        let mut nonce = [0; 24];
        nonce[..COOKIE_LENGTH].copy_from_slice(cookie);
        nonce[COOKIE_LENGTH..].copy_from_slice(&sequence.to_le_bytes());
        *sequence += 1;
        nonce
    }

    fn hello(&mut self, frame: &[u8]) {
        let client_key: [u8; KEY_LENGTH] = frame[..KEY_LENGTH].try_into().unwrap();
        let client_key = PublicKey::from(client_key);
        self.client_cookie = frame[KEY_LENGTH..].try_into().unwrap();
        let mut response = data_encoding::HEXLOWER
            .decode(self.temporary_key.public_key().to_string().as_bytes())
            .unwrap();
        response.extend_from_slice(&self.client_cookie);
        let hello_key = self
            .permanent_key
            .derive_shared_secret(&client_key)
            .unwrap();
        let hello_cipher = XSalsa20Poly1305::new((&*hello_key).into());
        let nonce = Self::nonce(&self.server_cookie, &mut self.server_sequence);
        let boxed = hello_cipher
            .encrypt((&nonce).into(), response.as_slice())
            .unwrap();
        let mut hello = self.server_cookie.to_vec();
        hello.extend(boxed);
        self.inbound.send(Ok(hello)).unwrap();
        let session_key = self
            .temporary_key
            .derive_shared_secret(&client_key)
            .unwrap();
        self.cipher = Some(XSalsa20Poly1305::new((&*session_key).into()));
    }

    fn encrypted(&mut self, plain: &[u8]) -> Vec<u8> {
        let nonce = Self::nonce(&self.server_cookie, &mut self.server_sequence);
        self.cipher
            .as_ref()
            .unwrap()
            .encrypt((&nonce).into(), plain)
            .unwrap()
    }

    /// The login's two boxes used up the client's first two sequence numbers.
    fn login(&mut self) {
        self.client_sequence = 3;
        let mut data = vec![0; 4];
        data.extend_from_slice(
            &u64::try_from(crate::messages::now_ms())
                .unwrap()
                .to_le_bytes(),
        );
        data.extend_from_slice(&0_u32.to_le_bytes());
        let ack = self.encrypted(&data);
        self.inbound.send(Ok(ack)).unwrap();
    }

    fn frame(payload_type: u8, body: &[u8]) -> Vec<u8> {
        let mut payload = vec![payload_type];
        payload.extend_from_slice(&PAYLOAD_HEADER);
        payload.extend_from_slice(body);
        payload
    }

    /// The bytes of a payload to the client, as a frame.
    fn wire_frame(&mut self, payload_type: u8, body: &[u8]) -> Vec<u8> {
        let boxed = self.encrypted(&Self::frame(payload_type, body));
        let mut frame = u16::try_from(boxed.len()).unwrap().to_le_bytes().to_vec();
        frame.extend(boxed);
        frame
    }

    fn decrypted(&mut self, frame: &[u8]) -> Vec<u8> {
        let length = usize::from(u16::from_le_bytes([frame[0], frame[1]]));
        assert_eq!(length + 2, frame.len(), "a write is one frame");
        let nonce = Self::nonce(&self.client_cookie, &mut self.client_sequence);
        self.cipher
            .as_ref()
            .unwrap()
            .decrypt((&nonce).into(), &frame[2..])
            .expect("every frame decrypts under the next sequence number")
    }

    fn understood(&self, payload: &[u8]) -> Seen {
        let (payload_type, body) = (payload[0], &payload[4..]);
        let store_committed = self.store.conn.is_autocommit();
        match payload_type {
            from_client::MESSAGE => {
                let receiver = identity_at(body, 8);
                let id = id_at(body, 16);
                let metadata_length = usize::from(u16::from_le_bytes([
                    body[METADATA_LENGTH_AT],
                    body[METADATA_LENGTH_AT + 1],
                ]));
                let nonce_at = METADATA_LENGTH_AT + 2 + NICKNAME_LENGTH + metadata_length;
                let nonce = body[nonce_at..nonce_at + 24].to_vec();
                Seen::Message {
                    nonce_on_record: self.store.has_nonce(&nonce).unwrap(),
                    store_committed,
                    state_when_sent: state_of(&self.store, &receiver, id),
                    receiver,
                    id,
                    nonce,
                }
            }
            from_client::ACK => {
                let sender = identity_at(body, 0);
                let id = id_at(body, 8);
                Seen::Ack {
                    stored: self.store.has_incoming(&sender, id).unwrap(),
                    store_committed,
                    sender,
                    id,
                }
            }
            from_client::ECHO_RESPONSE => Seen::EchoResponse(body.to_vec()),
            _ => Seen::Other,
        }
    }

    fn written(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        if self.broken {
            return Err(TransportError::Closed);
        }
        self.writes += 1;
        match self.writes {
            1 => self.hello(frame),
            2 => self.login(),
            _ => {
                let payload = self.decrypted(frame);
                let seen = self.understood(&payload);
                if self.breaks_at.is_some_and(|breaks| breaks(&seen)) {
                    self.broken = true;
                    let _ = self.inbound.send(Err(TransportError::Closed));
                    return Err(TransportError::Closed);
                }
                self.seen.push(seen);
                if payload[0] == from_client::ECHO_REQUEST {
                    let answer = self.wire_frame(to_client::ECHO_RESPONSE, &payload[4..]);
                    self.inbound.send(Ok(answer)).unwrap();
                }
                self.changed.notify_one();
            }
        }
        Ok(())
    }
}

/// The state of our message `id` to `receiver`, or to the group.
fn state_of(store: &Store, receiver: &str, id: u64) -> Option<i64> {
    for conversation in [receiver, GROUP] {
        let found = store
            .conversation_messages(conversation, 100)
            .unwrap()
            .into_iter()
            .find(|message| message.message_id == id && message.outgoing);
        if let Some(message) = found {
            return Some(message.state);
        }
    }
    None
}

#[derive(Clone)]
struct FakeServer(Rc<RefCell<ServerState>>);

impl FakeServer {
    fn new(store: &Rc<Store>, permanent_key: &ClientKey) -> (Self, Transport) {
        let (inbound, read) = mpsc::unbounded_channel();
        let state = Rc::new(RefCell::new(ServerState {
            store: Rc::clone(store),
            writes: 0,
            permanent_key: ClientKey::from(*permanent_key.as_bytes()),
            temporary_key: ClientKey::random(),
            client_cookie: [0; COOKIE_LENGTH],
            server_cookie: rand::random(),
            cipher: None,
            client_sequence: 1,
            server_sequence: 1,
            seen: Vec::new(),
            breaks_at: None,
            broken: false,
            session_ended: false,
            next_echo: 0,
            inbound,
            changed: Rc::new(Notify::new()),
        }));
        let writer = Rc::clone(&state);
        let transport = Transport::Scripted(Box::new(ScriptedWire {
            written: Box::new(move |frame| writer.borrow_mut().written(frame)),
            inbound: read,
        }));
        (Self(state), transport)
    }

    fn send(&self, payload_type: u8, body: &[u8]) {
        let mut state = self.0.borrow_mut();
        let frame = state.wire_frame(payload_type, body);
        state.inbound.send(Ok(frame)).unwrap();
    }

    fn deliver(&self, message: &MessageWithMetadataBox) {
        self.send(to_client::MESSAGE, &message.bytes);
    }

    fn ack(&self, receiver: &str, id: u64) {
        let mut body = receiver.as_bytes().to_vec();
        body.extend_from_slice(&id.to_le_bytes());
        self.send(to_client::ACK, &body);
    }

    fn close(&self) {
        let mut state = self.0.borrow_mut();
        state.broken = true;
        let _ = state.inbound.send(Err(TransportError::Closed));
    }

    fn seen(&self) -> Vec<Seen> {
        self.0.borrow().seen.clone()
    }

    /// The messages that reached the server: (receiver, id).
    fn messages(&self) -> Vec<(String, u64)> {
        self.seen()
            .iter()
            .filter_map(|seen| match seen {
                Seen::Message { receiver, id, .. } => Some((receiver.clone(), *id)),
                _ => None,
            })
            .collect()
    }

    fn nonces(&self) -> Vec<Vec<u8>> {
        self.seen()
            .into_iter()
            .filter_map(|seen| match seen {
                Seen::Message { nonce, .. } => Some(nonce),
                _ => None,
            })
            .collect()
    }

    fn acks(&self) -> Vec<(String, u64)> {
        self.seen()
            .iter()
            .filter_map(|seen| match seen {
                Seen::Ack { sender, id, .. } => Some((sender.clone(), *id)),
                _ => None,
            })
            .collect()
    }

    /// Until this holds. A session that ends first won't make it true any more.
    async fn until(&self, what: &str, holds: impl Fn(&Self) -> bool) {
        loop {
            if holds(self) {
                return;
            }
            let (ended, changed) = {
                let state = self.0.borrow();
                (state.session_ended, Rc::clone(&state.changed))
            };
            assert!(!ended, "the session ended before: {what}");
            changed.notified().await;
        }
    }

    async fn until_messages(&self, count: usize) {
        self.until("messages", |server| server.messages().len() >= count)
            .await;
    }

    async fn until_ack_of(&self, sender: &str, id: u64) {
        self.until("an ack", |server| {
            server.acks().contains(&(sender.to_owned(), id))
        })
        .await;
    }

    /// Until the session has ended, by itself or because the connection broke.
    async fn until_session_ended(&self) {
        let changed = Rc::clone(&self.0.borrow().changed);
        while !self.0.borrow().session_ended {
            changed.notified().await;
        }
    }

    /// Until the session has handled everything sent to it so far: frames are handled in order,
    /// and this one wants an answer.
    async fn caught_up(&self) {
        let echo = {
            let mut state = self.0.borrow_mut();
            state.next_echo += 1;
            state.next_echo.to_le_bytes().to_vec()
        };
        self.send(to_client::ECHO_REQUEST, &echo);
        self.until("the answer to an echo", |server| {
            server
                .seen()
                .iter()
                .any(|seen| matches!(seen, Seen::EchoResponse(answer) if *answer == echo))
        })
        .await;
    }

    fn session_ended(&self) {
        let mut state = self.0.borrow_mut();
        state.session_ended = true;
        state.changed.notify_one();
    }
}

/// What the app tells the connection.
struct Commands(mpsc::UnboundedSender<ToConnection>);

impl Commands {
    /// Asks it to send what is queued, and waits until it has been through that.
    async fn send_pending(&self) {
        let (done, handled) = oneshot::channel();
        self.0.send(ToConnection::SendPending).unwrap();
        self.0.send(ToConnection::Flush(done)).unwrap();
        handled.await.unwrap();
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    _app: App,
    setup: ConnectionSetup,
    store: Rc<Store>,
    server_key: ClientKey,
    ui: mpsc::UnboundedSender<FromConnection>,
    from_connection: RefCell<mpsc::UnboundedReceiver<FromConnection>>,
}

fn harness() -> Harness {
    let (dir, app) = app();
    let identity = app.store.identity().unwrap().unwrap();
    let server_key = ClientKey::from([7; 32]);
    let mut config = (*app.config).clone();
    config.chat_server_public_keys = vec![server_key.public_key()];
    let setup = ConnectionSetup {
        store: Rc::clone(&app.store),
        media: Arc::clone(&app.media),
        https: app.https.clone(),
        config: Arc::new(config),
        identity: identity.threema_id.parse().unwrap(),
        client_key: Zeroizing::new(*identity.client_key),
        server_group: ChatServerGroup::try_from(identity.server_group.as_str()).unwrap(),
        device_cookie: DeviceCookie([1; 16]),
        client_info: app.version.client_info(),
        fs_mask_checks: FsMaskChecks::default(),
        storage_failing: StorageFailing::default(),
        failing_items: FailingItems::default(),
        carried_events: RefCell::default(),
        wake_hold: WakeHold::default(),
        picture_uploads: Rc::default(),
    };
    let (ui, from_connection) = mpsc::unbounded_channel();
    Harness {
        store: Rc::clone(&setup.store),
        _dir: dir,
        _app: app,
        setup,
        server_key,
        ui,
        from_connection: RefCell::new(from_connection),
    }
}

impl Harness {
    /// One connection, for as long as `script` runs: the script's end is the server hanging up.
    async fn connection<T>(&self, script: impl AsyncFnOnce(&FakeServer, &Commands) -> T) -> T {
        self.ending(None, script).await.1
    }

    /// The same, with how the session ended. With `breaks_at` the connection breaks at the first
    /// write of what it picks, even one that goes out before the script begins.
    async fn ending<T>(
        &self,
        breaks_at: Option<fn(&Seen) -> bool>,
        script: impl AsyncFnOnce(&FakeServer, &Commands) -> T,
    ) -> (Result<(), ConnectionError>, T) {
        let (server, transport) = FakeServer::new(&self.store, &self.server_key);
        server.0.borrow_mut().breaks_at = breaks_at;
        let fs_disabled = self.store.forward_security_disabled().unwrap();
        let session = Session::log_in(&self.setup, &self.ui, transport, None, fs_disabled)
            .await
            .expect("the login goes through");
        let (commands, mut to_connection) = mpsc::unbounded_channel();
        let commands = Commands(commands);
        let session = async {
            let ended = session.run(&mut to_connection).await;
            server.session_ended();
            ended
        };
        let script = async {
            let result = script(&server, &commands).await;
            server.close();
            result
        };
        timeout(STUCK, async { tokio::join!(session, script) })
            .await
            .expect("nothing left that could run: the test is stuck")
    }

    /// A contact that gets no forward secrecy, so that one message is one frame.
    fn contact(&self, identity: &str) {
        // They have our (lack of a) picture already: no message of that comes before ours.
        self.store.picture_sent(identity, None).unwrap();
        let mut contact = crate::contacts::test_contact(identity);
        contact.feature_mask = libthreema::common::FeatureMask(0);
        contact.public_key = sender_key().public_key();
        contact.acquaintance_level =
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::Direct;
        self.store.upsert_contact(&contact).unwrap();
    }

    fn group(&self, members: &[&str]) {
        for member in members {
            self.contact(member);
        }
        self.store
            .upsert_group(&crate::groups::Group {
                key: GROUP.to_owned(),
                identity: crate::groups::parse_key(GROUP).unwrap(),
                name: "Crew".into(),
                members: members.iter().map(|&member| member.to_owned()).collect(),
                left: false,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
    }

    fn group_message(&self, id: u64, members: &[&str]) -> i64 {
        let message = self.store.insert_outgoing(GROUP, id, "text", "hi").unwrap();
        let receivers: Vec<_> = members.iter().map(|&member| member.to_owned()).collect();
        self.store
            .set_pending_receivers(GROUP, id, &receivers)
            .unwrap();
        message.id
    }

    fn state(&self, message: i64) -> i64 {
        self.store.message(message).unwrap().unwrap().state
    }

    /// A message from `sender` as the server hands it out.
    fn incoming(&self, sender: &str, id: u64, text: &str) -> MessageWithMetadataBox {
        encode_outgoing_contact_message(
            sender.parse().unwrap(),
            &sender_key(),
            Delta::Unchanged,
            self.setup.identity,
            &ClientKey::from(*self.setup.client_key).public_key(),
            &OutgoingMessage {
                id: MessageId(id),
                overrides: MessageOverrides::default(),
                created_at: 1,
                body: OutgoingMessageBody::Contact(OutgoingContactMessageBody {
                    receiver_identity: self.setup.identity,
                    body: ContactMessageBody::Text(TextMessage { text: text.into() }),
                }),
            },
            Nonce([u8::try_from(id).unwrap(); 24]),
        )
        .unwrap()
    }

    fn ui_heard_of_new_messages(&self) -> usize {
        let mut from_connection = self.from_connection.borrow_mut();
        let mut added = 0;
        while let Ok(message) = from_connection.try_recv() {
            if let FromConnection::Event(event) = message
                && matches!(*event, CoreEvent::MessageAdded { .. })
            {
                added += 1;
            }
        }
        added
    }

    fn refuse_new_messages(&self) {
        self.store
            .conn
            .execute_batch(
                "CREATE TRIGGER refuse BEFORE INSERT ON messages
                 BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
            )
            .unwrap();
    }

    fn accept_new_messages(&self) {
        self.store
            .conn
            .execute_batch("DROP TRIGGER refuse")
            .unwrap();
    }
}

fn sender_key() -> ClientKey {
    ClientKey::from([5; 32])
}

fn is_sent(state: i64) -> bool {
    state == OutgoingState::Sent as i64
}

fn is_message(seen: &Seen) -> bool {
    matches!(seen, Seen::Message { .. })
}

fn is_ack(seen: &Seen) -> bool {
    matches!(seen, Seen::Ack { .. })
}

fn distinct<T: std::hash::Hash + Eq>(items: Vec<T>) -> usize {
    items.into_iter().collect::<HashSet<_>>().len()
}

// The fake server itself, and what any connection does.

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_login_reaches_the_payloads_and_echoes_are_answered() {
    let test = harness();
    let (ended, ()) = test
        .ending(None, async |server: &FakeServer, _: &Commands| {
            server.caught_up().await;
            server.send(to_client::QUEUE_SEND_COMPLETE, &[]);
            server.caught_up().await;
        })
        .await;
    assert!(matches!(ended, Err(ConnectionError::Io(_))), "{ended:?}");
    let mut heard = test.from_connection.borrow_mut();
    assert!(matches!(
        heard.try_recv(),
        Ok(FromConnection::State(ConnectionState::Connected))
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn silence_only_brings_an_echo_and_the_connection_stays() {
    let test = harness();
    test.connection(async |server: &FakeServer, _: &Commands| {
        // Nothing else can run: the paused clock goes straight to the echo's time.
        server
            .until("an echo request", |server| {
                server.seen().iter().any(|seen| matches!(seen, Seen::Other))
            })
            .await;
        server.caught_up().await;
    })
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_close_error_ends_the_session_with_the_servers_words() {
    let test = harness();
    let (ended, ()) = test
        .ending(None, async |server: &FakeServer, _: &Commands| {
            let mut close = vec![0];
            close.extend_from_slice(b"Another connection");
            server.send(to_client::CLOSE_ERROR, &close);
            server.until_session_ended().await;
        })
        .await;
    let Err(ConnectionError::Closed {
        message,
        can_reconnect,
    }) = ended
    else {
        panic!("expected the close error, got {ended:?}");
    };
    assert_eq!(message, "Another connection");
    assert!(!can_reconnect);
}

/// The stream comes in any chunks: a frame cut byte by byte, two in one chunk.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn frames_come_through_however_they_are_chunked() {
    let test = harness();
    test.contact(ALICE);
    let (first, second, third) = (
        test.incoming(ALICE, 1, "one"),
        test.incoming(ALICE, 2, "two"),
        test.incoming(ALICE, 3, "three"),
    );
    test.connection(async |server: &FakeServer, _: &Commands| {
        let frame = {
            let mut state = server.0.borrow_mut();
            state.wire_frame(to_client::MESSAGE, &first.bytes)
        };
        for byte in frame {
            server.0.borrow().inbound.send(Ok(vec![byte])).unwrap();
        }
        server.until_ack_of(ALICE, 1).await;
        let together = {
            let mut state = server.0.borrow_mut();
            let mut together = state.wire_frame(to_client::MESSAGE, &second.bytes);
            together.extend(state.wire_frame(to_client::MESSAGE, &third.bytes));
            together
        };
        server.0.borrow().inbound.send(Ok(together)).unwrap();
        server.until_ack_of(ALICE, 3).await;
        assert_eq!(
            server.acks(),
            [(ALICE.into(), 1), (ALICE.into(), 2), (ALICE.into(), 3)]
        );
    })
    .await;
    assert_eq!(test.ui_heard_of_new_messages(), 3);
}

// Receiving: what the server is told it may forget is safe here.

/// The ack is the server's licence to delete its copy, so when it goes out the message, with its
/// nonce, is committed.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_ack_goes_out_only_with_the_message_committed() {
    let test = harness();
    test.contact(ALICE);
    let message = test.incoming(ALICE, 1, "hi");
    let seen = test
        .connection(async |server: &FakeServer, _: &Commands| {
            server.deliver(&message);
            server.until_ack_of(ALICE, 1).await;
            server.seen()
        })
        .await;
    let ack = seen.iter().find(|seen| is_ack(seen)).unwrap();
    let Seen::Ack {
        stored,
        store_committed,
        ..
    } = ack
    else {
        unreachable!()
    };
    assert!(*stored, "acknowledged before it was stored");
    assert!(*store_committed, "acknowledged before the commit");
    assert!(test.store.has_nonce(&[1; 24]).unwrap());
    assert_eq!(test.ui_heard_of_new_messages(), 1);
}

/// The store says no: the server keeps the message, and brings it on the next connection.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_message_the_store_refused_is_not_acked_and_comes_again() {
    let test = harness();
    test.contact(ALICE);
    let message = test.incoming(ALICE, 1, "hi");
    test.refuse_new_messages();
    let (ended, acks) = test
        .ending(None, async |server: &FakeServer, _: &Commands| {
            server.deliver(&message);
            server.until_session_ended().await;
            server.acks()
        })
        .await;
    assert!(
        matches!(ended, Err(ConnectionError::Storage { .. })),
        "{ended:?}"
    );
    assert!(acks.is_empty(), "acknowledged what the store refused");
    assert!(!test.store.has_incoming(ALICE, 1).unwrap());
    assert_eq!(test.ui_heard_of_new_messages(), 0);

    test.accept_new_messages();
    let again = test.incoming(ALICE, 1, "hi");
    test.connection(async |server: &FakeServer, _: &Commands| {
        server.deliver(&again);
        server.until_ack_of(ALICE, 1).await;
    })
    .await;
    assert!(test.store.has_incoming(ALICE, 1).unwrap());
    assert_eq!(test.ui_heard_of_new_messages(), 1);
}

/// The same with the failure as late as it gets: everything written, the commit refused.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_commit_that_fails_takes_the_ack_with_it() {
    let test = harness();
    test.contact(ALICE);
    let message = test.incoming(ALICE, 1, "hi");
    test.store.conn.commit_hook(Some(|| true));
    let (ended, acks) = test
        .ending(None, async |server: &FakeServer, _: &Commands| {
            server.deliver(&message);
            server.until_session_ended().await;
            server.acks()
        })
        .await;
    test.store.conn.commit_hook(None::<fn() -> bool>);
    assert!(
        matches!(ended, Err(ConnectionError::Storage { .. })),
        "{ended:?}"
    );
    assert!(acks.is_empty(), "acknowledged what didn't commit");
    assert!(!test.store.has_incoming(ALICE, 1).unwrap());
    assert_eq!(test.ui_heard_of_new_messages(), 0);
}

/// The connection breaks at the ack. The message is here, and the server brings it again because
/// it can't know: no second copy, no second notification, and this time the ack arrives.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_ack_brings_the_message_again_without_a_second_copy() {
    let test = harness();
    test.contact(ALICE);
    let message = test.incoming(ALICE, 1, "hi");
    let (_lost, ()) = test
        .ending(Some(is_ack), async |server: &FakeServer, _: &Commands| {
            server.deliver(&message);
            server.until_session_ended().await;
            assert!(server.acks().is_empty());
        })
        .await;
    assert_eq!(test.ui_heard_of_new_messages(), 1);

    let again = test.incoming(ALICE, 1, "hi");
    test.connection(async |server: &FakeServer, _: &Commands| {
        server.deliver(&again);
        server.until_ack_of(ALICE, 1).await;
    })
    .await;
    assert_eq!(test.ui_heard_of_new_messages(), 0);
    assert_eq!(
        test.store.conversation_messages(ALICE, 10).unwrap().len(),
        1
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_message_flagged_for_no_ack_gets_none() {
    let test = harness();
    test.contact(ALICE);
    let mut quiet = test.incoming(ALICE, 1, "hi");
    quiet.bytes[FLAGS_AT] |= MessageFlags::NO_SERVER_ACKNOWLEDGEMENT;
    let other = test.incoming(ALICE, 2, "hi");
    let acks = test
        .connection(async |server: &FakeServer, _: &Commands| {
            server.deliver(&quiet);
            server.deliver(&other);
            server.until_ack_of(ALICE, 2).await;
            server.acks()
        })
        .await;
    assert!(test.store.has_incoming(ALICE, 1).unwrap());
    assert_eq!(acks, [(ALICE.to_owned(), 2)]);
}

// Sending: only the server's ack ends a message's way out.

/// Written out isn't sent: that takes the server's ack, for this very receiver and message.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_message_counts_as_sent_only_at_the_servers_ack() {
    let test = harness();
    test.contact(ALICE);
    let message = test
        .store
        .insert_outgoing(ALICE, 1, "text", "hi")
        .unwrap()
        .id;
    let seen = test
        .connection(async |server: &FakeServer, _: &Commands| {
            server.until_messages(1).await;
            server.caught_up().await;
            assert!(!is_sent(test.state(message)), "sent without an ack");
            server.ack(BOB, 1);
            server.ack(ALICE, 2);
            server.caught_up().await;
            assert!(!is_sent(test.state(message)), "someone else's ack counted");
            server.ack(ALICE, 1);
            server.caught_up().await;
            assert!(is_sent(test.state(message)));
            server.seen()
        })
        .await;
    let Some(Seen::Message {
        state_when_sent, ..
    }) = seen.iter().find(|seen| is_message(seen))
    else {
        panic!("no message reached the server");
    };
    assert_eq!(*state_when_sent, Some(OutgoingState::Pending as i64));
    assert!(test.store.message(message).unwrap().unwrap().written_out);
    assert!(test.store.pending_outgoing().unwrap().is_empty());
}

/// Nothing goes out under a nonce that isn't on record yet: a crash after the write must not let
/// it be used again.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_nonce_is_committed_before_its_message_goes_out() {
    let test = harness();
    test.contact(ALICE);
    test.contact(BOB);
    test.store.insert_outgoing(ALICE, 1, "text", "hi").unwrap();
    test.group(&[BOB]);
    test.group_message(2, &[BOB]);
    let seen = test
        .connection(async |server: &FakeServer, _: &Commands| {
            server.until_messages(2).await;
            server.seen()
        })
        .await;
    let mut messages = 0;
    for seen in &seen {
        if let Seen::Message {
            nonce_on_record,
            store_committed,
            ..
        } = seen
        {
            messages += 1;
            assert!(*nonce_on_record, "out under a nonce not on record");
            assert!(*store_committed, "out with its nonce not committed");
        }
    }
    assert_eq!(messages, 2);
}

/// The connection drops with no ack: the message goes again on the next one, under a nonce of its
/// own, and after its ack it stays home.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unacked_message_goes_again_after_a_reconnect() {
    let test = harness();
    test.contact(ALICE);
    let message = test
        .store
        .insert_outgoing(ALICE, 1, "text", "hi")
        .unwrap()
        .id;
    let first = test
        .connection(async |server: &FakeServer, _: &Commands| {
            server.until_messages(1).await;
            server.nonces()
        })
        .await;
    assert!(!is_sent(test.state(message)));
    assert_eq!(test.store.pending_outgoing().unwrap().len(), 1);

    let second = test
        .connection(async |server: &FakeServer, _: &Commands| {
            server.until_messages(1).await;
            assert_eq!(server.messages(), [(ALICE.to_owned(), 1)]);
            server.ack(ALICE, 1);
            server.caught_up().await;
            server.nonces()
        })
        .await;
    assert!(is_sent(test.state(message)));
    assert_ne!(first, second, "the same nonce twice");

    test.connection(async |server: &FakeServer, _: &Commands| {
        server.caught_up().await;
        assert!(server.messages().is_empty(), "sent again after its ack");
    })
    .await;
}

/// The connection breaks while writing it: nothing on record says it was sent, and the next
/// connection sends it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_message_lost_in_the_write_goes_again() {
    let test = harness();
    test.contact(ALICE);
    let message = test
        .store
        .insert_outgoing(ALICE, 1, "text", "hi")
        .unwrap()
        .id;
    let (ended, ()) = test
        .ending(
            Some(is_message),
            async |server: &FakeServer, _: &Commands| {
                server.until_session_ended().await;
                assert!(server.messages().is_empty());
            },
        )
        .await;
    assert!(ended.is_err());
    assert!(!is_sent(test.state(message)));
    assert_eq!(test.store.pending_outgoing().unwrap().len(), 1);

    test.connection(async |server: &FakeServer, _: &Commands| {
        server.until_messages(1).await;
        server.ack(ALICE, 1);
        server.caught_up().await;
    })
    .await;
    assert!(is_sent(test.state(message)));
}

/// Asked again while the ack is on its way, the session doesn't send it twice.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_message_on_its_way_is_not_sent_twice_on_one_connection() {
    let test = harness();
    test.contact(ALICE);
    test.store.insert_outgoing(ALICE, 1, "text", "hi").unwrap();
    test.store
        .outbox_add(
            ALICE,
            2,
            OUTBOX_DELIVERY_RECEIPT,
            &serde_json::json!({ "messageId": "1" }),
        )
        .unwrap();
    test.connection(async |server: &FakeServer, commands: &Commands| {
        server.until_messages(2).await;
        commands.send_pending().await;
        commands.send_pending().await;
        assert_eq!(
            server.messages(),
            [(ALICE.to_owned(), 1), (ALICE.to_owned(), 2)]
        );
    })
    .await;
}

/// All outgoing frames of all connections: never a nonce twice.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn no_nonce_is_used_twice_across_connections() {
    let test = harness();
    test.contact(ALICE);
    test.contact(BOB);
    test.group(&[ALICE, BOB]);
    test.store.insert_outgoing(ALICE, 1, "text", "hi").unwrap();
    test.group_message(2, &[ALICE, BOB]);
    let mut nonces = Vec::new();
    for _ in 0..3 {
        let sent = test
            .connection(async |server: &FakeServer, _: &Commands| {
                server.until_messages(3).await;
                server.nonces()
            })
            .await;
        nonces.extend(sent);
    }
    assert_eq!(nonces.len(), 9);
    assert_eq!(distinct(nonces), 9);
}

/// A forward secrecy session goes on across the drop: its Init first, and the message again.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_message_with_forward_secrecy_goes_again_after_a_reconnect() {
    let test = harness();
    let mut contact = crate::contacts::test_contact(ALICE);
    contact.public_key = sender_key().public_key();
    test.store.upsert_contact(&contact).unwrap();
    test.store.picture_sent(ALICE, None).unwrap();
    let message = test
        .store
        .insert_outgoing(ALICE, 1, "text", "hi")
        .unwrap()
        .id;
    let first = test
        .connection(async |server: &FakeServer, _: &Commands| {
            server
                .until("the message", |server| {
                    server.messages().iter().any(|(_, id)| *id == 1)
                })
                .await;
            server.messages()
        })
        .await;
    assert!(first.len() > 1, "an Init comes before the first message");
    assert_eq!(first.last(), Some(&(ALICE.to_owned(), 1)));
    assert!(!is_sent(test.state(message)));

    let nonces = test
        .connection(async |server: &FakeServer, _: &Commands| {
            server
                .until("the message", |server| {
                    server.messages().iter().any(|(_, id)| *id == 1)
                })
                .await;
            server.ack(ALICE, 1);
            server.caught_up().await;
            server.nonces()
        })
        .await;
    assert!(is_sent(test.state(message)));
    assert_eq!(distinct(nonces.clone()), nonces.len());
}

// Groups: every member on their own.

/// One member has it when the connection drops. The next connection goes to the others only, and
/// the message is sent once the last of them has it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_group_message_survives_a_drop_member_by_member() {
    let test = harness();
    let members = [ALICE, BOB, CAROL];
    test.group(&members);
    let message = test.group_message(1, &members);
    test.connection(async |server: &FakeServer, _: &Commands| {
        server.until_messages(3).await;
        assert_eq!(
            distinct(server.messages()),
            3,
            "one frame for each member: {:?}",
            server.messages()
        );
        server.ack(ALICE, 1);
        server.caught_up().await;
        assert!(
            !is_sent(test.state(message)),
            "sent with members still to go"
        );
    })
    .await;
    assert_eq!(
        test.store.pending_receivers(GROUP, 1).unwrap(),
        [BOB.to_owned(), CAROL.to_owned()]
    );

    test.connection(async |server: &FakeServer, _: &Commands| {
        server.until_messages(2).await;
        assert_eq!(
            server.messages(),
            [(BOB.to_owned(), 1), (CAROL.to_owned(), 1)]
        );
        server.ack(BOB, 1);
        server.caught_up().await;
        assert!(!is_sent(test.state(message)));
        server.ack(CAROL, 1);
        server.caught_up().await;
        assert!(is_sent(test.state(message)));
    })
    .await;
    assert!(test.store.pending_receivers(GROUP, 1).unwrap().is_empty());

    test.connection(async |server: &FakeServer, _: &Commands| {
        server.caught_up().await;
        assert!(server.messages().is_empty());
    })
    .await;
}

/// A group's control message: out of the outbox only when every member has it, however many
/// connections that takes, and not repeated to who has it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_group_outbox_entry_stays_until_every_member_has_it() {
    let test = harness();
    let members = [ALICE, BOB];
    test.group(&members);
    let receivers: Vec<_> = members.iter().map(|&member| member.to_owned()).collect();
    test.store
        .group_outbox_add(
            GROUP,
            &receivers,
            crate::groups::OUTBOX_GROUP_NAME,
            &serde_json::json!({ "name": "New name" }),
        )
        .unwrap();
    let id = test.store.outbox().unwrap()[0].message_id;
    test.connection(async |server: &FakeServer, _: &Commands| {
        server.until_messages(2).await;
        server.ack(ALICE, id);
        server.caught_up().await;
    })
    .await;
    assert_eq!(test.store.outbox().unwrap().len(), 1);
    assert_eq!(
        test.store.pending_receivers(GROUP, id).unwrap(),
        [BOB.to_owned()]
    );

    test.connection(async |server: &FakeServer, _: &Commands| {
        server.until_messages(1).await;
        assert_eq!(server.messages(), [(BOB.to_owned(), id)]);
        server.ack(BOB, id);
        server.caught_up().await;
    })
    .await;
    assert!(test.store.outbox().unwrap().is_empty());
}

/// Before a group's text the outbox entries queued before it, members discard it otherwise; and
/// that order holds for a message that goes again.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn what_was_queued_first_for_a_group_goes_first_on_every_connection() {
    let test = harness();
    let members = [ALICE];
    test.group(&members);
    let receivers = vec![ALICE.to_owned()];
    test.store
        .group_outbox_add(
            GROUP,
            &receivers,
            crate::groups::OUTBOX_GROUP_NAME,
            &serde_json::json!({ "name": "New name" }),
        )
        .unwrap();
    let control = test.store.outbox().unwrap()[0].message_id;
    test.group_message(control.wrapping_add(1), &members);
    for _ in 0..2 {
        test.connection(async |server: &FakeServer, _: &Commands| {
            server.until_messages(2).await;
            assert_eq!(server.messages()[0], (ALICE.to_owned(), control));
        })
        .await;
    }
}

// The outbox, 1:1.

/// Reactions, edits and receipts: in the outbox until the server has them.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_outbox_entry_goes_again_until_it_is_acked() {
    let test = harness();
    test.contact(ALICE);
    test.store
        .outbox_add(
            ALICE,
            5,
            crate::reactions::OUTBOX_EDIT,
            &serde_json::json!({ "messageId": "1", "text": "changed" }),
        )
        .unwrap();
    test.connection(async |server: &FakeServer, _: &Commands| {
        server.until_messages(1).await;
        server.caught_up().await;
        assert_eq!(test.store.outbox().unwrap().len(), 1, "gone without an ack");
    })
    .await;
    assert_eq!(test.store.outbox().unwrap().len(), 1);

    test.connection(async |server: &FakeServer, _: &Commands| {
        server.until_messages(1).await;
        assert_eq!(server.messages(), [(ALICE.to_owned(), 5)]);
        server.ack(ALICE, 5);
        server.caught_up().await;
    })
    .await;
    assert!(test.store.outbox().unwrap().is_empty());

    test.connection(async |server: &FakeServer, _: &Commands| {
        server.caught_up().await;
        assert!(server.messages().is_empty());
    })
    .await;
}

/// The whole way once: a message comes in while ours is on its way, and both arrive at their end.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn sending_and_receiving_go_on_side_by_side() {
    let test = harness();
    test.contact(ALICE);
    let ours = test
        .store
        .insert_outgoing(ALICE, 9, "text", "hi")
        .unwrap()
        .id;
    let theirs = test.incoming(ALICE, 1, "hello");
    test.connection(async |server: &FakeServer, _: &Commands| {
        server.until_messages(1).await;
        server.deliver(&theirs);
        server.ack(ALICE, 9);
        server.until_ack_of(ALICE, 1).await;
        server.caught_up().await;
    })
    .await;
    assert!(is_sent(test.state(ours)));
    assert!(test.store.has_incoming(ALICE, 1).unwrap());
}
