// SPDX-License-Identifier: AGPL-3.0-only
//! The QR code Threema Web shows for a new session (threema-web `docs/qr_code.md`, Android's
//! `WebSessionQRCodeParserImpl`): Base64 of protocol version (u16, 2), options (bit 0
//! self-hosted, bit 1 permanent), the browser's permanent public key, the SaltyRTC auth token,
//! the server's permanent public key (all zero: none), port (u16) and host (UTF-8, the rest).

use data_encoding::BASE64;
use zeroize::Zeroizing;

/// The only protocol version (Android's `Protocol.PROTOCOL_VERSION`).
pub const PROTOCOL_VERSION: u16 = 2;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum QrError {
    #[error("not a Threema Web code")]
    Invalid,
    #[error("Threema Web protocol version {0} is not supported")]
    Version(u16),
}

/// A parsed Threema Web QR code.
pub struct WebQr {
    pub self_hosted: bool,
    /// The user asked the browser to remember the session (it will send `keyPersisted`).
    pub permanent: bool,
    pub initiator_key: [u8; 32],
    pub auth_token: Zeroizing<[u8; 32]>,
    pub server_key: Option<[u8; 32]>,
    pub host: String,
    pub port: u16,
}

impl std::fmt::Debug for WebQr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebQr")
            .field("self_hosted", &self.self_hosted)
            .field("host", &self.host)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

/// Parse the scanned text.
///
/// # Errors
/// Not a Threema Web code, or another protocol version.
pub fn parse(text: &str) -> Result<WebQr, QrError> {
    let bytes = BASE64
        .decode(text.trim().as_bytes())
        .map_err(|_| QrError::Invalid)?;
    if bytes.len() < 102 {
        return Err(QrError::Invalid);
    }
    let version = u16::from_be_bytes([bytes[0], bytes[1]]);
    if version != PROTOCOL_VERSION {
        return Err(QrError::Version(version));
    }
    let options = bytes[2];
    let key = |at: usize| -> [u8; 32] { bytes[at..at + 32].try_into().unwrap_or([0; 32]) };
    let server_key = key(67);
    let host = std::str::from_utf8(&bytes[101..])
        .map_err(|_| QrError::Invalid)?
        .to_owned();
    if host.is_empty() || host.contains(['/', ':', '@', '?', '#', ' ']) {
        return Err(QrError::Invalid);
    }
    Ok(WebQr {
        self_hosted: options & 0x01 != 0,
        permanent: options & 0x02 != 0,
        initiator_key: key(3),
        auth_token: Zeroizing::new(key(35)),
        server_key: (server_key != [0; 32]).then_some(server_key),
        port: u16::from_be_bytes([bytes[99], bytes[100]]),
        host,
    })
}

/// A host under threema.ch: connected with Threema's pins only, never overridden (ADR 0013).
#[must_use]
pub fn is_threema_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "threema.ch" || host.ends_with(".threema.ch")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(version: u16, options: u8, host: &str) -> String {
        let mut bytes = version.to_be_bytes().to_vec();
        bytes.push(options);
        bytes.extend([1; 32]);
        bytes.extend([2; 32]);
        bytes.extend([0; 32]);
        bytes.extend(443u16.to_be_bytes());
        bytes.extend(host.as_bytes());
        BASE64.encode(&bytes)
    }

    #[test]
    fn parses_a_code() {
        let qr = parse(&code(2, 0x02, "saltyrtc-01.threema.ch")).unwrap();
        assert!(qr.permanent && !qr.self_hosted);
        assert_eq!(qr.initiator_key, [1; 32]);
        assert_eq!(*qr.auth_token, [2; 32]);
        assert_eq!(qr.server_key, None);
        assert_eq!((qr.host.as_str(), qr.port), ("saltyrtc-01.threema.ch", 443));
        assert!(is_threema_host(&qr.host));
    }

    #[test]
    fn refuses_bad_codes() {
        assert_eq!(
            parse(&code(1, 0, "x.threema.ch")).unwrap_err(),
            QrError::Version(1)
        );
        assert_eq!(parse("hello").unwrap_err(), QrError::Invalid);
        assert_eq!(
            parse(&code(2, 0, "evil/path")).unwrap_err(),
            QrError::Invalid
        );
        assert!(!is_threema_host("threema.ch.example.com"));
        assert!(!is_threema_host("notthreema.ch"));
    }
}
