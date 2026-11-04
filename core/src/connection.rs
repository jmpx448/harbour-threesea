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
        ChatServerGroup, ClientInfo, CspDeviceId, D2xDeviceId, DeviceCookie, FeatureMask,
        GroupIdentity, MessageId, Nonce, ThreemaId,
        config::{Config, Flavor},
        keys::{ClientKey, PublicKey},
    },
    csp::{
        CspProtocol, CspProtocolContextInit, CspProtocolInstruction, CspStateUpdate,
        payload::{EchoPayload, IncomingPayload, MessageWithMetadataBox, OutgoingPayload},
    },
    csp_e2e::{
        CspE2eContextInit, CspE2eProtocol, CspE2eProtocolContextInit, D2mRole, D2xContextInit,
        ReflectId,
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
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
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
    CONNECT_TIMEOUT.as_secs()
        + TRANSACTION_BEGIN_LIMIT.as_secs()
        + JOIN_TRANSFER_LIMIT.as_secs()
        + JOIN_REGISTER_LIMIT.as_secs()
        + crate::rendezvous::CLOSE_TIMEOUT.as_secs()
        + D2M_REPLY_LIMIT.as_secs()
        + 60,
);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// How long we stay away when a server told us not to come back by ourselves.
const HOLD_OFF: Duration = Duration::from_secs(600);
/// A session this long was a working connection: the next attempts start over.
const STABLE_SESSION: Duration = Duration::from_secs(60);

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
    /// Connect now if disconnected; if connected, verify the connection with an echo (15 s).
    Wake,
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

/// From the connection to the app/UI.
pub enum FromConnection {
    State(ConnectionState),
    Event(Box<CoreEvent>),
    /// The server closed the connection with a message for the user.
    ServerMessage {
        message: String,
        can_reconnect: bool,
    },
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
}

#[derive(Debug, thiserror::Error)]
enum ConnectionError {
    #[error("network: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("connection timed out")]
    Timeout,
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
}

impl From<TransportError> for ConnectionError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::Io(error) => Self::Io(error),
            TransportError::Closed => Self::Io(std::io::ErrorKind::ConnectionAborted.into()),
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

fn protocol_error(error: impl std::fmt::Display) -> ConnectionError {
    ConnectionError::Protocol(error.to_string())
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

/// Run the connection until the app goes away (`rx` closed). Reconnects with backoff.
pub async fn run(
    setup: ConnectionSetup,
    mut rx: mpsc::UnboundedReceiver<ToConnection>,
    tx: mpsc::UnboundedSender<FromConnection>,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let _ = tx.send(FromConnection::State(ConnectionState::Connecting));
        let started = Instant::now();
        let result = Session::run(&setup, &mut rx, &tx).await;
        let _ = tx.send(FromConnection::State(ConnectionState::Disconnected));
        match result {
            Err(ConnectionError::Stopped) => return,
            Err(ConnectionError::Reconnect) => {
                backoff = Duration::from_secs(1);
                continue;
            }
            Err(ConnectionError::Closed {
                message,
                can_reconnect,
            }) => {
                tracing::warn!(can_reconnect, "chat server closed the connection");
                let _ = tx.send(FromConnection::ServerMessage {
                    message,
                    can_reconnect,
                });
                if !can_reconnect {
                    // E.g. another device took over this ID. Don't fight it; wait for a wake-up.
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
            // reflection queue full); wait for a wake-up.
            Err(ConnectionError::MediatorClosed { code, reason })
                if (4100..4200).contains(&code) =>
            {
                // log-ok: Threema's mediator's own text
                tracing::warn!(code, %reason, "mediator closed the connection");
                backoff = HOLD_OFF;
            }
            Err(error) => tracing::info!(%error, "chat server connection lost"),
            Ok(()) => {}
        }
        backoff = backoff_after_session(backoff, started.elapsed());
        // Wait for the backoff, or reconnect immediately on a wake-up.
        let wait = sleep(backoff);
        tokio::pin!(wait);
        loop {
            tokio::select! {
                () = &mut wait => break,
                message = rx.recv() => match message {
                    None => return,
                    Some(ToConnection::Wake | ToConnection::Reconnect) => break,
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
        backoff = (backoff * 2).min(MAX_BACKOFF);
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
    /// unread notifies now.
    fn drop(&mut self) {
        self.release_held_notifications(true);
    }
}

impl<'a> Session<'a> {
    async fn run(
        setup: &'a ConnectionSetup,
        rx: &mut mpsc::UnboundedReceiver<ToConnection>,
        tx: &'a mpsc::UnboundedSender<FromConnection>,
    ) -> Result<(), ConnectionError> {
        let mut session = Self::connect(setup, tx).await?;
        let _ = tx.send(FromConnection::State(ConnectionState::Connected));
        tracing::info!("connected to chat server");
        while let Some(payload) = session.d2m_queue.pop_front() {
            session.handle_d2m(payload).await?;
        }
        session.terminate_fs_if_pending().await?;
        session.reflect_queued_syncs().await?;
        session.send_pending().await?;
        session.payload_loop(rx).await
    }

    /// TCP straight to the chat server (no multi-device).
    async fn connect_direct(setup: &ConnectionSetup) -> Result<TcpStream, ConnectionError> {
        let mut last_error = None;
        let mut stream = None;
        for address in setup
            .config
            .chat_server_address
            .addresses(setup.server_group)
        {
            let host = crate::https::server_url(&address.0);
            match timeout(
                CONNECT_TIMEOUT,
                TcpStream::connect((host.as_ref(), address.1)),
            )
            .await
            {
                Ok(Ok(s)) => {
                    // log-ok: Threema's server and port
                    tracing::info!(%host, port = address.1, "chat server reached");
                    stream = Some(s);
                    break;
                }
                Ok(Err(error)) => last_error = Some(ConnectionError::Io(error)),
                Err(_) => last_error = Some(ConnectionError::Timeout),
            }
        }
        let stream = stream.ok_or_else(|| last_error.unwrap_or(ConnectionError::Timeout))?;
        stream.set_nodelay(true)?;
        Ok(stream)
    }

    async fn connect(
        setup: &'a ConnectionSetup,
        tx: &'a mpsc::UnboundedSender<FromConnection>,
    ) -> Result<Session<'a>, ConnectionError> {
        // Multi-device (ADR 0011): through the mediator, without forward security (SE-199).
        let multi_device = setup.store.multi_device().map_err(protocol_error)?;
        let fs_disabled = setup
            .store
            .forward_security_disabled()
            .map_err(protocol_error)?;
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
                    CONNECT_TIMEOUT,
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
                                .map_err(protocol_error)?
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
                    let mut registered = setup.store.multi_device().map_err(protocol_error)?;
                    if let Some(registered) = registered.as_mut() {
                        registered.registered = true;
                        setup
                            .store
                            .set_multi_device(Some(registered))
                            .map_err(protocol_error)?;
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
                // No sessions with multi-device (SE-199) or when the user switched forward
                // security off (Settings → Advanced): incoming FS messages are then rejected
                // and the session terminated, so the peer falls back, like Android with a
                // device group.
                fs_sessions: if multi_device.is_some() || fs_disabled {
                    None
                } else {
                    Some(Box::new(RefCell::new(StoreFs {
                        store: Rc::clone(store),
                        my_identity: setup.identity,
                        events: Rc::clone(&events),
                    })))
                },
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
        Ok(inbound)
    }

    /// client-hello → server-hello → login → login-ack (through the mediator: proxied, while
    /// mediator payloads are queued for later).
    async fn handshake(&mut self, client_hello: Vec<u8>) -> Result<(), ConnectionError> {
        self.transport.write_csp(&client_hello).await?;
        let mut buffer = vec![0_u8; 8192];
        loop {
            while let Some(instruction) = self.csp.poll().map_err(protocol_error)? {
                if instruction.incoming_payload.is_some() {
                    return Err(ConnectionError::Protocol("payload during handshake".into()));
                }
                if let Some(frame) = instruction.outgoing_frame {
                    self.transport.write_csp(&frame.0).await?;
                }
                if let Some(CspStateUpdate::PostHandshake(_)) = instruction.state_update {
                    return Ok(());
                }
            }
            match self.read_inbound(&mut buffer).await? {
                Inbound::Csp(bytes) => self.csp.add_chunks(&[&bytes]).map_err(protocol_error)?,
                Inbound::D2m(payload) => self.d2m_queue.push_back(payload),
            }
        }
    }

    async fn send_payload(&mut self, payload: &OutgoingPayload) -> Result<(), ConnectionError> {
        let instruction = self.csp.create_payload(payload).map_err(protocol_error)?;
        self.handle_instruction(instruction).await
    }

    /// Send every pending outgoing message from the store, then the outbox.
    async fn send_pending(&mut self) -> Result<(), ConnectionError> {
        for message in self
            .setup
            .store
            .pending_outgoing()
            .map_err(protocol_error)?
        {
            let group = crate::groups::parse_key(&message.conversation);
            if group.is_none()
                && self
                    .awaiting_ack
                    .iter()
                    .any(|(to, id)| *to == message.conversation && *id == message.message_id)
            {
                continue;
            }
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
                        .map_err(protocol_error)?;
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
                        .map_err(protocol_error)?;
                    if !changed.is_empty() {
                        self.emit(CoreEvent::MessagesChanged(changed));
                    }
                }
                continue;
            }
            if matches!(
                body,
                ContactMessageBody::Text(_)
                    | ContactMessageBody::Location(_)
                    | ContactMessageBody::File(_)
                    | ContactMessageBody::PollSetup(_)
                    | ContactMessageBody::PollVote(_)
            ) {
                self.distribute_picture(&message.conversation).await?;
            }
            let body = Outgoing::Contact(body);
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
                        .map_err(protocol_error)?
                    {
                        self.emit(CoreEvent::MessagesChanged(vec![failed]));
                    }
                }
                Err(error) => return Err(error),
            }
        }
        self.send_outbox().await
    }

    /// The _Profile Picture Distribution Steps_ for `receiver`: our picture (uploaded once per
    /// week) or a removal, if they don't have the current state yet. Failures only skip it.
    async fn distribute_picture(&mut self, receiver: &str) -> Result<(), ConnectionError> {
        use crate::profile_pictures::{Distribution, OWN_PICTURE, Upload};
        let store = Rc::clone(&self.setup.store);
        let own = self.setup.media.read_picture(OWN_PICTURE).ok().flatten();
        let body = match store
            .picture_distribution(receiver, own.is_some())
            .map_err(protocol_error)?
        {
            Distribution::None => return Ok(()),
            Distribution::Delete => {
                store.picture_sent(receiver, None).map_err(protocol_error)?;
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
                            .map_err(protocol_error)?;
                        upload
                    }
                    (None, None) => return Ok(()),
                };
                store
                    .picture_sent(receiver, Some(&upload.blob_id))
                    .map_err(protocol_error)?;
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
    /// `true` if nobody is left. Members we have no contact for yet are tried again later.
    async fn send_to_group(
        &mut self,
        key: &str,
        group: GroupIdentity,
        message_id: u64,
        created_at: i64,
        body: &GroupMessageBody,
    ) -> Result<bool, ConnectionError> {
        let receivers = self
            .setup
            .store
            .pending_receivers(key, message_id)
            .map_err(protocol_error)?;
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
            match self
                .encode(
                    &receiver,
                    message_id,
                    created_at,
                    Outgoing::Group(group, body.clone()),
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
                None => tracing::info!("group member without a contact yet, sending later"),
            }
        }
        Ok(false)
    }

    /// Send the outbox (reactions, edits, deletions, group control): removed once the server
    /// acknowledged it (for every receiver of a group), or if unsendable.
    async fn send_outbox(&mut self) -> Result<(), ConnectionError> {
        for entry in self.setup.store.outbox().map_err(protocol_error)? {
            if let Some(group) = crate::groups::parse_key(&entry.conversation) {
                let Some(body) = outbox_group_body(&entry) else {
                    tracing::warn!(kind = entry.kind, "dropping unsendable group outbox entry");
                    self.setup
                        .store
                        .outbox_remove(&entry.conversation, entry.message_id)
                        .map_err(protocol_error)?;
                    continue;
                };
                if self
                    .send_to_group(
                        &entry.conversation,
                        group,
                        entry.message_id,
                        entry.created_at,
                        &body,
                    )
                    .await?
                {
                    self.setup
                        .store
                        .outbox_remove(&entry.conversation, entry.message_id)
                        .map_err(protocol_error)?;
                }
                continue;
            }
            if self
                .awaiting_ack
                .iter()
                .any(|(to, id)| *to == entry.conversation && *id == entry.message_id)
            {
                continue;
            }
            let encoded = match outbox_body(&entry) {
                Some(body) => {
                    self.reflect_outgoing(
                        &entry.conversation,
                        entry.message_id,
                        entry.created_at,
                        &Outgoing::Contact(body.clone()),
                    )
                    .await?;
                    self.encode(
                        &entry.conversation,
                        entry.message_id,
                        entry.created_at,
                        Outgoing::Contact(body),
                    )
                    .await?
                }
                None => None,
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
                    .map_err(protocol_error)?;
            }
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
                .map_err(protocol_error)?
            {
                break nonce;
            }
        };
        self.setup
            .store
            .add_nonces(&[&nonce.0])
            .map_err(protocol_error)?;
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
        envelope: &[u8],
    ) -> Result<Option<MessageWithMetadataBox>, ConnectionError> {
        let Some(contact) = self.setup.store.contact(receiver).map_err(protocol_error)? else {
            return Ok(None);
        };
        let nonce = self.fresh_nonce()?;
        match encode_outgoing_fs_control_message(
            self.setup.identity,
            &ClientKey::from(*self.setup.client_key),
            contact.identity,
            &contact.public_key,
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

    /// Encode a message to a contact: forward-security encapsulated if the contact supports it
    /// (with an `Init` first for a new session), otherwise plain. `Ok(None)` if it can never be
    /// sent (unknown/revoked contact).
    async fn encode(
        &self,
        receiver: &str,
        message_id: u64,
        created_at: i64,
        body: Outgoing,
    ) -> Result<Option<Vec<MessageWithMetadataBox>>, ConnectionError> {
        let Some(contact) = self.setup.store.contact(receiver).map_err(protocol_error)? else {
            return Ok(None);
        };
        let receiver_identity = contact.identity;
        let receiver_key: PublicKey = contact.public_key;
        let nickname = self
            .setup
            .store
            .setting(crate::store::keys::NICKNAME)
            .map_err(protocol_error)?;
        let client_key = ClientKey::from(*self.setup.client_key);
        let mut payloads = Vec::new();

        // Forward security, if the contact supports it.
        let mut envelope = None;
        // An existing session proves support even if our copy of the feature mask is stale (the
        // peer may have just enabled FS, e.g. by unlinking a multi-device client).
        let has_session = self
            .fs_store()
            .get_best(receiver_identity)
            .map_err(protocol_error)?
            .is_some();
        // Never with multi-device (SE-199): the other devices couldn't follow the ratchet.
        // Nor when the user switched forward security off (Settings → Advanced).
        let use_fs = !self.transport.is_mediator()
            && !self.fs_disabled
            && (has_session || contact.feature_mask.0 & FeatureMask::FORWARD_SECURITY_SUPPORT != 0);
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
            match fs_protocol::encapsulate(
                &mut self.fs_store(),
                self.setup.identity,
                &client_key,
                receiver_identity,
                &receiver_key,
                inner_type,
                &inner_data,
                u64::try_from(crate::messages::now_ms()).unwrap_or(0),
            ) {
                Ok(encapsulation) => {
                    for control in encapsulation.prelude {
                        payloads.extend(self.encode_control(receiver, &control)?);
                    }
                    envelope = encapsulation.envelope;
                }
                // Never fall back silently for a broken session: try again on the next send.
                Err(error) => {
                    return Err(ConnectionError::Protocol(format!(
                        "forward security: {error}"
                    )));
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
                nickname.as_deref(),
                receiver_identity,
                &receiver_key,
                &message,
                envelope,
                nonce,
            ),
            None => encode_outgoing_contact_message(
                self.setup.identity,
                &client_key,
                nickname.as_deref(),
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

    async fn send_receipt(
        &mut self,
        receiver: &str,
        receipt_type: DeliveryReceiptType,
        message_ids: Vec<u64>,
    ) -> Result<(), ConnectionError> {
        if message_ids.is_empty() {
            return Ok(());
        }
        if matches!(receipt_type, DeliveryReceiptType::Read) {
            self.reflect_read(receiver, &message_ids).await?;
        }
        let body = ContactMessageBody::DeliveryReceipt(DeliveryReceiptMessage {
            receipt_type,
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
            CoreEvent::FsControl { peer, envelope } => {
                if let Some(payload) = self.encode_control(&peer, &envelope)? {
                    self.send_payload(&OutgoingPayload::MessageWithMetadataBox(payload))
                        .await?;
                }
                Ok(None)
            }
            CoreEvent::FsStatus { peer, status } => {
                let store = &self.setup.store;
                if let FsStatus::MessageRejected { message_id } = status {
                    // Once per message and connection: a peer that rejects the re-send too
                    // would otherwise bounce the message back and forth without end — the
                    // second rejection leaves it failed for the user to resend by hand.
                    if !self.fs_resent.insert((peer.clone(), message_id)) {
                        tracing::warn!("message rejected again; left failed");
                        if let Some(message) = store
                            .message_by_id(&peer, true, message_id)
                            .map_err(protocol_error)?
                            && let Some(failed) = store
                                .set_outgoing_failed(message.id)
                                .map_err(protocol_error)?
                        {
                            self.emit(CoreEvent::MessagesChanged(vec![failed]));
                        }
                    } else {
                        // A group message: the peer gets it again.
                        if store
                            .requeue_group_receiver(&peer, message_id)
                            .map_err(protocol_error)?
                        {
                            self.send_pending().await?;
                        }
                        // Like Android: the message needs to be sent again (new session).
                        let reset = store
                            .reset_outgoing(&peer, message_id)
                            .map_err(protocol_error)?;
                        if !reset.is_empty() {
                            self.emit(CoreEvent::MessagesChanged(reset));
                            self.send_pending().await?;
                        }
                    }
                }
                let code = match status {
                    FsStatus::Established => "fsEstablished",
                    FsStatus::Reset | FsStatus::MessageRejected { .. } => "fsReset",
                    FsStatus::Terminated => "fsTerminated",
                };
                Ok(store
                    .insert_status(&peer, code)
                    .map_err(protocol_error)?
                    .map(|message| CoreEvent::MessageAdded {
                        message,
                        send_delivery_receipt: false,
                    }))
            }
            CoreEvent::OutboxChanged => {
                self.send_pending().await?;
                Ok(None)
            }
            other => Ok(Some(other)),
        }
    }

    fn emit(&self, event: CoreEvent) {
        let _ = self.tx.send(FromConnection::Event(Box::new(event)));
    }

    /// After multi-device was turned on: end every forward security session and tell the peers
    /// (Android's `disableForwardSecurity`: Terminate with cause DISABLED_BY_LOCAL), so they stop
    /// sending what we could no longer read.
    async fn terminate_fs_if_pending(&mut self) -> Result<(), ConnectionError> {
        use libthreema::{common::ThreemaId, protobuf::csp_e2e_fs::terminate::Cause};
        let store = Rc::clone(&self.setup.store);
        if store
            .setting(crate::multidevice::SETTING_FS_TERMINATE)
            .map_err(protocol_error)?
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
                .map_err(protocol_error)?;
            return Ok(());
        }
        let peers = store.fs_peers().map_err(protocol_error)?;
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
        let events: Vec<CoreEvent> = self.events.borrow_mut().drain(..).collect();
        for event in events {
            if let Some(event) = Box::pin(self.handle_fs_event(event)).await? {
                self.emit(event);
            }
        }
        store
            .delete_setting(crate::multidevice::SETTING_FS_TERMINATE)
            .map_err(protocol_error)?;
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
        while let Some((id, scope, content)) = store.next_sync().map_err(protocol_error)? {
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
            store.sync_done(id).map_err(protocol_error)?;
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

    /// Another device's reflection: apply it, then acknowledge (also when it can't be used,
    /// or the mediator would send it again at every login).
    async fn handle_reflected(
        &mut self,
        reflected: libthreema::d2m::payload::Reflected,
    ) -> Result<(), ConnectionError> {
        let device_group_key = self.device_group_key()?;
        let store = Rc::clone(&self.setup.store);
        match crate::reflected::open(&store, &device_group_key, &reflected.envelope) {
            Ok(envelope) => {
                // log-ok: the kind of content, not the content
                tracing::info!(
                    content = crate::reflected::content_name(envelope.content.as_ref()),
                    "reflected"
                );
                if let Err(error) =
                    crate::reflected::apply(&store, self.setup.identity, envelope, &self.events)
                {
                    tracing::warn!(%error, "reflected envelope not applied");
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
        let events: Vec<CoreEvent> = self.events.borrow_mut().drain(..).collect();
        for event in events {
            // A message another device received first: its notification waits a moment.
            if let CoreEvent::MessageAdded { message, .. } = &event
                && !message.outgoing
            {
                self.setup
                    .store
                    .held_notifications
                    .borrow_mut()
                    .insert(message.id);
                self.held.push((message.id, Instant::now() + NOTIFY_GRACE));
            }
            if let Some(event) = Box::pin(self.handle_fs_event(event)).await? {
                self.emit(event);
            }
        }
        Ok(())
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
            }
            // Only the leader receives from the chat server (Android's MonitoringLayer).
            D2mIncoming::RolePromotedToLeader => {
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
            tokio::select! {
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
                        self.send_receipt(&identity, DeliveryReceiptType::Read, message_ids).await?;
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
                                self.setup.store.set_multi_device(None).map_err(protocol_error)?;
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
                    Some(ToConnection::Wake) => if !self.liveness.awaiting_echo() {
                        self.send_payload(&OutgoingPayload::EchoRequest(EchoPayload(Vec::new()))).await?;
                        // Give the echo 15 s (instead of the usual interval) to come back.
                        self.liveness.echo_sent(Instant::now(), Duration::from_secs(15));
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
                    .map_err(protocol_error)?;
                let mut changed = store
                    .raise_outgoing_state(&receiver, &[ack.id.0], OutgoingState::Sent)
                    .map_err(protocol_error)?;
                // Group messages: sent once every member has it.
                for conversation in store
                    .receiver_done(&receiver, ack.id.0)
                    .map_err(protocol_error)?
                {
                    store
                        .outbox_remove(&conversation, ack.id.0)
                        .map_err(protocol_error)?;
                    changed.extend(
                        store
                            .raise_outgoing_state(&conversation, &[ack.id.0], OutgoingState::Sent)
                            .map_err(protocol_error)?,
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
                let _ = self.tx.send(FromConnection::ServerMessage {
                    message: alert.0,
                    can_reconnect: true,
                });
            }
            IncomingPayload::QueueSendComplete => tracing::debug!("message queue drained"),
            // Like Android's TaskManager: warn (unless our own cookie is new) and clear the
            // server's flag. Left set, every login of every device in the group gets it again
            // (Threema Desktop: "Connection from another device detected", on every start).
            IncomingPayload::DeviceCookieChangeIndication => {
                let store = &self.setup.store;
                let fresh = store
                    .setting(crate::store::keys::DEVICE_COOKIE_FRESH)
                    .map_err(protocol_error)?
                    .is_some();
                if fresh {
                    tracing::info!("device cookie change indication for our new cookie");
                    store
                        .delete_setting(crate::store::keys::DEVICE_COOKIE_FRESH)
                        .map_err(protocol_error)?;
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
        let mut task = self.e2e.handle_incoming_message(message);
        let result = loop {
            match task.poll(self.e2e.context()).map_err(protocol_error)? {
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
                    .map_err(protocol_error)?;
                }
                // Multi-device: reflect to the other devices before acknowledging; contacts are
                // created/updated in a transaction.
                IncomingMessageLoop::Instruction(IncomingMessageInstruction::ReflectMessage(
                    instruction,
                )) => {
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
        if let Some(ack) = result.outgoing_message_ack {
            self.send_payload(&OutgoingPayload::MessageAck(ack)).await?;
        }
        let events: Vec<CoreEvent> = self.events.borrow_mut().drain(..).collect();
        for event in events {
            let Some(event) = Box::pin(self.handle_fs_event(event)).await? else {
                continue;
            };
            if let CoreEvent::MessageAdded {
                message:
                    Message {
                        conversation,
                        message_id,
                        ..
                    },
                send_delivery_receipt: true,
            } = &event
            {
                let (conversation, message_id) = (conversation.clone(), *message_id);
                self.send_receipt(
                    &conversation,
                    DeliveryReceiptType::Received,
                    vec![message_id],
                )
                .await?;
            }
            self.emit(event);
        }
        Ok(())
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
mod device_group_error_tests {
    use super::*;

    #[test]
    fn only_a_lost_connection_counts_as_offline() {
        for lost in [
            ConnectionError::Timeout,
            ConnectionError::Reconnect,
            ConnectionError::from(TransportError::Closed),
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
        assert!(JOIN_REPLY_LIMIT > steps + CONNECT_TIMEOUT);
    }
}
