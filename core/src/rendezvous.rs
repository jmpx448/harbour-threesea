// SPDX-License-Identifier: AGPL-3.0-only
//! Device linking, first half (ADR 0011): the QR code Threema Desktop shows, and the Connection
//! Rendezvous Protocol over its WebSocket relay.
//!
//! Upstream: Android's `DeviceLinkingPartOneTask` and `RendezvousConnection` (we are always the
//! existing device: RRD and nominator). The crypto and framing are libthreema's
//! `RendezvousProtocol`; this is the I/O around it.

use data_encoding::BASE64URL_NOPAD;
use libthreema::{
    d2d_rendezvous::{
        RendezvousAuthenticationKey, RendezvousOutgoingFrame, RendezvousPathStateUpdate,
        RendezvousProtocol,
    },
    protobuf::{
        d2d::ProtocolVersion,
        url::{DeviceGroupJoinRequestOrOffer, device_group_join_request_or_offer::variant::Type},
    },
};
use prost::Message as _;
use tokio::time::{Duration, timeout};

use crate::websocket::{Datagrams, WebSocketError};

/// The QR code's URL prefix (Android: `DEVICE_JOIN_OFFER_URI_PREFIX`).
const JOIN_PREFIX: &str = "threema://device-group/join#";
/// Largest WebSocket message we write (Android chunks frames to 1 MiB).
const CHUNK: usize = 1024 * 1024;
/// Time for the other device to answer on the relay.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Time for one chunk to leave: a relay that stopped reading doesn't hold us up for ever. (One
/// chunk, not one message: how long a message may take depends on its size.)
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);
/// Time for the goodbye on the relay.
pub const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    /// Not a Threema Desktop QR code (`unknownCode` in the UI).
    #[error("not a device join code")]
    NotJoinCode,
    /// A Threema Web QR code (Android tells the user so).
    #[error("this is a Threema Web code")]
    WebCode,
    /// The other app is too old or too new.
    #[error("unsupported version")]
    Version,
    #[error("{0}")]
    Socket(#[from] WebSocketError),
    #[error("rendezvous: {0}")]
    Protocol(String),
    #[error("timed out")]
    Timeout,
}

fn protocol(error: impl std::fmt::Display) -> LinkError {
    LinkError::Protocol(error.to_string())
}

/// Parse a scanned code. Checks like Android: join URL, offer version 1.0, D2D protocol at least
/// 0.2, a request to join (the desktop app starts) with a relayed WebSocket path.
pub fn parse_join_code(text: &str) -> Result<DeviceGroupJoinRequestOrOffer, LinkError> {
    let text = text.trim();
    let Some(payload) = text.strip_prefix(JOIN_PREFIX) else {
        // Threema Web codes are plain base64 of a binary blob starting with version 2.
        let looks_like_web = data_encoding::BASE64
            .decode(text.as_bytes())
            .is_ok_and(|bytes| bytes.len() > 101 && bytes.get(..2) == Some(&[0, 2][..]));
        return Err(if looks_like_web {
            LinkError::WebCode
        } else {
            LinkError::NotJoinCode
        });
    };
    let bytes = BASE64URL_NOPAD
        .decode(payload.trim_end_matches('=').as_bytes())
        .map_err(|_| LinkError::NotJoinCode)?;
    let offer = DeviceGroupJoinRequestOrOffer::decode(bytes.as_slice())
        .map_err(|_| LinkError::NotJoinCode)?;
    if offer.version != 0 || offer.d2d_protocol_version < ProtocolVersion::V02 as u32 {
        return Err(LinkError::Version);
    }
    let is_request = matches!(
        offer.variant.as_ref().and_then(|v| v.r#type.as_ref()),
        Some(Type::RequestToJoin(_))
    );
    let relayed = offer
        .rendezvous_init
        .as_ref()
        .and_then(|init| init.relayed_web_socket.as_ref());
    if !is_request || relayed.is_none_or(|path| !path.url.starts_with("wss://")) {
        return Err(LinkError::NotJoinCode);
    }
    if offer
        .rendezvous_init
        .as_ref()
        .is_none_or(|init| init.ak.len() != 32)
    {
        return Err(LinkError::NotJoinCode);
    }
    Ok(offer)
}

/// The nominated rendezvous path to the new device: end-to-end encrypted user data (ULP).
pub struct RendezvousLink {
    socket: Datagrams,
    protocol: RendezvousProtocol,
    pid: u32,
    /// The Rendezvous Path Hash: both devices show emojis derived from it.
    pub rph: [u8; 32],
}

impl RendezvousLink {
    /// Connect to the relay and run the handshake up to nomination.
    pub async fn connect(offer: &DeviceGroupJoinRequestOrOffer) -> Result<Self, LinkError> {
        let init = offer
            .rendezvous_init
            .as_ref()
            .ok_or(LinkError::NotJoinCode)?;
        let path = init
            .relayed_web_socket
            .as_ref()
            .ok_or(LinkError::NotJoinCode)?;
        let ak: [u8; 32] = init
            .ak
            .as_slice()
            .try_into()
            .map_err(|_| LinkError::NotJoinCode)?;
        let pid = path.path_id;
        let socket = timeout(CONNECT_TIMEOUT, Datagrams::connect(&path.url))
            .await
            .map_err(|_| LinkError::Timeout)??;
        Self::over(socket, ak, pid).await
    }

    /// The handshake up to nomination over a connected relay.
    async fn over(socket: Datagrams, ak: [u8; 32], pid: u32) -> Result<Self, LinkError> {
        let (protocol, initial) =
            RendezvousProtocol::new_as_rrd(true, RendezvousAuthenticationKey(ak), &[pid]);
        let mut link = Self {
            socket,
            protocol,
            pid,
            rph: [0; 32],
        };
        for (_, frame) in initial {
            link.write_frame(&frame).await?;
        }
        timeout(CONNECT_TIMEOUT, link.nominate())
            .await
            .map_err(|_| LinkError::Timeout)??;
        Ok(link)
    }

    async fn write_frame(&mut self, frame: &RendezvousOutgoingFrame) -> Result<(), LinkError> {
        let (header, payload) = frame.encode();
        let mut bytes = Vec::with_capacity(header.len() + payload.len());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(payload);
        for chunk in bytes.chunks(CHUNK) {
            timeout(WRITE_TIMEOUT, self.socket.send(chunk.to_vec()))
                .await
                .map_err(|_| LinkError::Timeout)??;
        }
        Ok(())
    }

    /// The nomination loop (Android's `runNominationLoop`).
    async fn nominate(&mut self) -> Result<(), LinkError> {
        loop {
            let chunk = self.socket.receive().await?;
            self.protocol
                .add_chunks(self.pid, &[&chunk])
                .map_err(protocol)?;
            let mut result = self.protocol.process_frame(self.pid).map_err(protocol)?;
            while let Some(current) = result.take() {
                if let Some(frame) = &current.outgoing_frame {
                    self.write_frame(frame).await?;
                }
                match current.state_update {
                    Some(RendezvousPathStateUpdate::AwaitingNominate { .. }) => {
                        // One path only: nominate it at once.
                        result = Some(self.protocol.nominate_path(self.pid).map_err(protocol)?);
                    }
                    Some(RendezvousPathStateUpdate::Nominated { rph }) => {
                        self.rph = rph.0;
                        return Ok(());
                    }
                    None => {}
                }
            }
        }
    }

    /// Send user data (a `d2d_join.EdToNd`).
    pub async fn send(&mut self, data: Vec<u8>) -> Result<(), LinkError> {
        let result = self.protocol.create_ulp_frame(data).map_err(protocol)?;
        let frame = result
            .outgoing_frame
            .ok_or_else(|| LinkError::Protocol("no frame".into()))?;
        self.write_frame(&frame).await
    }

    /// The next user data from the new device (a `d2d_join.NdToEd`). Cancel-safe between
    /// WebSocket messages.
    pub async fn receive(&mut self) -> Result<Vec<u8>, LinkError> {
        loop {
            let chunk = self.socket.receive().await?;
            self.protocol
                .add_chunks(self.pid, &[&chunk])
                .map_err(protocol)?;
            if let Some(data) = self
                .protocol
                .process_frame(self.pid)
                .map_err(protocol)?
                .and_then(|result| result.incoming_ulp_data)
            {
                return Ok(data);
            }
        }
    }

    /// The three emoji indices both devices show (Android: `rph[0..2] % 128`).
    #[must_use]
    pub fn emoji_indices(&self) -> [usize; 3] {
        [
            usize::from(self.rph[0] % 128),
            usize::from(self.rph[1] % 128),
            usize::from(self.rph[2] % 128),
        ]
    }

    pub async fn close(self) {
        let _ = timeout(CLOSE_TIMEOUT, self.socket.close()).await;
    }
}

/// A code as Threema for Desktop shows it, with this D2D protocol version.
#[cfg(test)]
pub(crate) fn test_join_code(d2d_protocol_version: u32) -> String {
    tests::code(
        Type::RequestToJoin(libthreema::protobuf::common::Unit {}),
        d2d_protocol_version,
        "wss://rendezvous-1.threema.ch/11/abc",
    )
}

#[cfg(test)]
mod tests {
    use libthreema::protobuf::{
        common::Unit,
        d2d_rendezvous::{RendezvousInit, rendezvous_init::RelayedWebSocket},
        url::device_group_join_request_or_offer::Variant,
    };

    use super::*;
    use libthreema::d2d_rendezvous::{RendezvousOutgoingFrame, RendezvousPathStateUpdate};

    pub(super) fn code(variant: Type, version: u32, url: &str) -> String {
        let offer = DeviceGroupJoinRequestOrOffer {
            version: 0,
            d2d_protocol_version: version,
            variant: Some(Variant {
                r#type: Some(variant),
            }),
            rendezvous_init: Some(RendezvousInit {
                version: 0,
                ak: vec![1; 32],
                relayed_web_socket: Some(RelayedWebSocket {
                    path_id: 1,
                    network_cost: 0,
                    url: url.into(),
                }),
                direct_tcp_server: None,
            }),
        };
        format!(
            "{JOIN_PREFIX}{}",
            BASE64URL_NOPAD.encode(&offer.encode_to_vec())
        )
    }

    #[test]
    fn join_codes() {
        let url = "wss://rendezvous-1.threema.ch/11/abc";
        let good = code(
            Type::RequestToJoin(Unit {}),
            ProtocolVersion::V02 as u32,
            url,
        );
        let offer = parse_join_code(&good).unwrap();
        assert_eq!(
            offer
                .rendezvous_init
                .unwrap()
                .relayed_web_socket
                .unwrap()
                .url,
            url
        );
        assert!(matches!(
            parse_join_code(&code(
                Type::OfferToJoin(Unit {}),
                ProtocolVersion::V02 as u32,
                url
            )),
            Err(LinkError::NotJoinCode)
        ));
        assert!(matches!(
            parse_join_code(&code(
                Type::RequestToJoin(Unit {}),
                ProtocolVersion::V01 as u32,
                url
            )),
            Err(LinkError::Version)
        ));
        assert!(matches!(
            parse_join_code(&code(
                Type::RequestToJoin(Unit {}),
                ProtocolVersion::V02 as u32,
                "ws://x"
            )),
            Err(LinkError::NotJoinCode)
        ));
        assert!(matches!(
            parse_join_code("3mid:ECHOECHO,abc"),
            Err(LinkError::NotJoinCode)
        ));
        let mut web = vec![0, 2, 3];
        web.extend_from_slice(&[7; 110]);
        assert!(matches!(
            parse_join_code(&data_encoding::BASE64.encode(&web)),
            Err(LinkError::WebCode)
        ));
    }

    /// Threema for Desktop's side of the relay: the new device, the rendezvous initiator, which
    /// leaves the nomination to us.
    struct NewDevice {
        socket: Datagrams,
        protocol: RendezvousProtocol,
        pid: u32,
        rph: [u8; 32],
    }

    impl NewDevice {
        async fn write(socket: &mut Datagrams, frame: &RendezvousOutgoingFrame) {
            let (header, payload) = frame.encode();
            socket.send([&header[..], payload].concat()).await.unwrap();
        }

        /// Up to the nomination; an error ends it as an app would, closing the connection.
        async fn handshake(mut socket: Datagrams, ak: [u8; 32], pid: u32) -> Result<Self, String> {
            let mut protocol =
                RendezvousProtocol::new_as_rid(false, RendezvousAuthenticationKey(ak), &[pid]);
            loop {
                let chunk = socket.receive().await.map_err(|error| error.to_string())?;
                protocol
                    .add_chunks(pid, &[&chunk])
                    .map_err(|error| error.to_string())?;
                while let Some(result) = protocol
                    .process_frame(pid)
                    .map_err(|error| error.to_string())?
                {
                    if let Some(frame) = &result.outgoing_frame {
                        Self::write(&mut socket, frame).await;
                    }
                    if let Some(RendezvousPathStateUpdate::Nominated { rph }) = result.state_update
                    {
                        return Ok(Self {
                            socket,
                            protocol,
                            pid,
                            rph: rph.0,
                        });
                    }
                }
            }
        }

        async fn receive(&mut self) -> Vec<u8> {
            loop {
                let chunk = self.socket.receive().await.unwrap();
                self.protocol.add_chunks(self.pid, &[&chunk]).unwrap();
                while let Some(result) = self.protocol.process_frame(self.pid).unwrap() {
                    if let Some(data) = result.incoming_ulp_data {
                        return data;
                    }
                }
            }
        }

        async fn send(&mut self, data: Vec<u8>) {
            let frame = self
                .protocol
                .create_ulp_frame(data)
                .unwrap()
                .outgoing_frame
                .unwrap();
            Self::write(&mut self.socket, &frame).await;
        }
    }

    /// Both devices end up on the same path with the same emojis, and data goes both ways, the
    /// essential data in chunks.
    #[tokio::test(flavor = "current_thread")]
    async fn linking_runs_both_ways_over_the_relay() {
        let (ours, theirs) = crate::websocket::loopback().await;
        let (link, device) = tokio::join!(
            RendezvousLink::over(ours, [3; 32], 7),
            NewDevice::handshake(theirs, [3; 32], 7)
        );
        let (mut link, mut device) = (link.unwrap(), device.unwrap());
        assert_eq!(link.rph, device.rph);
        assert_eq!(
            link.emoji_indices(),
            [0, 1, 2].map(|i| usize::from(device.rph[i] % 128))
        );

        let essential: Vec<u8> = (0..CHUNK * 5 / 2).map(|i| (i % 251) as u8).collect();
        // The limit only keeps a defect from hanging the test run.
        let ((), received) = timeout(
            Duration::from_secs(120),
            futures_util::future::join(
                async { link.send(essential.clone()).await.unwrap() },
                device.receive(),
            ),
        )
        .await
        .expect("the essential data never arrived whole");
        assert!(received == essential, "the essential data, whole");

        device.send(b"registered".to_vec()).await;
        assert_eq!(link.receive().await.unwrap(), b"registered");
    }

    /// A new device with another key (a code from somewhere else) gets nowhere.
    #[tokio::test(flavor = "current_thread")]
    async fn another_key_ends_the_linking() {
        let (ours, theirs) = crate::websocket::loopback().await;
        let (link, device) = tokio::join!(
            RendezvousLink::over(ours, [3; 32], 7),
            NewDevice::handshake(theirs, [4; 32], 7)
        );
        assert!(device.is_err());
        let error = link.err().expect("no link");
        assert!(matches!(error, LinkError::Socket(_)), "{error}");
    }
}
