// SPDX-License-Identifier: AGPL-3.0-only
//! The chat server connection (CSP): connect, handshake, keepalive, incoming messages, outgoing
//! queue, reconnect with backoff. Socket handling around libthreema's sans-I/O `CspProtocol` and
//! `CspE2eProtocol`, structured like libthreema's `csp_e2e_receive` example; upstream Android
//! equivalent: `ch.threema.domain.protocol.connection`.
//!
//! Runs as a local task on the core thread (libthreema state is not `Send`).

use core::cell::RefCell;
use std::{rc::Rc, sync::Arc, time::Duration};

use libthreema::{
    common::{
        ChatServerGroup, ClientInfo, CspDeviceId, D2xDeviceId, Delta, DeviceCookie, GroupIdentity,
        MessageFlags, MessageId, Nonce, ThreemaId,
        config::{Config, Flavor},
        keys::{ClientKey, PublicKey},
    },
    csp::{
        CspProtocol, CspProtocolContextInit, CspProtocolInstruction, CspStateUpdate,
        payload::{
            EchoPayload, IncomingPayload, MessageAck, MessageWithMetadataBox, OutgoingPayload,
        },
    },
    csp_e2e::{
        CspE2eContextInit, CspE2eProtocol, CspE2eProtocolContextInit, CspE2eProtocolError, D2mRole,
        D2xContextInit, ReflectId,
        contacts::{
            create::{CreateContactsInstruction, CreateContactsResponse},
            lookup::ContactsLookupResponse,
            update::{UpdateContactsInstruction, UpdateContactsResponse},
        },
        fs::{self as fs_protocol, FsSessionStore as _, FsStatus},
        message::task::{
            incoming::{IncomingMessageInstruction, IncomingMessageLoop, IncomingMessageResponse},
            outgoing::{
                encode_message_body, encode_outgoing_contact_message,
                encode_outgoing_contact_message_encapsulated, encode_outgoing_fs_control_message,
            },
        },
        reflect::{ReflectInstruction, ReflectResponse},
        transaction::{
            begin::{BeginTransactionInstruction, BeginTransactionReply, BeginTransactionResponse},
            commit::{CommitTransactionInstruction, CommitTransactionResponse},
        },
    },
    d2m::payload::{
        BeginTransaction, IncomingPayload as D2mIncoming, OutgoingPayload as D2mOutgoing,
        Reflect as D2mReflect, ReflectFlags as D2mReflectFlags,
    },
    model::message::{
        CallAnswerMessage, CallHangupMessage, CallIceCandidateMessage, CallOfferMessage,
        CallRingingMessage, ContactMessageBody, DeleteMessage, DeleteProfilePictureMessage,
        DeliveryReceiptMessage, DeliveryReceiptType, EditMessage, GroupCallStartMessage,
        GroupLeaveMessage, GroupMessageBody, GroupNameMessage, GroupSetupMessage,
        GroupSyncRequestMessage, LegacyReactionMessage, MessageOverrides,
        OutgoingContactMessageBody, OutgoingGroupMessageBody, OutgoingMessage, OutgoingMessageBody,
        PollSetupMessage, PollVoteMessage, ReactionAction, ReactionMessage,
        RequestProfilePictureMessage, SetProfilePictureMessage, TextMessage,
        TypingIndicatorMessage,
    },
};
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot},
    time::{Instant, sleep, timeout},
};
use zeroize::Zeroizing;

use crate::{
    fs_store::StoreFs,
    https::HttpsClient,
    messages::{Message, OutgoingState},
    providers::{
        CallSignalKind, CoreEvent, EventSink, StoreContacts, StoreConversations, StoreNonces,
        StoreSettings, WebShortcuts,
    },
    reactions,
    store::Store,
    transport::{Inbound, Mediator, MediatorLogin, Transport, TransportError},
    websocket::WebSocketError,
};

/// Send an echo request after this much silence; give up if it isn't answered in as long again.
const ECHO_INTERVAL: Duration = Duration::from_secs(60);
/// A handler that reads nothing can outlast an echo's deadline with the answer already in the
/// socket: this long to read it before the connection counts as dead.
const ECHO_LAST_LOOK: Duration = Duration::from_secs(1);
/// How long the notification of a message another device received first waits for that
/// device's "read": long enough for a reply typed there, short enough to still feel live.
const NOTIFY_GRACE: Duration = Duration::from_secs(8);
/// Typing indicators older than this when the connection gets to them are not sent.
const TYPING_MAX_AGE: Duration = Duration::from_secs(5);
/// How often one contact's feature mask is looked up because of what it sent: anyone can send
/// any number of Rejects, and each of a burst would ask the directory the same.
const FS_MASK_CHECK_INTERVAL: Duration = Duration::from_secs(60);
/// The same for an Init against a stored "no". The "no" from a moment ago may be from a linking
/// on the peer's side that has been rolled back since, which is just when an Init arrives.
const FS_MASK_INIT_CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// The lookup holds up the incoming messages, like looking up an unknown sender.
const FS_MASK_LOOKUP_LIMIT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The whole way into the mediator: lookup, addresses in turn (`dial`), TLS and its login. An
/// IPv6 address that takes all its 3 s still leaves IPv4 its own 15 s.
const MEDIATOR_CONNECT_LIMIT: Duration = Duration::from_secs(25);
/// How long a transaction may take to begin, other devices' transactions included.
const TRANSACTION_BEGIN_LIMIT: Duration = Duration::from_secs(90);
/// How long the mediator gets for one answer.
const D2M_REPLY_LIMIT: Duration = Duration::from_secs(30);
/// How long the data for a new device may take to go over the relay: its pictures can be many
/// and the uplink slow (each chunk also has a limit of its own, `rendezvous::WRITE_TIMEOUT`).
const JOIN_TRANSFER_LIMIT: Duration = Duration::from_secs(300);
/// How long the new device may take to register after it has all the data.
const JOIN_REGISTER_LIMIT: Duration = Duration::from_secs(180);
/// How long the core waits for the connection's answer on a join: everything the connection
/// may spend on it, its first connection through the mediator and a minute to spare. Only a
/// connection that no longer works gets here: the join itself always answers within this.
pub(crate) const JOIN_REPLY_LIMIT: Duration = Duration::from_secs(
    MEDIATOR_CONNECT_LIMIT.as_secs()
        + TRANSACTION_BEGIN_LIMIT.as_secs()
        + JOIN_TRANSFER_LIMIT.as_secs()
        + JOIN_REGISTER_LIMIT.as_secs()
        + crate::rendezvous::CLOSE_TIMEOUT.as_secs()
        + D2M_REPLY_LIMIT.as_secs()
        + 60,
);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// The least wait while the store fails: a new connection brings the message it failed on again,
/// and a wake-up or "Connect now" tries at once anyway.
const STORAGE_BACKOFF: Duration = Duration::from_secs(30);
/// The mediator's close code for the chat server having closed its connection (md-d2m.struct.yml).
const MEDIATOR_CHAT_SERVER_CLOSED: u16 = 4000;
/// How long we stay away when a server told us not to come back by ourselves. Only "Connect now"
/// ends this early: what wakes the connection by itself would be coming back by ourselves.
const HOLD_OFF: Duration = Duration::from_secs(600);
/// A login may fail on a bad network, and a server can have a bad minute: it takes this many refused
/// in a row, over at least this long, before the user is told that the server is refusing.
const REFUSED_LOGINS_TO_TELL: u32 = 3;
const REFUSED_LOGINS_SPAN: Duration = Duration::from_secs(5 * 60);
/// How many times the chat server may say "Another connection" with us coming back regardless
/// (Android's `MonitoringLayer`); after that two of them are fighting over the ID.
const ANOTHER_CONNECTION_IGNORED: u32 = 3;
/// A session this long wasn't part of a fight, and starts that count over.
const ANOTHER_CONNECTION_FORGOTTEN: Duration = HOLD_OFF;
/// How long an echo gets after a wake-up: the device stays up for this, the usual interval would
/// outlast that.
const WAKE_ECHO_LIMIT: Duration = Duration::from_secs(15);
/// A wake-up is over this long after the last thing received, so that what comes in a burst gets
/// handled with the device up.
const WAKE_QUIET: Duration = Duration::from_secs(2);
/// A wake-up that found the connection dead waits for the next attempt if it comes this soon.
const WAKE_RETRY_SOON: Duration = Duration::from_secs(2);
/// A session this long was a working connection: the next attempts start over.
const STABLE_SESSION: Duration = Duration::from_secs(60);
/// libthreema's rule for `RateLimitExceeded`: no new connection attempt before this.
const RATE_LIMIT_FLOOR: Duration = Duration::from_secs(10);
/// How far the wait grows while every login meets the rate limit again: each one repeats the
/// lookup that failed, because the message waits for it.
const RATE_LIMIT_MAX_BACKOFF: Duration = Duration::from_secs(300);

/// Connection state as shown in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

impl ConnectionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
        }
    }
}

/// From the app to the connection.
pub enum ToConnection {
    /// A message was queued in the store (state pending): send it if connected.
    SendPending,
    /// Send read receipts for these incoming message ids of a conversation.
    ReadReceipts {
        identity: String,
        message_ids: Vec<u64>,
    },
    /// Send a typing indicator (ephemeral: dropped by the server if the contact is offline).
    Typing {
        identity: String,
        typing: bool,
        at: Instant,
    },
    /// Answered once everything sent before this has been handled; dropped while disconnected.
    Flush(oneshot::Sender<()>),
    /// By itself (timer, network change, back in the app): connect now if disconnected, unless a
    /// server told us to stay away; if connected, verify the connection with an echo. Answered with
    /// [`FromConnection::WakeSettled`].
    Wake,
    /// The user asked ("Connect now"): the same, and it also ends a wait a server asked for.
    ConnectNow,
    /// Drop the connection and connect again at once (the server addresses changed).
    Reconnect,
    /// Multi-device: the linked devices (fails without the mediator).
    Devices(oneshot::Sender<Result<Vec<LinkedDevice>, DeviceGroupError>>),
    /// Multi-device: send the essential data to a new device over its rendezvous link, inside a
    /// NEW_DEVICE_SYNC transaction (device join, ADR 0011).
    NewDeviceSync {
        link: Box<crate::rendezvous::RendezvousLink>,
        reply: oneshot::Sender<JoinOutcome>,
    },
    /// Multi-device: a sync envelope (contact, group, profile, settings changed here) for the
    /// other devices, inside a transaction of its scope (Android's `Reflect*SyncTask`).
    /// (They are queued in the store first.)
    ReflectSync,
    /// Multi-device: remove devices from the group (Android's `DropDevicesSteps`): these, or
    /// with `None` all the others. Once no other device would remain, or with our own id,
    /// multi-device ends here and the connection goes back to the chat server (so no ids at
    /// all is Android's "deactivate if alone").
    DropDevices {
        ids: Option<Vec<u64>>,
        reply: oneshot::Sender<Result<DropOutcome, DeviceGroupError>>,
    },
    /// A forward secrecy control message, from outside the incoming messages: the Terminate for
    /// a deleted contact. Dropped while disconnected: the contact's next message is rejected.
    FsControl {
        peer: String,
        peer_key: Option<PublicKey>,
        envelope: Vec<u8>,
    },
    /// A call signalling message (ADR 0012); dropped while disconnected, the call times out.
    CallSignal {
        peer: String,
        signal: crate::calls::Signal,
    },
}

/// Why the connection couldn't answer a question about the device group.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceGroupError {
    /// No working connection now (the UI says to check the network).
    #[error("not connected")]
    NotConnected,
    /// The server or the protocol said no.
    #[error("{0}")]
    Failed(String),
}

impl From<ConnectionError> for DeviceGroupError {
    fn from(error: ConnectionError) -> Self {
        match error {
            ConnectionError::Io(_)
            | ConnectionError::Timeout
            | ConnectionError::Stopped
            | ConnectionError::Reconnect => Self::NotConnected,
            other => Self::Failed(other.to_string()),
        }
    }
}

/// How a device join ended. What matters after a failure is whether the identity has left this
/// device: until then nothing happened that needs undoing on the mediator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinOutcome {
    /// The new device has registered.
    Joined,
    /// Failed before the essential data went out.
    FailedBeforeHandover(String),
    /// Failed with the essential data out (or on its way): the new device may have the ID and
    /// may be in the group, or get there yet.
    FailedAfterHandover(String),
}

impl JoinOutcome {
    /// `registered` is all that decides success: a commit that fails afterwards only releases
    /// the lock late, and the new device is in the group either way.
    fn from_steps(handed_over: bool, registered: Result<(), String>) -> Self {
        match registered {
            Ok(()) => Self::Joined,
            Err(reason) if handed_over => Self::FailedAfterHandover(reason),
            Err(reason) => Self::FailedBeforeHandover(reason),
        }
    }
}

/// What removing devices left: what the mediator listed minus what it confirmed removed.
#[derive(Debug, Clone)]
pub struct DropOutcome {
    pub remaining: Vec<LinkedDevice>,
    /// This device left as well: multi-device is off.
    pub left_group: bool,
}

/// A device of the device group (multi-device), for the Linked devices page.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedDevice {
    /// Hex (a u64 doesn't fit a JavaScript number).
    pub id: String,
    #[serde(skip)]
    pub raw_id: u64,
    /// This device.
    pub current: bool,
    pub label: String,
    /// `android`, `ios`, `desktop`, `web` or `unknown`.
    pub platform: &'static str,
    pub platform_details: String,
    pub app_version: String,
    /// Connected since (ms), if connected.
    pub connected_since: Option<u64>,
    /// Last disconnect (ms), if not connected.
    pub last_disconnect_at: Option<u64>,
}

/// What Threema's servers said, or why they don't let us in: for the user to know until the next
/// login, as from outside it all looks like a bad network otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfflineReason {
    /// An alert or a close error, in the words of the chat server (Android's `ServerMessageModel`).
    Server(String),
    /// The mediator closed the connection with a code that rules out reconnecting by itself: the
    /// UI words it, by the code.
    Mediator(u16),
    /// The chat server keeps closing the connection during login, without a word.
    LoginRefused,
}

impl OfflineReason {
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Server(text) => serde_json::json!({ "kind": "server", "text": text }),
            Self::Mediator(code) => serde_json::json!({ "kind": "mediator", "code": code }),
            Self::LoginRefused => serde_json::json!({ "kind": "loginRefused" }),
        }
    }
}

/// From the connection to the app/UI.
pub enum FromConnection {
    State(ConnectionState),
    Event(Box<CoreEvent>),
    /// Something the user should know, for as long as the next login is away.
    OfflineReason(OfflineReason),
    /// What a wake-up was for has happened or won't soon: the device may sleep again.
    WakeSettled,
}

/// Everything the connection needs; owned by the connection task.
pub struct ConnectionSetup {
    pub store: Rc<Store>,
    /// Our profile picture (uploaded for distribution).
    pub media: std::sync::Arc<crate::media::MediaStore>,
    pub https: HttpsClient,
    pub config: Arc<Config>,
    pub identity: ThreemaId,
    pub client_key: Zeroizing<[u8; 32]>,
    pub server_group: ChatServerGroup,
    pub device_cookie: DeviceCookie,
    /// Who we are to the chat server and the mediator.
    pub client_info: ClientInfo,
    /// Feature mask lookups for forward secrecy's sake; outlive a connection.
    pub fs_mask_checks: FsMaskChecks,
    /// Where the store is failing; the UI has been told.
    pub storage_failing: StorageFailing,
    pub failing_items: FailingItems,
    /// Answers to what has been received that the last connection couldn't send
    /// ([`waits_for_the_next_connection`]). In the store too (`FS_EVENTS_CARRIED`); here for when
    /// writing that failed, as it may well.
    pub carried_events: RefCell<Vec<CoreEvent>>,
    /// What a wake-up is waiting for; it may take a new connection to find out.
    pub wake_hold: WakeHold,
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
enum WakeWait {
    #[default]
    Nothing,
    /// For the first sign of life.
    Answer,
    /// Something came in: until this, unless more does.
    QuietUntil(Instant),
}

/// How far the last wake-up has got. The device is held awake for it, which costs battery every
/// few minutes, and letting go too early leaves a dead connection dead until the next time. So it
/// lasts until something has been received and has stopped coming, or until nothing can come soon.
/// After a login that takes the servers saying they have handed out what they kept (like Android's
/// `BaseServerConnection`, which holds on until the queue is sent); if they never do, the UI's limit
/// ends it.
#[derive(Default)]
pub struct WakeHold {
    wait: core::cell::Cell<WakeWait>,
    /// The chat server's queue isn't through yet.
    chat_queue_open: core::cell::Cell<bool>,
    /// The mediator's queue isn't through yet.
    reflection_queue_open: core::cell::Cell<bool>,
}

impl WakeHold {
    fn asked(&self) {
        // Nothing was waiting for those queues, on a session that may be hours old.
        if self.wait.get() == WakeWait::Nothing {
            self.chat_queue_open.set(false);
            self.reflection_queue_open.set(false);
        }
        self.wait.set(WakeWait::Answer);
    }

    /// A new session. Through the mediator the chat server only hands out to the leader.
    fn logged_in(&self, through_mediator: bool) {
        self.chat_queue_open.set(!through_mediator);
        self.reflection_queue_open.set(through_mediator);
        self.wait_for_the_queues();
    }

    fn promoted_to_leader(&self) {
        self.chat_queue_open.set(true);
        self.wait_for_the_queues();
    }

    fn wait_for_the_queues(&self) {
        if self.wait.get() != WakeWait::Nothing {
            self.wait.set(WakeWait::Answer);
        }
    }

    fn chat_queue_through(&self, now: Instant) {
        self.chat_queue_open.set(false);
        self.heard(now);
    }

    fn reflection_queue_through(&self, now: Instant) {
        self.reflection_queue_open.set(false);
        self.heard(now);
    }

    /// The connection works, as far as it can be seen from here.
    fn heard(&self, now: Instant) {
        if self.wait.get() != WakeWait::Nothing
            && !self.chat_queue_open.get()
            && !self.reflection_queue_open.get()
        {
            self.wait.set(WakeWait::QuietUntil(now + WAKE_QUIET));
        }
    }

    fn settles_at(&self) -> Option<Instant> {
        match self.wait.get() {
            WakeWait::QuietUntil(at) => Some(at),
            WakeWait::Nothing | WakeWait::Answer => None,
        }
    }

    fn settle(&self, tx: &mpsc::UnboundedSender<FromConnection>) {
        if self.wait.take() != WakeWait::Nothing {
            let _ = tx.send(FromConnection::WakeSettled);
        }
    }
}

/// What was being done when the store failed. Each of these has its own way of showing that it
/// works again, and only that may end the notice: a multi-device login, for one, runs through the
/// mediator's queue every time and only then comes to the chat server's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageStep {
    /// Handling what the chat server sent. Over when a message got through or its queue is empty.
    Incoming,
    /// Applying what another device reflected. Over when one got applied or the queue is empty.
    Reflected,
    /// Anything else. Over when a connection gets through what it does at the start.
    Other,
}

/// Which of the [`StorageStep`]s the store is failing in.
#[derive(Default)]
pub struct StorageFailing(core::cell::Cell<[bool; 3]>);

impl StorageFailing {
    /// Whether this is the first failure of a streak.
    fn failed(&self, step: StorageStep) -> bool {
        let mut failing = self.0.get();
        let first = !failing.contains(&true);
        failing[step as usize] = true;
        self.0.set(failing);
        first
    }

    /// Whether this ended the streak.
    fn works(&self, step: StorageStep) -> bool {
        let mut failing = self.0.get();
        let was_failing = core::mem::take(&mut failing[step as usize]);
        self.0.set(failing);
        was_failing && !failing.contains(&true)
    }
}

/// A message or reflected envelope is given up on when it has failed this many times in a row in
/// a way that says trying again won't help ([`ConnectionError::fails_every_time`]). It would hold
/// up all that comes after it, for good: one bug of ours must not stop someone's whole inbox.
const TRIES_BEFORE_DROPPING: u32 = 3;
/// More than this many failing at once isn't one bad message; start counting over.
const FAILING_ITEMS_MAX: usize = 64;

/// What came in and failed.
#[derive(Clone, PartialEq, Eq, Hash)]
enum FailingItem {
    Message {
        sender: String,
        id: u64,
    },
    /// By the nonce the encrypted envelope starts with.
    Envelope(Vec<u8>),
}

/// How often each of them has failed in a row; outlives a connection.
#[derive(Default)]
pub struct FailingItems(RefCell<std::collections::HashMap<FailingItem, u32>>);

impl FailingItems {
    /// One more failure: how many that makes.
    fn failed(&self, item: &FailingItem) -> u32 {
        let mut items = self.0.borrow_mut();
        if items.len() >= FAILING_ITEMS_MAX {
            items.clear();
        }
        let failures = items.entry(item.clone()).or_default();
        *failures += 1;
        *failures
    }

    fn forget(&self, item: &FailingItem) {
        let _ = self.0.borrow_mut().remove(item);
    }
}

/// When a contact's feature mask was last looked up for forward secrecy's sake, and whether we can
/// go by what that found.
#[derive(Default)]
pub struct FsMaskChecks(RefCell<std::collections::HashMap<String, (Instant, bool)>>);

impl FsMaskChecks {
    /// Whether the last lookup answered, if there was one within `interval`.
    fn recent(&self, peer: &str, now: Instant, interval: Duration) -> Option<bool> {
        let checks = self.0.borrow();
        let (at, answered) = checks.get(peer)?;
        (now.saturating_duration_since(*at) < interval).then_some(*answered)
    }

    fn record(&self, peer: &str, now: Instant, answered: bool) {
        let _ = self.0.borrow_mut().insert(peer.to_owned(), (now, answered));
    }

    /// The last answer may be out of date: no going by it, and no new lookup before its time.
    fn distrust(&self, peer: &str) {
        if let Some((_, answered)) = self.0.borrow_mut().get_mut(peer) {
            *answered = false;
        }
    }
}

/// What a look at a contact's feature mask came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MaskCheck {
    /// The directory has just answered.
    Fresh,
    /// It answered a moment ago; asked again, it would too soon.
    Recent,
    /// No answer we can go by: the stored mask may be stale.
    Unknown,
}

#[derive(Debug, thiserror::Error)]
enum ConnectionError {
    #[error("network: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("connection timed out")]
    Timeout,
    /// A directory lookup inside libthreema's message task met its rate limit.
    #[error("rate limited")]
    RateLimited,
    /// The chat server closed the connection after our login, instead of answering it.
    #[error("login refused")]
    LoginRefused,
    #[error("closed by server: {message}")]
    Closed {
        message: String,
        can_reconnect: bool,
    },
    #[error("stopped")]
    Stopped,
    #[error("reconnecting")]
    Reconnect,
    /// The mediator closed the connection (its close codes: md-d2m.struct.yml).
    #[error("closed by the mediator ({code}): {reason}")]
    MediatorClosed { code: u16, reason: String },
    /// Our store failed (SQLite's code): nothing the network can help with.
    #[error("storage failed ({code})")]
    Storage { code: i32, step: StorageStep },
    /// libthreema gave up on a message for a reason of its own, neither storage nor network.
    #[error("incoming message: {0}")]
    IncomingMessage(String),
}

/// What tells a reflected envelope from the others: the random nonce at its start.
fn envelope_nonce(envelope: &[u8]) -> Vec<u8> {
    envelope.iter().take(24).copied().collect()
}

fn storage_error(error: crate::store::StoreError) -> ConnectionError {
    ConnectionError::Storage {
        code: error.sqlite_code(),
        step: StorageStep::Other,
    }
}

impl ConnectionError {
    /// If the store failed: while doing this.
    fn while_doing(self, step: StorageStep) -> Self {
        match self {
            Self::Storage { code, .. } => Self::Storage { code, step },
            other => other,
        }
    }

    /// Trying again won't help: this comes from what is being handled, not from the state of the
    /// device or the network.
    fn fails_every_time(&self) -> bool {
        match self {
            Self::Storage { code, .. } => crate::store::fails_every_time(*code),
            Self::IncomingMessage(_) => true,
            _ => false,
        }
    }

    /// What failed was the connection, not what it was doing: that can be done on the next one.
    /// (Anything else could fail the same way every time, and end every connection.)
    fn is_connection_lost(&self) -> bool {
        matches!(
            self,
            Self::Io(_)
                | Self::Timeout
                | Self::LoginRefused
                | Self::Closed { .. }
                | Self::Stopped
                | Self::Reconnect
                | Self::MediatorClosed { .. }
        )
    }
}

/// Handling this event takes the connection. Nothing else may wait for that: whatever the store
/// holds, the UI hears of even if the connection is gone.
fn needs_the_connection(event: &CoreEvent) -> bool {
    matches!(
        event,
        CoreEvent::FsControl { .. }
            | CoreEvent::FsStatus { .. }
            | CoreEvent::OutboxChanged
            | CoreEvent::ReceiptQueued { .. }
    )
}

/// Handled by the message's own connection, before the next message of a burst; if that connection
/// ends, the app sees to it (`UnknownMembers`), or the next one does (`SyncQueued`).
fn runs_before_the_next_message(event: &CoreEvent) -> bool {
    matches!(event, CoreEvent::SyncQueued | CoreEvent::UnknownMembers(_))
}

/// An event nothing else in the store stands for: the message that caused it is acknowledged, with
/// its nonce noted, so libthreema won't answer again. An answer lost here is a session the peer
/// never gets to use or a message of theirs that stays "sent", and a lost report of a Reject is a
/// session gone and our message not sent again.
fn waits_for_the_next_connection(event: &CoreEvent) -> bool {
    matches!(
        event,
        CoreEvent::FsControl { .. } | CoreEvent::FsStatus { .. }
    )
}

/// How such an event lies in the store, encrypted like everything there. All it holds: IDs, and a
/// control envelope (Accept, Reject or Terminate), which has a session id, a public key, the id of
/// a rejected message and its group, and never a message's content.
fn carried_to_json(event: &CoreEvent) -> Option<serde_json::Value> {
    use serde_json::json;
    match event {
        CoreEvent::FsControl {
            peer,
            peer_key,
            envelope,
        } => Some(json!({
            "peer": peer,
            "peerKey": peer_key.map(|key| key.to_string()),
            "envelope": data_encoding::HEXLOWER.encode(envelope),
        })),
        CoreEvent::FsStatus { peer, status } => {
            let mut stored = match status {
                FsStatus::Established => json!({"status": "established"}),
                FsStatus::Reset => json!({"status": "reset"}),
                FsStatus::MessageWithoutFs => json!({"status": "messageWithoutFs"}),
                FsStatus::Terminated { session_known } => {
                    json!({"status": "terminated", "sessionKnown": session_known})
                }
                FsStatus::MessageRejected {
                    message_id,
                    group_identity,
                    session_known,
                } => json!({
                    "status": "messageRejected",
                    "messageId": format!("{message_id:016x}"),
                    "group": group_identity.as_ref().map(crate::groups::key),
                    "sessionKnown": session_known,
                }),
            };
            stored["peer"] = json!(peer);
            Some(stored)
        }
        _ => None,
    }
}

fn carried_from_json(stored: &serde_json::Value) -> Option<CoreEvent> {
    let peer = stored["peer"].as_str()?.to_owned();
    let session_known = stored["sessionKnown"].as_bool().unwrap_or(true);
    if let Some(envelope) = stored["envelope"].as_str() {
        let peer_key = match stored["peerKey"].as_str() {
            Some(key) => {
                let key = data_encoding::HEXLOWER.decode(key.as_bytes()).ok()?;
                Some(PublicKey::try_from(key.as_slice()).ok()?)
            }
            None => None,
        };
        return Some(CoreEvent::FsControl {
            peer,
            peer_key,
            envelope: data_encoding::HEXLOWER.decode(envelope.as_bytes()).ok()?,
        });
    }
    let status = match stored["status"].as_str()? {
        "established" => FsStatus::Established,
        "reset" => FsStatus::Reset,
        "messageWithoutFs" => FsStatus::MessageWithoutFs,
        "terminated" => FsStatus::Terminated { session_known },
        "messageRejected" => FsStatus::MessageRejected {
            message_id: u64::from_str_radix(stored["messageId"].as_str()?, 16).ok()?,
            group_identity: stored["group"].as_str().and_then(crate::groups::parse_key),
            session_known,
        },
        _ => return None,
    };
    Some(CoreEvent::FsStatus { peer, status })
}

/// What a connection ended on, for the next one, also after a restart.
fn save_carried_events(
    store: &crate::store::Store,
    events: &[CoreEvent],
) -> Result<(), crate::store::StoreError> {
    use crate::store::keys::FS_EVENTS_CARRIED;
    if events.is_empty() {
        return store.delete_setting(FS_EVENTS_CARRIED);
    }
    let stored: Vec<_> = events.iter().filter_map(carried_to_json).collect();
    store.set_setting(
        FS_EVENTS_CARRIED,
        &serde_json::Value::from(stored).to_string(),
    )
}

fn saved_carried_events(
    store: &crate::store::Store,
) -> Result<Vec<CoreEvent>, crate::store::StoreError> {
    let saved = store.setting(crate::store::keys::FS_EVENTS_CARRIED)?;
    Ok(saved
        .and_then(|saved| serde_json::from_str::<Vec<serde_json::Value>>(&saved).ok())
        .unwrap_or_default()
        .iter()
        .filter_map(carried_from_json)
        .collect())
}

impl From<TransportError> for ConnectionError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::Io(error) => Self::Io(error),
            TransportError::Closed => Self::Io(std::io::ErrorKind::ConnectionAborted.into()),
            TransportError::WriteTimeout => Self::Timeout,
            // The socket itself, or its end without a close code.
            TransportError::WebSocket(WebSocketError::Socket(error)) => {
                Self::Io(std::io::Error::other(error))
            }
            TransportError::WebSocket(WebSocketError::Closed { code: None, .. }) => {
                Self::Io(std::io::ErrorKind::ConnectionAborted.into())
            }
            TransportError::WebSocket(WebSocketError::Closed {
                code: Some(code),
                reason,
            }) => Self::MediatorClosed { code, reason },
            other => Self::Protocol(other.to_string()),
        }
    }
}

/// The D2D conversation id of our conversation key (contacts, groups).
fn d2d_conversation(conversation: &str) -> Option<libthreema::protobuf::d2d::ConversationId> {
    use libthreema::protobuf::d2d::{ConversationId, conversation_id::Id};
    let id = match crate::groups::parse_key(conversation) {
        Some(group) => Id::Group(libthreema::protobuf::common::GroupIdentity {
            group_id: group.group_id,
            creator_identity: group.creator_identity.as_str().to_owned(),
        }),
        None if conversation.len() == 8 => Id::Contact(conversation.to_owned()),
        None => return None,
    };
    Some(ConversationId { id: Some(id) })
}

/// A mediator payload's kind, for logs (its content may be private).
fn d2m_name(payload: &D2mIncoming) -> &'static str {
    match payload {
        D2mIncoming::Proxy(_) => "proxy",
        D2mIncoming::ReflectionQueueDry => "reflection-queue-dry",
        D2mIncoming::RolePromotedToLeader => "role-promoted-to-leader",
        D2mIncoming::DevicesInfo(_) => "devices-info",
        D2mIncoming::DropDeviceAck(_) => "drop-device-ack",
        D2mIncoming::BeginTransactionAck => "begin-transaction-ack",
        D2mIncoming::CommitTransactionAck => "commit-transaction-ack",
        D2mIncoming::TransactionRejected(_) => "transaction-rejected",
        D2mIncoming::TransactionEnded(_) => "transaction-ended",
        D2mIncoming::ReflectAck(_) => "reflect-ack",
        D2mIncoming::Reflected(_) => "reflected",
        D2mIncoming::UnknownPayload { .. } => "unknown",
    }
}

/// What libthreema reports, in which a provider's storage failure is only text.
fn protocol_error(error: impl std::fmt::Display) -> ConnectionError {
    let error = error.to_string();
    match crate::providers::storage_code(&error) {
        Some(code) => ConnectionError::Storage {
            code,
            step: StorageStep::Other,
        },
        None => ConnectionError::Protocol(error),
    }
}

/// What our messages say about our nickname. Without one, they tell contacts to drop the name
/// they know; for an ID whose user hasn't answered yet (restored from an ID backup, which holds no
/// nickname), that would be the app's doing, so they say nothing.
fn nickname_delta(nickname: Option<&str>, unanswered: bool) -> Delta<&str> {
    match nickname {
        Some(nickname) => Delta::Update(nickname),
        None if unanswered => Delta::Unchanged,
        None => Delta::Remove,
    }
}

/// Like [`protocol_error`], but a rate limit stays what it is (it has its own wait), and what isn't
/// the network's or a server's doing is the message's.
fn e2e_error(error: CspE2eProtocolError) -> ConnectionError {
    match error {
        CspE2eProtocolError::RateLimitExceeded => ConnectionError::RateLimited,
        CspE2eProtocolError::NetworkError(_)
        | CspE2eProtocolError::ServerError(_)
        | CspE2eProtocolError::InvalidCredentials => protocol_error(error),
        other => match protocol_error(other) {
            ConnectionError::Protocol(text) => ConnectionError::IncomingMessage(text),
            storage => storage,
        },
    }
}

/// A message without forward secrecy reports it before the message is stored, and its line goes
/// into the chat after the message: the events have to say so in that order too, or a live chat
/// shows the line above the message until it is opened again.
fn status_after_messages(mut events: Vec<CoreEvent>) -> Vec<CoreEvent> {
    events.sort_by_key(|event| {
        matches!(
            event,
            CoreEvent::FsStatus {
                status: FsStatus::MessageWithoutFs,
                ..
            }
        )
    });
    events
}

/// What became of a rejected message.
#[derive(Default)]
struct Requeued {
    /// Something waits in the store: send it.
    send: bool,
    /// The rejected message goes out again.
    resending: bool,
    /// The messages put back in the queue, for the UI.
    changed: Vec<Message>,
    /// The group whose state goes out to the peer instead.
    state_of: Option<String>,
}

/// Put a rejected message back in the queue, if it is ours to send again (only the store, no
/// sending). With a group named it goes by that group, without one by the ID: a group message of
/// ours for a member, or the 1:1 message, which like on Android needs a new session.
fn requeue_rejected(
    store: &crate::store::Store,
    user: &str,
    peer: &str,
    message_id: u64,
    group_identity: Option<GroupIdentity>,
) -> Result<Requeued, crate::store::StoreError> {
    if let Some(group_identity) = group_identity {
        let rejected = crate::groups::message_rejected(
            store,
            user,
            peer,
            group_identity,
            message_id,
            &|identity| store.is_explicitly_blocked(identity),
        )?;
        return Ok(Requeued {
            send: rejected.send,
            resending: rejected.resending,
            changed: Vec::new(),
            state_of: rejected
                .state_sent
                .then(|| crate::groups::key(&group_identity)),
        });
    }
    let in_group = store.requeue_group_receiver(peer, message_id)?;
    let changed = store.reset_outgoing(peer, message_id)?;
    let resending = in_group || !changed.is_empty();
    Ok(Requeued {
        send: resending,
        resending,
        changed,
        state_of: None,
    })
}

/// The status line for a message that came without forward secrecy on a session that has it.
const FS_MESSAGE_WITHOUT: &str = "fsMessageWithoutFs";

/// The status line for a forward secrecy event, if it gets one. Lines only appear for sessions
/// that existed, and say what happened as far as we know it: a contact that ended a session has
/// not necessarily turned anything off, that is for the feature mask to say, and "sent again"
/// is only said of messages that really are.
fn fs_status_line(status: &FsStatus, supports_fs: bool, resending: bool) -> Option<&'static str> {
    match status {
        FsStatus::Established => supports_fs.then_some("fsEstablished"),
        FsStatus::Reset => Some("fsReset"),
        FsStatus::MessageWithoutFs if supports_fs => Some(FS_MESSAGE_WITHOUT),
        FsStatus::MessageWithoutFs => Some("fsUnavailable"),
        FsStatus::MessageRejected {
            session_known: false,
            ..
        }
        | FsStatus::Terminated {
            session_known: false,
        } => None,
        FsStatus::MessageRejected { .. } | FsStatus::Terminated { .. } if !supports_fs => {
            Some("fsUnavailable")
        }
        FsStatus::MessageRejected { .. } if resending => Some("fsMessageRejected"),
        FsStatus::MessageRejected { .. } | FsStatus::Terminated { .. } => Some("fsReset"),
    }
}

/// Android shows the servers' words as HTML. They stay plain text here, which nothing gets to parse
/// or load: what is markup goes, and what would show as markup becomes what it stands for.
fn plain_server_text(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        plain.push_str(&rest[..start]);
        let tag = &rest[start + 1..];
        let opens_a_tag =
            tag.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!');
        match tag.find('>').filter(|_| opens_a_tag) {
            Some(end) => {
                let name = tag[..end].trim_end_matches('/').trim().to_ascii_lowercase();
                if name == "br" || name == "/p" {
                    plain.push('\n');
                }
                rest = &tag[end + 1..];
            }
            // A lone "<", as in "a < b".
            None => {
                plain.push('<');
                rest = tag;
            }
        }
    }
    plain.push_str(rest);
    // `&amp;` last: "&amp;lt;" stands for the text "&lt;".
    [
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&nbsp;", " "),
        ("&amp;", "&"),
    ]
    .iter()
    .fold(plain, |text, (entity, character)| {
        text.replace(entity, character)
    })
    .trim()
    .to_owned()
}

/// What the chat server's words tell the user; nothing, if there aren't any.
fn server_reason(message: &str) -> Option<OfflineReason> {
    let text = plain_server_text(message);
    (!text.is_empty()).then_some(OfflineReason::Server(text))
}

/// "Another connection" is often our own earlier connection after a change of network, and another
/// device has a warning of its own, the device cookie's. Android checks for it just like this.
fn is_another_connection(message: &str) -> bool {
    message.contains("Another connection")
}

/// What a close error tells the user. Not "Another connection" (Android's
/// `IncomingMessageProcessorImpl` leaves it out too).
fn close_reason(message: &str) -> Option<OfflineReason> {
    if is_another_connection(message) {
        None
    } else {
        server_reason(message)
    }
}

/// How often the server has said "Another connection" lately.
#[derive(Default)]
struct AnotherConnection(u32);

impl AnotherConnection {
    /// It said so again, after a session this long. Whether to stay away, if it asks us to: not
    /// the first few times, as on Android, since it mostly is a change of network, and coming back
    /// after a moment beats ten minutes offline without a word. A real fight over the ID doesn't
    /// go on past those: then a login every [`HOLD_OFF`] at most, however long this runs.
    fn stay_away(&mut self, session: Duration) -> bool {
        if session > ANOTHER_CONNECTION_FORGOTTEN {
            self.0 = 0;
        }
        self.0 = self.0.saturating_add(1);
        self.0 > ANOTHER_CONNECTION_IGNORED
    }
}

/// What it means when reading failed during the handshake. After our login the server has an
/// answer to give, and closing the connection instead is how it says no. It doesn't send a word
/// before the login is through, so there is no reason to go by. Through the mediator that end of
/// the chat server's connection comes as a close code.
fn handshake_read_failed(login_sent: bool, error: ConnectionError) -> ConnectionError {
    match error {
        ConnectionError::Io(ref io)
            if login_sent
                && matches!(
                    io.kind(),
                    std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::ConnectionReset
                ) =>
        {
            ConnectionError::LoginRefused
        }
        ConnectionError::MediatorClosed {
            code: MEDIATOR_CHAT_SERVER_CLOSED,
            ..
        } if login_sent => ConnectionError::LoginRefused,
        other => other,
    }
}

/// Logins refused one after the other, with nothing else going wrong in between.
#[derive(Default)]
struct RefusedLogins {
    /// The first of them, and how many so far.
    streak: Option<(Stopwatch, u32)>,
    told: bool,
}

impl RefusedLogins {
    /// One more. Returns whether it is the first, and whether to tell the user now: once per streak.
    fn refused(&mut self) -> (bool, bool) {
        let first = self.streak.is_none();
        let (began, count) = self.streak.get_or_insert_with(|| (Stopwatch::start(), 0));
        *count = count.saturating_add(1);
        let tell = !self.told
            && *count >= REFUSED_LOGINS_TO_TELL
            && began.elapsed() >= REFUSED_LOGINS_SPAN;
        self.told |= tell;
        (first, tell)
    }

    fn over(&mut self) {
        *self = Self::default();
    }
}

/// The wait before the next attempt, given the wait so far and how long the session lasted. A
/// working connection starts over, unless the server told us to stay away: then how long the
/// session lasted says nothing (another device took over after an hour, say), and coming back at
/// once would be the fight the hold is there to avoid.
fn backoff_after_session(backoff: Duration, session: Duration) -> Duration {
    if backoff < HOLD_OFF && session > STABLE_SESSION {
        Duration::from_secs(1)
    } else {
        backoff
    }
}

/// How long something took, sleep of the device included: tokio's clock stands still for that, and
/// a session in a pocket would never get to look like one that worked.
struct Stopwatch {
    awake: Instant,
    wall: std::time::SystemTime,
}

impl Stopwatch {
    fn start() -> Self {
        Self {
            awake: Instant::now(),
            wall: std::time::SystemTime::now(),
        }
    }

    /// The system clock may be set back: then it's what tokio's clock says.
    fn elapsed(&self) -> Duration {
        self.awake
            .elapsed()
            .max(self.wall.elapsed().unwrap_or_default())
    }
}

/// Whether a wake-up keeps the device up for the next attempt, after a session or a failed login:
/// only if that comes soon, and only after something that got through. Otherwise a wake-up that
/// found the connection dead would let the device sleep with nothing connected for another round.
fn next_attempt_is_worth_holding_for(came_up: bool, backoff: Duration) -> bool {
    came_up && backoff <= WAKE_RETRY_SOON
}

/// The wait after the one just over.
fn next_backoff(backoff: Duration, rate_limited: bool) -> Duration {
    let most = if rate_limited {
        RATE_LIMIT_MAX_BACKOFF
    } else {
        MAX_BACKOFF
    };
    (backoff * 2).min(most)
}

/// To the user nothing else tells a failing store from a bad network: one word per streak, however
/// many connections it takes.
fn note_storage_failure(
    setup: &ConnectionSetup,
    tx: &mpsc::UnboundedSender<FromConnection>,
    code: i32,
    step: StorageStep,
) {
    if !setup.storage_failing.failed(step) {
        tracing::info!(code, "storage still failing");
    } else {
        tracing::error!(code, "storage failed while connected");
        let event = CoreEvent::StorageProblem { active: true };
        let _ = tx.send(FromConnection::Event(Box::new(event)));
    }
}

/// Whether a wake-up that came by itself ends this wait. After a server told us to stay away it
/// doesn't, until that time has passed by the clock: the wait's own timer stands still while the
/// device sleeps, and would stretch ten minutes to hours in a pocket.
fn wake_ends_wait(backoff: Duration, waited_by_the_clock: Duration) -> bool {
    backoff < HOLD_OFF || waited_by_the_clock >= backoff
}

/// Wait for the backoff, or end it on a wake-up: after a rate limit, as soon as its floor allows.
/// `false`: the app has gone away.
async fn wait_for_next_attempt(
    backoff: Duration,
    earliest: Option<Instant>,
    rx: &mut mpsc::UnboundedReceiver<ToConnection>,
    tx: &mpsc::UnboundedSender<FromConnection>,
    wake_hold: &WakeHold,
) -> bool {
    let began = std::time::SystemTime::now();
    let wait = sleep(backoff);
    tokio::pin!(wait);
    loop {
        tokio::select! {
            () = &mut wait => return true,
            message = rx.recv() => match message {
                None => return false,
                Some(ToConnection::ConnectNow | ToConnection::Reconnect) => match earliest {
                    Some(at) if Instant::now() < at => wait.as_mut().reset(at),
                    _ => return true,
                },
                Some(ToConnection::Wake) => {
                    wake_hold.asked();
                    let waited = began.elapsed().unwrap_or_default();
                    match earliest {
                        _ if !wake_ends_wait(backoff, waited) => wake_hold.settle(tx),
                        Some(at) if Instant::now() < at => {
                            wait.as_mut().reset(at);
                            wake_hold.settle(tx);
                        }
                        _ => return true,
                    }
                }
                Some(ToConnection::Devices(reply)) => {
                    let _ = reply.send(Err(DeviceGroupError::NotConnected));
                }
                Some(ToConnection::DropDevices { reply, .. }) => {
                    let _ = reply.send(Err(DeviceGroupError::NotConnected));
                }
                // Lost while offline (like Android's tasks, which need a connection too).
                // Queued in the store: sent after connecting.
                Some(ToConnection::ReflectSync) => {}
                Some(ToConnection::NewDeviceSync { reply, .. }) => {
                    // The mediator connection after activating multi-device did not come
                    // up: the join cannot run (the UI rolls the activation back).
                    tracing::warn!("device join failed: not connected");
                    let reason = DeviceGroupError::NotConnected.to_string();
                    let _ = reply.send(JoinOutcome::FailedBeforeHandover(reason));
                }
                Some(_) => {}, // queued in the store, sent after connecting
            },
        }
    }
}

/// Run the connection until the app goes away (`rx` closed). Reconnects with backoff.
pub async fn run(
    setup: ConnectionSetup,
    mut rx: mpsc::UnboundedReceiver<ToConnection>,
    tx: mpsc::UnboundedSender<FromConnection>,
) {
    let mut backoff = Duration::from_secs(1);
    let mut refused_logins = RefusedLogins::default();
    let mut another_connection = AnotherConnection::default();
    loop {
        let _ = tx.send(FromConnection::State(ConnectionState::Connecting));
        let started = Stopwatch::start();
        let session = Session::connect(&setup, &tx).await;
        let came_up = session.is_ok();
        let result = match session {
            Ok(session) => session.run(&mut rx).await,
            Err(error) => Err(error),
        };
        let _ = tx.send(FromConnection::State(ConnectionState::Disconnected));
        // Anything else means it wasn't one refusal after the other.
        if !matches!(result, Err(ConnectionError::LoginRefused)) {
            refused_logins.over();
        }
        let rate_limited = matches!(result, Err(ConnectionError::RateLimited));
        match result {
            Err(ConnectionError::Stopped) => return,
            Err(ConnectionError::RateLimited) => setup.store.rest_directory_calls(),
            Err(ConnectionError::Reconnect) => {
                backoff = Duration::from_secs(1);
                continue;
            }
            Err(ConnectionError::Closed {
                message,
                can_reconnect,
            }) => {
                tracing::warn!(can_reconnect, "chat server closed the connection");
                if let Some(reason) = close_reason(&message) {
                    let _ = tx.send(FromConnection::OfflineReason(reason));
                }
                let stay_away = if is_another_connection(&message) {
                    another_connection.stay_away(started.elapsed())
                } else {
                    true
                };
                if !can_reconnect && stay_away {
                    // E.g. another device keeps taking over this ID. Don't fight it.
                    backoff = HOLD_OFF;
                }
            }
            // Upstream's close codes (md-d2m.struct.yml): this device was dropped from the group
            // or its slot doesn't match: multi-device is over here, go back to the chat server.
            Err(ConnectionError::MediatorClosed {
                code: code @ (4113 | 4115),
                ..
            }) => {
                tracing::warn!(code, "removed from the device group");
                if let Err(error) = setup.store.set_multi_device(None) {
                    tracing::error!(%error, "removing multi-device failed");
                }
                let _ = tx.send(FromConnection::Event(Box::new(
                    CoreEvent::MultiDeviceChanged,
                )));
                backoff = Duration::from_secs(1);
                continue;
            }
            // 41xx: don't reconnect by ourselves (version, device limit, duplicate connection,
            // reflection queue full).
            Err(ConnectionError::MediatorClosed { code, reason })
                if (4100..4200).contains(&code) =>
            {
                // log-ok: Threema's mediator's own text
                tracing::warn!(code, %reason, "mediator closed the connection");
                let _ = tx.send(FromConnection::OfflineReason(OfflineReason::Mediator(code)));
                backoff = HOLD_OFF;
            }
            // One word per streak in the log, which it would otherwise fill by the day.
            Err(ConnectionError::LoginRefused) => {
                let (first, tell) = refused_logins.refused();
                if first {
                    tracing::warn!("chat server closed the connection during login");
                } else {
                    tracing::info!("login refused again");
                }
                if tell {
                    tracing::warn!("logins keep being refused");
                    let reason = OfflineReason::LoginRefused;
                    let _ = tx.send(FromConnection::OfflineReason(reason));
                }
            }
            // A server's answer we can't make sense of, not a bad network.
            Err(error @ ConnectionError::Protocol(_)) if !came_up => {
                tracing::warn!(%error, "login failed");
            }
            // No use trying harder.
            Err(ConnectionError::Storage { code, step }) => {
                note_storage_failure(&setup, &tx, code, step);
                backoff = backoff.max(STORAGE_BACKOFF);
            }
            Err(error) => tracing::info!(%error, "chat server connection lost"),
            Ok(()) => {}
        }
        // A login that failed isn't a session, however long the device slept in the middle of it.
        let session_length = if came_up {
            started.elapsed()
        } else {
            Duration::ZERO
        };
        backoff = backoff_after_session(backoff, session_length);
        let earliest = rate_limited.then(|| {
            backoff = backoff.max(RATE_LIMIT_FLOOR);
            tokio::time::Instant::now() + RATE_LIMIT_FLOOR
        });
        if !next_attempt_is_worth_holding_for(came_up, backoff) {
            setup.wake_hold.settle(&tx);
        }
        if !wait_for_next_attempt(backoff, earliest, &mut rx, &tx, &setup.wake_hold).await {
            return;
        }
        backoff = next_backoff(backoff, rate_limited);
    }
}

/// Which devices to remove, from what the mediator lists now (Android's `runDropDevicesSteps`):
/// only ones that are in the group; none named means all of them. When that leaves no other
/// device, this one goes too: multi-device ends rather than lingering with forward security off.
fn plan_drop(requested: Option<&[u64]>, others: &[u64], own: u64) -> (Vec<u64>, bool) {
    let to_drop: Vec<u64> = match requested {
        Some(ids) => others
            .iter()
            .copied()
            .filter(|other| ids.contains(other))
            .collect(),
        None => others.to_vec(),
    };
    let drop_self =
        to_drop.len() == others.len() || requested.is_some_and(|ids| ids.contains(&own));
    (to_drop, drop_self)
}

/// Whether the connection is alive: anything read counts, wherever it was read, and an echo gets
/// its full time from when it went out. Time a long handler spent without reading says nothing
/// about the connection.
#[derive(Debug, Clone, Copy)]
struct Liveness {
    last_received: Instant,
    /// An echo is out and this is when it must have been answered by.
    echo_deadline: Option<Instant>,
}

impl Liveness {
    const fn new(now: Instant) -> Self {
        Self {
            last_received: now,
            echo_deadline: None,
        }
    }

    const fn received(&mut self, now: Instant) {
        self.last_received = now;
        self.echo_deadline = None;
    }

    const fn awaiting_echo(&self) -> bool {
        self.echo_deadline.is_some()
    }

    /// An echo went out, to be answered within `time`.
    fn echo_sent(&mut self, now: Instant, time: Duration) {
        self.echo_deadline = Some(now + time);
    }

    /// The echo that is out has to be answered by then at the latest.
    fn echo_due_by(&mut self, at: Instant) {
        self.echo_deadline = self.echo_deadline.map(|deadline| deadline.min(at));
    }

    /// When to act: send an echo after the silence, or give up on the one that is out.
    fn deadline(&self) -> Instant {
        self.echo_deadline
            .unwrap_or(self.last_received + ECHO_INTERVAL)
    }
}

/// An echo's time ran out: what the read brings within [`ECHO_LAST_LOOK`], or the timeout.
async fn last_look_for_echo<T>(read: impl Future<Output = T>) -> Result<T, ConnectionError> {
    tokio::time::timeout(ECHO_LAST_LOOK, read)
        .await
        .map_err(|_| ConnectionError::Timeout)
}

struct Session<'a> {
    setup: &'a ConnectionSetup,
    transport: Transport,
    liveness: Liveness,
    /// Mediator payloads that arrived during the CSP handshake or while waiting for a
    /// specific one, handled in order afterwards.
    d2m_queue: std::collections::VecDeque<D2mIncoming>,
    /// Multi-device membership (with the mediator transport).
    multi_device: Option<crate::multidevice::Properties>,
    /// Forward security switched off by the user (Settings → Advanced).
    fs_disabled: bool,
    /// Messages already re-sent once after an FS rejection: (peer, message id).
    fs_resent: std::collections::HashSet<(String, u64)>,
    /// Forwarded incoming messages (another device leads) whose notification waits, with
    /// when it is due: read there meanwhile, the phone stays silent (user report 2026-10-28).
    held: Vec<(i64, Instant)>,
    /// The mediator's backlog after login is not handed out completely yet: what it holds
    /// is decided at the end, when the read updates in it have arrived too.
    reflection_backlog: bool,
    /// Our reflect ids (apart from libthreema's, which count from 0 per connection).
    next_reflect_id: u32,
    /// Outgoing messages reflected on this connection: (conversation, message id).
    reflected: std::collections::HashSet<(String, u64)>,
    csp: CspProtocol,
    e2e: CspE2eProtocol,
    events: EventSink,
    tx: &'a mpsc::UnboundedSender<FromConnection>,
    /// Outgoing messages sent but not acknowledged by the server yet: (receiver, message id).
    awaiting_ack: Vec<(String, u64)>,
}

impl Drop for Session<'_> {
    /// The connection ends with notifications still held: they are not lost, whatever is
    /// unread notifies now. So do the events of what has been received, whichever step failed:
    /// the store has it, and the server won't send it again.
    fn drop(&mut self) {
        self.emit_events_unless(needs_the_connection);
        self.release_held_notifications(true);
        let mut carried = self.setup.carried_events.borrow_mut();
        carried.extend(
            self.events
                .borrow_mut()
                .drain(..)
                .filter(waits_for_the_next_connection),
        );
        if let Err(error) = save_carried_events(&self.setup.store, &carried) {
            tracing::warn!(
                code = error.sqlite_code(),
                "saving what is left to send failed"
            );
        }
    }
}

impl<'a> Session<'a> {
    async fn run(
        mut self,
        rx: &mut mpsc::UnboundedReceiver<ToConnection>,
    ) -> Result<(), ConnectionError> {
        let _ = self
            .tx
            .send(FromConnection::State(ConnectionState::Connected));
        tracing::info!("connected to chat server");
        self.setup.wake_hold.logged_in(self.transport.is_mediator());
        while let Some(payload) = self.d2m_queue.pop_front() {
            self.handle_d2m(payload).await?;
        }
        self.resume_carried_events().await?;
        self.terminate_fs_if_pending().await?;
        self.reflect_queued_syncs().await?;
        self.send_pending().await?;
        self.storage_works(StorageStep::Other);
        self.payload_loop(rx).await
    }

    /// TCP straight to the chat server (no multi-device).
    async fn connect_direct(setup: &ConnectionSetup) -> Result<TcpStream, ConnectionError> {
        let hosts: Vec<_> = setup
            .config
            .chat_server_address
            .addresses(setup.server_group)
            .into_iter()
            .map(|address| (crate::https::server_url(&address.0).into_owned(), address.1))
            .collect();
        let endpoints: Vec<_> = hosts
            .iter()
            .map(|(host, port)| (host.as_str(), *port))
            .collect();
        let stream = crate::dial::connect_tcp(&endpoints)
            .await
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::TimedOut => ConnectionError::Timeout,
                _ => ConnectionError::Io(error),
            })?;
        // log-ok: Threema's server's address
        tracing::info!(address = ?stream.peer_addr().ok(), "chat server reached");
        stream.set_nodelay(true)?;
        Ok(stream)
    }

    async fn connect(
        setup: &'a ConnectionSetup,
        tx: &'a mpsc::UnboundedSender<FromConnection>,
    ) -> Result<Session<'a>, ConnectionError> {
        // Multi-device (ADR 0011): through the mediator, without forward security (SE-199).
        let multi_device = setup.store.multi_device().map_err(storage_error)?;
        let fs_disabled = setup
            .store
            .forward_security_disabled()
            .map_err(storage_error)?;
        let device_group_key = multi_device
            .as_ref()
            .map(|properties| {
                properties.device_group_key().ok_or_else(|| {
                    ConnectionError::Protocol("stored device group key invalid".into())
                })
            })
            .transpose()?;
        let transport = match (&multi_device, device_group_key) {
            (Some(properties), Some(device_group_key)) => {
                let mediator = timeout(
                    MEDIATOR_CONNECT_LIMIT,
                    Mediator::connect(
                        &setup.config,
                        MediatorLogin {
                            device_group_key,
                            device_id: properties.mediator_device_id,
                            registered: properties.registered,
                            server_group: setup.server_group,
                            device_label: setup
                                .store
                                .setting(crate::multidevice::SETTING_DEVICE_LABEL)
                                .map_err(storage_error)?
                                .filter(|label| !label.is_empty())
                                .unwrap_or_else(|| crate::multidevice::DEVICE_LABEL.to_owned()),
                            client_info: setup.client_info.clone(),
                        },
                    ),
                )
                .await
                .map_err(|_| ConnectionError::Timeout)??;
                tracing::info!(
                    max_device_slots = mediator
                        .server_info
                        .as_ref()
                        .map(|info| info.max_device_slots),
                    "mediator reached"
                );
                if !properties.registered {
                    // The slot exists now; from here on the mediator must know us.
                    let mut registered = setup.store.multi_device().map_err(storage_error)?;
                    if let Some(registered) = registered.as_mut() {
                        registered.registered = true;
                        setup
                            .store
                            .set_multi_device(Some(registered))
                            .map_err(storage_error)?;
                    }
                }
                Transport::Mediator(Box::new(mediator))
            }
            _ => Transport::Direct(Self::connect_direct(setup).await?),
        };

        let client_key = ClientKey::from(*setup.client_key);
        let (csp, client_hello) = CspProtocol::new(
            CspProtocolContextInit {
                permanent_server_keys: setup.config.chat_server_public_keys.clone(),
                identity: setup.identity,
                client_key: ClientKey::from(*setup.client_key),
                client_info: setup.client_info.clone(),
                device_cookie: Some(setup.device_cookie),
                csp_device_id: multi_device
                    .as_ref()
                    .map(|properties| CspDeviceId(properties.csp_device_id)),
            }
            .try_into()
            .map_err(protocol_error)?,
        );

        let events: EventSink = Rc::new(RefCell::new(Vec::new()));
        let store = &setup.store;
        let e2e = CspE2eProtocol::new(CspE2eProtocolContextInit {
            client_info: setup.client_info.clone(),
            config: Arc::clone(&setup.config),
            csp_e2e: CspE2eContextInit {
                user_identity: setup.identity,
                client_key,
                flavor: Flavor::Consumer,
                nonce_storage: Box::new(RefCell::new(StoreNonces(Rc::clone(store)))),
                fs_sessions: Some(Box::new(RefCell::new(StoreFs {
                    store: Rc::clone(store),
                    my_identity: setup.identity,
                    events: Rc::clone(&events),
                }))),
                // No sessions with multi-device (SE-199) or when the user switched forward
                // security off (Settings → Advanced). Contacts may not know yet and messages may
                // already wait on the server: those get a Reject, so that they are sent again
                // without, and aren't lost until the contact's mask refreshes.
                fs_answering_only: multi_device.is_some() || fs_disabled,
            },
            d2x: match &multi_device {
                Some(properties) => Some(D2xContextInit {
                    device_id: D2xDeviceId(properties.mediator_device_id),
                    device_group_key: properties.device_group_key().ok_or_else(|| {
                        ConnectionError::Protocol("stored device group key invalid".into())
                    })?,
                    nonce_storage: Box::new(RefCell::new(crate::multidevice::StoreD2dNonces(
                        Rc::clone(store),
                    ))),
                }),
                None => None,
            },
            shortcut: Box::new(WebShortcuts {
                events: Rc::clone(&events),
            }),
            settings: Box::new(RefCell::new(StoreSettings(Rc::clone(store)))),
            contacts: Box::new(RefCell::new(StoreContacts {
                store: Rc::clone(store),
                user_identity: setup.identity,
                events: Rc::clone(&events),
            })),
            conversations: Box::new(RefCell::new(StoreConversations {
                store: Rc::clone(store),
                events: Rc::clone(&events),
                user: setup.identity,
                from_sync: false,
            })),
        });

        let mut session = Session {
            setup,
            transport,
            liveness: Liveness::new(Instant::now()),
            d2m_queue: std::collections::VecDeque::new(),
            multi_device,
            fs_disabled,
            fs_resent: std::collections::HashSet::new(),
            held: Vec::new(),
            reflection_backlog: true,
            next_reflect_id: 0x8000_0000,
            reflected: std::collections::HashSet::new(),
            csp,
            e2e,
            events,
            tx,
            awaiting_ack: Vec::new(),
        };
        timeout(CONNECT_TIMEOUT, session.handshake(client_hello.0))
            .await
            .map_err(|_| ConnectionError::Timeout)??;
        Ok(session)
    }

    /// The next thing from the server; whichever step reads it, it shows the connection works.
    async fn read_inbound(&mut self, buffer: &mut [u8]) -> Result<Inbound, TransportError> {
        let inbound = self.transport.read(buffer).await?;
        self.liveness.received(Instant::now());
        self.setup.wake_hold.heard(Instant::now());
        Ok(inbound)
    }

    /// client-hello → server-hello → login → login-ack (through the mediator: proxied, while
    /// mediator payloads are queued for later).
    async fn handshake(&mut self, client_hello: Vec<u8>) -> Result<(), ConnectionError> {
        self.transport.write_csp(&client_hello).await?;
        let mut buffer = vec![0_u8; 8192];
        let mut login_sent = false;
        loop {
            while let Some(instruction) = self.csp.poll().map_err(protocol_error)? {
                if instruction.incoming_payload.is_some() {
                    return Err(ConnectionError::Protocol("payload during handshake".into()));
                }
                if let Some(frame) = instruction.outgoing_frame {
                    self.transport.write_csp(&frame.0).await?;
                    login_sent = true;
                }
                if let Some(CspStateUpdate::PostHandshake(_)) = instruction.state_update {
                    return Ok(());
                }
            }
            let inbound = self.read_inbound(&mut buffer).await;
            match inbound.map_err(|error| handshake_read_failed(login_sent, error.into()))? {
                Inbound::Csp(bytes) => self.csp.add_chunks(&[&bytes]).map_err(protocol_error)?,
                Inbound::D2m(payload) => self.d2m_queue.push_back(payload),
            }
        }
    }

    async fn send_payload(&mut self, payload: &OutgoingPayload) -> Result<(), ConnectionError> {
        let instruction = self.csp.create_payload(payload).map_err(protocol_error)?;
        self.handle_instruction(instruction).await
    }

    /// Send every pending outgoing message from the store, then the outbox. In a group the user's
    /// order holds across the two: members discard a text for a group whose setup is still to
    /// come, and a text after our leave. So what was queued before a group message goes first.
    async fn send_pending(&mut self) -> Result<(), ConnectionError> {
        for message in self.setup.store.pending_outgoing().map_err(storage_error)? {
            let group = crate::groups::parse_key(&message.conversation);
            if group.is_some() {
                for entry in self
                    .setup
                    .store
                    .outbox_until(&message.conversation, message.created_at)
                    .map_err(storage_error)?
                {
                    self.send_outbox_entry(entry).await?;
                }
            }
            if group.is_none()
                && self
                    .awaiting_ack
                    .iter()
                    .any(|(to, id)| *to == message.conversation && *id == message.message_id)
            {
                continue;
            }
            // As it is now: the awaits below let a command run, an edit or a removal among them.
            let Some(message) = self
                .setup
                .store
                .message(message.id)
                .map_err(storage_error)?
                .filter(|now| now.state == OutgoingState::Pending as i64 && now.kind != "deleted")
            else {
                continue;
            };
            let body = match message.kind.as_str() {
                "text" => ContactMessageBody::Text(TextMessage {
                    text: message.body.clone(),
                }),
                "poll" => {
                    let poll: serde_json::Value =
                        serde_json::from_str(&message.body).unwrap_or_default();
                    let Some(poll_id) = poll["pollId"]
                        .as_str()
                        .and_then(|id| u64::from_str_radix(id, 16).ok())
                    else {
                        continue;
                    };
                    // Announced open; closing is a message of its own (`pollClosed`).
                    let mut open = poll.clone();
                    open["state"] = serde_json::json!(0);
                    ContactMessageBody::PollSetup(PollSetupMessage {
                        poll_id,
                        poll: crate::polls::setup_json(&open),
                    })
                }
                // Closing our poll: the closed setup with the results.
                crate::polls::KIND_CLOSED => {
                    let poll: serde_json::Value =
                        serde_json::from_str(&message.body).unwrap_or_default();
                    let Some(poll_id) = poll["pollId"]
                        .as_str()
                        .and_then(|id| u64::from_str_radix(id, 16).ok())
                    else {
                        continue;
                    };
                    ContactMessageBody::PollSetup(PollSetupMessage {
                        poll_id,
                        poll: crate::polls::setup_json(&poll),
                    })
                }
                "location" => match crate::messages::location_message(&message.body) {
                    Some(location) => ContactMessageBody::Location(location),
                    None => continue,
                },
                // Only once uploaded (the upload task asks for this again when it is done).
                "file" => {
                    let body: serde_json::Value =
                        serde_json::from_str(&message.body).unwrap_or_default();
                    let keys = self
                        .setup
                        .store
                        .file_keys(message.id)
                        .map_err(storage_error)?;
                    match keys
                        .and_then(|keys| crate::app_media::outgoing_file_message(&body, &keys))
                    {
                        Some(file) => ContactMessageBody::File(file),
                        None => continue,
                    }
                }
                other => {
                    tracing::warn!(kind = other, "cannot send message kind yet");
                    continue;
                }
            };
            if let Some(group) = group {
                let Some(body) = to_group_body(body) else {
                    continue;
                };
                self.note_written_out(message.id)?;
                if self
                    .send_to_group(
                        &message.conversation,
                        group,
                        message.message_id,
                        message.created_at,
                        &body,
                    )
                    .await?
                {
                    // Nobody (left) to send it to, e.g. a group of one.
                    let changed = self
                        .setup
                        .store
                        .raise_outgoing_state(
                            &message.conversation,
                            &[message.message_id],
                            OutgoingState::Sent,
                        )
                        .map_err(storage_error)?;
                    if !changed.is_empty() {
                        self.emit(CoreEvent::MessagesChanged(changed));
                    }
                }
                continue;
            }
            let body = Outgoing::Contact(body);
            // Queued before the contact was blocked or revoked: not sent, and nothing about it
            // reaches the other devices or the contact.
            if !self.may_receive(&message.conversation, &body)? {
                if let Some(failed) = self
                    .setup
                    .store
                    .set_outgoing_failed(message.id)
                    .map_err(storage_error)?
                {
                    self.emit(CoreEvent::MessagesChanged(vec![failed]));
                }
                continue;
            }
            self.note_written_out(message.id)?;
            if matches!(
                &body,
                Outgoing::Contact(
                    ContactMessageBody::Text(_)
                        | ContactMessageBody::Location(_)
                        | ContactMessageBody::File(_)
                        | ContactMessageBody::PollSetup(_)
                        | ContactMessageBody::PollVote(_)
                )
            ) {
                self.distribute_picture(&message.conversation).await?;
            }
            self.reflect_outgoing(
                &message.conversation,
                message.message_id,
                message.created_at,
                &body,
            )
            .await?;
            match self
                .encode(
                    &message.conversation,
                    message.message_id,
                    message.created_at,
                    body,
                )
                .await
            {
                Ok(Some(payloads)) => {
                    for payload in payloads {
                        self.send_payload(&OutgoingPayload::MessageWithMetadataBox(payload))
                            .await?;
                    }
                    self.awaiting_ack
                        .push((message.conversation.clone(), message.message_id));
                }
                Ok(None) => {
                    if let Some(failed) = self
                        .setup
                        .store
                        .set_outgoing_failed(message.id)
                        .map_err(storage_error)?
                    {
                        self.emit(CoreEvent::MessagesChanged(vec![failed]));
                    }
                }
                Err(error) => return Err(error),
            }
        }
        self.send_outbox().await
    }

    /// Before a message's first byte goes out: someone may have it from here on, and the chat hears
    /// so, as offering Edit depends on it.
    fn note_written_out(&self, id: i64) -> Result<(), ConnectionError> {
        if let Some(changed) = self
            .setup
            .store
            .set_written_out(id)
            .map_err(storage_error)?
        {
            self.emit(CoreEvent::MessagesChanged(vec![changed]));
        }
        Ok(())
    }

    /// The _Profile Picture Distribution Steps_ for `receiver`: our picture (uploaded once per
    /// week) or a removal, if they don't have the current state yet. Failures only skip it.
    async fn distribute_picture(&mut self, receiver: &str) -> Result<(), ConnectionError> {
        use crate::profile_pictures::{Distribution, OWN_PICTURE, Upload};
        // Not even recorded as sent: they get it if they are unblocked.
        if !crate::providers::may_receive(&self.setup.store, &self.setup.config, receiver, false)
            .map_err(storage_error)?
        {
            return Ok(());
        }
        let store = Rc::clone(&self.setup.store);
        let own = self.setup.media.read_picture(OWN_PICTURE).ok().flatten();
        let body = match store
            .picture_distribution(receiver, own.is_some())
            .map_err(storage_error)?
        {
            Distribution::None => return Ok(()),
            Distribution::Delete => {
                store.picture_sent(receiver, None).map_err(storage_error)?;
                ContactMessageBody::DeleteProfilePicture(DeleteProfilePictureMessage)
            }
            Distribution::Set(upload) => {
                let upload = match (upload, own) {
                    (Some(upload), _) => upload,
                    (None, Some(picture)) => {
                        let mut key = [0_u8; 32];
                        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut key);
                        let blob = crate::media::encrypt_blob(
                            &key,
                            crate::media::MediaPart::File,
                            &picture,
                        );
                        let blob_id =
                            match crate::media::upload(&self.setup.https, &blob, true).await {
                                Ok(id) => id,
                                Err(error) => {
                                    tracing::warn!(%error, "profile picture upload failed");
                                    return Ok(());
                                }
                            };
                        let upload = Upload {
                            blob_id,
                            key,
                            size: u32::try_from(picture.len()).unwrap_or(u32::MAX),
                            uploaded_at: crate::messages::now_ms(),
                        };
                        store
                            .set_picture_upload(Some(&upload))
                            .map_err(storage_error)?;
                        upload
                    }
                    (None, None) => return Ok(()),
                };
                store
                    .picture_sent(receiver, Some(&upload.blob_id))
                    .map_err(storage_error)?;
                ContactMessageBody::SetProfilePicture(SetProfilePictureMessage {
                    blob_id: upload.blob_id,
                    size: upload.size,
                    key: upload.key,
                })
            }
        };
        let created_at = crate::messages::now_ms();
        for payload in self
            .encode(
                receiver,
                MessageId::random().0,
                created_at,
                Outgoing::Contact(body),
            )
            .await?
            .unwrap_or_default()
        {
            self.send_payload(&OutgoingPayload::MessageWithMetadataBox(payload))
                .await?;
        }
        Ok(())
    }

    /// Send a group message (same id for everyone) to the members that don't have it yet;
    /// `true` if nobody is left. Members we have no contact for yet are tried again later;
    /// revoked and (unless the message is exempt) blocked ones never get it.
    async fn send_to_group(
        &mut self,
        key: &str,
        group: GroupIdentity,
        message_id: u64,
        created_at: i64,
        body: &GroupMessageBody,
    ) -> Result<bool, ConnectionError> {
        let receivers = crate::providers::group_receivers(
            &self.setup.store,
            &self.setup.config,
            key,
            message_id,
            body.is_exempt_from_blocking(),
            !goes_to_non_members(body),
        )
        .map_err(storage_error)?;
        // Before the members: our own other devices want it even when there are none — a notes
        // group (Threema's group of one) would otherwise never reach Threema Desktop.
        self.reflect_outgoing(
            key,
            message_id,
            created_at,
            &Outgoing::Group(group, body.clone()),
        )
        .await?;
        if receivers.is_empty() {
            return Ok(true);
        }
        for receiver in receivers {
            if self
                .awaiting_ack
                .iter()
                .any(|(to, id)| *to == receiver && *id == message_id)
            {
                continue;
            }
            if distributes_profile(body) {
                self.distribute_picture(&receiver).await?;
            }
            let features = self
                .setup
                .store
                .contact(&receiver)
                .map_err(storage_error)?
                .map(|contact| contact.feature_mask);
            match self
                .encode(
                    &receiver,
                    message_id,
                    created_at,
                    Outgoing::Group(group, body_for_receiver(body, features)),
                )
                .await?
            {
                Some(payloads) => {
                    for payload in payloads {
                        self.send_payload(&OutgoingPayload::MessageWithMetadataBox(payload))
                            .await?;
                    }
                    self.awaiting_ack.push((receiver, message_id));
                }
                None => tracing::info!("group member can't be sent to yet, sending later"),
            }
        }
        Ok(false)
    }

    /// Send the outbox (reactions, edits, deletions, group control): removed once the server
    /// acknowledged it (for every receiver of a group), or if unsendable.
    async fn send_outbox(&mut self) -> Result<(), ConnectionError> {
        for entry in self.setup.store.outbox().map_err(storage_error)? {
            self.send_outbox_entry(entry).await?;
        }
        Ok(())
    }

    async fn send_outbox_entry(
        &mut self,
        entry: reactions::OutboxEntry,
    ) -> Result<(), ConnectionError> {
        if let Some(group) = crate::groups::parse_key(&entry.conversation) {
            let Some(body) = outbox_group_body(&entry) else {
                tracing::warn!(kind = entry.kind, "dropping unsendable group outbox entry");
                self.setup
                    .store
                    .outbox_remove(&entry.conversation, entry.message_id)
                    .map_err(storage_error)?;
                return Ok(());
            };
            if self
                .send_to_group(
                    &entry.conversation,
                    group,
                    entry.message_id,
                    outbox_sent_as_of(&entry),
                    &body,
                )
                .await?
            {
                self.setup
                    .store
                    .outbox_remove(&entry.conversation, entry.message_id)
                    .map_err(storage_error)?;
            }
            return Ok(());
        }
        if self
            .awaiting_ack
            .iter()
            .any(|(to, id)| *to == entry.conversation && *id == entry.message_id)
        {
            return Ok(());
        }
        let encoded = match outbox_body(&entry) {
            Some(body)
                if self.may_receive(&entry.conversation, &Outgoing::Contact(body.clone()))? =>
            {
                // A delivery receipt says nothing the other devices go by.
                if entry.kind != reactions::OUTBOX_DELIVERY_RECEIPT {
                    self.reflect_outgoing(
                        &entry.conversation,
                        entry.message_id,
                        entry.created_at,
                        &Outgoing::Contact(body.clone()),
                    )
                    .await?;
                }
                self.encode(
                    &entry.conversation,
                    entry.message_id,
                    entry.created_at,
                    Outgoing::Contact(body),
                )
                .await?
            }
            _ => None,
        };
        if let Some(payloads) = encoded {
            for payload in payloads {
                self.send_payload(&OutgoingPayload::MessageWithMetadataBox(payload))
                    .await?;
            }
            self.awaiting_ack
                .push((entry.conversation.clone(), entry.message_id));
        } else {
            tracing::warn!(kind = entry.kind, "dropping unsendable outbox entry");
            self.setup
                .store
                .outbox_remove(&entry.conversation, entry.message_id)
                .map_err(storage_error)?;
        }
        Ok(())
    }

    /// A fresh nonce, recorded as used (like libthreema does for incoming ones).
    fn fresh_nonce(&self) -> Result<Nonce, ConnectionError> {
        let nonce = loop {
            let nonce = Nonce::random();
            if !self
                .setup
                .store
                .has_nonce(&nonce.0)
                .map_err(storage_error)?
            {
                break nonce;
            }
        };
        self.setup
            .store
            .add_nonces(&[&nonce.0])
            .map_err(storage_error)?;
        Ok(nonce)
    }

    fn fs_store(&self) -> StoreFs {
        StoreFs {
            store: Rc::clone(&self.setup.store),
            my_identity: self.setup.identity,
            events: Rc::clone(&self.events),
        }
    }

    /// Encode a forward security control envelope for `receiver`.
    fn encode_control(
        &self,
        receiver: &str,
        peer_key: Option<PublicKey>,
        envelope: &[u8],
    ) -> Result<Option<MessageWithMetadataBox>, ConnectionError> {
        let Some((identity, public_key)) = crate::providers::control_receiver(
            &self.setup.store,
            &self.setup.config,
            receiver,
            peer_key,
        )
        .map_err(storage_error)?
        else {
            return Ok(None);
        };
        let nonce = self.fresh_nonce()?;
        match encode_outgoing_fs_control_message(
            self.setup.identity,
            &ClientKey::from(*self.setup.client_key),
            identity,
            &public_key,
            envelope,
            u64::try_from(crate::messages::now_ms()).unwrap_or(0),
            nonce,
        ) {
            Ok(payload) => Ok(Some(payload)),
            Err(error) => {
                tracing::warn!(%error, "cannot encode FS control message");
                Ok(None)
            }
        }
    }

    /// Whether `body` may go to `receiver` (see [`crate::providers::may_receive`]).
    fn may_receive(&self, receiver: &str, body: &Outgoing) -> Result<bool, ConnectionError> {
        crate::providers::may_receive(
            &self.setup.store,
            &self.setup.config,
            receiver,
            body.is_exempt_from_blocking(),
        )
        .map_err(storage_error)
    }

    /// Encode a message to a contact: forward-security encapsulated if the contact supports it
    /// (with an `Init` first for a new session), otherwise plain. `Ok(None)` if it can't be sent:
    /// an unknown or revoked contact, or a blocked one unless the message is exempt. Every message
    /// passes here, so receipts, typing, picture requests and call signals stay away from them too,
    /// and no forward secrecy session starts.
    async fn encode(
        &self,
        receiver: &str,
        message_id: u64,
        created_at: i64,
        body: Outgoing,
    ) -> Result<Option<Vec<MessageWithMetadataBox>>, ConnectionError> {
        let Some(contact) = self.setup.store.contact(receiver).map_err(storage_error)? else {
            return Ok(None);
        };
        if !self.may_receive(receiver, &body)? {
            return Ok(None);
        }
        let receiver_identity = contact.identity;
        let receiver_key: PublicKey = contact.public_key;
        let nickname = self
            .setup
            .store
            .setting(crate::store::keys::NICKNAME)
            .map_err(storage_error)?;
        let nickname_unanswered = self
            .setup
            .store
            .setting(crate::store::keys::NICKNAME_UNANSWERED)
            .map_err(storage_error)?
            .is_some();
        let client_key = ClientKey::from(*self.setup.client_key);
        let mut payloads = Vec::new();

        // Forward security, if the contact supports it.
        let mut envelope = None;
        // Never with multi-device (SE-199): the other devices couldn't follow the ratchet.
        // Nor when the user switched forward security off (Settings → Advanced).
        let fs_on = !self.transport.is_mediator() && !self.fs_disabled;
        // A session against a stored "no": the mask is stale, or the session is left over. The
        // directory decides, and without its answer the session stays.
        let mut contact = contact;
        if fs_on
            && !crate::contacts::has_fs_support(&contact)
            && self.has_fs_session(receiver_identity)
        {
            let check = self
                .check_feature_mask(receiver, FS_MASK_CHECK_INTERVAL)
                .await?;
            let Some(fresh) = self.setup.store.contact(receiver).map_err(storage_error)? else {
                return Ok(None);
            };
            contact = fresh;
            if check != MaskCheck::Unknown && !crate::contacts::has_fs_support(&contact) {
                // Nothing here may keep this message from going out without forward secrecy.
                match self.end_fs_sessions(receiver) {
                    Ok(controls) => {
                        if !controls.is_empty() {
                            self.show_fs_line(receiver, "fsUnavailable")?;
                        }
                        payloads.extend(controls);
                    }
                    Err(error) => tracing::warn!(%error, "cannot end forward secrecy sessions"),
                }
            }
        }
        // The feature mask alone decides, like Android: a session is no proof, since a peer that
        // dropped forward security answers everything in it with a Terminate or nothing at all.
        // That the mask can be stale the other way round is seen to when an Init arrives.
        let use_fs = fs_on && crate::contacts::has_fs_support(&contact);
        // log-ok: the contact's feature mask only
        tracing::debug!(
            feature_mask = contact.feature_mask.0,
            use_fs,
            "encoding outgoing message"
        );
        let body = match body {
            Outgoing::Contact(body) => OutgoingMessageBody::Contact(OutgoingContactMessageBody {
                receiver_identity,
                body,
            }),
            Outgoing::Group(group_identity, body) => {
                OutgoingMessageBody::Group(OutgoingGroupMessageBody {
                    group_identity,
                    body,
                })
            }
        };
        if use_fs {
            let (inner_type, inner_data) = encode_message_body(&body);
            let group_identity = if let OutgoingMessageBody::Group(group) = &body {
                Some((&group.group_identity).into())
            } else {
                None
            };
            match crate::fs_store::encapsulate_or_replace(
                &mut self.fs_store(),
                &client_key,
                receiver_identity,
                &receiver_key,
                inner_type,
                &inner_data,
                group_identity,
                u64::try_from(crate::messages::now_ms()).unwrap_or(0),
            ) {
                Ok((encapsulation, replaced)) => {
                    if replaced {
                        // First the Terminate for the old session, then the new one's Init.
                        payloads.extend(self.take_fs_controls(receiver)?);
                        self.show_fs_line(receiver, "fsReset")?;
                    }
                    for control in encapsulation.prelude {
                        payloads.extend(self.encode_control(
                            receiver,
                            Some(receiver_key),
                            &control,
                        )?);
                    }
                    envelope = encapsulation.envelope;
                }
                // Never fall back silently for a broken session. This contact's message stays
                // unsent; everyone else's, and everything incoming, must not depend on it.
                Err(error) => {
                    tracing::warn!(%error, "cannot encapsulate; message not sent");
                    return Ok(None);
                }
            }
        }

        let nonce = self.fresh_nonce()?;
        let message = OutgoingMessage {
            id: MessageId(message_id),
            overrides: MessageOverrides::default(),
            created_at: u64::try_from(created_at).unwrap_or(0),
            body,
        };
        let encoded = match &envelope {
            Some(envelope) => encode_outgoing_contact_message_encapsulated(
                self.setup.identity,
                &client_key,
                nickname_delta(nickname.as_deref(), nickname_unanswered),
                receiver_identity,
                &receiver_key,
                &message,
                envelope,
                nonce,
            ),
            None => encode_outgoing_contact_message(
                self.setup.identity,
                &client_key,
                nickname_delta(nickname.as_deref(), nickname_unanswered),
                receiver_identity,
                &receiver_key,
                &message,
                nonce,
            ),
        };
        match encoded {
            Ok(payload) => {
                payloads.push(payload);
                Ok(Some(payloads))
            }
            Err(error) => {
                tracing::warn!(%error, "cannot encode outgoing message");
                Ok(None)
            }
        }
    }

    async fn send_read_receipt(
        &mut self,
        receiver: &str,
        message_ids: Vec<u64>,
    ) -> Result<(), ConnectionError> {
        if message_ids.is_empty() {
            return Ok(());
        }
        self.reflect_read(receiver, &message_ids).await?;
        let body = ContactMessageBody::DeliveryReceipt(DeliveryReceiptMessage {
            receipt_type: DeliveryReceiptType::Read,
            message_ids: message_ids.into_iter().map(MessageId).collect(),
        });
        let created_at = crate::messages::now_ms();
        for payload in self
            .encode(
                receiver,
                MessageId::random().0,
                created_at,
                Outgoing::Contact(body),
            )
            .await?
            .unwrap_or_default()
        {
            self.send_payload(&OutgoingPayload::MessageWithMetadataBox(payload))
                .await?;
        }
        Ok(())
    }

    async fn send_typing(
        &mut self,
        receiver: &str,
        is_typing: bool,
    ) -> Result<(), ConnectionError> {
        let body = ContactMessageBody::TypingIndicator(TypingIndicatorMessage { is_typing });
        let created_at = crate::messages::now_ms();
        for payload in self
            .encode(
                receiver,
                MessageId::random().0,
                created_at,
                Outgoing::Contact(body),
            )
            .await?
            .unwrap_or_default()
        {
            self.send_payload(&OutgoingPayload::MessageWithMetadataBox(payload))
                .await?;
        }
        Ok(())
    }

    /// A call message (not stored, not reflected: our other devices don't take calls).
    async fn send_call_signal(
        &mut self,
        receiver: &str,
        signal: &crate::calls::Signal,
    ) -> Result<(), ConnectionError> {
        let (kind, json) = signal.encode();
        let body = match kind {
            CallSignalKind::Offer => ContactMessageBody::CallOffer(CallOfferMessage { json }),
            CallSignalKind::Answer => ContactMessageBody::CallAnswer(CallAnswerMessage { json }),
            CallSignalKind::IceCandidates => {
                ContactMessageBody::CallIceCandidate(CallIceCandidateMessage { json })
            }
            CallSignalKind::Hangup => ContactMessageBody::CallHangup(CallHangupMessage { json }),
            CallSignalKind::Ringing => ContactMessageBody::CallRinging(CallRingingMessage { json }),
        };
        for payload in self
            .encode(
                receiver,
                MessageId::random().0,
                crate::messages::now_ms(),
                Outgoing::Contact(body),
            )
            .await?
            .unwrap_or_default()
        {
            self.send_payload(&OutgoingPayload::MessageWithMetadataBox(payload))
                .await?;
        }
        Ok(())
    }

    /// Forward security side effects: send control messages, record status messages.
    async fn handle_fs_event(
        &mut self,
        event: CoreEvent,
    ) -> Result<Option<CoreEvent>, ConnectionError> {
        match event {
            CoreEvent::FsControl {
                peer,
                peer_key,
                envelope,
            } => {
                if let Some(payload) = self.encode_control(&peer, peer_key, &envelope)? {
                    self.send_payload(&OutgoingPayload::MessageWithMetadataBox(payload))
                        .await?;
                }
                Ok(None)
            }
            CoreEvent::FsStatus { peer, status } => {
                let store = Rc::clone(&self.setup.store);
                // In the store before anything that can take a while or fail: the Reject has been
                // acknowledged, and a message left "sent" now would be lost without a trace.
                let mut requeued = Requeued::default();
                if let FsStatus::MessageRejected {
                    message_id,
                    group_identity,
                    ..
                } = status
                {
                    // Once per message and connection: a peer that rejects the re-send too
                    // would otherwise bounce the message back and forth without end — the
                    // second rejection leaves it failed for the user to resend by hand.
                    if !self.fs_resent.insert((peer.clone(), message_id)) {
                        tracing::warn!("message rejected again; left failed");
                        if let Some(message) = store
                            .message_by_id(&peer, true, message_id)
                            .map_err(storage_error)?
                            && let Some(failed) = store
                                .set_outgoing_failed(message.id)
                                .map_err(storage_error)?
                        {
                            self.emit(CoreEvent::MessagesChanged(vec![failed]));
                        }
                    } else {
                        requeued = requeue_rejected(
                            &store,
                            &self.setup.identity.to_string(),
                            &peer,
                            message_id,
                            group_identity,
                        )
                        .map_err(storage_error)?;
                        if !requeued.changed.is_empty() {
                            self.emit(CoreEvent::MessagesChanged(core::mem::take(
                                &mut requeued.changed,
                            )));
                        }
                        if let Some(conversation) = requeued.state_of.take() {
                            self.emit(CoreEvent::AnnounceGroupCall {
                                conversation,
                                receivers: vec![peer.clone()],
                            });
                        }
                    }
                }
                let supports_fs = self.fs_support_after(&peer, &status).await?;
                if requeued.send {
                    self.send_pending().await?;
                }
                if !crate::providers::shows_fs_status(&store, &self.setup.config, &peer)
                    .map_err(storage_error)?
                {
                    return Ok(None);
                }
                let Some(code) = fs_status_line(&status, supports_fs, requeued.resending) else {
                    return Ok(None);
                };
                // A session is no news where there is nothing else: after a restore, every
                // contact's Accept would start a chat, or move a restored one to the top.
                if code == "fsEstablished"
                    && store.message_count(&peer).map_err(storage_error)? == 0
                {
                    return Ok(None);
                }
                // A visible message adds a row and so gets its line, as on Android; a run of
                // receipts or typing adds none and gets one.
                if code == FS_MESSAGE_WITHOUT
                    && store.newest_is_status(&peer, code).map_err(storage_error)?
                {
                    return Ok(None);
                }
                Ok(store
                    .insert_status(&peer, code)
                    .map_err(storage_error)?
                    .map(|message| CoreEvent::MessageAdded { message }))
            }
            CoreEvent::OutboxChanged => {
                self.send_pending().await?;
                Ok(None)
            }
            // This one alone: a burst of messages would otherwise go through all of the outbox for
            // each of them.
            CoreEvent::ReceiptQueued {
                receiver,
                receipt_id,
            } => {
                let entry = self
                    .setup
                    .store
                    .outbox_entry(&receiver, receipt_id)
                    .map_err(storage_error)?;
                if let Some(entry) = entry {
                    self.send_outbox_entry(entry).await?;
                }
                Ok(None)
            }
            other => Ok(Some(other)),
        }
    }

    /// Look up `peer`'s feature mask, unless that was done within `interval`. Without an answer
    /// the stored mask stays.
    async fn check_feature_mask(
        &self,
        peer: &str,
        interval: Duration,
    ) -> Result<MaskCheck, ConnectionError> {
        let now = Instant::now();
        let checks = &self.setup.fs_mask_checks;
        if let Some(answered) = checks.recent(peer, now, interval) {
            return Ok(if answered {
                MaskCheck::Recent
            } else {
                MaskCheck::Unknown
            });
        }
        let store = &self.setup.store;
        if store.contact(peer).map_err(storage_error)?.is_none() || store.directory_resting() {
            return Ok(MaskCheck::Unknown);
        }
        let identities = [peer.to_owned()];
        let lookup = crate::contacts::lookup(&self.setup.https, &self.setup.config, &identities);
        let found = match tokio::time::timeout(FS_MASK_LOOKUP_LIMIT, lookup).await {
            Ok(Ok(found)) => found,
            Ok(Err(error)) => {
                if matches!(error, crate::directory::DirectoryError::RateLimited) {
                    store.rest_directory_calls();
                }
                tracing::warn!(%error, "feature mask lookup failed");
                checks.record(peer, now, false);
                return Ok(MaskCheck::Unknown);
            }
            Err(_) => {
                tracing::warn!("feature mask lookup timed out");
                checks.record(peer, now, false);
                return Ok(MaskCheck::Unknown);
            }
        };
        let mut changed = false;
        for (_, result) in found {
            changed |= store
                .store_lookup(&self.setup.config, peer, result)
                .map_err(storage_error)?;
        }
        if changed {
            self.emit(CoreEvent::ContactsChanged);
        }
        checks.record(peer, now, true);
        Ok(MaskCheck::Fresh)
    }

    /// Whether we have a session with `identity`. Not being able to tell isn't worth more than a
    /// log line here: it only decides whether to look closer at a contact without forward secrecy.
    fn has_fs_session(&self, identity: libthreema::common::ThreemaId) -> bool {
        match self.fs_store().get_best(identity) {
            Ok(session) => session.is_some(),
            Err(error) => {
                tracing::warn!(%error, "cannot read forward secrecy sessions");
                false
            }
        }
    }

    /// What `status` says about the peer's forward secrecy support, like Android's
    /// `ForwardSecurityMessageProcessor`: a peer that ended a session or rejected a message may
    /// have dropped it (linked a device), so its mask is looked up at once; and if the directory
    /// says no, what is left of the sessions ends too. An Init from someone whose stored mask says
    /// no means the mask is stale, or else the Init is: only a lookup made now can tell.
    async fn fs_support_after(
        &mut self,
        peer: &str,
        status: &FsStatus,
    ) -> Result<bool, ConnectionError> {
        let stored = |session: &Self| -> Result<bool, ConnectionError> {
            let contact = session.setup.store.contact(peer).map_err(storage_error)?;
            // Without a contact there is nothing to go by and nobody to show a line to.
            Ok(contact.is_none_or(|contact| crate::contacts::has_fs_support(&contact)))
        };
        let ending = match status {
            FsStatus::Reset => return stored(self),
            FsStatus::Established => {
                if stored(self)? {
                    return Ok(true);
                }
                match self
                    .check_feature_mask(peer, FS_MASK_INIT_CHECK_INTERVAL)
                    .await?
                {
                    MaskCheck::Fresh => true,
                    MaskCheck::Recent => {
                        self.setup.fs_mask_checks.distrust(peer);
                        false
                    }
                    MaskCheck::Unknown => false,
                }
            }
            // Whoever holds the contact's long-term key can send without forward secrecy too, so
            // that goes by what the directory says as well.
            FsStatus::MessageRejected { .. }
            | FsStatus::Terminated { .. }
            | FsStatus::MessageWithoutFs => {
                self.check_feature_mask(peer, FS_MASK_CHECK_INTERVAL)
                    .await?
                    != MaskCheck::Unknown
            }
        };
        let supports_fs = stored(self)?;
        if ending && !supports_fs {
            for payload in self.end_fs_sessions(peer)? {
                self.send_payload(&OutgoingPayload::MessageWithMetadataBox(payload))
                    .await?;
            }
        }
        Ok(supports_fs)
    }

    /// `peer` has no forward secrecy (any more): end the sessions we still have and tell it, so
    /// that it refreshes what it knows about us too. What to send first, if anything.
    fn end_fs_sessions(&self, peer: &str) -> Result<Vec<MessageWithMetadataBox>, ConnectionError> {
        use libthreema::{common::ThreemaId, protobuf::csp_e2e_fs::terminate::Cause};
        let Ok(identity) = peer.parse::<ThreemaId>() else {
            return Ok(Vec::new());
        };
        if self
            .fs_store()
            .get_best(identity)
            .map_err(protocol_error)?
            .is_none()
        {
            return Ok(Vec::new());
        }
        tracing::info!("contact has no forward secrecy; sessions ended");
        fs_protocol::terminate_all_with_cause(
            &mut self.fs_store(),
            identity,
            Cause::DisabledByRemote,
        )
        .map_err(protocol_error)?;
        self.take_fs_controls(peer)
    }

    /// A forward secrecy status line, from where no event passes through [`Self::handle_fs_event`].
    fn show_fs_line(&self, peer: &str, code: &str) -> Result<(), ConnectionError> {
        let store = &self.setup.store;
        if crate::providers::shows_fs_status(store, &self.setup.config, peer)
            .map_err(storage_error)?
            && let Some(message) = store.insert_status(peer, code).map_err(storage_error)?
        {
            self.emit(CoreEvent::MessageAdded { message });
        }
        Ok(())
    }

    /// The control messages for `peer` waiting in the event queue, encoded, so that they go out
    /// ahead of whatever is being sent now.
    fn take_fs_controls(&self, peer: &str) -> Result<Vec<MessageWithMetadataBox>, ConnectionError> {
        let events: Vec<CoreEvent> = self.events.borrow_mut().drain(..).collect();
        let mut payloads = Vec::new();
        for event in events {
            match event {
                CoreEvent::FsControl {
                    peer: receiver,
                    peer_key,
                    envelope,
                } if receiver == peer => {
                    payloads.extend(self.encode_control(peer, peer_key, &envelope)?);
                }
                other => self.events.borrow_mut().push(other),
            }
        }
        Ok(payloads)
    }

    fn emit(&self, event: CoreEvent) {
        let _ = self.tx.send(FromConnection::Event(Box::new(event)));
    }

    /// Hand on the events that don't need the connection (`later`), so that it can't lose them.
    fn emit_events_unless(&self, later: fn(&CoreEvent) -> bool) {
        let events = core::mem::take(&mut *self.events.borrow_mut());
        let (kept, now): (Vec<_>, Vec<_>) = events.into_iter().partition(later);
        *self.events.borrow_mut() = kept;
        for event in now {
            self.emit(event);
        }
    }

    /// What a message stored just now leaves for the UI.
    fn emit_stored_message_events(&self) {
        self.emit_events_unless(|event| {
            needs_the_connection(event) || runs_before_the_next_message(event)
        });
    }

    /// What the last connection couldn't send, even if the app has been restarted since. It stays
    /// in the store until it is done: if this connection ends too, [`Self::drop`] replaces it.
    async fn resume_carried_events(&mut self) -> Result<(), ConnectionError> {
        let mut carried = core::mem::take(&mut *self.setup.carried_events.borrow_mut());
        if carried.is_empty() {
            carried = saved_carried_events(&self.setup.store).map_err(storage_error)?;
        }
        if carried.is_empty() {
            return Ok(());
        }
        self.events.borrow_mut().extend(carried);
        self.after_incoming().await?;
        save_carried_events(&self.setup.store, &[]).map_err(storage_error)
    }

    /// This step went fine: if the store was failing in it, that is over.
    fn storage_works(&self, step: StorageStep) {
        if self.setup.storage_failing.works(step) {
            tracing::info!("storage works again");
            self.emit(CoreEvent::StorageProblem { active: false });
        }
    }

    /// After multi-device was turned on: end every forward security session and tell the peers
    /// (Android's `disableForwardSecurity`: Terminate with cause DISABLED_BY_LOCAL), so they stop
    /// sending what we could no longer read.
    async fn terminate_fs_if_pending(&mut self) -> Result<(), ConnectionError> {
        use libthreema::{common::ThreemaId, protobuf::csp_e2e_fs::terminate::Cause};
        let store = Rc::clone(&self.setup.store);
        if store
            .setting(crate::multidevice::SETTING_FS_TERMINATE)
            .map_err(storage_error)?
            .is_none()
        {
            return Ok(());
        }
        // Only while forward security is really off. A linking that failed and was rolled
        // back (or the switch turned on again) leaves the request behind; acting on it ended
        // every session on the next direct login, and contacts saw a security notice per
        // attempt (user report 2026-10-28).
        if self.multi_device.is_none() && !self.fs_disabled {
            tracing::info!("forward security is on again; pending termination dropped");
            store
                .delete_setting(crate::multidevice::SETTING_FS_TERMINATE)
                .map_err(storage_error)?;
            return Ok(());
        }
        let peers = store.fs_peers().map_err(storage_error)?;
        for peer in &peers {
            let Ok(peer) = peer.parse::<ThreemaId>() else {
                continue;
            };
            fs_protocol::terminate_all_with_cause(
                &mut self.fs_store(),
                peer,
                Cause::DisabledByLocal,
            )
            .map_err(protocol_error)?;
        }
        self.after_incoming().await?;
        store
            .delete_setting(crate::multidevice::SETTING_FS_TERMINATE)
            .map_err(storage_error)?;
        tracing::info!(peers = peers.len(), "forward security sessions terminated");
        Ok(())
    }

    /// libthreema's reflect step: send each envelope, then wait for the acks of the ones that
    /// are not ephemeral.
    async fn run_reflect(
        &mut self,
        instruction: ReflectInstruction,
    ) -> Result<ReflectResponse, ConnectionError> {
        let mut awaiting = Vec::new();
        for message in instruction.reflect_messages {
            if !message.flags.ephemeral {
                awaiting.push(message.id.0);
            }
            self.transport
                .send_d2m(D2mOutgoing::Reflect(D2mReflect {
                    flags: D2mReflectFlags(if message.flags.ephemeral {
                        D2mReflectFlags::EPHEMERAL_MARKER
                    } else {
                        0
                    }),
                    reflect_id: message.id.0,
                    envelope: message.envelope,
                }))
                .await?;
        }
        let mut acknowledged_reflect_ids = Vec::new();
        while !awaiting.is_empty() {
            let D2mIncoming::ReflectAck(ack) = self
                .await_d2m(|payload| {
                    matches!(payload, D2mIncoming::ReflectAck(ack) if awaiting.contains(&ack.reflect_id))
                })
                .await?
            else {
                continue;
            };
            awaiting.retain(|id| *id != ack.reflect_id);
            acknowledged_reflect_ids.push(ReflectId(ack.reflect_id));
        }
        Ok(ReflectResponse {
            acknowledged_reflect_ids,
        })
    }

    /// libthreema's begin-transaction step.
    async fn run_begin_transaction(
        &mut self,
        instruction: BeginTransactionInstruction,
    ) -> Result<BeginTransactionResponse, ConnectionError> {
        use libthreema::protobuf::d2m as proto;
        match instruction {
            BeginTransactionInstruction::TransactionRejected => {
                self.await_d2m(|payload| matches!(payload, D2mIncoming::TransactionEnded(_)))
                    .await?;
                // Poll again: libthreema begins anew.
                Ok(BeginTransactionResponse::BeginTransactionReply(
                    BeginTransactionReply::TransactionRejected(
                        proto::TransactionRejected::default(),
                    ),
                ))
            }
            BeginTransactionInstruction::BeginTransaction { message } => {
                self.transport
                    .send_d2m(D2mOutgoing::BeginTransaction(BeginTransaction {
                        encrypted_scope: message.encrypted_scope,
                        // The server's maximum; libthreema's tasks don't set one either.
                        ttl: None,
                    }))
                    .await?;
                let reply = match self
                    .await_d2m(|payload| {
                        matches!(
                            payload,
                            D2mIncoming::BeginTransactionAck | D2mIncoming::TransactionRejected(_)
                        )
                    })
                    .await?
                {
                    D2mIncoming::TransactionRejected(rejected) => {
                        BeginTransactionReply::TransactionRejected(proto::TransactionRejected {
                            device_id: rejected.device_id.0,
                            encrypted_scope: rejected.encrypted_scope,
                        })
                    }
                    _ => BeginTransactionReply::BeginTransactionAck(proto::BeginTransactionAck {}),
                };
                Ok(BeginTransactionResponse::BeginTransactionReply(reply))
            }
            BeginTransactionInstruction::AbortTransaction { .. } => {
                self.transport
                    .send_d2m(D2mOutgoing::CommitTransaction)
                    .await?;
                self.await_d2m(|payload| matches!(payload, D2mIncoming::CommitTransactionAck))
                    .await?;
                Ok(BeginTransactionResponse::AbortTransactionResponse(
                    proto::CommitTransactionAck {},
                ))
            }
        }
    }

    /// libthreema's reflect-and-commit step.
    async fn run_commit_transaction(
        &mut self,
        instruction: CommitTransactionInstruction,
    ) -> Result<CommitTransactionResponse, ConnectionError> {
        let ReflectResponse {
            acknowledged_reflect_ids,
        } = self
            .run_reflect(ReflectInstruction {
                reflect_messages: instruction.reflect_messages,
            })
            .await?;
        self.commit_transaction().await?;
        Ok(CommitTransactionResponse {
            acknowledged_reflect_ids,
            commit_transaction_ack: libthreema::protobuf::d2m::CommitTransactionAck {},
        })
    }

    /// Reflect a D2D envelope content to the other devices and wait for the mediator's ack
    /// (multi-device only; without it nothing happens).
    async fn reflect(
        &mut self,
        content: libthreema::protobuf::d2d::envelope::Content,
    ) -> Result<(), ConnectionError> {
        use prost::Message as _;
        let Some(properties) = self.multi_device.as_ref() else {
            return Ok(());
        };
        let device_id = properties.mediator_device_id;
        // A random amount of padding, like libthreema's envelopes.
        #[expect(deprecated, reason = "Padding is still part of the envelope")]
        let envelope = libthreema::protobuf::d2d::Envelope {
            padding: vec![0; usize::from(rand::random::<u8>() % 32)],
            device_id,
            protocol_version: libthreema::protobuf::d2d::ProtocolVersion::V03 as u32,
            content: Some(content),
        };
        let (_, encrypted) = self
            .device_group_key()?
            .encrypt_envelope(&envelope.encode_to_vec())
            .map_err(protocol_error)?;
        let id = self.next_reflect_id;
        self.next_reflect_id = self.next_reflect_id.wrapping_add(1).max(0x8000_0000);
        self.transport
            .send_d2m(D2mOutgoing::Reflect(D2mReflect {
                flags: D2mReflectFlags(0),
                reflect_id: id,
                envelope: encrypted,
            }))
            .await?;
        self.await_d2m(
            |payload| matches!(payload, D2mIncoming::ReflectAck(ack) if ack.reflect_id == id),
        )
        .await?;
        Ok(())
    }

    /// Reflect the queued sync envelopes, oldest first, each in a transaction of its scope;
    /// removed from the queue once committed.
    async fn reflect_queued_syncs(&mut self) -> Result<(), ConnectionError> {
        use libthreema::protobuf::d2d::{Envelope, transaction_scope::Scope};
        use prost::Message as _;
        if self.multi_device.is_none() {
            return Ok(());
        }
        let store = Rc::clone(&self.setup.store);
        while let Some((id, scope, content)) = store.next_sync().map_err(storage_error)? {
            let decoded = Envelope::decode(content.as_slice())
                .ok()
                .and_then(|envelope| envelope.content)
                .zip(Scope::try_from(scope).ok());
            if let Some((content, scope)) = decoded {
                self.begin_transaction(scope).await?;
                let reflected = self.reflect(content).await;
                self.commit_transaction().await?;
                reflected?;
            }
            store.sync_done(id).map_err(storage_error)?;
        }
        Ok(())
    }

    /// Our outgoing message for the other devices (Android: `reflectMessage` before sending).
    /// Once per message and connection.
    async fn reflect_outgoing(
        &mut self,
        conversation: &str,
        message_id: u64,
        created_at: i64,
        body: &Outgoing,
    ) -> Result<(), ConnectionError> {
        use libthreema::protobuf::d2d::{OutgoingMessage, envelope::Content};
        if self.multi_device.is_none()
            || !self.reflected.insert((conversation.to_owned(), message_id))
        {
            return Ok(());
        }
        let Some(conversation_id) = d2d_conversation(conversation) else {
            return Ok(());
        };
        // Groups: the receiver in the body doesn't matter for the encoding.
        let receiver_identity = conversation
            .parse::<ThreemaId>()
            .unwrap_or(self.setup.identity);
        let outgoing_body = match body {
            Outgoing::Contact(body) => OutgoingMessageBody::Contact(OutgoingContactMessageBody {
                receiver_identity,
                body: body.clone(),
            }),
            Outgoing::Group(group_identity, body) => {
                OutgoingMessageBody::Group(OutgoingGroupMessageBody {
                    group_identity: *group_identity,
                    body: body.clone(),
                })
            }
        };
        let (message_type, data) = encode_message_body(&outgoing_body);
        self.reflect(Content::OutgoingMessage(OutgoingMessage {
            conversation: Some(conversation_id),
            message_id,
            thread_message_id: None,
            created_at: u64::try_from(created_at).unwrap_or_default(),
            r#type: i32::from(message_type),
            body: data,
            nonces: Vec::new(),
        }))
        .await
    }

    /// Tell the other devices the chat server has our message (Android's
    /// `reflectMessageUpdateSent`).
    async fn reflect_sent(
        &mut self,
        conversation: &str,
        message_id: u64,
    ) -> Result<(), ConnectionError> {
        use libthreema::protobuf::d2d::{
            OutgoingMessageUpdate,
            envelope::Content,
            outgoing_message_update::{Sent, Update, update},
        };
        if self.multi_device.is_none() {
            return Ok(());
        }
        let Some(conversation_id) = d2d_conversation(conversation) else {
            return Ok(());
        };
        self.reflect(Content::OutgoingMessageUpdate(OutgoingMessageUpdate {
            updates: vec![Update {
                conversation: Some(conversation_id),
                message_id,
                update: Some(update::Update::Sent(Sent {})),
            }],
        }))
        .await
    }

    /// Messages read here: read on the other devices too.
    async fn reflect_read(
        &mut self,
        conversation: &str,
        message_ids: &[u64],
    ) -> Result<(), ConnectionError> {
        use libthreema::protobuf::d2d::{
            IncomingMessageUpdate,
            envelope::Content,
            incoming_message_update::{Read, Update, update},
        };
        if self.multi_device.is_none() || message_ids.is_empty() {
            return Ok(());
        }
        let Some(conversation_id) = d2d_conversation(conversation) else {
            return Ok(());
        };
        let at = u64::try_from(crate::messages::now_ms()).unwrap_or_default();
        self.reflect(Content::IncomingMessageUpdate(IncomingMessageUpdate {
            updates: message_ids
                .iter()
                .map(|&message_id| Update {
                    conversation: Some(conversation_id.clone()),
                    message_id,
                    update: Some(update::Update::Read(Read { at })),
                })
                .collect(),
        }))
        .await
    }

    /// Another device's reflection: apply it, then acknowledge (also when it can't be used, or the
    /// mediator would send it again at every login; but not when our store failed on it).
    async fn handle_reflected(
        &mut self,
        reflected: libthreema::d2m::payload::Reflected,
    ) -> Result<(), ConnectionError> {
        let device_group_key = self.device_group_key()?;
        let store = Rc::clone(&self.setup.store);
        let applied = crate::reflected::open(&store, &device_group_key, &reflected.envelope)
            .and_then(|opened| {
                // log-ok: the kind of content, not the content
                tracing::info!(
                    content = crate::reflected::content_name(opened.envelope.content.as_ref()),
                    "reflected"
                );
                crate::reflected::apply_once(&store, self.setup.identity, opened, &self.events)
            });
        match applied {
            Ok(()) => self.storage_works(StorageStep::Reflected),
            // Unacknowledged, it comes again at the next login: this is its only way here.
            Err(error) if error.storage_code().is_some() => {
                let code = error.storage_code().unwrap_or_default();
                let failure = ConnectionError::Storage {
                    code,
                    step: StorageStep::Reflected,
                };
                if !failure.fails_every_time() {
                    return Err(failure);
                }
                // Holding on to it would hold up the mediator's queue, and with it the promotion
                // to leader and all that comes from the chat server.
                let failures = self
                    .setup
                    .failing_items
                    .failed(&FailingItem::Envelope(envelope_nonce(&reflected.envelope)));
                if failures < TRIES_BEFORE_DROPPING {
                    tracing::warn!(
                        code,
                        failures,
                        "a reflected envelope failed, it comes again"
                    );
                    return Err(failure);
                }
                tracing::error!(
                    code,
                    failures,
                    "a reflected envelope keeps failing and is dropped"
                );
                if let Err(error) =
                    crate::reflected::note_dropped(&store, &device_group_key, &reflected.envelope)
                {
                    tracing::warn!(%error, "noting a dropped envelope failed");
                }
            }
            Err(error) => tracing::warn!(
                %error,
                reflect_id = reflected.reflect_id,
                queued_at = reflected.timestamp,
                flags = reflected.flags.0,
                "reflected envelope dropped"
            ),
        }
        self.setup
            .failing_items
            .forget(&FailingItem::Envelope(envelope_nonce(&reflected.envelope)));
        // A message another device received first: its notification waits a moment.
        for event in self.events.borrow().iter() {
            if let CoreEvent::MessageAdded { message, .. } = event
                && !message.outgoing
            {
                self.setup
                    .store
                    .held_notifications
                    .borrow_mut()
                    .insert(message.id);
                self.held.push((message.id, Instant::now() + NOTIFY_GRACE));
            }
        }
        // Applied, so the UI hears of it even if the acknowledgement is what fails.
        self.emit_events_unless(needs_the_connection);
        // Every one, ephemeral ones (typing) too, like Android (IncomingReflectedMessageTask):
        // md-d2m.struct.yml says not to acknowledge those, but the mediator keeps them queued
        // until acknowledged and hands them out again at every login.
        self.transport
            .send_d2m(D2mOutgoing::ReflectedAck(
                libthreema::d2m::payload::ReflectedAck {
                    reflect_id: reflected.reflect_id,
                },
            ))
            .await?;
        self.after_incoming().await
    }

    /// Notify the held messages that are due (all of them when `all`) and still unread.
    fn release_held_notifications(&mut self, all: bool) {
        let now = Instant::now();
        let (due, waiting): (Vec<_>, Vec<_>) =
            self.held.drain(..).partition(|(_, at)| all || *at <= now);
        self.held = waiting;
        for (id, _) in due {
            self.setup.store.held_notifications.borrow_mut().remove(&id);
            if let Ok(Some(message)) = self.setup.store.message(id)
                && message.state == crate::messages::INCOMING_UNREAD
            {
                self.emit(CoreEvent::Notify(message));
            }
        }
    }

    /// Read until a mediator payload `wanted` arrives (`D2M_REPLY_LIMIT` at most); everything else is kept:
    /// CSP bytes go to libthreema, other mediator payloads to the queue.
    async fn await_d2m(
        &mut self,
        wanted: impl Fn(&D2mIncoming) -> bool,
    ) -> Result<D2mIncoming, ConnectionError> {
        let mut buffer = vec![0_u8; 8192];
        timeout(D2M_REPLY_LIMIT, async {
            loop {
                match self.read_inbound(&mut buffer).await? {
                    Inbound::Csp(bytes) => {
                        self.csp.add_chunks(&[&bytes]).map_err(protocol_error)?
                    }
                    Inbound::D2m(payload) if wanted(&payload) => return Ok(payload),
                    Inbound::D2m(payload) => self.d2m_queue.push_back(payload),
                }
            }
        })
        .await
        .map_err(|_| ConnectionError::Timeout)?
    }

    fn device_group_key(
        &self,
    ) -> Result<libthreema::common::keys::DeviceGroupKey, ConnectionError> {
        self.multi_device
            .as_ref()
            .and_then(crate::multidevice::Properties::device_group_key)
            .ok_or_else(|| ConnectionError::Protocol("multi-device is not active".into()))
    }

    /// Lock the device group for `scope` (retrying after other devices' transactions, within
    /// `TRANSACTION_BEGIN_LIMIT`), for the server's maximum time.
    async fn begin_transaction(
        &mut self,
        scope: libthreema::protobuf::d2d::transaction_scope::Scope,
    ) -> Result<(), ConnectionError> {
        use prost::Message as _;
        timeout(TRANSACTION_BEGIN_LIMIT, async {
            loop {
                let scope_bytes = libthreema::protobuf::d2d::TransactionScope {
                    scope: scope as i32,
                }
                .encode_to_vec();
                let encrypted_scope = self
                    .device_group_key()?
                    .encrypt_transaction_scope(&scope_bytes)
                    .map_err(protocol_error)?;
                self.transport
                    .send_d2m(D2mOutgoing::BeginTransaction(BeginTransaction {
                        encrypted_scope,
                        // The server's maximum (Android: TRANSACTION_TTL_MAX).
                        ttl: None,
                    }))
                    .await?;
                match self
                    .await_d2m(|payload| {
                        matches!(
                            payload,
                            D2mIncoming::BeginTransactionAck | D2mIncoming::TransactionRejected(_)
                        )
                    })
                    .await?
                {
                    D2mIncoming::BeginTransactionAck => return Ok(()),
                    _ => {
                        // Another device holds the lock: try again once it is released.
                        self.await_d2m(|payload| {
                            matches!(payload, D2mIncoming::TransactionEnded(_))
                        })
                        .await?;
                    }
                }
            }
        })
        .await
        .map_err(|_| ConnectionError::Timeout)?
    }

    async fn commit_transaction(&mut self) -> Result<(), ConnectionError> {
        self.transport
            .send_d2m(D2mOutgoing::CommitTransaction)
            .await?;
        self.await_d2m(|payload| matches!(payload, D2mIncoming::CommitTransactionAck))
            .await?;
        Ok(())
    }

    /// The device group's devices (Android's `GetLinkedDevicesTask`).
    async fn devices(&mut self) -> Result<Vec<LinkedDevice>, ConnectionError> {
        use libthreema::protobuf::{
            d2d::{DeviceInfo, device_info::Platform},
            d2m::devices_info::augmented_device_info::ConnectionState,
        };
        use prost::Message as _;
        let device_group_key = self.device_group_key()?;
        let own = self.multi_device.as_ref().map(|p| p.mediator_device_id);
        self.transport.send_d2m(D2mOutgoing::GetDevicesInfo).await?;
        let D2mIncoming::DevicesInfo(info) = self
            .await_d2m(|payload| matches!(payload, D2mIncoming::DevicesInfo(_)))
            .await?
        else {
            return Err(ConnectionError::Protocol("expected devices info".into()));
        };
        let mut devices: Vec<LinkedDevice> = info
            .0
            .into_iter()
            .map(|(id, device)| {
                let decoded = device_group_key
                    .decrypt_device_info(&device.encrypted_device_info)
                    .ok()
                    .and_then(|bytes| DeviceInfo::decode(bytes.as_slice()).ok());
                let (connected_since, last_disconnect_at) = match device.connection_state {
                    Some(ConnectionState::ConnectedSince(at)) => (Some(at), None),
                    Some(ConnectionState::LastDisconnectAt(at)) => (None, Some(at)),
                    None => (None, None),
                };
                LinkedDevice {
                    id: format!("{:016x}", id.0),
                    raw_id: id.0,
                    current: own == Some(id.0),
                    label: decoded
                        .as_ref()
                        .map(|d| d.label.clone())
                        .unwrap_or_default(),
                    platform: match decoded.as_ref().map(DeviceInfo::platform) {
                        Some(Platform::Android) => "android",
                        Some(Platform::Ios) => "ios",
                        Some(Platform::Desktop) => "desktop",
                        Some(Platform::Web) => "web",
                        _ => "unknown",
                    },
                    platform_details: decoded
                        .as_ref()
                        .map(|d| d.platform_details.clone())
                        .unwrap_or_default(),
                    app_version: decoded.map(|d| d.app_version).unwrap_or_default(),
                    connected_since,
                    last_disconnect_at,
                }
            })
            .collect();
        // This device first, then the others by label.
        devices.sort_by(|a, b| b.current.cmp(&a.current).then(a.label.cmp(&b.label)));
        Ok(devices)
    }

    /// The device join's data transfer (md-d2d-join.proto, Android's `DeviceLinkingPartTwoTask`):
    /// while the transaction is held nothing incoming is processed (CSP bytes and mediator
    /// payloads wait), then commit. A failure on the link still commits, which releases the lock.
    /// Every step has a limit (`JOIN_REPLY_LIMIT` is their sum). The second result: start a new
    /// connection, because a lock may still be held here.
    async fn new_device_sync(
        &mut self,
        mut link: crate::rendezvous::RendezvousLink,
    ) -> (JoinOutcome, bool) {
        use libthreema::protobuf::d2d::transaction_scope::Scope;
        let Some(dgk) = self
            .multi_device
            .as_ref()
            .and_then(crate::multidevice::Properties::dgk)
        else {
            link.close().await;
            let reason = "multi-device is not active".to_owned();
            return (JoinOutcome::FailedBeforeHandover(reason), false);
        };
        if let Err(error) = self.begin_transaction(Scope::NewDeviceSync).await {
            link.close().await;
            let restart = matches!(error, ConnectionError::Timeout);
            return (
                JoinOutcome::FailedBeforeHandover(error.to_string()),
                restart,
            );
        }
        let mut handed_over = false;
        let transfer = timeout(JOIN_TRANSFER_LIMIT, async {
            link.send(crate::join::begin_message()).await?;
            let data = crate::join::collect(&self.setup.store, &self.setup.media, &dgk)
                .map_err(|e| crate::rendezvous::LinkError::Protocol(e.to_string()))?;
            tracing::info!(blobs = data.blobs.len(), "sending essential data");
            for blob in data.blobs {
                link.send(blob.to_vec()).await?;
            }
            // Moved, not copied: the link encrypts this buffer in place (with room for the tag,
            // see join::collect), and a plain copy would hold the ID's private key.
            let mut essential_data = data.essential_data;
            // From the first byte on: part of it may arrive, and nothing here can tell how much.
            handed_over = true;
            link.send(std::mem::take(&mut *essential_data)).await?;
            Ok::<_, crate::rendezvous::LinkError>(())
        })
        .await;
        let registered = match transfer {
            Ok(Ok(())) => self.await_registered(&mut link).await,
            Ok(Err(error)) => Err(ConnectionError::Protocol(error.to_string())),
            Err(_) => Err(ConnectionError::Timeout),
        };
        // ND may close now or keep it open for the history exchange (not supported: we close).
        link.close().await;
        let commit = self.commit_transaction().await;
        if let Err(error) = &commit {
            tracing::warn!(%error, "committing the device join failed");
        }
        let outcome = JoinOutcome::from_steps(handed_over, registered.map_err(|e| e.to_string()));
        (outcome, commit.is_err())
    }

    /// Wait (`JOIN_REGISTER_LIMIT` at most) for the new device's `Registered`, keeping the
    /// mediator read.
    async fn await_registered(
        &mut self,
        link: &mut crate::rendezvous::RendezvousLink,
    ) -> Result<(), ConnectionError> {
        let mut buffer = vec![0_u8; 8192];
        timeout(JOIN_REGISTER_LIMIT, async {
            loop {
                tokio::select! {
                    data = link.receive() => {
                        let data = data.map_err(protocol_error)?;
                        if crate::join::is_registered(&data) {
                            tracing::info!("new device registered");
                            return Ok(());
                        }
                        return Err(ConnectionError::Protocol("unexpected message from the new device".into()));
                    }
                    read = self.read_inbound(&mut buffer) => match read? {
                        Inbound::Csp(bytes) => self.csp.add_chunks(&[&bytes]).map_err(protocol_error)?,
                        Inbound::D2m(payload) => self.d2m_queue.push_back(payload),
                    },
                }
            }
        })
        .await
        .map_err(|_| ConnectionError::Timeout)?
    }

    /// Drop devices inside a `DROP_DEVICE` transaction (Android's `runDropDevicesSteps`), deciding
    /// from the mediator's list there. Our own id goes last: the mediator answers that by closing
    /// the connection (4113, "dropped by itself"), which ends the transaction too; that counts
    /// as done.
    async fn drop_devices(&mut self, ids: Option<&[u64]>) -> Result<DropOutcome, ConnectionError> {
        use libthreema::protobuf::d2d::transaction_scope::Scope;
        let own = self
            .multi_device
            .as_ref()
            .map(|p| p.mediator_device_id)
            .ok_or_else(|| ConnectionError::Protocol("multi-device is not active".into()))?;
        self.begin_transaction(Scope::DropDevice).await?;
        let mut devices = self.devices().await?;
        let others: Vec<u64> = devices
            .iter()
            .filter(|device| !device.current)
            .map(|device| device.raw_id)
            .collect();
        let (to_drop, drop_self) = plan_drop(ids, &others, own);
        for &id in &to_drop {
            self.transport
                .send_d2m(D2mOutgoing::DropDevice(D2xDeviceId(id)))
                .await?;
            self.await_d2m(
                |payload| matches!(payload, D2mIncoming::DropDeviceAck(acked) if acked.0 == id),
            )
            .await?;
        }
        devices.retain(|device| !to_drop.contains(&device.raw_id));
        if !drop_self {
            self.commit_transaction().await?;
            return Ok(DropOutcome {
                remaining: devices,
                left_group: false,
            });
        }
        self.transport
            .send_d2m(D2mOutgoing::DropDevice(D2xDeviceId(own)))
            .await?;
        match self
            .await_d2m(
                |payload| matches!(payload, D2mIncoming::DropDeviceAck(acked) if acked.0 == own),
            )
            .await
        {
            Ok(_) | Err(ConnectionError::MediatorClosed { code: 4113, .. }) => Ok(DropOutcome {
                remaining: Vec::new(),
                left_group: true,
            }),
            Err(error) => Err(error),
        }
    }

    /// A mediator payload (multi-device, ADR 0011).
    async fn handle_d2m(&mut self, payload: D2mIncoming) -> Result<(), ConnectionError> {
        match payload {
            D2mIncoming::ReflectionQueueDry => {
                tracing::info!("reflection queue dry");
                // The backlog is in, read updates included: what is still unread notifies.
                self.reflection_backlog = false;
                self.release_held_notifications(true);
                self.storage_works(StorageStep::Reflected);
                self.setup
                    .wake_hold
                    .reflection_queue_through(Instant::now());
            }
            // Only the leader receives from the chat server (Android's MonitoringLayer).
            D2mIncoming::RolePromotedToLeader => {
                self.setup.wake_hold.promoted_to_leader();
                self.e2e
                    .update_d2m_state(D2mRole::Leader)
                    .map_err(protocol_error)?;
                self.send_payload(&OutgoingPayload::UnblockIncomingMessages)
                    .await?;
                tracing::info!("promoted to leader, receiving messages");
            }
            D2mIncoming::Reflected(reflected) => self.handle_reflected(reflected).await?,
            // log-ok: the payload's type name
            other => tracing::debug!(payload = d2m_name(&other), "mediator payload"),
        }
        Ok(())
    }

    async fn handle_inbound(&mut self, inbound: Inbound) -> Result<(), ConnectionError> {
        match inbound {
            Inbound::Csp(bytes) => self.csp.add_chunks(&[&bytes]).map_err(protocol_error)?,
            Inbound::D2m(payload) => self.handle_d2m(payload).await?,
        }
        Ok(())
    }

    async fn payload_loop(
        &mut self,
        rx: &mut mpsc::UnboundedReceiver<ToConnection>,
    ) -> Result<(), ConnectionError> {
        let mut read_buffer = vec![0_u8; 8192];
        loop {
            // Process everything libthreema can already do before waiting.
            while let Some(instruction) = self.csp.poll().map_err(protocol_error)? {
                self.handle_instruction(instruction).await?;
            }
            while let Some(payload) = self.d2m_queue.pop_front() {
                self.handle_d2m(payload).await?;
            }
            let idle_deadline = self.liveness.deadline();
            // The next held notification (live forwarded messages; the backlog waits for the
            // end of the mediator's queue instead).
            let next_held = (!self.reflection_backlog)
                .then(|| self.held.iter().map(|(_, at)| *at).min())
                .flatten();
            // A notification that is being held goes out with the device up.
            let wake_settles = self
                .setup
                .wake_hold
                .settles_at()
                .map(|at| self.held.iter().map(|(_, due)| *due).fold(at, Ord::max));
            tokio::select! {
                () = async {
                    match wake_settles {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                } => self.setup.wake_hold.settle(self.tx),
                () = async {
                    match next_held {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                } => self.release_held_notifications(false),
                read = self.read_inbound(&mut read_buffer) => self.handle_inbound(read?).await?,
                message = rx.recv() => match message {
                    None => return Err(ConnectionError::Stopped),
                    Some(ToConnection::SendPending) => self.send_pending().await?,
                    Some(ToConnection::ReadReceipts { identity, message_ids }) => {
                        self.send_read_receipt(&identity, message_ids).await?;
                    },
                    // Queued while disconnected: stale by now, the contact's timer has expired anyway.
                    Some(ToConnection::Typing { identity, typing, at }) => if at.elapsed() < TYPING_MAX_AGE {
                        self.send_typing(&identity, typing).await?;
                    },
                    Some(ToConnection::Reconnect) => return Err(ConnectionError::Reconnect),
                    Some(ToConnection::NewDeviceSync { link, reply }) => {
                        let (outcome, restart) = self.new_device_sync(*link).await;
                        if let JoinOutcome::FailedBeforeHandover(reason)
                        | JoinOutcome::FailedAfterHandover(reason) = &outcome
                        {
                            // log-ok: our own connection errors and the mediator's text
                            tracing::warn!(%reason, "device join failed");
                        }
                        let _ = reply.send(outcome);
                        if restart {
                            return Err(ConnectionError::Reconnect);
                        }
                    },
                    Some(ToConnection::ReflectSync) => self.reflect_queued_syncs().await?,
                    Some(ToConnection::CallSignal { peer, signal }) => {
                        self.send_call_signal(&peer, &signal).await?;
                    },
                    Some(ToConnection::FsControl { peer, peer_key, envelope }) => {
                        if let Some(payload) = self.encode_control(&peer, peer_key, &envelope)? {
                            self.send_payload(&OutgoingPayload::MessageWithMetadataBox(payload))
                                .await?;
                        }
                    },
                    Some(ToConnection::Flush(reply)) => {
                        let _ = reply.send(());
                    },
                    Some(ToConnection::Devices(reply)) => {
                        let devices = self.devices().await;
                        let _ = reply.send(devices.map_err(DeviceGroupError::from));
                    },
                    Some(ToConnection::DropDevices { ids, reply }) => match self.drop_devices(ids.as_deref()).await {
                        Ok(outcome) => {
                            let left_group = outcome.left_group;
                            if left_group {
                                // Like Android's removeMultiDeviceLocally: forget the group, then
                                // connect directly again (forward security comes back with it:
                                // the core publishes the feature mask on this event).
                                self.setup.store.set_multi_device(None).map_err(storage_error)?;
                                self.emit(CoreEvent::MultiDeviceChanged);
                            }
                            let _ = reply.send(Ok(outcome));
                            if left_group {
                                return Err(ConnectionError::Reconnect);
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "removing devices failed");
                            // A timeout leaves it open what the mediator did with the transaction:
                            // a new connection releases it.
                            let restart = matches!(error, ConnectionError::Timeout);
                            let _ = reply.send(Err(error.into()));
                            if restart {
                                return Err(ConnectionError::Reconnect);
                            }
                        }
                    },
                    // Woken up (e.g. after the device slept): check the connection is still alive.
                    Some(ToConnection::Wake | ToConnection::ConnectNow) => {
                        self.setup.wake_hold.asked();
                        if self.liveness.awaiting_echo() {
                            // Out since before the device slept, perhaps, with most of its time left.
                            self.liveness.echo_due_by(Instant::now() + WAKE_ECHO_LIMIT);
                        } else {
                            self.send_payload(&OutgoingPayload::EchoRequest(EchoPayload(Vec::new()))).await?;
                            self.liveness.echo_sent(Instant::now(), WAKE_ECHO_LIMIT);
                        }
                    },
                },
                () = tokio::time::sleep_until(idle_deadline) => {
                    if self.liveness.awaiting_echo() {
                        // select! picks at random among what is ready, so a read that is ready
                        // too doesn't settle it by itself.
                        let read = last_look_for_echo(self.read_inbound(&mut read_buffer)).await?;
                        self.handle_inbound(read?).await?;
                    } else {
                        self.send_payload(&OutgoingPayload::EchoRequest(EchoPayload(Vec::new()))).await?;
                        self.liveness.echo_sent(Instant::now(), ECHO_INTERVAL);
                    }
                },
            }
        }
    }

    async fn handle_instruction(
        &mut self,
        instruction: CspProtocolInstruction,
    ) -> Result<(), ConnectionError> {
        if let Some(frame) = instruction.outgoing_frame {
            self.transport.write_csp(&frame.0).await?;
        }
        let Some(payload) = instruction.incoming_payload else {
            return Ok(());
        };
        match payload {
            IncomingPayload::EchoRequest(echo) => {
                Box::pin(self.send_payload(&OutgoingPayload::EchoResponse(echo))).await?;
            }
            IncomingPayload::MessageWithMetadataBox(message) => {
                Box::pin(self.handle_incoming(message)).await?;
            }
            IncomingPayload::MessageAck(ack) => {
                let receiver = ack.sender_identity.to_string();
                self.awaiting_ack
                    .retain(|(to, id)| !(*to == receiver && *id == ack.id.0));
                let store = &self.setup.store;
                store
                    .outbox_remove(&receiver, ack.id.0)
                    .map_err(storage_error)?;
                let mut changed = store
                    .raise_outgoing_state(&receiver, &[ack.id.0], OutgoingState::Sent)
                    .map_err(storage_error)?;
                // Group messages: sent once every member has it.
                for conversation in store
                    .receiver_done(&receiver, ack.id.0)
                    .map_err(storage_error)?
                {
                    store
                        .outbox_remove(&conversation, ack.id.0)
                        .map_err(storage_error)?;
                    changed.extend(
                        store
                            .raise_outgoing_state(&conversation, &[ack.id.0], OutgoingState::Sent)
                            .map_err(storage_error)?,
                    );
                }
                // Multi-device: tell the other devices (once per message: a group message once
                // every member has it).
                for message in changed.iter().filter(|m| m.outgoing) {
                    if self
                        .reflected
                        .remove(&(message.conversation.clone(), message.message_id))
                    {
                        Box::pin(self.reflect_sent(&message.conversation, message.message_id))
                            .await?;
                    }
                }
                if !changed.is_empty() {
                    self.emit(CoreEvent::MessagesChanged(changed));
                }
            }
            IncomingPayload::CloseError(close) => {
                return Err(ConnectionError::Closed {
                    message: close.message,
                    can_reconnect: close.can_reconnect,
                });
            }
            IncomingPayload::ServerAlert(alert) => {
                tracing::info!("chat server sent an alert");
                if let Some(reason) = server_reason(&alert.0) {
                    let _ = self.tx.send(FromConnection::OfflineReason(reason));
                }
            }
            IncomingPayload::QueueSendComplete => {
                tracing::debug!("message queue drained");
                self.storage_works(StorageStep::Incoming);
                self.setup.wake_hold.chat_queue_through(Instant::now());
            }
            // Like Android's TaskManager: warn (unless our own cookie is new) and clear the
            // server's flag. Left set, every login of every device in the group gets it again
            // (Threema Desktop: "Connection from another device detected", on every start).
            IncomingPayload::DeviceCookieChangeIndication => {
                let store = &self.setup.store;
                let fresh = store
                    .setting(crate::store::keys::DEVICE_COOKIE_FRESH)
                    .map_err(storage_error)?
                    .is_some();
                if fresh {
                    tracing::info!("device cookie change indication for our new cookie");
                    store
                        .delete_setting(crate::store::keys::DEVICE_COOKIE_FRESH)
                        .map_err(storage_error)?;
                } else {
                    tracing::warn!("device cookie change indication: another device used this ID");
                    self.emit(CoreEvent::DeviceCookieChanged);
                }
                Box::pin(self.send_payload(&OutgoingPayload::ClearDeviceCookieChangeIndiciation))
                    .await?;
            }
            IncomingPayload::EchoResponse(_) | IncomingPayload::UnknownPayload { .. } => {}
        }
        Ok(())
    }

    /// Run libthreema's incoming message task to completion, then acknowledge and notify.
    async fn handle_incoming(
        &mut self,
        message: MessageWithMetadataBox,
    ) -> Result<(), ConnectionError> {
        let (sender_identity, id, flags) = (message.sender_identity, message.id, message.flags.0);
        let item = FailingItem::Message {
            sender: sender_identity.to_string(),
            id: id.0,
        };
        let ack = match self.run_incoming_task(message).await {
            Ok(ack) => {
                self.storage_works(StorageStep::Incoming);
                ack
            }
            Err(error) => {
                let error = error.while_doing(StorageStep::Incoming);
                if !error.fails_every_time() {
                    if matches!(error, ConnectionError::Protocol(_)) {
                        tracing::warn!("an incoming message failed, it comes again");
                    }
                    return Err(error);
                }
                // As Android does right away, whatever failed: only the server can hold it, and
                // it holds up the rest of the queue until it is acknowledged.
                let failures = self.setup.failing_items.failed(&item);
                if failures < TRIES_BEFORE_DROPPING {
                    tracing::warn!(failures, "an incoming message failed, it comes again");
                    return Err(error);
                }
                tracing::error!(failures, "an incoming message keeps failing and is dropped");
                (flags & MessageFlags::NO_SERVER_ACKNOWLEDGEMENT == 0).then_some(MessageAck {
                    sender_identity,
                    id,
                })
            }
        };
        // It's in the store by now: before anything that can fail on the network.
        self.emit_stored_message_events();
        if let Some(ack) = ack {
            self.send_payload(&OutgoingPayload::MessageAck(ack)).await?;
        }
        self.setup.failing_items.forget(&item);
        self.after_incoming().await
    }

    /// libthreema's incoming message task, to its end. What is left to do is the acknowledgement,
    /// if any.
    async fn run_incoming_task(
        &mut self,
        message: MessageWithMetadataBox,
    ) -> Result<Option<MessageAck>, ConnectionError> {
        let (sender, message_id) = (message.sender_identity.to_string(), message.id.0);
        let mut task = self.e2e.handle_incoming_message(message);
        let result = loop {
            match task.poll(self.e2e.context()).map_err(e2e_error)? {
                IncomingMessageLoop::Instruction(IncomingMessageInstruction::FetchSender(
                    lookup,
                )) => {
                    let directory_result = self.setup.https.send(lookup.directory_request).await;
                    task.response(IncomingMessageResponse::FetchSender(
                        ContactsLookupResponse {
                            directory_result,
                            work_directory_result: None,
                        },
                    ))
                    .map_err(e2e_error)?;
                }
                // Multi-device: reflect to the other devices before acknowledging; contacts are
                // created/updated in a transaction.
                IncomingMessageLoop::Instruction(IncomingMessageInstruction::ReflectMessage(
                    instruction,
                )) => {
                    // The message is stored: the wait for the mediator is no reason to hold it back.
                    self.emit_stored_message_events();
                    let response = self.run_reflect(instruction).await?;
                    task.response(IncomingMessageResponse::ReflectMessage(response))
                        .map_err(protocol_error)?;
                }
                IncomingMessageLoop::Instruction(IncomingMessageInstruction::CreateContact(
                    instruction,
                )) => {
                    let response = match instruction {
                        CreateContactsInstruction::BeginTransaction(begin) => {
                            CreateContactsResponse::BeginTransactionResponse(
                                self.run_begin_transaction(begin).await?,
                            )
                        }
                        CreateContactsInstruction::ReflectAndCommitTransaction(commit) => {
                            CreateContactsResponse::CommitTransactionResponse(
                                self.run_commit_transaction(commit).await?,
                            )
                        }
                    };
                    task.response(IncomingMessageResponse::CreateContact(response))
                        .map_err(protocol_error)?;
                }
                IncomingMessageLoop::Instruction(IncomingMessageInstruction::UpdateContact(
                    instruction,
                )) => {
                    let response = match instruction {
                        UpdateContactsInstruction::BeginTransaction(begin) => {
                            UpdateContactsResponse::BeginTransactionResponse(
                                self.run_begin_transaction(begin).await?,
                            )
                        }
                        UpdateContactsInstruction::ReflectAndCommitTransaction(commit) => {
                            UpdateContactsResponse::CommitTransactionResponse(
                                self.run_commit_transaction(commit).await?,
                            )
                        }
                    };
                    task.response(IncomingMessageResponse::UpdateContact(response))
                        .map_err(protocol_error)?;
                }
                IncomingMessageLoop::Done(result) => break result,
            }
        };
        if self.multi_device.is_some() {
            self.setup
                .store
                .reflection_done(&sender, message_id)
                .map_err(storage_error)?;
        }
        Ok(result.outgoing_message_ack)
    }

    /// Look up group members we have no contact for (`app_groups::resolve_members`) and send
    /// whatever waited for them.
    async fn resolve_members(&mut self, identities: &[String]) -> Result<(), ConnectionError> {
        if let Err(error) = self.setup.store.owe_member_lookups(identities) {
            tracing::warn!(%error, "noting group members to look up failed");
        }
        if self.setup.store.directory_resting() {
            return Ok(());
        }
        let Some(resolved) = crate::app_groups::resolve_members(
            &self.setup.https,
            &self.setup.config,
            &self.setup.store,
            identities,
        )
        .await
        else {
            return Ok(());
        };
        if resolved.synced {
            self.reflect_queued_syncs().await?;
        }
        if resolved.groups_changed {
            self.emit(CoreEvent::GroupsChanged);
        }
        self.emit(CoreEvent::ContactsChanged);
        for message in resolved.lines {
            self.emit(CoreEvent::MessageAdded { message });
        }
        self.send_pending().await
    }

    /// What handling an incoming message left to do. If that fails, what is still to come stays
    /// in the event queue (and what was just tried, if only the connection failed it) for
    /// [`Self::drop`] to see to.
    async fn after_incoming(&mut self) -> Result<(), ConnectionError> {
        let events = status_after_messages(self.events.borrow_mut().drain(..).collect());
        let mut events = std::collections::VecDeque::from(events);
        while let Some(event) = events.pop_front() {
            if let Err(error) = self.after_incoming_event(event.clone()).await {
                if error.is_connection_lost() {
                    events.push_front(event);
                }
                let mut queue = self.events.borrow_mut();
                let newer = core::mem::take(&mut *queue);
                queue.extend(events);
                queue.extend(newer);
                return Err(error);
            }
        }
        Ok(())
    }

    async fn after_incoming_event(&mut self, event: CoreEvent) -> Result<(), ConnectionError> {
        match event {
            // A group change goes out before the next message of a burst is even looked at, or
            // the other devices get a message for a group they don't have yet, and drop it.
            CoreEvent::SyncQueued => self.reflect_queued_syncs().await,
            // Before the next message too, which may be from one of them, and so that what we
            // send to the group reaches them. A failed lookup is tried again later (`on_connected`,
            // `maintenance`); the setup is acknowledged by now and won't come again.
            CoreEvent::UnknownMembers(identities) => self.resolve_members(&identities).await,
            event => {
                if let Some(event) = Box::pin(self.handle_fs_event(event)).await? {
                    self.emit(event);
                }
                Ok(())
            }
        }
    }
}

/// Group messages that aren't for the members: to say that someone is out of the group (leave,
/// empty setup), or to the creator, who needn't be listed in a damaged group (sync request).
fn goes_to_non_members(body: &GroupMessageBody) -> bool {
    match body {
        GroupMessageBody::Leave(_) | GroupMessageBody::SyncRequest(_) => true,
        GroupMessageBody::Setup(setup) => setup.members.is_empty(),
        _ => false,
    }
}

/// Messages that distribute the user profile (nickname, picture) to their receivers.
fn distributes_profile(body: &GroupMessageBody) -> bool {
    matches!(
        body,
        GroupMessageBody::Text(_)
            | GroupMessageBody::Location(_)
            | GroupMessageBody::File(_)
            | GroupMessageBody::PollSetup(_)
            | GroupMessageBody::PollVote(_)
    )
}

/// What one member gets of a group message: the message itself, except that a member without
/// `REACTION_SUPPORT` gets a reaction as the thumbs up or down it maps to, if it does (the group
/// reaction steps in `csp-e2e.proto`). Any other reaction goes to them as it is, which the steps
/// intend; Android leaves them out instead.
fn body_for_receiver(
    body: &GroupMessageBody,
    features: Option<libthreema::common::FeatureMask>,
) -> GroupMessageBody {
    if let GroupMessageBody::Reaction(ReactionMessage {
        message_id,
        action: ReactionAction::Apply(emoji),
    }) = body
        && features
            .is_some_and(|mask| mask.0 & libthreema::common::FeatureMask::REACTION_SUPPORT == 0)
        && let Some(reaction_type) = reactions::legacy_reaction(emoji)
    {
        return GroupMessageBody::LegacyReaction(LegacyReactionMessage {
            reaction_type,
            message_ids: vec![*message_id],
        });
    }
    body.clone()
}

/// The group variant of a message body (what can be sent to groups at all).
fn to_group_body(body: ContactMessageBody) -> Option<GroupMessageBody> {
    Some(match body {
        ContactMessageBody::Text(message) => GroupMessageBody::Text(message),
        ContactMessageBody::Location(message) => GroupMessageBody::Location(message),
        ContactMessageBody::File(message) => GroupMessageBody::File(message),
        ContactMessageBody::PollSetup(message) => GroupMessageBody::PollSetup(message),
        ContactMessageBody::PollVote(message) => GroupMessageBody::PollVote(message),
        ContactMessageBody::Reaction(message) => GroupMessageBody::Reaction(message),
        ContactMessageBody::LegacyReaction(message) => GroupMessageBody::LegacyReaction(message),
        ContactMessageBody::Edit(message) => GroupMessageBody::Edit(message),
        ContactMessageBody::Delete(message) => GroupMessageBody::Delete(message),
        ContactMessageBody::DeliveryReceipt(_)
        | ContactMessageBody::SetProfilePicture(_)
        | ContactMessageBody::DeleteProfilePicture(_)
        | ContactMessageBody::RequestProfilePicture(_)
        | ContactMessageBody::TypingIndicator(_)
        | ContactMessageBody::WebSessionResume(_)
        | ContactMessageBody::CallOffer(_)
        | ContactMessageBody::CallAnswer(_)
        | ContactMessageBody::CallIceCandidate(_)
        | ContactMessageBody::CallHangup(_)
        | ContactMessageBody::CallRinging(_) => return None,
    })
}

/// A group outbox entry's timestamp on the wire: when it was queued, or for a repeat of a call's
/// start the call's own start (csp.struct.yml). `created_at` stays when it was queued, as the
/// order of what we send goes by that.
fn outbox_sent_as_of(entry: &reactions::OutboxEntry) -> i64 {
    entry.body["startedAt"].as_i64().unwrap_or(entry.created_at)
}

/// The body of a group outbox entry (control messages, or a reaction/edit/deletion).
fn outbox_group_body(entry: &reactions::OutboxEntry) -> Option<GroupMessageBody> {
    use crate::groups::{
        OUTBOX_GROUP_DELETE_PICTURE, OUTBOX_GROUP_LEAVE, OUTBOX_GROUP_NAME, OUTBOX_GROUP_SETUP,
        OUTBOX_GROUP_SYNC_REQUEST,
    };
    Some(match entry.kind.as_str() {
        OUTBOX_GROUP_SETUP => GroupMessageBody::Setup(GroupSetupMessage {
            members: entry.body["members"]
                .as_array()?
                .iter()
                .filter_map(|m| ThreemaId::try_from(m.as_str()?).ok())
                .collect(),
        }),
        OUTBOX_GROUP_NAME => GroupMessageBody::Name(GroupNameMessage {
            name: entry.body["name"].as_str().unwrap_or_default().to_owned(),
        }),
        OUTBOX_GROUP_LEAVE => GroupMessageBody::Leave(GroupLeaveMessage),
        OUTBOX_GROUP_SYNC_REQUEST => GroupMessageBody::SyncRequest(GroupSyncRequestMessage),
        OUTBOX_GROUP_DELETE_PICTURE => {
            GroupMessageBody::DeleteProfilePicture(DeleteProfilePictureMessage)
        }
        crate::groups::OUTBOX_GROUP_SET_PICTURE => {
            let picture = crate::groups::GroupPicture::from_json(&entry.body)?;
            GroupMessageBody::SetProfilePicture(SetProfilePictureMessage {
                blob_id: picture.blob_id,
                size: picture.size,
                key: picture.key,
            })
        }
        crate::groups::OUTBOX_GROUP_CALL_START => {
            GroupMessageBody::CallStart(GroupCallStartMessage {
                protocol_version: u32::try_from(entry.body["version"].as_u64()?).ok()?,
                gck: data_encoding::HEXLOWER
                    .decode(entry.body["gck"].as_str()?.as_bytes())
                    .ok()?
                    .try_into()
                    .ok()?,
                sfu_base_url: entry.body["url"].as_str()?.to_owned(),
            })
        }
        _ => return to_group_body(outbox_body(entry)?),
    })
}

/// A message body for one receiver: 1:1, or a group message (sent to each member).
pub(crate) enum Outgoing {
    Contact(ContactMessageBody),
    Group(GroupIdentity, GroupMessageBody),
}

impl Outgoing {
    fn is_exempt_from_blocking(&self) -> bool {
        match self {
            Self::Contact(body) => body.is_exempt_from_blocking(),
            Self::Group(_, body) => body.is_exempt_from_blocking(),
        }
    }
}

/// The message body of an outbox entry (see [`reactions::OUTBOX_REACTION`] and friends).
fn outbox_body(entry: &reactions::OutboxEntry) -> Option<ContactMessageBody> {
    let poll_id = entry.body["pollId"]
        .as_str()
        .and_then(|id| u64::from_str_radix(id, 16).ok())
        .unwrap_or(0);
    let message_id = MessageId(
        entry.body["messageId"]
            .as_str()
            .and_then(|id| u64::from_str_radix(id, 16).ok())
            .unwrap_or(0),
    );
    let emoji = || entry.body["emoji"].as_str().map(str::to_owned);
    match entry.kind.as_str() {
        reactions::OUTBOX_REACTION => Some(ContactMessageBody::Reaction(ReactionMessage {
            message_id,
            action: if entry.body["apply"].as_bool()? {
                ReactionAction::Apply(emoji()?)
            } else {
                ReactionAction::Withdraw(emoji()?)
            },
        })),
        reactions::OUTBOX_LEGACY_REACTION => {
            Some(ContactMessageBody::LegacyReaction(LegacyReactionMessage {
                reaction_type: reactions::legacy_reaction(&emoji()?)?,
                message_ids: vec![message_id],
            }))
        }
        reactions::OUTBOX_EDIT => Some(ContactMessageBody::Edit(EditMessage {
            message_id,
            text: entry.body["text"].as_str()?.to_owned(),
        })),
        reactions::OUTBOX_DELETE => Some(ContactMessageBody::Delete(DeleteMessage { message_id })),
        reactions::OUTBOX_DELIVERY_RECEIPT => Some(ContactMessageBody::DeliveryReceipt(
            DeliveryReceiptMessage {
                receipt_type: DeliveryReceiptType::Received,
                message_ids: vec![message_id],
            },
        )),
        crate::profile_pictures::OUTBOX_REQUEST_PICTURE => Some(
            ContactMessageBody::RequestProfilePicture(RequestProfilePictureMessage),
        ),
        crate::polls::OUTBOX_POLL_VOTE => Some(ContactMessageBody::PollVote(PollVoteMessage {
            creator_identity: ThreemaId::try_from(entry.body["creator"].as_str()?).ok()?,
            poll_id,
            choices: entry.body["choices"].as_str()?.to_owned(),
        })),
        crate::polls::OUTBOX_POLL_CLOSE => Some(ContactMessageBody::PollSetup(PollSetupMessage {
            poll_id,
            poll: entry.body["poll"].as_str()?.to_owned(),
        })),
        _ => None,
    }
}

#[cfg(test)]
mod fs_status_tests {
    use super::*;

    #[test]
    fn a_mask_check_counts_for_its_interval_and_a_distrusted_one_gives_no_answer() {
        let checks = FsMaskChecks::default();
        let start = Instant::now();
        let minute = Duration::from_secs(60);
        assert_eq!(checks.recent("ABCD1234", start, minute), None);

        checks.record("ABCD1234", start, true);
        assert_eq!(checks.recent("ABCD1234", start, minute), Some(true));
        assert_eq!(checks.recent("EFGH5678", start, minute), None);
        let later = start + Duration::from_secs(30);
        assert_eq!(checks.recent("ABCD1234", later, minute), Some(true));
        // An Init looks sooner than others do.
        assert_eq!(
            checks.recent("ABCD1234", later, FS_MASK_INIT_CHECK_INTERVAL),
            None
        );

        // An Init within its own interval: no going by that "no", and no extra lookup either.
        checks.distrust("ABCD1234");
        assert_eq!(checks.recent("ABCD1234", later, minute), Some(false));
        // A failed or distrusted lookup is tried again after the interval.
        let after = start + minute;
        assert_eq!(checks.recent("ABCD1234", after, minute), None);
    }

    #[test]
    fn the_line_for_a_message_without_forward_secrecy_follows_the_message() {
        let without = || CoreEvent::FsStatus {
            peer: "ABCD1234".into(),
            status: FsStatus::MessageWithoutFs,
        };
        let established = || CoreEvent::FsStatus {
            peer: "ABCD1234".into(),
            status: FsStatus::Established,
        };
        let events =
            status_after_messages(vec![without(), established(), CoreEvent::ContactsChanged]);
        assert!(matches!(
            events.as_slice(),
            [
                CoreEvent::FsStatus {
                    status: FsStatus::Established,
                    ..
                },
                CoreEvent::ContactsChanged,
                CoreEvent::FsStatus {
                    status: FsStatus::MessageWithoutFs,
                    ..
                },
            ]
        ));
    }

    #[test]
    fn a_rejected_message_is_back_in_the_queue_with_nothing_sent() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        store
            .upsert_contact(&crate::contacts::test_contact("ABCD1234"))
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO messages
                     (conversation, message_id, outgoing, kind, body, created_at, updated_at, state)
                 VALUES ('ABCD1234', 7, 1, 'text', 'hi', 1, 1, 1)",
                [],
            )
            .unwrap();
        assert!(store.pending_outgoing().unwrap().is_empty());

        let requeued = requeue_rejected(&store, "EFGH5678", "ABCD1234", 7, None).unwrap();
        assert!(requeued.send && requeued.resending);
        assert_eq!(requeued.changed.len(), 1);
        assert_eq!(store.pending_outgoing().unwrap().len(), 1);

        // What isn't ours to send: nothing to do, and no claim that it goes out.
        let requeued = requeue_rejected(&store, "EFGH5678", "ABCD1234", 8, None).unwrap();
        assert!(!requeued.send && !requeued.resending && requeued.changed.is_empty());
    }

    fn rejected(session_known: bool) -> FsStatus {
        FsStatus::MessageRejected {
            message_id: 1,
            group_identity: None,
            session_known,
        }
    }

    #[test]
    fn lines_only_for_sessions_that_existed() {
        assert_eq!(fs_status_line(&rejected(false), true, true), None);
        assert_eq!(fs_status_line(&rejected(false), false, true), None);
        let terminated = FsStatus::Terminated {
            session_known: false,
        };
        assert_eq!(fs_status_line(&terminated, false, false), None);
    }

    #[test]
    fn a_rejected_message_is_only_said_to_go_out_again_if_it_does() {
        assert_eq!(
            fs_status_line(&rejected(true), true, true),
            Some("fsMessageRejected")
        );
        assert_eq!(
            fs_status_line(&rejected(true), true, false),
            Some("fsReset")
        );
    }

    #[test]
    fn the_feature_mask_says_why_a_session_ended() {
        let terminated = FsStatus::Terminated {
            session_known: true,
        };
        assert_eq!(fs_status_line(&terminated, true, false), Some("fsReset"));
        assert_eq!(
            fs_status_line(&terminated, false, false),
            Some("fsUnavailable")
        );
        assert_eq!(
            fs_status_line(&rejected(true), false, true),
            Some("fsUnavailable")
        );
    }

    #[test]
    fn a_message_without_forward_secrecy_warns_while_the_contact_supports_it() {
        assert_eq!(
            fs_status_line(&FsStatus::MessageWithoutFs, true, false),
            Some(FS_MESSAGE_WITHOUT)
        );
        assert_eq!(
            fs_status_line(&FsStatus::MessageWithoutFs, false, false),
            Some("fsUnavailable")
        );
    }

    #[test]
    fn forward_secrecy_is_only_called_active_if_the_mask_agrees() {
        assert_eq!(
            fs_status_line(&FsStatus::Established, true, false),
            Some("fsEstablished")
        );
        assert_eq!(fs_status_line(&FsStatus::Established, false, false), None);
    }
}

#[cfg(test)]
mod backoff_tests {
    use super::*;

    const LONG: Duration = Duration::from_secs(3600);
    const SHORT: Duration = Duration::from_secs(5);

    #[test]
    fn a_working_connection_starts_over() {
        assert_eq!(
            backoff_after_session(MAX_BACKOFF, LONG),
            Duration::from_secs(1)
        );
        assert_eq!(backoff_after_session(MAX_BACKOFF, SHORT), MAX_BACKOFF);
    }

    #[test]
    fn told_to_stay_away_holds_however_long_the_session_was() {
        assert_eq!(backoff_after_session(HOLD_OFF, LONG), HOLD_OFF);
        assert_eq!(backoff_after_session(HOLD_OFF, SHORT), HOLD_OFF);
    }

    #[test]
    fn only_a_user_who_answered_tells_contacts_to_drop_the_name() {
        assert_eq!(
            nickname_delta(Some("Tester"), false),
            Delta::Update("Tester")
        );
        assert_eq!(
            nickname_delta(Some("Tester"), true),
            Delta::Update("Tester")
        );
        assert_eq!(nickname_delta(None, false), Delta::Remove);
        assert_eq!(nickname_delta(None, true), Delta::Unchanged);
    }

    #[test]
    fn a_rate_limit_keeps_its_identity_and_gets_a_longer_wait() {
        assert!(matches!(
            e2e_error(CspE2eProtocolError::RateLimitExceeded),
            ConnectionError::RateLimited
        ));
        assert!(matches!(
            e2e_error(CspE2eProtocolError::NetworkError("gone".into())),
            ConnectionError::Protocol(_)
        ));

        // Login after login meets the limit: the wait keeps growing past the usual cap.
        let mut wait = RATE_LIMIT_FLOOR;
        let mut waits = Vec::new();
        for _ in 0..6 {
            wait = next_backoff(wait, true);
            waits.push(wait.as_secs());
        }
        assert_eq!(waits, [20, 40, 80, 160, 300, 300]);
        // Over: back to the usual cap.
        assert_eq!(next_backoff(RATE_LIMIT_MAX_BACKOFF, false), MAX_BACKOFF);
        assert_eq!(
            next_backoff(Duration::from_secs(1), false),
            Duration::from_secs(2)
        );
    }
}

#[cfg(test)]
mod liveness_tests {
    use super::*;

    #[test]
    fn silence_asks_with_an_echo_and_its_time_runs_out() {
        let start = Instant::now();
        let mut liveness = Liveness::new(start);
        assert!(!liveness.awaiting_echo());
        assert_eq!(liveness.deadline(), start + ECHO_INTERVAL);
        liveness.echo_sent(start + ECHO_INTERVAL, ECHO_INTERVAL);
        assert!(liveness.awaiting_echo());
        assert_eq!(liveness.deadline(), start + ECHO_INTERVAL * 2);
    }

    #[test]
    fn data_read_inside_a_long_handler_counts() {
        let start = Instant::now();
        let mut liveness = Liveness::new(start);
        // A link's registration ran for 3 minutes, reading the mediator all along.
        let later = start + Duration::from_secs(180);
        liveness.received(later);
        assert_eq!(liveness.deadline(), later + ECHO_INTERVAL);
        assert!(!liveness.awaiting_echo());
    }

    #[test]
    fn a_handler_that_read_nothing_gets_an_echo_before_any_timeout() {
        let start = Instant::now();
        let mut liveness = Liveness::new(start);
        // Past the old limit of twice the interval: still only time to ask.
        let later = start + ECHO_INTERVAL * 3;
        assert!(liveness.deadline() <= later);
        assert!(!liveness.awaiting_echo(), "the first thing due is an echo");
        liveness.echo_sent(later, ECHO_INTERVAL);
        assert_eq!(
            liveness.deadline(),
            later + ECHO_INTERVAL,
            "the echo has its full time"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_answer_in_the_socket_beats_the_passed_deadline() {
        let read = last_look_for_echo(std::future::ready(7)).await;
        assert!(matches!(read, Ok(7)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_answer_within_the_last_look_is_a_timeout() {
        let read = last_look_for_echo(std::future::pending::<()>()).await;
        assert!(matches!(read, Err(ConnectionError::Timeout)));
    }

    #[test]
    fn a_wake_up_cuts_short_the_time_of_an_echo_that_is_out() {
        let start = Instant::now();
        let mut liveness = Liveness::new(start);
        liveness.echo_sent(start, ECHO_INTERVAL);
        // The device slept with most of the minute left: nobody is up for that long.
        liveness.echo_due_by(start + WAKE_ECHO_LIMIT);
        assert_eq!(liveness.deadline(), start + WAKE_ECHO_LIMIT);
        // Another wake-up doesn't give it more.
        liveness.echo_due_by(start + WAKE_ECHO_LIMIT * 2);
        assert_eq!(liveness.deadline(), start + WAKE_ECHO_LIMIT);
    }

    #[test]
    fn an_answer_ends_the_wait_for_an_echo() {
        let start = Instant::now();
        let mut liveness = Liveness::new(start);
        liveness.echo_sent(start, Duration::from_secs(15));
        assert_eq!(liveness.deadline(), start + Duration::from_secs(15));
        let answered = start + Duration::from_secs(1);
        liveness.received(answered);
        assert!(!liveness.awaiting_echo());
        assert_eq!(liveness.deadline(), answered + ECHO_INTERVAL);
    }
}

#[cfg(test)]
mod wake_tests {
    use super::*;

    const WAIT: Duration = Duration::from_secs(20);

    struct Waiting {
        to: mpsc::UnboundedSender<ToConnection>,
        rx: mpsc::UnboundedReceiver<ToConnection>,
        tx: mpsc::UnboundedSender<FromConnection>,
        from: mpsc::UnboundedReceiver<FromConnection>,
        hold: WakeHold,
    }

    fn waiting() -> Waiting {
        let (to, rx) = mpsc::unbounded_channel();
        let (tx, from) = mpsc::unbounded_channel();
        Waiting {
            to,
            rx,
            tx,
            from,
            hold: WakeHold::default(),
        }
    }

    impl Waiting {
        /// How long the wait lasted, given what is in the channel when it begins.
        async fn lasted(&mut self, backoff: Duration, earliest: Option<Instant>) -> Duration {
            let start = Instant::now();
            let goes_on =
                wait_for_next_attempt(backoff, earliest, &mut self.rx, &self.tx, &self.hold).await;
            assert!(goes_on);
            start.elapsed()
        }

        fn settled(&mut self) -> bool {
            matches!(self.from.try_recv(), Ok(FromConnection::WakeSettled))
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_wake_up_ends_the_ordinary_wait_and_stays_for_the_attempt() {
        let mut wait = waiting();
        wait.to.send(ToConnection::Wake).unwrap();
        assert_eq!(wait.lasted(WAIT, None).await, Duration::ZERO);
        assert!(!wait.settled(), "its outcome is the attempt's");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_wake_up_by_itself_leaves_the_wait_a_server_asked_for() {
        let mut wait = waiting();
        wait.to.send(ToConnection::Wake).unwrap();
        wait.to.send(ToConnection::Wake).unwrap();
        assert_eq!(wait.lasted(HOLD_OFF, None).await, HOLD_OFF);
        assert!(wait.settled(), "the device can go back to sleep");
        assert!(wait.settled());
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn connect_now_ends_the_wait_a_server_asked_for() {
        let mut wait = waiting();
        wait.to.send(ToConnection::ConnectNow).unwrap();
        assert_eq!(wait.lasted(HOLD_OFF, None).await, Duration::ZERO);
    }

    /// The IPv6 switch, the forward secrecy switch, a device's name, multi-device on and off: all
    /// of these need the new connection now, whatever it was waiting for. A link's next step
    /// waits for that connection in the queue.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_changed_setting_ends_any_wait() {
        for backoff in [WAIT, HOLD_OFF] {
            let mut wait = waiting();
            wait.to.send(ToConnection::Reconnect).unwrap();
            wait.to.send(ToConnection::SendPending).unwrap();
            assert_eq!(wait.lasted(backoff, None).await, Duration::ZERO);
            assert!(matches!(wait.rx.try_recv(), Ok(ToConnection::SendPending)));
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn nothing_comes_before_the_floor_of_a_rate_limit() {
        for wake in [
            ToConnection::Wake,
            ToConnection::ConnectNow,
            ToConnection::Reconnect,
        ] {
            let mut wait = waiting();
            wait.to.send(wake).unwrap();
            let floor = Instant::now() + RATE_LIMIT_FLOOR;
            assert_eq!(wait.lasted(WAIT, Some(floor)).await, RATE_LIMIT_FLOOR);
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn the_app_going_away_ends_the_wait() {
        let mut wait = waiting();
        drop(wait.to);
        assert!(!wait_for_next_attempt(HOLD_OFF, None, &mut wait.rx, &wait.tx, &wait.hold).await);
    }

    #[test]
    fn the_clock_counts_for_the_wait_while_the_device_slept() {
        assert!(wake_ends_wait(MAX_BACKOFF, Duration::ZERO));
        assert!(wake_ends_wait(RATE_LIMIT_MAX_BACKOFF, Duration::ZERO));
        assert!(!wake_ends_wait(HOLD_OFF, HOLD_OFF / 2));
        assert!(wake_ends_wait(HOLD_OFF, HOLD_OFF));
    }

    /// A network whose address translation forgets us between wake-ups: every wake-up finds the
    /// connection dead after a session that was minutes long by the clock, and little by tokio's.
    /// Each must stay for the new connection, or only every second one gets it back.
    #[test]
    fn every_wake_up_of_a_pocket_gets_its_connection_back() {
        let mut backoff = Duration::from_secs(1);
        for _ in 0..6 {
            let session = Stopwatch {
                awake: Instant::now(),
                wall: std::time::SystemTime::now() - Duration::from_secs(300),
            };
            backoff = backoff_after_session(backoff, session.elapsed());
            assert!(next_attempt_is_worth_holding_for(true, backoff));
            backoff = next_backoff(backoff, false);
        }
    }

    #[test]
    fn a_stopwatch_goes_by_whichever_clock_went_further() {
        let awake = Stopwatch {
            awake: Instant::now() - Duration::from_secs(90),
            wall: std::time::SystemTime::now() + Duration::from_secs(3600),
        };
        assert!(awake.elapsed() >= Duration::from_secs(90), "clock set back");
    }

    #[test]
    fn a_wake_up_stays_only_for_an_attempt_that_is_near() {
        assert!(next_attempt_is_worth_holding_for(true, WAKE_RETRY_SOON));
        assert!(!next_attempt_is_worth_holding_for(
            true,
            WAKE_RETRY_SOON * 2
        ));
        assert!(!next_attempt_is_worth_holding_for(false, WAKE_RETRY_SOON));
        assert!(!next_attempt_is_worth_holding_for(true, HOLD_OFF));
    }

    #[test]
    fn after_a_login_it_takes_the_chat_servers_queue() {
        let hold = WakeHold::default();
        let start = Instant::now();
        hold.asked();
        hold.logged_in(false);
        hold.heard(start);
        assert_eq!(hold.settles_at(), None, "messages are still coming");
        hold.chat_queue_through(start);
        assert_eq!(hold.settles_at(), Some(start + WAKE_QUIET));
    }

    #[test]
    fn through_the_mediator_it_takes_its_queue_and_the_leaders() {
        let hold = WakeHold::default();
        let start = Instant::now();
        hold.asked();
        hold.logged_in(true);
        hold.heard(start);
        assert_eq!(hold.settles_at(), None);

        // Another device leads: nothing more is coming from the chat server.
        hold.reflection_queue_through(start);
        assert_eq!(hold.settles_at(), Some(start + WAKE_QUIET));

        // We lead after all: now it has a queue for us.
        hold.promoted_to_leader();
        assert_eq!(hold.settles_at(), None);
        hold.chat_queue_through(start);
        assert_eq!(hold.settles_at(), Some(start + WAKE_QUIET));
    }

    /// Nobody was waiting when the session began, so its queues needn't have been noted as through
    /// by anything that counts: an echo's answer is enough for a wake-up hours later.
    #[test]
    fn a_later_wake_up_only_needs_an_answer() {
        let hold = WakeHold::default();
        let start = Instant::now();
        hold.logged_in(false);
        hold.asked();
        hold.heard(start);
        assert_eq!(hold.settles_at(), Some(start + WAKE_QUIET));
    }

    #[test]
    fn a_wake_up_lasts_until_things_have_stopped_coming() {
        let (tx, mut from) = mpsc::unbounded_channel();
        let hold = WakeHold::default();
        let start = Instant::now();
        hold.heard(start);
        assert_eq!(hold.settles_at(), None, "not waiting for anything");

        hold.asked();
        assert_eq!(hold.settles_at(), None, "no answer yet");
        hold.heard(start);
        assert_eq!(hold.settles_at(), Some(start + WAKE_QUIET));
        let later = start + Duration::from_secs(1);
        hold.heard(later);
        assert_eq!(hold.settles_at(), Some(later + WAKE_QUIET), "more came");

        hold.settle(&tx);
        assert!(matches!(from.try_recv(), Ok(FromConnection::WakeSettled)));
        assert_eq!(hold.settles_at(), None);
        hold.settle(&tx);
        assert!(from.try_recv().is_err(), "said once");
    }
}

#[cfg(test)]
mod device_group_error_tests {
    use super::*;

    #[test]
    fn only_a_lost_connection_counts_as_offline() {
        for lost in [
            ConnectionError::Timeout,
            ConnectionError::Reconnect,
            ConnectionError::from(TransportError::Closed),
            ConnectionError::from(TransportError::WriteTimeout),
            ConnectionError::from(TransportError::Io(
                std::io::ErrorKind::ConnectionReset.into(),
            )),
            ConnectionError::from(TransportError::WebSocket(WebSocketError::Closed {
                code: None,
                reason: "end of stream".into(),
            })),
        ] {
            assert_eq!(DeviceGroupError::from(lost), DeviceGroupError::NotConnected);
        }
        for refused in [
            ConnectionError::Protocol("expected devices info".into()),
            ConnectionError::MediatorClosed {
                code: 4114,
                reason: String::new(),
            },
            ConnectionError::Closed {
                message: "maintenance".into(),
                can_reconnect: true,
            },
        ] {
            assert!(matches!(
                DeviceGroupError::from(refused),
                DeviceGroupError::Failed(_)
            ));
        }
    }
}

#[cfg(test)]
mod drop_plan_tests {
    use super::*;

    const OWN: u64 = 1;
    const DESKTOP: u64 = 2;
    const LAPTOP: u64 = 3;
    const GONE: u64 = 4;

    #[test]
    fn removing_the_last_other_device_ends_multi_device() {
        assert_eq!(
            plan_drop(Some(&[DESKTOP]), &[DESKTOP], OWN),
            (vec![DESKTOP], true)
        );
    }

    #[test]
    fn removing_one_of_two_keeps_the_group() {
        assert_eq!(
            plan_drop(Some(&[DESKTOP]), &[DESKTOP, LAPTOP], OWN),
            (vec![DESKTOP], false)
        );
    }

    #[test]
    fn removing_all_named_ends_multi_device() {
        assert_eq!(
            plan_drop(Some(&[DESKTOP, LAPTOP]), &[LAPTOP, DESKTOP], OWN),
            (vec![LAPTOP, DESKTOP], true)
        );
    }

    #[test]
    fn deactivating_removes_whatever_the_mediator_lists() {
        assert_eq!(
            plan_drop(None, &[DESKTOP, LAPTOP], OWN),
            (vec![DESKTOP, LAPTOP], true)
        );
        assert_eq!(plan_drop(None, &[], OWN), (Vec::new(), true));
    }

    #[test]
    fn a_device_that_is_gone_already_is_not_dropped_again() {
        assert_eq!(
            plan_drop(Some(&[GONE]), &[DESKTOP], OWN),
            (Vec::new(), false)
        );
        // ... and if it was the last one, this device is alone.
        assert_eq!(plan_drop(Some(&[GONE]), &[], OWN), (Vec::new(), true));
    }

    #[test]
    fn alone_ends_multi_device_and_with_others_nothing_happens() {
        assert_eq!(plan_drop(Some(&[]), &[], OWN), (Vec::new(), true));
        assert_eq!(plan_drop(Some(&[]), &[DESKTOP], OWN), (Vec::new(), false));
    }

    #[test]
    fn this_device_alone_may_leave_while_others_stay() {
        assert_eq!(plan_drop(Some(&[OWN]), &[DESKTOP], OWN), (Vec::new(), true));
    }
}

#[cfg(test)]
mod sync_order_tests {
    use libthreema::model::{
        message::{
            GroupMessageBody, GroupSetupMessage, IncomingGroupMessageBody, IncomingMessage,
            IncomingMessageBody, TextMessage,
        },
        provider::ConversationProvider as _,
    };
    use tokio::net::{TcpListener, TcpStream};

    use super::*;
    use crate::{
        app::{App, StartConfig},
        store::StoredIdentity,
    };

    const USER: &str = "USER0001";
    const CREATOR: &str = "ABCD1234";

    fn app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(
            &StartConfig {
                data_dir: dir.path().to_owned(),
                app_version: "0.1.0".into(),
                language: "en".into(),
                country: "US".into(),
                model: "test".into(),
                os_version: "5.1".into(),
                pictures_dir: None,
                calls: false,
                log_level: None,
                update_check: false,
            },
            &[3; 32],
        )
        .unwrap();
        app.store
            .set_identity(&StoredIdentity {
                threema_id: USER.into(),
                client_key: Zeroizing::new([9; 32]),
                server_group: "01".into(),
            })
            .unwrap();
        (dir, app)
    }

    /// A contact whose stored mask says no, with a forward secrecy session left, and what it takes
    /// to talk to it: no lookup goes out as long as `fs_mask_checks` has an entry.
    fn contact_without_fs_support(app: &App) -> (ConnectionSetup, libthreema::common::ThreemaId) {
        let mut contact = crate::contacts::test_contact(CREATOR);
        contact.feature_mask = libthreema::common::FeatureMask(0);
        app.store.upsert_contact(&contact).unwrap();
        let identity = app.store.identity().unwrap().unwrap();
        let setup = ConnectionSetup {
            store: Rc::clone(&app.store),
            media: Arc::clone(&app.media),
            https: app.https.clone(),
            config: Arc::clone(&app.config),
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
        };
        let mut fs = StoreFs {
            store: Rc::clone(&setup.store),
            my_identity: setup.identity,
            events: EventSink::default(),
        };
        fs_protocol::encapsulate(
            &mut fs,
            setup.identity,
            &ClientKey::from(*setup.client_key),
            contact.identity,
            &contact.public_key,
            0x01,
            b"hello",
            None,
            1,
        )
        .unwrap();
        (setup, contact.identity)
    }

    fn text() -> Outgoing {
        Outgoing::Contact(ContactMessageBody::Text(TextMessage { text: "hi".into() }))
    }

    /// In a mixed group the thumbs reach a member on an old client as what they understand; what
    /// has no such form, and everything else, goes as it is.
    #[test]
    fn a_group_reaction_is_chosen_per_receiver() {
        use libthreema::{common::FeatureMask, model::message::LegacyReactionType};
        let reaction = |action| {
            GroupMessageBody::Reaction(ReactionMessage {
                message_id: MessageId(5),
                action,
            })
        };
        let thumb = reaction(ReactionAction::Apply("\u{1F44D}".to_owned()));
        let (old, new) = (
            Some(FeatureMask(0)),
            Some(FeatureMask(FeatureMask::REACTION_SUPPORT)),
        );

        let GroupMessageBody::LegacyReaction(legacy) = body_for_receiver(&thumb, old) else {
            panic!("expected the legacy form");
        };
        assert!(matches!(
            legacy.reaction_type,
            LegacyReactionType::Acknowledge
        ));
        assert_eq!(legacy.message_ids[0].0, 5);
        assert!(matches!(
            body_for_receiver(&thumb, new),
            GroupMessageBody::Reaction(_)
        ));
        // Nobody to look the mask up for: `encode` keeps it for later anyway.
        assert!(matches!(
            body_for_receiver(&thumb, None),
            GroupMessageBody::Reaction(_)
        ));
        // No legacy form: a heart, or taking a thumb back.
        for action in [
            ReactionAction::Apply("\u{2764}\u{fe0f}".to_owned()),
            ReactionAction::Withdraw("\u{1F44D}".to_owned()),
        ] {
            assert!(matches!(
                body_for_receiver(&reaction(action), old),
                GroupMessageBody::Reaction(_)
            ));
        }
        let text = GroupMessageBody::Text(TextMessage { text: "hi".into() });
        assert!(matches!(
            body_for_receiver(&text, old),
            GroupMessageBody::Text(_)
        ));
    }

    /// The mark that someone may have a message is on record before the first byte goes out, and
    /// not for what stays home: a file still uploading, a contact no longer to be written to.
    #[tokio::test(flavor = "current_thread")]
    async fn a_send_is_on_record_as_begun_before_anything_goes_out() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let store = Rc::clone(&setup.store);
        let group_key = "g:USER0001:0000000000000001";
        store
            .upsert_group(&crate::groups::Group {
                key: group_key.to_owned(),
                identity: crate::groups::parse_key(group_key).unwrap(),
                name: "Crew".into(),
                members: vec![CREATOR.to_owned()],
                left: false,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        let text = store.insert_outgoing(CREATOR, 1, "text", "hi").unwrap();
        let in_group = store.insert_outgoing(group_key, 2, "text", "hi").unwrap();
        store
            .set_pending_receivers(group_key, 2, &[CREATOR.to_owned()])
            .unwrap();
        let file = store
            .insert_outgoing(CREATOR, 3, "file", r#"{"uploadState": 1}"#)
            .unwrap();
        let written = |id: i64| store.message(id).unwrap().unwrap().written_out;
        assert!(!written(text.id) && !written(in_group.id) && !written(file.id));

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut session = session(&setup, &tx).await;
        // No handshake was made in this test, so the first payload ends the pass.
        let _ = session.send_pending().await;
        assert!(written(text.id));
        assert!(!written(file.id));
        // The chat hears of it, once: Edit depends on it.
        let mut told = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let FromConnection::Event(event) = event
                && let CoreEvent::MessagesChanged(changed) = *event
            {
                told.extend(changed.iter().filter(|m| m.written_out).map(|m| m.id));
            }
        }
        assert_eq!(told, [text.id]);
        assert!(store.set_written_out(text.id).unwrap().is_none());

        // The group's turn, once the text is out of the way.
        store.remove_message(text.id).unwrap();
        let _ = session.send_pending().await;
        assert!(written(in_group.id));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_new_session_starts_no_chat_but_shows_in_one() {
        let (_dir, app) = app();
        let (setup, _peer) = contact_without_fs_support(&app);
        let mut contact = setup.store.contact(CREATOR).unwrap().unwrap();
        contact.feature_mask = libthreema::common::FeatureMask(
            libthreema::common::FeatureMask::FORWARD_SECURITY_SUPPORT,
        );
        setup.store.upsert_contact(&contact).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut session = session(&setup, &tx).await;
        let established = || CoreEvent::FsStatus {
            peer: CREATOR.into(),
            status: FsStatus::Established,
        };
        assert!(
            session
                .handle_fs_event(established())
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(setup.store.message_count(CREATOR).unwrap(), 0);

        setup
            .store
            .insert_incoming(CREATOR, 1, "text", "hi", 5)
            .unwrap();
        assert!(
            session
                .handle_fs_event(established())
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(setup.store.message_count(CREATOR).unwrap(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_session_against_a_stored_no_ends_only_on_the_directorys_word() {
        let (_dir, app) = app();
        let (setup, peer) = contact_without_fs_support(&app);
        let (tx, _rx) = mpsc::unbounded_channel();
        let session = session(&setup, &tx).await;
        let peers = || setup.store.fs_peers().unwrap().len();

        // The lookup failed: the session stays, the message goes out plain.
        setup.fs_mask_checks.record(CREATOR, Instant::now(), false);
        let sent = session
            .encode(CREATOR, 1, 1, text())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(peers(), 1);
        assert!(session.fs_store().get_best(peer).unwrap().is_some());

        // It answered, and the mask stands: a Terminate first, and the chat says so.
        setup.fs_mask_checks.record(CREATOR, Instant::now(), true);
        let sent = session
            .encode(CREATOR, 2, 1, text())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(peers(), 0);
        let newest: String = setup
            .store
            .conn
            .query_row(
                "SELECT body FROM messages WHERE conversation = ?1 ORDER BY id DESC LIMIT 1",
                [CREATOR],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(newest, "fsUnavailable");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_init_does_not_go_by_the_no_from_a_moment_ago() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut session = session(&setup, &tx).await;

        // The peer's linking got us a "no", was rolled back, and here is its Init.
        setup.fs_mask_checks.record(CREATOR, Instant::now(), true);
        let event = CoreEvent::FsStatus {
            peer: CREATOR.into(),
            status: FsStatus::Established,
        };
        assert!(session.handle_fs_event(event).await.unwrap().is_none());
        assert_eq!(setup.store.fs_peers().unwrap().len(), 1);

        // Nor does sending, until a lookup has said.
        let sent = session
            .encode(CREATOR, 1, 1, text())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(setup.store.fs_peers().unwrap().len(), 1);
    }

    /// A session after its handshake, on a plain socket: every mediator step fails on it, so a
    /// reflection that is attempted shows as an error.
    async fn session<'a>(
        setup: &'a ConnectionSetup,
        tx: &'a mpsc::UnboundedSender<FromConnection>,
    ) -> Session<'a> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let events: EventSink = Rc::new(RefCell::new(Vec::new()));
        let store = &setup.store;
        let (csp, _) = CspProtocol::new(
            CspProtocolContextInit {
                permanent_server_keys: setup.config.chat_server_public_keys.clone(),
                identity: setup.identity,
                client_key: ClientKey::from(*setup.client_key),
                client_info: setup.client_info.clone(),
                device_cookie: Some(setup.device_cookie),
                csp_device_id: None,
            }
            .try_into()
            .unwrap(),
        );
        let mut e2e = CspE2eProtocol::new(CspE2eProtocolContextInit {
            client_info: setup.client_info.clone(),
            config: Arc::clone(&setup.config),
            csp_e2e: CspE2eContextInit {
                user_identity: setup.identity,
                client_key: ClientKey::from(*setup.client_key),
                flavor: Flavor::Consumer,
                nonce_storage: Box::new(RefCell::new(StoreNonces(Rc::clone(store)))),
                fs_sessions: None,
                fs_answering_only: false,
            },
            d2x: store
                .multi_device()
                .unwrap()
                .map(|properties| D2xContextInit {
                    device_id: D2xDeviceId(properties.mediator_device_id),
                    device_group_key: properties.device_group_key().unwrap(),
                    nonce_storage: Box::new(RefCell::new(crate::multidevice::StoreD2dNonces(
                        Rc::clone(store),
                    ))),
                }),
            shortcut: Box::new(WebShortcuts {
                events: Rc::clone(&events),
            }),
            settings: Box::new(RefCell::new(StoreSettings(Rc::clone(store)))),
            contacts: Box::new(RefCell::new(StoreContacts {
                store: Rc::clone(store),
                user_identity: setup.identity,
                events: Rc::clone(&events),
            })),
            conversations: Box::new(RefCell::new(StoreConversations {
                store: Rc::clone(store),
                events: Rc::clone(&events),
                user: setup.identity,
                from_sync: false,
            })),
        });
        if store.multi_device().unwrap().is_some() {
            e2e.update_d2m_state(D2mRole::Leader).unwrap();
        }
        Session {
            setup,
            transport: Transport::Direct(stream),
            liveness: Liveness::new(Instant::now()),
            d2m_queue: std::collections::VecDeque::new(),
            multi_device: store.multi_device().unwrap(),
            fs_disabled: false,
            fs_resent: std::collections::HashSet::new(),
            held: Vec::new(),
            reflection_backlog: false,
            next_reflect_id: 0x8000_0000,
            reflected: std::collections::HashSet::new(),
            csp,
            e2e,
            events,
            tx,
            awaiting_ack: Vec::new(),
        }
    }

    /// A session whose echo is out, with the server's end of its socket.
    async fn session_awaiting_echo<'a>(
        setup: &'a ConnectionSetup,
        tx: &'a mpsc::UnboundedSender<FromConnection>,
    ) -> (Session<'a>, TcpStream) {
        let mut session = session(setup, tx).await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (server, _) = listener.accept().await.unwrap();
        session.transport = Transport::Direct(stream);
        session.liveness.echo_sent(Instant::now(), ECHO_INTERVAL);
        (session, server)
    }

    #[test]
    fn another_connection_is_no_news_and_any_other_close_is() {
        // Android's check: `contains`, whatever comes around it.
        assert_eq!(close_reason("Another connection for this ID"), None);
        assert_eq!(
            close_reason("Your ID was revoked"),
            Some(OfflineReason::Server("Your ID was revoked".into()))
        );
    }

    /// Like Android: three times back at once, as it mostly is a change of network. Then a fight
    /// over the ID, and ten minutes between logins for as long as it goes on.
    #[test]
    fn another_connection_is_ignored_a_few_times_and_not_in_a_fight() {
        const SHORT: Duration = Duration::from_secs(30);
        let mut another = AnotherConnection::default();
        for _ in 0..ANOTHER_CONNECTION_IGNORED {
            assert!(!another.stay_away(SHORT));
        }
        assert!(another.stay_away(SHORT));
        assert!(another.stay_away(SHORT));
        // Back after a hold, and kicked out at once: still the fight.
        assert!(another.stay_away(Duration::from_secs(1)));
        // A long session in between: this one is news again.
        assert!(!another.stay_away(ANOTHER_CONNECTION_FORGOTTEN + SHORT));
    }

    /// The markup goes, and nothing that merely looks like it.
    #[test]
    fn the_servers_words_are_shown_without_their_markup() {
        for (text, plain) in [
            ("plain words", "plain words"),
            ("<b>Update</b> now", "Update now"),
            ("one<br>two<br/>three", "one\ntwo\nthree"),
            ("<p>one</p><p>two</p>", "one\ntwo"),
            ("<a href=\"https://threema.ch\">Threema</a>", "Threema"),
            ("<img src=\"file:///etc/passwd\">words", "words"),
            ("a < b and b > a", "a < b and b > a"),
            ("3 <4", "3 <4"),
            ("cut off <b", "cut off <b"),
            ("Tom &amp; Jerry &lt;b&gt;", "Tom & Jerry <b>"),
            ("&amp;lt;", "&lt;"),
            ("Grüße <i>aus</i> Köln", "Grüße aus Köln"),
        ] {
            assert_eq!(plain_server_text(text), plain, "{text}");
        }
        assert_eq!(server_reason("<br>"), None);
        assert_eq!(server_reason("  "), None);
    }

    #[test]
    fn the_ui_gets_a_reason_in_words_or_by_its_code() {
        assert_eq!(
            OfflineReason::Server("maintenance".into()).to_json(),
            serde_json::json!({ "kind": "server", "text": "maintenance" })
        );
        assert_eq!(
            OfflineReason::Mediator(4111).to_json(),
            serde_json::json!({ "kind": "mediator", "code": 4111 })
        );
        assert_eq!(
            OfflineReason::LoginRefused.to_json(),
            serde_json::json!({ "kind": "loginRefused" })
        );
    }

    /// Only an end of the connection after our login counts as a no: before it the server hasn't
    /// been asked, and a timeout says nothing.
    #[test]
    fn a_refused_login_is_the_connection_closing_after_the_login() {
        let refused = |login_sent, error: TransportError| {
            matches!(
                handshake_read_failed(login_sent, error.into()),
                ConnectionError::LoginRefused
            )
        };
        assert!(refused(true, TransportError::Closed));
        assert!(refused(
            true,
            TransportError::Io(std::io::ErrorKind::ConnectionReset.into())
        ));
        assert!(!refused(false, TransportError::Closed));
        assert!(!refused(
            true,
            TransportError::Io(std::io::ErrorKind::TimedOut.into())
        ));
        assert!(!refused(true, TransportError::WriteTimeout));
        // Through the mediator: its close code for the chat server's connection having ended.
        let mediator_closed = |code| {
            TransportError::WebSocket(WebSocketError::Closed {
                code: Some(code),
                reason: String::new(),
            })
        };
        assert!(refused(true, mediator_closed(MEDIATOR_CHAT_SERVER_CLOSED)));
        assert!(!refused(
            false,
            mediator_closed(MEDIATOR_CHAT_SERVER_CLOSED)
        ));
        // Could not be established, or the mediator's own affair.
        assert!(!refused(true, mediator_closed(4001)));
        assert!(!refused(true, mediator_closed(4111)));
    }

    /// A short hiccup at the server takes a few refusals in seconds: no alarm, however many.
    /// Minutes of it, with nothing else going wrong, and it tells once.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_refused_login_is_told_only_after_minutes_of_them() {
        let mut logins = RefusedLogins::default();
        assert_eq!(logins.refused(), (true, false));
        for _ in 0..10 {
            tokio::time::advance(Duration::from_secs(10)).await;
            assert_eq!(logins.refused(), (false, false));
        }
        tokio::time::advance(REFUSED_LOGINS_SPAN).await;
        assert_eq!(logins.refused(), (false, true));
        assert_eq!(logins.refused(), (false, false));

        // Long enough but too few: a device asleep in between, say.
        let mut logins = RefusedLogins::default();
        logins.refused();
        tokio::time::advance(REFUSED_LOGINS_SPAN).await;
        assert_eq!(logins.refused(), (false, false));
        assert_eq!(logins.refused(), (false, true));

        // Something else went wrong in between, or a login worked: it starts over.
        let mut logins = RefusedLogins::default();
        logins.refused();
        tokio::time::advance(REFUSED_LOGINS_SPAN).await;
        logins.refused();
        logins.over();
        assert_eq!(logins.refused(), (true, false));
        assert_eq!(logins.refused(), (false, false));
        assert_eq!(logins.refused(), (false, false));
    }

    /// The device stays up for as long as it takes, and no longer.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_wake_up_is_over_soon_after_the_answer() {
        use tokio::io::AsyncWriteExt as _;
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let (tx, mut from) = mpsc::unbounded_channel();
        let (mut session, mut server) = session_awaiting_echo(&setup, &tx).await;
        let (to, mut to_rx) = mpsc::unbounded_channel();
        to.send(ToConnection::Wake).unwrap();
        let start = Instant::now();
        let answer_after = Duration::from_secs(1);
        let answered_and_quiet = async {
            sleep(answer_after).await;
            server.write_all(&[0]).await.unwrap();
            while !matches!(from.recv().await, Some(FromConnection::WakeSettled)) {}
        };
        tokio::select! {
            ended = session.payload_loop(&mut to_rx) => panic!("ended: {:?}", ended.err()),
            () = answered_and_quiet => {}
        }
        assert_eq!(start.elapsed(), answer_after + WAKE_QUIET);
    }

    /// The server's own word on its queue, wherever it stands among the session's steps.
    #[tokio::test(flavor = "current_thread")]
    async fn a_wake_up_after_a_login_waits_for_the_queues_end() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let (tx, _from) = mpsc::unbounded_channel();
        let mut session = session(&setup, &tx).await;
        setup.wake_hold.asked();
        setup.wake_hold.logged_in(false);
        assert_eq!(setup.wake_hold.settles_at(), None);
        session
            .handle_instruction(CspProtocolInstruction {
                state_update: None,
                outgoing_frame: None,
                incoming_payload: Some(IncomingPayload::QueueSendComplete),
            })
            .await
            .unwrap();
        assert!(setup.wake_hold.settles_at().is_some());
    }

    /// An echo sent before the device slept has most of a minute left; the wake-up is up for less.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_wake_up_finds_a_dead_connection_within_its_time() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let (tx, mut from) = mpsc::unbounded_channel();
        let (mut session, _server) = session_awaiting_echo(&setup, &tx).await;
        let (to, mut to_rx) = mpsc::unbounded_channel();
        to.send(ToConnection::Wake).unwrap();
        let start = Instant::now();
        let ended = session.payload_loop(&mut to_rx).await;
        assert!(matches!(ended, Err(ConnectionError::Timeout)));
        assert_eq!(start.elapsed(), WAKE_ECHO_LIMIT + ECHO_LAST_LOOK);
        assert!(
            !std::iter::from_fn(|| from.try_recv().ok())
                .any(|event| matches!(event, FromConnection::WakeSettled)),
            "it stays for the new connection"
        );
    }

    /// A message `CREATOR` sends us, as the chat server hands it over (the same each time, for the
    /// same id); `CREATOR` is a contact.
    fn from_creator(setup: &ConnectionSetup, id: u64, text: &str) -> MessageWithMetadataBox {
        let key = ClientKey::from([5; 32]);
        let mut contact = crate::contacts::test_contact(CREATOR);
        contact.public_key = key.public_key();
        contact.acquaintance_level =
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::Direct;
        setup.store.upsert_contact(&contact).unwrap();
        encode_outgoing_contact_message(
            CREATOR.parse().unwrap(),
            &key,
            Delta::Unchanged,
            setup.identity,
            &ClientKey::from(*setup.client_key).public_key(),
            &OutgoingMessage {
                id: MessageId(id),
                overrides: MessageOverrides::default(),
                created_at: 1,
                body: OutgoingMessageBody::Contact(OutgoingContactMessageBody {
                    receiver_identity: setup.identity,
                    body: ContactMessageBody::Text(TextMessage { text: text.into() }),
                }),
            },
            Nonce([u8::try_from(id).unwrap(); 24]),
        )
        .unwrap()
    }

    /// The events the UI has got so far.
    fn sent_to_ui(rx: &mut mpsc::UnboundedReceiver<FromConnection>) -> Vec<CoreEvent> {
        let mut events = Vec::new();
        while let Ok(message) = rx.try_recv() {
            if let FromConnection::Event(event) = message {
                events.push(*event);
            }
        }
        events
    }

    fn storage_notices(events: &[CoreEvent]) -> Vec<bool> {
        events
            .iter()
            .filter_map(|event| match event {
                CoreEvent::StorageProblem { active } => Some(*active),
                _ => None,
            })
            .collect()
    }

    /// A store that fails on a message ends the connection as what it is, not as a network
    /// problem, without acknowledging: the message comes again. The user hears of it once, and
    /// once more when a message could be stored.
    #[tokio::test(flavor = "current_thread")]
    async fn a_failing_store_is_named_and_told_once_per_streak() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let message = from_creator(&setup, 1, "hi");
        let mut session = session(&setup, &tx).await;
        setup
            .store
            .conn
            .execute_batch(
                "CREATE TRIGGER refuse BEFORE INSERT ON messages
                 BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
            )
            .unwrap();

        let Err(ConnectionError::Storage { code, step }) = session.handle_incoming(message).await
        else {
            panic!("expected a storage failure");
        };
        assert_ne!(code, 0, "SQLite's code");
        assert_eq!(step, StorageStep::Incoming);
        note_storage_failure(&setup, &tx, code, step);
        note_storage_failure(&setup, &tx, code, step);
        assert_eq!(storage_notices(&sent_to_ui(&mut rx)), [true]);

        setup
            .store
            .conn
            .execute_batch("DROP TRIGGER refuse")
            .unwrap();
        // It's stored this time: what fails now, on this socket without a handshake, is the ack.
        let again = from_creator(&setup, 1, "hi");
        let error = session.handle_incoming(again).await.unwrap_err();
        assert!(matches!(error, ConnectionError::Protocol(_)), "{error}");
        assert!(setup.store.has_incoming(CREATOR, 1).unwrap());
        assert_eq!(storage_notices(&sent_to_ui(&mut rx)), [false]);
    }

    /// A multi-device login runs through the mediator's queue before it gets to the chat server. If
    /// that is where the store fails, the end of that queue says nothing about it: without this the
    /// notice would go and come back at every attempt.
    #[tokio::test(flavor = "current_thread")]
    async fn only_what_failed_ends_a_storage_notice() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        setup
            .store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut session = session(&setup, &tx).await;
        let queue_dry = D2mIncoming::ReflectionQueueDry;

        note_storage_failure(&setup, &tx, 13, StorageStep::Incoming);
        assert_eq!(storage_notices(&sent_to_ui(&mut rx)), [true]);
        session.handle_d2m(queue_dry).await.unwrap();
        session.storage_works(StorageStep::Other);
        session.storage_works(StorageStep::Reflected);
        assert!(storage_notices(&sent_to_ui(&mut rx)).is_empty());

        // Another step failing too is the same streak, and it lasts until both work.
        note_storage_failure(&setup, &tx, 13, StorageStep::Reflected);
        assert!(storage_notices(&sent_to_ui(&mut rx)).is_empty());
        session
            .handle_d2m(D2mIncoming::ReflectionQueueDry)
            .await
            .unwrap();
        assert!(storage_notices(&sent_to_ui(&mut rx)).is_empty());
        session
            .handle_instruction(CspProtocolInstruction {
                state_update: None,
                outgoing_frame: None,
                incoming_payload: Some(IncomingPayload::QueueSendComplete),
            })
            .await
            .unwrap();
        assert_eq!(storage_notices(&sent_to_ui(&mut rx)), [false]);

        // On the mediator's side, the end of its queue is the sign.
        note_storage_failure(&setup, &tx, 13, StorageStep::Reflected);
        assert_eq!(storage_notices(&sent_to_ui(&mut rx)), [true]);
        session
            .handle_d2m(D2mIncoming::ReflectionQueueDry)
            .await
            .unwrap();
        assert_eq!(storage_notices(&sent_to_ui(&mut rx)), [false]);
    }

    /// Another device read `CREATOR`'s message 1.
    fn read_update() -> libthreema::protobuf::d2d::Envelope {
        use libthreema::protobuf::d2d::{
            Envelope, IncomingMessageUpdate, envelope::Content, incoming_message_update,
        };
        Envelope {
            content: Some(Content::IncomingMessageUpdate(IncomingMessageUpdate {
                updates: vec![incoming_message_update::Update {
                    conversation: d2d_conversation(CREATOR),
                    message_id: 1,
                    update: Some(incoming_message_update::update::Update::Read(
                        incoming_message_update::Read { at: 1 },
                    )),
                }],
            })),
            ..Default::default()
        }
    }

    /// What fails the same way every time mustn't hold up the queue for good: after a few tries the
    /// message is acknowledged (on this socket without a handshake that attempt is a protocol
    /// error). The state of the device is no such reason, however long it lasts.
    #[tokio::test(flavor = "current_thread")]
    async fn a_message_that_fails_every_time_is_given_up_on() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut session = session(&setup, &tx).await;
        let store = &setup.store;
        let is_storage = |error: &ConnectionError| matches!(error, ConnectionError::Storage { .. });

        // A full disk, a read-only file: never.
        let copies: Vec<_> = (0..2 * TRIES_BEFORE_DROPPING)
            .map(|_| from_creator(&setup, 1, "hi"))
            .collect();
        store.conn.execute_batch("PRAGMA query_only = ON").unwrap();
        for copy in copies {
            let error = session.handle_incoming(copy).await.unwrap_err();
            assert!(is_storage(&error), "{error}");
        }
        store.conn.execute_batch("PRAGMA query_only = OFF").unwrap();

        // A rule of ours that the message breaks, every time.
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER refuse BEFORE INSERT ON messages
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        for _ in 1..TRIES_BEFORE_DROPPING {
            let copy = from_creator(&setup, 1, "hi");
            let error = session.handle_incoming(copy).await.unwrap_err();
            assert!(is_storage(&error), "{error}");
        }
        let copy = from_creator(&setup, 1, "hi");
        let error = session.handle_incoming(copy).await.unwrap_err();
        assert!(matches!(error, ConnectionError::Protocol(_)), "{error}");

        // Another message starts over.
        let other = from_creator(&setup, 2, "hi");
        let error = session.handle_incoming(other).await.unwrap_err();
        assert!(is_storage(&error), "{error}");
    }

    #[test]
    fn what_libthreema_reports_says_whose_doing_it_is() {
        for own in [
            CspE2eProtocolError::InvalidState("x"),
            CspE2eProtocolError::DesyncError("x".into()),
        ] {
            let error = e2e_error(own);
            assert!(
                matches!(error, ConnectionError::IncomingMessage(_)),
                "{error}"
            );
            assert!(error.fails_every_time());
        }
        for network in [
            CspE2eProtocolError::NetworkError("x".into()),
            CspE2eProtocolError::ServerError("x".into()),
            CspE2eProtocolError::InvalidCredentials,
            CspE2eProtocolError::RateLimitExceeded,
        ] {
            assert!(!e2e_error(network).fails_every_time());
        }
        let full = e2e_error(CspE2eProtocolError::DesyncError("storage[13]: x".into()));
        assert!(matches!(full, ConnectionError::Storage { code: 13, .. }));
        assert!(!full.fails_every_time());
        let rule = e2e_error(CspE2eProtocolError::DesyncError("storage[19]: x".into()));
        assert!(rule.fails_every_time());
    }

    /// A reflected envelope too, or the mediator's queue never ends, and with it no promotion to
    /// leader and nothing from the chat server. Afterwards it counts as seen.
    #[tokio::test(flavor = "current_thread")]
    async fn a_reflection_that_fails_every_time_is_given_up_on() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let properties = crate::multidevice::Properties::generate();
        let key = properties.device_group_key().unwrap();
        setup.store.set_multi_device(Some(&properties)).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut session = session(&setup, &tx).await;
        setup
            .store
            .insert_incoming(CREATOR, 1, "text", "x", 1)
            .unwrap();
        setup
            .store
            .conn
            .execute_batch(
                "CREATE TRIGGER refuse BEFORE UPDATE ON messages
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        let (nonce, data) = key
            .encrypt_envelope(&prost::Message::encode_to_vec(&read_update()))
            .unwrap();
        let reflected = || libthreema::d2m::payload::Reflected {
            reflect_id: 1,
            flags: D2mReflectFlags(0),
            timestamp: 1,
            envelope: data.clone(),
        };
        for _ in 1..TRIES_BEFORE_DROPPING {
            let error = session.handle_reflected(reflected()).await.unwrap_err();
            assert!(matches!(error, ConnectionError::Storage { .. }), "{error}");
            assert!(!setup.store.has_d2d_nonce(&nonce).unwrap());
        }
        // On this socket the acknowledgement fails, as an error of its own.
        let error = session.handle_reflected(reflected()).await.unwrap_err();
        assert!(!matches!(error, ConnectionError::Storage { .. }), "{error}");
        assert!(setup.store.has_d2d_nonce(&nonce).unwrap());
    }

    /// The mediator hands out again only what wasn't acknowledged. On this socket the attempt to
    /// acknowledge shows as an error of its own.
    #[tokio::test(flavor = "current_thread")]
    async fn a_reflection_the_store_failed_on_is_not_acknowledged() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let properties = crate::multidevice::Properties::generate();
        let key = properties.device_group_key().unwrap();
        setup.store.set_multi_device(Some(&properties)).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut session = session(&setup, &tx).await;
        let envelope = read_update();
        let (_, data) = key
            .encrypt_envelope(&prost::Message::encode_to_vec(&envelope))
            .unwrap();
        let reflected = |data: &[u8]| libthreema::d2m::payload::Reflected {
            reflect_id: 1,
            flags: D2mReflectFlags(0),
            timestamp: 1,
            envelope: data.to_vec(),
        };

        setup
            .store
            .conn
            .execute_batch("PRAGMA query_only = ON")
            .unwrap();
        let error = session
            .handle_reflected(reflected(&data))
            .await
            .unwrap_err();
        assert!(matches!(error, ConnectionError::Storage { .. }), "{error}");

        setup
            .store
            .conn
            .execute_batch("PRAGMA query_only = OFF")
            .unwrap();
        let error = session
            .handle_reflected(reflected(&data))
            .await
            .unwrap_err();
        assert!(!matches!(error, ConnectionError::Storage { .. }), "{error}");
    }

    /// The connection dies right after a message was stored, on the acknowledgement here: the UI
    /// still hears of it and the receipt waits for the next connection. The server's next copy
    /// is known by then, and is only acknowledged.
    #[tokio::test(flavor = "current_thread")]
    async fn a_stored_message_reaches_the_ui_whatever_the_connection_does() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let message = from_creator(&setup, 1, "hi");
        let mut session = session(&setup, &tx).await;

        assert!(session.handle_incoming(message).await.is_err());
        let added = |events: &[CoreEvent]| {
            events
                .iter()
                .filter(|event| matches!(event, CoreEvent::MessageAdded { .. }))
                .count()
        };
        assert_eq!(added(&sent_to_ui(&mut rx)), 1);
        let queued = |store: &crate::store::Store| {
            store
                .outbox()
                .unwrap()
                .into_iter()
                .filter(|entry| entry.kind == reactions::OUTBOX_DELIVERY_RECEIPT)
                .count()
        };
        assert_eq!(queued(&setup.store), 1);

        let again = from_creator(&setup, 1, "hi");
        assert!(session.handle_incoming(again).await.is_err());
        assert_eq!(added(&sent_to_ui(&mut rx)), 0);
        assert_eq!(queued(&setup.store), 1);
    }

    /// The task itself can end the connection after the store write too: its events are out when
    /// the session is.
    #[tokio::test(flavor = "current_thread")]
    async fn a_session_that_ends_hands_on_its_events() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let status = || CoreEvent::FsStatus {
            peer: CREATOR.into(),
            status: FsStatus::Reset,
        };
        {
            let session = session(&setup, &tx).await;
            let mut queue = session.events.borrow_mut();
            queue.push(CoreEvent::ContactsChanged);
            queue.push(status());
            queue.push(CoreEvent::OutboxChanged);
            queue.push(CoreEvent::UnknownMembers(vec!["B0B0B0B0".into()]));
        }
        let events = sent_to_ui(&mut rx);
        assert!(matches!(
            events.as_slice(),
            [CoreEvent::ContactsChanged, CoreEvent::UnknownMembers(_)]
        ));
        // The store holds what the outbox event is for.
        assert_eq!(setup.carried_events.borrow().len(), 1);

        // The next connection does what the last one couldn't: here, a line in the chat.
        let mut session = session(&setup, &tx).await;
        session.resume_carried_events().await.unwrap();
        assert!(setup.carried_events.borrow().is_empty());
        assert!(matches!(
            sent_to_ui(&mut rx).as_slice(),
            [CoreEvent::MessageAdded { message }] if message.body == "fsReset"
        ));
    }

    /// A message the peer rejected goes out again and a Reject of ours goes to the peer, whatever
    /// happens to the app in between: they are in the store from when the connection ends.
    #[tokio::test(flavor = "current_thread")]
    async fn what_a_connection_left_to_do_survives_a_restart() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let (tx, _rx) = mpsc::unbounded_channel();
        let sent = setup
            .store
            .insert_outgoing(CREATOR, 7, "text", "hi")
            .unwrap();
        setup
            .store
            .raise_outgoing_state(CREATOR, &[7], OutgoingState::Sent)
            .unwrap();
        {
            let session = session(&setup, &tx).await;
            session.events.borrow_mut().push(CoreEvent::FsStatus {
                peer: CREATOR.into(),
                status: FsStatus::MessageRejected {
                    message_id: 7,
                    group_identity: None,
                    session_known: true,
                },
            });
        }

        // A new process: nothing in memory.
        let (setup, _) = contact_without_fs_support(&app);
        setup.fs_mask_checks.record(CREATOR, Instant::now(), true);
        assert!(setup.carried_events.borrow().is_empty());
        let mut session = session(&setup, &tx).await;
        // Sending fails on this socket without a handshake; it's queued by then.
        let _ = session.resume_carried_events().await;
        let state = setup.store.message(sent.id).unwrap().unwrap().state;
        assert_eq!(state, OutgoingState::Pending as i64);
    }

    /// All there is to know of a control message or a status comes back, and what isn't either
    /// isn't stored. Only an envelope and IDs are: this is what lies in the store.
    #[test]
    fn carried_events_keep_what_they_say() {
        let group = GroupIdentity {
            creator_identity: CREATOR.parse().unwrap(),
            group_id: 7,
        };
        let key = ClientKey::from([5; 32]).public_key();
        let events = [
            CoreEvent::FsControl {
                peer: CREATOR.into(),
                peer_key: Some(key),
                envelope: vec![1, 2, 3],
            },
            CoreEvent::FsControl {
                peer: CREATOR.into(),
                peer_key: None,
                envelope: vec![4],
            },
            CoreEvent::FsStatus {
                peer: CREATOR.into(),
                status: FsStatus::MessageRejected {
                    message_id: u64::MAX,
                    group_identity: Some(group),
                    session_known: false,
                },
            },
            CoreEvent::FsStatus {
                peer: CREATOR.into(),
                status: FsStatus::MessageRejected {
                    message_id: 1,
                    group_identity: None,
                    session_known: true,
                },
            },
            CoreEvent::FsStatus {
                peer: CREATOR.into(),
                status: FsStatus::Terminated {
                    session_known: false,
                },
            },
            CoreEvent::FsStatus {
                peer: CREATOR.into(),
                status: FsStatus::Established,
            },
            CoreEvent::FsStatus {
                peer: CREATOR.into(),
                status: FsStatus::Reset,
            },
            CoreEvent::FsStatus {
                peer: CREATOR.into(),
                status: FsStatus::MessageWithoutFs,
            },
        ];
        for event in &events {
            let stored = carried_to_json(event).unwrap();
            let back = carried_from_json(&stored).unwrap();
            assert_eq!(format!("{back:?}"), format!("{event:?}"));
        }
        assert!(carried_to_json(&CoreEvent::ContactsChanged).is_none());
        assert!(carried_from_json(&serde_json::json!({"peer": CREATOR})).is_none());
        assert!(carried_from_json(&serde_json::json!({"status": "reset"})).is_none());
    }

    /// What is still to come when a step fails isn't lost with it. A step that fails for another
    /// reason than the connection isn't tried again: it would end every connection.
    #[tokio::test(flavor = "current_thread")]
    async fn a_failing_step_leaves_the_rest_for_the_next_connection() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        let (tx, _rx) = mpsc::unbounded_channel();
        let control = || CoreEvent::FsControl {
            peer: CREATOR.into(),
            peer_key: None,
            envelope: vec![1, 2, 3],
        };
        {
            let mut session = session(&setup, &tx).await;
            session.events.borrow_mut().push(control());
            session.events.borrow_mut().push(CoreEvent::FsStatus {
                peer: CREATOR.into(),
                status: FsStatus::Reset,
            });
            // On this socket without a handshake, that is a protocol error.
            let error = session.after_incoming().await.unwrap_err();
            assert!(matches!(error, ConnectionError::Protocol(_)), "{error}");
        }
        assert!(matches!(
            setup.carried_events.borrow().as_slice(),
            [CoreEvent::FsStatus { .. }]
        ));
        let lost = ConnectionError::Io(std::io::ErrorKind::ConnectionReset.into());
        assert!(lost.is_connection_lost());
        let storage = ConnectionError::Storage {
            code: 13,
            step: StorageStep::Other,
        };
        assert!(!storage.is_connection_lost());
    }

    /// With other devices, the connection can die while waiting for the mediator to confirm a
    /// stored message. The server's next copy passes and is reflected, without being stored or
    /// announced again. Once the task is through it counts as known.
    #[tokio::test(flavor = "current_thread")]
    async fn a_message_whose_reflection_failed_is_reflected_again() {
        let (_dir, app) = app();
        let (setup, _) = contact_without_fs_support(&app);
        setup
            .store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let message = from_creator(&setup, 1, "hi");
        let mut first = session(&setup, &tx).await;
        let added = |events: &[CoreEvent]| {
            events
                .iter()
                .filter(|event| matches!(event, CoreEvent::MessageAdded { .. }))
                .count()
        };

        // On this socket every mediator step fails.
        let reflection_failed = |error: ConnectionError| {
            assert!(error.to_string().contains("mediator"), "{error}");
        };
        reflection_failed(first.handle_incoming(message).await.unwrap_err());
        assert!(setup.store.has_incoming(CREATOR, 1).unwrap());
        assert!(setup.store.awaits_reflection(CREATOR, 1).unwrap());
        drop(first);
        assert_eq!(added(&sent_to_ui(&mut rx)), 1);

        let mut second = session(&setup, &tx).await;
        let again = from_creator(&setup, 1, "hi");
        reflection_failed(second.handle_incoming(again).await.unwrap_err());
        drop(second);
        assert_eq!(added(&sent_to_ui(&mut rx)), 0);
        assert_eq!(
            setup
                .store
                .conversation_messages(CREATOR, 10)
                .unwrap()
                .len(),
            1
        );

        // A message the task ends without reflecting (blocked by now) needs no more of it.
        setup
            .store
            .set_setting(
                crate::store::keys::APP_SETTINGS,
                &serde_json::json!({ "blockedContacts": [CREATOR] }).to_string(),
            )
            .unwrap();
        let mut third = session(&setup, &tx).await;
        let again = from_creator(&setup, 1, "hi");
        let error = third.handle_incoming(again).await.unwrap_err();
        assert!(!error.to_string().contains("mediator"), "{error}");
        assert!(!setup.store.awaits_reflection(CREATOR, 1).unwrap());
    }

    #[test]
    fn only_farewells_and_sync_requests_go_to_non_members() {
        use libthreema::model::message::{GroupLeaveMessage, GroupSyncRequestMessage};
        let setup = |members| GroupMessageBody::Setup(GroupSetupMessage { members });
        assert!(goes_to_non_members(&GroupMessageBody::Leave(
            GroupLeaveMessage
        )));
        assert!(goes_to_non_members(&GroupMessageBody::SyncRequest(
            GroupSyncRequestMessage
        )));
        assert!(goes_to_non_members(&setup(vec![])));
        assert!(!goes_to_non_members(&setup(vec![CREATOR.parse().unwrap()])));
        assert!(!goes_to_non_members(&GroupMessageBody::Text(TextMessage {
            text: "hi".into()
        })));
    }

    #[test]
    fn a_queued_delivery_receipt_goes_out_as_a_received_receipt() {
        let body = outbox_body(&reactions::OutboxEntry {
            id: 1,
            conversation: CREATOR.into(),
            message_id: 2,
            created_at: 3,
            kind: reactions::OUTBOX_DELIVERY_RECEIPT.into(),
            body: serde_json::json!({"messageId": "00000000000000ff"}),
        });
        let Some(ContactMessageBody::DeliveryReceipt(receipt)) = body else {
            panic!("expected a receipt");
        };
        assert!(matches!(
            receipt.receipt_type,
            DeliveryReceiptType::Received
        ));
        assert_eq!(receipt.message_ids[0].0, 0xff);
    }

    #[test]
    fn a_repeated_call_start_says_when_the_call_started() {
        let entry = |body: serde_json::Value| reactions::OutboxEntry {
            id: 1,
            conversation: "g:ABCD1234:0000000000000001".into(),
            message_id: 1,
            created_at: 500,
            kind: crate::groups::OUTBOX_GROUP_CALL_START.into(),
            body,
        };
        assert_eq!(
            outbox_sent_as_of(&entry(serde_json::json!({"startedAt": 100}))),
            100
        );
        assert_eq!(outbox_sent_as_of(&entry(serde_json::json!({}))), 500);
    }

    /// What `send_pending` puts before `message`.
    fn sent_before(app: &App, message: &Message) -> Vec<String> {
        app.store
            .outbox_until(&message.conversation, message.created_at)
            .unwrap()
            .into_iter()
            .map(|entry| entry.kind)
            .collect()
    }

    /// Two steps of the user: the second one's timestamp is later.
    fn next_step() {
        std::thread::sleep(Duration::from_millis(2));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_group_created_while_disconnected_goes_out_before_its_first_text() {
        let (_dir, mut app) = app();
        app.store
            .upsert_contact(&crate::contacts::test_contact(CREATOR))
            .unwrap();
        let group = app.create_group("Crew", &[CREATOR.to_owned()]).unwrap();
        let key = group["key"].as_str().unwrap().to_owned();
        next_step();
        app.send_text(&key, "first", None).unwrap();
        let pending = app.store.pending_outgoing().unwrap();
        let text = pending.iter().find(|m| m.kind == "text").unwrap();
        assert_eq!(
            sent_before(&app, text),
            vec![
                crate::groups::OUTBOX_GROUP_SETUP,
                crate::groups::OUTBOX_GROUP_NAME,
                crate::groups::OUTBOX_GROUP_DELETE_PICTURE,
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_text_written_before_leaving_goes_out_before_the_leave() {
        let (_dir, mut app) = app();
        app.store
            .upsert_contact(&crate::contacts::test_contact(CREATOR))
            .unwrap();
        let identity = libthreema::common::GroupIdentity {
            creator_identity: CREATOR.parse().unwrap(),
            group_id: 7,
        };
        let key = crate::groups::key(&identity);
        app.store
            .upsert_group(&crate::groups::Group {
                key: key.clone(),
                identity,
                name: "Crew".into(),
                members: vec![CREATOR.into()],
                left: false,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        app.send_text(&key, "bye", None).unwrap();
        next_step();
        app.leave_group(&key).unwrap();
        let pending = app.store.pending_outgoing().unwrap();
        let text = pending.iter().find(|m| m.kind == "text").unwrap();
        assert!(sent_before(&app, text).is_empty());
        // The leave is still there for the rest of the round.
        assert_eq!(
            app.store.outbox().unwrap()[0].kind,
            crate::groups::OUTBOX_GROUP_LEAVE
        );
    }

    /// There is no mediator here: this shows the reflection is tried within the setup's own step,
    /// not the order a real mediator would see on the wire.
    #[tokio::test(flavor = "current_thread")]
    async fn a_group_change_is_reflected_before_the_next_message_of_a_burst() {
        let (_dir, app) = app();
        app.store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        app.store
            .upsert_contact(&crate::contacts::test_contact(CREATOR))
            .unwrap();
        let identity = app.store.identity().unwrap().unwrap();
        let setup = ConnectionSetup {
            store: Rc::clone(&app.store),
            media: Arc::clone(&app.media),
            https: app.https.clone(),
            config: Arc::clone(&app.config),
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
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut session = session(&setup, &tx).await;
        let mut conversations = StoreConversations {
            store: Rc::clone(&setup.store),
            events: Rc::clone(&session.events),
            user: setup.identity,
            from_sync: false,
        };
        let group = libthreema::common::GroupIdentity {
            creator_identity: CREATOR.parse().unwrap(),
            group_id: 7,
        };
        let message = |id: u64, body: GroupMessageBody| IncomingMessage {
            sender_identity: group.creator_identity,
            id: MessageId(id),
            overrides: MessageOverrides::default(),
            created_at: 1,
            body: IncomingMessageBody::Group(IncomingGroupMessageBody {
                group_identity: group,
                body,
            }),
        };

        // The setup's step doesn't end without trying, and keeps the entry for the next try.
        conversations
            .add_or_update_incoming_message(message(
                1,
                GroupMessageBody::Setup(GroupSetupMessage {
                    members: vec![setup.identity],
                }),
            ))
            .unwrap();
        assert!(setup.store.next_sync().unwrap().is_some());
        assert!(session.after_incoming().await.is_err());
        assert!(setup.store.next_sync().unwrap().is_some());

        // The text that follows wants nothing of the mediator: the queue was the setup's to send.
        while let Some((id, ..)) = setup.store.next_sync().unwrap() {
            setup.store.sync_done(id).unwrap();
        }
        conversations
            .add_or_update_incoming_message(message(
                2,
                GroupMessageBody::Text(TextMessage { text: "hi".into() }),
            ))
            .unwrap();
        assert!(session.after_incoming().await.is_ok());
        let mut added = false;
        while let Ok(event) = rx.try_recv() {
            added |= matches!(
                event,
                FromConnection::Event(event) if matches!(*event, CoreEvent::MessageAdded { .. })
            );
        }
        assert!(added);
    }
}

#[cfg(test)]
mod join_tests {
    use super::*;

    #[test]
    fn a_registered_device_counts_whatever_the_commit_does() {
        assert_eq!(
            JoinOutcome::from_steps(true, Ok(())),
            JoinOutcome::Joined,
            "the commit isn't part of the verdict"
        );
    }

    #[test]
    fn a_failure_says_whether_the_identity_left_this_device() {
        assert_eq!(
            JoinOutcome::from_steps(false, Err("x".into())),
            JoinOutcome::FailedBeforeHandover("x".into())
        );
        assert_eq!(
            JoinOutcome::from_steps(true, Err("x".into())),
            JoinOutcome::FailedAfterHandover("x".into())
        );
    }

    #[test]
    fn the_core_waits_longer_than_the_join_can_take() {
        let steps = TRANSACTION_BEGIN_LIMIT
            + JOIN_TRANSFER_LIMIT
            + JOIN_REGISTER_LIMIT
            + crate::rendezvous::CLOSE_TIMEOUT
            + D2M_REPLY_LIMIT;
        assert!(JOIN_REPLY_LIMIT > steps + MEDIATOR_CONNECT_LIMIT);
    }
}
