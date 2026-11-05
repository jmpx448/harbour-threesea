// SPDX-License-Identifier: AGPL-3.0-only
//! Where the chat server connection's bytes go: straight to the chat server (TCP), or, with
//! multi-device active (ADR 0011), through the mediator (WebSocket + libthreema's sans-I/O
//! `D2mProtocol`), where CSP frames travel as D2M `proxy` payloads and the mediator adds its own
//! payloads (reflection, transactions, device management).
//!
//! Upstream: Android's `D2mFrameLayer` / `MultiplexLayer` (`domain/.../connection`).

use std::{collections::VecDeque, time::Duration};

use libthreema::{
    common::{ClientInfo, D2xDeviceId, config::Config},
    d2m::{
        D2mContext, D2mProtocol, D2mStateUpdate,
        payload::{IncomingPayload as D2mIncoming, OutgoingPayload as D2mOutgoing, ServerInfo},
    },
    protobuf::d2m::{DeviceSlotExpirationPolicy, DeviceSlotState, client_hello},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
    time::timeout,
};

use crate::websocket::{Datagrams, WebSocketError};

/// How long one write may take. It only waits when the kernel's send buffer is full, and while it
/// waits the connection's loop hears nothing, timers and "Connect now" included: on a stalled link
/// TCP alone would take many minutes to give up.
const WRITE_LIMIT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("network: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    WebSocket(#[from] WebSocketError),
    #[error("mediator protocol: {0}")]
    D2m(String),
    #[error("connection closed")]
    Closed,
    #[error("writing timed out")]
    WriteTimeout,
}

fn d2m_error(error: impl std::fmt::Display) -> TransportError {
    TransportError::D2m(error.to_string())
}

/// What arrived.
pub enum Inbound {
    /// Bytes of the CSP stream (any chunking).
    Csp(Vec<u8>),
    /// A mediator payload (only with multi-device).
    D2m(D2mIncoming),
}

pub enum Transport {
    Direct(TcpStream),
    Mediator(Box<Mediator>),
    #[cfg(test)]
    Scripted(Box<ScriptedWire>),
}

#[cfg(test)]
type Written = Box<dyn FnMut(&[u8]) -> Result<(), TransportError>>;

/// A test's fake server where the socket would be. A write reaches the server within the call, so
/// it can look at the store as it was when those bytes went out, and no clock or socket is
/// involved in what the test sees.
#[cfg(test)]
pub struct ScriptedWire {
    pub written: Written,
    pub inbound: tokio::sync::mpsc::UnboundedReceiver<Result<Vec<u8>, TransportError>>,
}

impl Transport {
    pub async fn write_csp(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        within_write_limit(async {
            match self {
                Self::Direct(stream) => Ok(stream.write_all(frame).await?),
                Self::Mediator(mediator) => mediator.send(D2mOutgoing::Proxy(frame.to_vec())).await,
                #[cfg(test)]
                Self::Scripted(wire) => (wire.written)(frame),
            }
        })
        .await
    }

    /// A mediator payload (multi-device only; an error without it).
    pub async fn send_d2m(&mut self, payload: D2mOutgoing) -> Result<(), TransportError> {
        match self {
            Self::Direct(_) => Err(TransportError::D2m(
                "not connected through the mediator".into(),
            )),
            Self::Mediator(mediator) => within_write_limit(mediator.send(payload)).await,
            #[cfg(test)]
            Self::Scripted(_) => Err(TransportError::D2m(
                "not connected through the mediator".into(),
            )),
        }
    }

    /// The next thing that arrives. Safe to cancel (in `select!`): nothing received is lost.
    pub async fn read(&mut self, buffer: &mut [u8]) -> Result<Inbound, TransportError> {
        match self {
            Self::Direct(stream) => {
                let length = stream.read(buffer).await?;
                if length == 0 {
                    return Err(TransportError::Closed);
                }
                Ok(Inbound::Csp(buffer[..length].to_vec()))
            }
            Self::Mediator(mediator) => mediator.read().await,
            #[cfg(test)]
            Self::Scripted(wire) => match wire.inbound.recv().await {
                Some(read) => read.map(Inbound::Csp),
                None => Err(TransportError::Closed),
            },
        }
    }

    #[must_use]
    pub fn is_mediator(&self) -> bool {
        matches!(self, Self::Mediator(_))
    }
}

async fn within_write_limit(
    write: impl Future<Output = Result<(), TransportError>>,
) -> Result<(), TransportError> {
    timeout(WRITE_LIMIT, write)
        .await
        .map_err(|_| TransportError::WriteTimeout)?
}

/// The connection to the mediator, after its handshake.
pub struct Mediator {
    socket: Datagrams,
    d2m: D2mProtocol,
    /// Datagrams libthreema produced while we were reading, sent before the next read.
    outgoing: VecDeque<Vec<u8>>,
    pub server_info: Option<ServerInfo>,
}

/// What the mediator needs to know about this device.
pub struct MediatorLogin {
    pub device_group_key: libthreema::common::keys::DeviceGroupKey,
    pub device_id: u64,
    pub registered: bool,
    pub server_group: libthreema::common::ChatServerGroup,
    /// What the other devices show for this one (the user's setting).
    pub device_label: String,
    /// Who we are to the mediator and the other devices.
    pub client_info: ClientInfo,
}

impl Mediator {
    /// Connect and run the D2M handshake (server-hello, client-hello, server-info).
    pub async fn connect(config: &Config, login: MediatorLogin) -> Result<Self, TransportError> {
        let (d2m, url) = Self::protocol(config, login)?;
        let socket = Datagrams::connect(&url).await?;
        Self::handshake(socket, d2m).await
    }

    /// libthreema's state machine for this device, and the mediator's URL.
    fn protocol(
        config: &Config,
        login: MediatorLogin,
    ) -> Result<(D2mProtocol, String), TransportError> {
        let multi_device = config
            .multi_device
            .as_ref()
            .ok_or_else(|| TransportError::D2m("no multi-device configuration".into()))?;
        let (d2m, url) = D2mProtocol::new(D2mContext {
            mediator_server_url: multi_device.mediator_server_url.clone(),
            device_group_key: login.device_group_key,
            csp_server_group: login.server_group,
            device_id: D2xDeviceId(login.device_id),
            // Like Android: never push another device out; keep the slot until it is dropped.
            device_slots_exhausted_policy: client_hello::DeviceSlotsExhaustedPolicy::Reject,
            device_slot_expiration_policy: DeviceSlotExpirationPolicy::Persistent,
            expected_device_slot_state: if login.registered {
                DeviceSlotState::Existing
            } else {
                DeviceSlotState::New
            },
            client_info: login.client_info,
            device_label: Some(login.device_label.clone()),
        });
        Ok((d2m, url))
    }

    /// The handshake over a connected socket.
    async fn handshake(socket: Datagrams, d2m: D2mProtocol) -> Result<Self, TransportError> {
        let mut mediator = Self {
            socket,
            d2m,
            outgoing: VecDeque::new(),
            server_info: None,
        };
        loop {
            let datagram = mediator.socket.receive().await?;
            mediator
                .d2m
                .add_datagrams(vec![datagram])
                .map_err(d2m_error)?;
            while let Some(instruction) = mediator.d2m.poll().map_err(d2m_error)? {
                if instruction.incoming_payload.is_some() {
                    return Err(TransportError::D2m("payload during handshake".into()));
                }
                if let Some(datagram) = instruction.outgoing_datagram {
                    mediator.socket.send(datagram.0).await?;
                }
                if let Some(D2mStateUpdate::PostHandshake(server_info)) = instruction.state_update {
                    mediator.server_info = Some(server_info);
                    return Ok(mediator);
                }
            }
        }
    }

    async fn send(&mut self, payload: D2mOutgoing) -> Result<(), TransportError> {
        self.flush().await?;
        let instruction = self.d2m.create_payload(payload).map_err(d2m_error)?;
        if let Some(datagram) = instruction.outgoing_datagram {
            self.socket.send(datagram.0).await?;
        }
        if instruction.incoming_payload.is_some() || instruction.state_update.is_some() {
            return Err(TransportError::D2m("unexpected result of sending".into()));
        }
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), TransportError> {
        while let Some(datagram) = self.outgoing.front() {
            self.socket.send(datagram.clone()).await?;
            self.outgoing.pop_front();
        }
        Ok(())
    }

    async fn read(&mut self) -> Result<Inbound, TransportError> {
        loop {
            while let Some(instruction) = self.d2m.poll().map_err(d2m_error)? {
                if instruction.state_update.is_some() {
                    return Err(TransportError::D2m(
                        "state update after the handshake".into(),
                    ));
                }
                if let Some(datagram) = instruction.outgoing_datagram {
                    self.outgoing.push_back(datagram.0);
                }
                match instruction.incoming_payload {
                    Some(D2mIncoming::Proxy(data)) => return Ok(Inbound::Csp(data)),
                    Some(other) => return Ok(Inbound::D2m(other)),
                    None => {}
                }
            }
            self.flush().await?;
            let datagram = self.socket.receive().await?;
            self.d2m.add_datagrams(vec![datagram]).map_err(d2m_error)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::{net::TcpListener, time::Instant};

    use super::*;

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_write_into_a_stalled_link_gives_up() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        // Accepted, and never read: more than the kernel's buffers hold can't go anywhere.
        let (_peer, _) = listener.accept().await.unwrap();
        let mut transport = Transport::Direct(stream);
        let start = Instant::now();
        let written = transport.write_csp(&vec![0_u8; 64 * 1024 * 1024]).await;
        assert!(matches!(written, Err(TransportError::WriteTimeout)));
        assert_eq!(start.elapsed(), WRITE_LIMIT);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_write_that_goes_through_is_not_touched() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut peer, _) = listener.accept().await.unwrap();
        let mut transport = Transport::Direct(stream);
        transport.write_csp(b"frame").await.unwrap();
        let mut read = [0_u8; 5];
        peer.read_exact(&mut read).await.unwrap();
        assert_eq!(&read, b"frame");
    }

    /// A D2M datagram: its type, three reserved bytes, the body.
    fn datagram(kind: u8, body: &[u8]) -> Vec<u8> {
        [&[kind, 0, 0, 0][..], body].concat()
    }

    /// The mediator's half of the handshake, as a stand-in on this machine.
    async fn mediator_side(mut socket: Datagrams) -> Datagrams {
        use libthreema::protobuf::d2m::{ClientHello, ServerHello, ServerInfo};
        use prost::Message as _;
        // The X25519 base point: a key that is no low-order point.
        let mut esk = vec![0_u8; 32];
        esk[0] = 9;
        let hello = ServerHello {
            version: 0,
            esk,
            challenge: vec![7; 32],
        };
        socket
            .send(datagram(0x10, &hello.encode_to_vec()))
            .await
            .unwrap();
        let answer = socket.receive().await.unwrap();
        assert_eq!(answer[0], 0x11, "a client hello");
        let answer = ClientHello::decode(&answer[4..]).unwrap();
        assert_eq!(answer.device_id, 42);
        assert_eq!(
            answer.expected_device_slot_state,
            DeviceSlotState::New as i32
        );
        assert_eq!(
            answer.response.len(),
            24 + 32 + 16,
            "the challenge, encrypted"
        );
        let info = ServerInfo {
            current_time: u64::try_from(crate::messages::now_ms()).unwrap(),
            max_device_slots: 4,
            device_slot_state: DeviceSlotState::New as i32,
            encrypted_shared_device_data: Vec::new(),
            reflection_queue_length: 0,
        };
        socket
            .send(datagram(0x12, &info.encode_to_vec()))
            .await
            .unwrap();
        socket
    }

    /// Through the mediator, CSP and the mediator's own payloads go both ways.
    #[tokio::test(flavor = "current_thread")]
    async fn the_mediator_carries_both_ways() {
        let (ours, theirs) = crate::websocket::loopback().await;
        let login = MediatorLogin {
            device_group_key: libthreema::common::keys::DeviceGroupKey::from([5; 32]),
            device_id: 42,
            registered: false,
            server_group: libthreema::common::ChatServerGroup(1),
            device_label: "Phone".into(),
            client_info: ClientInfo::Libthreema,
        };
        let config = Config::from(libthreema::common::config::ConfigEnvironment::Production);
        let (d2m, url) = Mediator::protocol(&config, login).unwrap();
        assert!(crate::https::is_threema_url(&url), "{url}");

        let (mediator, mut theirs) =
            tokio::join!(Mediator::handshake(ours, d2m), mediator_side(theirs));
        let mediator = mediator.unwrap();
        assert_eq!(mediator.server_info.as_ref().unwrap().max_device_slots, 4);
        let mut transport = Transport::Mediator(Box::new(mediator));
        assert!(transport.is_mediator());
        let mut buffer = vec![0; 1024];

        theirs
            .send(datagram(0x00, b"from the chat server"))
            .await
            .unwrap();
        let Inbound::Csp(bytes) = transport.read(&mut buffer).await.unwrap() else {
            panic!("not CSP");
        };
        assert_eq!(bytes, b"from the chat server");
        transport.write_csp(b"to the chat server").await.unwrap();
        assert_eq!(
            theirs.receive().await.unwrap(),
            datagram(0x00, b"to the chat server")
        );

        theirs.send(datagram(0x20, &[])).await.unwrap();
        assert!(matches!(
            transport.read(&mut buffer).await.unwrap(),
            Inbound::D2m(D2mIncoming::ReflectionQueueDry)
        ));
        transport
            .send_d2m(D2mOutgoing::GetDevicesInfo)
            .await
            .unwrap();
        assert_eq!(theirs.receive().await.unwrap()[0], 0x30);
    }

    /// Straight to the chat server: CSP both ways, no mediator payloads, and its end noticed.
    #[tokio::test(flavor = "current_thread")]
    async fn a_direct_connection_carries_csp_both_ways() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut peer, _) = listener.accept().await.unwrap();
        let mut transport = Transport::Direct(stream);
        assert!(!transport.is_mediator());
        assert!(matches!(
            transport.send_d2m(D2mOutgoing::GetDevicesInfo).await,
            Err(TransportError::D2m(_))
        ));
        peer.write_all(b"server").await.unwrap();
        let mut buffer = vec![0; 6];
        let mut got = Vec::new();
        while got.len() < 6 {
            let Inbound::Csp(bytes) = transport.read(&mut buffer).await.unwrap() else {
                panic!("not CSP");
            };
            got.extend(bytes);
        }
        assert_eq!(got, b"server");
        drop(peer);
        assert!(matches!(
            transport.read(&mut buffer).await,
            Err(TransportError::Closed)
        ));
    }
}
