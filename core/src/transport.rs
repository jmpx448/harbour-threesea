// SPDX-License-Identifier: AGPL-3.0-only
//! Where the chat server connection's bytes go: straight to the chat server (TCP), or, with
//! multi-device active (ADR 0011), through the mediator (WebSocket + libthreema's sans-I/O
//! `D2mProtocol`), where CSP frames travel as D2M `proxy` payloads and the mediator adds its own
//! payloads (reflection, transactions, device management).
//!
//! Upstream: Android's `D2mFrameLayer` / `MultiplexLayer` (`domain/.../connection`).

use std::collections::VecDeque;

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
};

use crate::websocket::{Datagrams, WebSocketError};

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
}

impl Transport {
    pub async fn write_csp(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        match self {
            Self::Direct(stream) => Ok(stream.write_all(frame).await?),
            Self::Mediator(mediator) => mediator.send(D2mOutgoing::Proxy(frame.to_vec())).await,
        }
    }

    /// A mediator payload (multi-device only; an error without it).
    pub async fn send_d2m(&mut self, payload: D2mOutgoing) -> Result<(), TransportError> {
        match self {
            Self::Direct(_) => Err(TransportError::D2m(
                "not connected through the mediator".into(),
            )),
            Self::Mediator(mediator) => mediator.send(payload).await,
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
        }
    }

    #[must_use]
    pub fn is_mediator(&self) -> bool {
        matches!(self, Self::Mediator(_))
    }
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
        let socket = Datagrams::connect(&url).await?;
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
