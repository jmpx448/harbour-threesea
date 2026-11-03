// SPDX-License-Identifier: AGPL-3.0-only
//! WebSockets to Threema's mediator and rendezvous servers (multi-device, ADR 0011) and to
//! Threema Web's signalling server (ADR 0013): binary datagrams over the same TLS as every HTTPS
//! request (`https::tls_config`: Threema hosts only, pinned), or for a self-hosted Threema Web
//! server the user allowed, `https::tofu_tls_config`.

use std::sync::Arc;

use futures_util::{SinkExt as _, StreamExt as _};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
    tungstenite::{Message, client::IntoClientRequest as _, protocol::WebSocketConfig},
};

/// Largest datagram we accept (D2M envelopes are at most 64 KiB; rendezvous chunks 1 MiB).
const MAX_MESSAGE: usize = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WebSocketError {
    #[error("not a wss:// URL")]
    NotSecure,
    #[error("not a Threema host")]
    NotThreema,
    #[error("TLS setup: {0}")]
    Tls(String),
    #[error("websocket: {0}")]
    Socket(#[from] tokio_tungstenite::tungstenite::Error),
    /// Closed by the server, with its close code (e.g. the mediator's 4xxx codes).
    #[error("closed ({code:?}): {reason}")]
    Closed { code: Option<u16>, reason: String },
    #[error("unexpected text message")]
    Text,
}

/// A connected WebSocket carrying binary datagrams.
pub struct Datagrams {
    stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl Datagrams {
    /// Connect to `url` (must be `wss://`; the TLS configuration only lets Threema hosts through).
    pub async fn connect(url: &str) -> Result<Self, WebSocketError> {
        // Not even a connection attempt to other hosts (e.g. a forged join code's relay).
        if !crate::https::is_threema_url(url) {
            return Err(WebSocketError::NotThreema);
        }
        let tls = crate::https::tls_config().map_err(WebSocketError::Tls)?;
        Self::connect_with(url, None, tls).await
    }

    /// Connect to `url` (`wss://`) with this TLS configuration, asking for `subprotocol`.
    pub async fn connect_with(
        url: &str,
        subprotocol: Option<&str>,
        tls: rustls::ClientConfig,
    ) -> Result<Self, WebSocketError> {
        if !url.starts_with("wss://") {
            return Err(WebSocketError::NotSecure);
        }
        let mut request = url.into_client_request()?;
        if let Some(subprotocol) = subprotocol {
            request.headers_mut().insert(
                "Sec-WebSocket-Protocol",
                subprotocol
                    .parse()
                    .map_err(|_| WebSocketError::Tls("invalid subprotocol".into()))?,
            );
        }
        let mut config = WebSocketConfig::default();
        config.max_message_size = Some(MAX_MESSAGE);
        config.max_frame_size = Some(MAX_MESSAGE);
        let (stream, _response) = connect_async_tls_with_config(
            request,
            Some(config),
            true,
            Some(Connector::Rustls(Arc::new(tls))),
        )
        .await?;
        Ok(Self { stream })
    }

    pub async fn send(&mut self, datagram: Vec<u8>) -> Result<(), WebSocketError> {
        self.stream.send(Message::Binary(datagram.into())).await?;
        Ok(())
    }

    /// The next binary datagram (pings are answered by tungstenite while reading).
    pub async fn receive(&mut self) -> Result<Vec<u8>, WebSocketError> {
        loop {
            let Some(message) = self.stream.next().await else {
                return Err(WebSocketError::Closed {
                    code: None,
                    reason: "end of stream".into(),
                });
            };
            match message? {
                Message::Binary(bytes) => return Ok(bytes.to_vec()),
                Message::Text(_) => return Err(WebSocketError::Text),
                Message::Close(frame) => {
                    return Err(WebSocketError::Closed {
                        code: frame.as_ref().map(|f| u16::from(f.code)),
                        reason: frame.map(|f| f.reason.to_string()).unwrap_or_default(),
                    });
                }
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }

    /// Close normally (errors don't matter any more).
    pub async fn close(mut self) {
        let _ = self.stream.close(None).await;
    }
}
