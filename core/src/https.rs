// SPDX-License-Identifier: AGPL-3.0-only
//! HTTPS client for libthreema's [`HttpsRequest`]s and our own directory calls.
//!
//! Security properties (CLAUDE.md §7):
//! - Only Threema hosts are reachable at all (`threema.ch`, `threema.com` and subdomains); any
//!   other host fails the TLS handshake.
//! - Normal WebPKI validation (Mozilla roots) **and** SPKI pinning of the server certificate,
//!   using upstream's pin sets (`app/src/main/res/xml/network_security_config.xml`).
//! - Redirects are followed only to Threema hosts, at most [`MAX_REDIRECTS`] of them: another
//!   host doesn't even get a DNS lookup or a TLS greeting.

use std::sync::Arc;

use data_encoding::BASE64;
use libthreema::https::{HttpsError, HttpsMethod, HttpsRequest, HttpsResponse, HttpsResult};
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use sha2::{Digest as _, Sha256};

/// SHA-256 SPKI pins per domain (subdomains included), copied from upstream 6.5.3-1216
/// `app/src/main/res/xml/network_security_config.xml`. The most specific domain wins.
const PINS: &[(&str, &[&str])] = &[
    (
        "sfu.threema.ch",
        &[
            "useMPV2qPBEgxVucMPuqexG27L64zFAksHh9BehZpY0=",
            "88JttF0tDWrGT6g8H9uEZ0T8xosvZtZwWlsZuD4NvHA=",
            "F82gDLif130AsVx454ZsMxPGl9EpzB5LqY39CzVKWDQ=",
            "Jo4Re5X+mksn/Ankgrnov07caZwkkT8NezJMQf1i8cI=",
        ],
    ),
    (
        "test.threema.ch",
        &[
            "Dvvb0s1E8Y2tG67GoyzIkmJNAebvOqQjz4TnPMq3yhI=",
            "/mH2CVdSxVBMuKiRHplBP2GTFWs/l+KXHriAxRFNsaw=",
            "bmsGGaIGEs1HWAUIE082NSPYTURvChScDmE7PjL+5RQ=",
        ],
    ),
    (
        "threema.ch",
        &[
            "vGQZ8hm2h+km+q7rnJ7kF9S17BwSY0rbhwjz6nIupf0=",
            "3L0bKTNfTwVUCjYqqhZXJIO03qC00bSnuxZFsb09OUo=",
            "NN6Lb+2AE7CN3HWZKoWOe6mmHROOnywWoKZYWL1oHIU=",
            "efJCZofFPR3oV/bBk0wmehqnhy3Vv+s9P+3sjhHem/E=",
            "2Vpy8qUQCqc2+Lg6BgRO8G6e6vh7NmvVHTljfwP/Pfk=",
            "KKBJHJn1PQSdNTmoAfhxqWTO61r8O8bPi/JeGtP/6gg=",
        ],
    ),
    (
        "threema.com",
        &[
            "saKwtUPx8bCj9CW+c55nU2jb4aOpr0vBD8XMJveXq34=",
            "nZWRY8rNSEqxjQDQjaunWlUL+YBOTK1xN5Bb0wMq/K0=",
            "5AfgU7xFqhx5AS69cQZlAGv6JpmLm0A+Z6yBrLPOCP8=",
            "GGSYKwkV3h6SRIY16Ixsh8LEKuGuhx3B4CamRde4xgY=",
            "oD0JGdy32wZtoUT4n9ac3HLOgEosnx2kq+qaJnmtsQk=",
            "j4n4RTr0MLfQ3gBmONIFreXDq5/Kkb2oquVTmq0n5pI=",
        ],
    ),
];

/// The largest response body read: an encrypted blob of the largest size, with room to spare.
pub(crate) const MAX_RESPONSE: usize = crate::media::MAX_BLOB_SIZE + 64 * 1024;

/// Redirects followed for one request (each to a Threema host).
const MAX_REDIRECTS: usize = 5;

/// May a request that was redirected `hops` times so far go on to `target`?
fn redirect_allowed(target: &str, hops: usize) -> bool {
    hops < MAX_REDIRECTS && is_threema_url(target)
}

/// Whether `url` (https:// or wss://) names a Threema host. Checked before connecting: the
/// certificate check would refuse any other host too, but only after DNS, TCP and the TLS
/// greeting already reached it (security audit 2026-10-27, CLAUDE.md §7.6).
pub(crate) fn is_threema_url(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(pins_for_host))
        .is_some_and(|pins| pins.is_some())
}

/// Pin set for a host, or `None` if the host is not a Threema host (and must not be contacted).
fn pins_for_host(host: &str) -> Option<&'static [&'static str]> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    PINS.iter()
        .find(|(domain, _)| {
            host == *domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
        .map(|(_, pins)| *pins)
}

/// Base64 SHA-256 of the certificate's DER-encoded SubjectPublicKeyInfo.
fn spki_sha256(cert: &CertificateDer<'_>) -> Result<String, rustls::Error> {
    let ee = webpki::EndEntityCert::try_from(cert)
        .map_err(|_| rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
    Ok(BASE64.encode(&Sha256::digest(ee.subject_public_key_info().as_ref())))
}

#[derive(Debug)]
struct PinningVerifier {
    webpki: Arc<WebPkiServerVerifier>,
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let ServerName::DnsName(host) = server_name else {
            return Err(rustls::Error::General("Not a Threema host".into()));
        };
        let pins = pins_for_host(host.as_ref())
            .ok_or_else(|| rustls::Error::General("Not a Threema host".into()))?;
        self.webpki.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;
        let spki = spki_sha256(end_entity)?;
        if pins.contains(&spki.as_str()) {
            Ok(ServerCertVerified::assertion())
        } else {
            tracing::error!(host = host.as_ref(), "TLS pin mismatch");
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.webpki.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.webpki.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.webpki.supported_verify_schemes()
    }
}

/// User agent for every request: our own name first, as a third-party client, then a compatibility
/// token that the blob server expects.
pub fn user_agent() -> String {
    format!(
        "Threesea/{} (Threema compatible; Sailfish OS)",
        crate::VERSION
    )
}

/// HTTPS client restricted to Threema hosts with certificate pinning.
/// Android's "IPv6 for messages" off: Threema's IPv4-only hosts instead of the dual-stack ones
/// (`ds-apip`, `ds-blobp-…`, `ds.g-…`) that libthreema and we use by default.
pub static IPV4_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `url` (or a bare host name) for the IP version chosen in [`IPV4_ONLY`]: the dual-stack hosts
/// lose their `ds-` / `ds.` prefix. Other hosts (Threema Safe, …) stay as they are.
#[must_use]
pub fn server_url(url: &str) -> std::borrow::Cow<'_, str> {
    if !IPV4_ONLY.load(std::sync::atomic::Ordering::Relaxed) {
        return url.into();
    }
    let (scheme, rest) = url.split_at(url.find("://").map_or(0, |i| i + 3));
    match rest
        .strip_prefix("ds-")
        .or_else(|| rest.strip_prefix("ds."))
    {
        Some(host)
            if host
                .split(['/', ':'])
                .next()
                .is_some_and(|h| h.ends_with(".threema.ch")) =>
        {
            format!("{scheme}{host}").into()
        }
        _ => url.into(),
    }
}

/// The TLS configuration for every connection to Threema (HTTPS and WebSockets): WebPKI and
/// the pins above, so only Threema hosts can be reached.
///
/// # Errors
/// Only if the configuration cannot be built (a programming error).
pub fn tls_config() -> Result<rustls::ClientConfig, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = Arc::new(rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    });
    let webpki = WebPkiServerVerifier::builder_with_provider(roots, Arc::clone(&provider))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinningVerifier { webpki }))
        .with_no_client_auth())
}

/// Trust on first use for a self-hosted Threema Web server the user allowed (ADR 0013; never
/// for Threema's own hosts): a valid WebPKI certificate whose public key matches `expected`
/// (`None`: the first connection, nothing pinned yet). The key presented ends up in `seen`, so a
/// first connection can pin it and a mismatch can be shown to the user.
#[derive(Debug)]
struct TofuVerifier {
    webpki: Arc<WebPkiServerVerifier>,
    expected: Option<String>,
    seen: Arc<std::sync::Mutex<Option<String>>>,
}

impl ServerCertVerifier for TofuVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if let ServerName::DnsName(host) = server_name
            && pins_for_host(host.as_ref()).is_some()
        {
            // Threema's hosts only ever go through their built-in pins.
            return Err(rustls::Error::General(
                "Threema host without its pins".into(),
            ));
        }
        self.webpki.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;
        let spki = spki_sha256(end_entity)?;
        *self
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(spki.clone());
        match &self.expected {
            Some(expected) if *expected != spki => Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            )),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.webpki.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.webpki.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.webpki.supported_verify_schemes()
    }
}

/// TLS for a self-hosted Threema Web server the user allowed (see [`TofuVerifier`]).
///
/// # Errors
/// Only if the configuration cannot be built (a programming error).
pub fn tofu_tls_config(
    expected: Option<String>,
    seen: Arc<std::sync::Mutex<Option<String>>>,
) -> Result<rustls::ClientConfig, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = Arc::new(rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    });
    let webpki = WebPkiServerVerifier::builder_with_provider(roots, Arc::clone(&provider))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TofuVerifier {
            webpki,
            expected,
            seen,
        }))
        .with_no_client_auth())
}

const RESPONSE_TOO_LARGE: &str = "response too large";

/// Whether a request failed because the answer was over its limit.
pub fn is_too_large(error: &HttpsError) -> bool {
    matches!(error, HttpsError::InvalidResponse(message) if message == RESPONSE_TOO_LARGE)
}

/// A response body that may not grow past its limit: refused up front if the server says it
/// would, else as soon as a chunk takes it there, so what a server sends can't fill the memory.
struct LimitedBody {
    bytes: Vec<u8>,
    limit: usize,
}

impl LimitedBody {
    fn new(content_length: Option<u64>, limit: usize) -> Result<Self, HttpsError> {
        if content_length.is_some_and(|length| length > limit as u64) {
            return Err(HttpsError::InvalidResponse(RESPONSE_TOO_LARGE.into()));
        }
        // Told the size, it needn't grow by copying: a large blob would be there nearly twice.
        let announced = content_length.map_or(0, |length| usize::try_from(length).unwrap_or(limit));
        Ok(Self {
            bytes: Vec::with_capacity(announced),
            limit,
        })
    }

    fn push(&mut self, chunk: &[u8]) -> Result<(), HttpsError> {
        if self.bytes.len() + chunk.len() > self.limit {
            return Err(HttpsError::InvalidResponse(RESPONSE_TOO_LARGE.into()));
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }
}

/// Connecting to a server (name lookup, TCP, TLS), whatever the request may take after that.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long data sent may go unacknowledged before the system gives the connection up. reqwest's
/// 30 s would end an upload, to start again in full, at the first blackout of that length, so it
/// is longer here than a blob's stall limit, which is what decides.
const TCP_USER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// How much of a transfer's body is handed over at a time. Taking the next piece says the connection
/// took the last one, into the system's send buffer: that it has left the phone is more than we can
/// see, so a very slow link can fail a stall limit (docs/protocol-notes.md, "Blob server").
const BODY_PIECE: usize = 64 * 1024;

/// How long the answer to a body may take once the connection took all of it. Longer than a stall:
/// the system's send buffer still has to go out over what may be a slow link, and the server stores
/// the blob before it answers.
const ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(300);

/// By when a transfer has to have moved again.
#[derive(Clone)]
struct Progress(Arc<std::sync::Mutex<tokio::time::Instant>>);

impl Progress {
    fn new(within: std::time::Duration) -> Self {
        Self(Arc::new(std::sync::Mutex::new(
            tokio::time::Instant::now() + within,
        )))
    }

    fn moved(&self, within: std::time::Duration) {
        if let Ok(mut deadline) = self.0.lock() {
            *deadline = tokio::time::Instant::now() + within;
        }
    }

    fn deadline(&self) -> tokio::time::Instant {
        self.0
            .lock()
            .map_or_else(|_| tokio::time::Instant::now(), |deadline| *deadline)
    }
}

/// A request body that reports each piece it hands over. Its exact size is known, so it goes out
/// with a `content-length` as any other. It can be sent only once: reqwest hands a 307 or 308
/// back as the response instead of following it, and doesn't send it again by itself on an HTTP/2
/// refused stream, so that one fails as a network error does.
struct ProgressBody {
    rest: bytes::Bytes,
    progress: Progress,
    stall: std::time::Duration,
    answer_wait: std::time::Duration,
}

impl http_body::Body for ProgressBody {
    type Data = bytes::Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<bytes::Bytes>, Self::Error>>> {
        let this = self.get_mut();
        if this.rest.is_empty() {
            return std::task::Poll::Ready(None);
        }
        let piece = this.rest.split_to(BODY_PIECE.min(this.rest.len()));
        this.progress.moved(if this.rest.is_empty() {
            this.answer_wait
        } else {
            this.stall
        });
        std::task::Poll::Ready(Some(Ok(http_body::Frame::data(piece))))
    }

    fn is_end_stream(&self) -> bool {
        self.rest.is_empty()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        http_body::SizeHint::with_exact(self.rest.len() as u64)
    }
}

/// Run a request for as long as it keeps moving: it ends with a timeout once nothing went out or
/// came in for `stall` (`answer_wait`, not shorter, once the body is out), however far it got and
/// however long it took so far.
async fn until_stalled(
    builder: reqwest::RequestBuilder,
    body: Vec<u8>,
    limit: usize,
    stall: std::time::Duration,
    answer_wait: std::time::Duration,
) -> HttpsResult {
    let progress = Progress::new(stall);
    let builder = if body.is_empty() {
        builder
    } else {
        builder.body(reqwest::Body::wrap(ProgressBody {
            rest: body.into(),
            progress: progress.clone(),
            stall,
            answer_wait,
        }))
    };
    let moved = progress.clone();
    let exchange = async move {
        let mut response = builder.send().await.map_err(map_error)?;
        moved.moved(stall);
        let status = response.status().as_u16();
        let mut body = LimitedBody::new(response.content_length(), limit)?;
        while let Some(chunk) = response.chunk().await.map_err(map_error)? {
            moved.moved(stall);
            body.push(&chunk)?;
        }
        Ok(HttpsResponse {
            status,
            body: body.bytes,
        })
    };
    tokio::pin!(exchange);
    loop {
        tokio::select! {
            result = &mut exchange => return result,
            () = tokio::time::sleep_until(progress.deadline()) => {
                if progress.deadline() <= tokio::time::Instant::now() {
                    return Err(HttpsError::Timeout("no progress".into()));
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct HttpsClient {
    client: reqwest::Client,
}

impl HttpsClient {
    /// Build the client.
    ///
    /// # Errors
    /// Fails only if the TLS configuration cannot be built (a programming error).
    pub fn new() -> Result<Self, HttpsError> {
        let tls = tls_config().map_err(HttpsError::InvalidRequest)?;
        let client = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .user_agent(user_agent())
            .https_only(true)
            .connect_timeout(CONNECT_TIMEOUT)
            .tcp_user_timeout(TCP_USER_TIMEOUT)
            // Stopping hands the redirect back as the response: an unexpected status for callers.
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if redirect_allowed(attempt.url().as_str(), attempt.previous().len()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()
            .map_err(|e| HttpsError::InvalidRequest(e.to_string()))?;
        Ok(Self { client })
    }

    /// Execute a libthreema [`HttpsRequest`], reading up to the largest answer Threema's servers
    /// give (a blob), not whatever a server sends.
    pub async fn send(&self, request: HttpsRequest) -> HttpsResult {
        self.send_limited(request, MAX_RESPONSE).await
    }

    /// Like [`Self::send`], refusing an answer of more than `limit` bytes while it comes in; see
    /// [`is_too_large`].
    pub async fn send_limited(&self, request: HttpsRequest, limit: usize) -> HttpsResult {
        #[cfg(test)]
        stalled_if_asked().await;
        let builder = self
            .builder(request.method, &request.url, request.headers)?
            .timeout(request.timeout)
            .body(request.body);
        let mut response = builder.send().await.map_err(map_error)?;
        let status = response.status().as_u16();
        let mut body = LimitedBody::new(response.content_length(), limit)?;
        while let Some(chunk) = response.chunk().await.map_err(map_error)? {
            body.push(&chunk)?;
        }
        Ok(HttpsResponse {
            status,
            body: body.bytes,
        })
    }

    /// Like [`Self::send_limited`], for a transfer that takes as long as the link needs:
    /// `request.timeout` is how long nothing may move, not how long the whole may take.
    pub async fn transfer(&self, request: HttpsRequest, limit: usize) -> HttpsResult {
        #[cfg(test)]
        stalled_if_asked().await;
        let builder = self.builder(request.method, &request.url, request.headers)?;
        until_stalled(builder, request.body, limit, request.timeout, ANSWER_WAIT).await
    }

    fn builder(
        &self,
        method: HttpsMethod,
        url: &str,
        headers: Vec<libthreema::https::HttpsHeader>,
    ) -> Result<reqwest::RequestBuilder, HttpsError> {
        let method = match method {
            HttpsMethod::Delete => reqwest::Method::DELETE,
            HttpsMethod::Get => reqwest::Method::GET,
            HttpsMethod::Post => reqwest::Method::POST,
            HttpsMethod::Put => reqwest::Method::PUT,
        };
        let url = server_url(url);
        if !is_threema_url(&url) {
            return Err(HttpsError::InvalidRequest("not a Threema host".into()));
        }
        let mut builder = self.client.request(method, url.as_ref());
        for header in headers {
            // We send our own user agent instead of libthreema's.
            if header.name.eq_ignore_ascii_case("user-agent") {
                continue;
            }
            builder = builder.header(header.name, header.value);
        }
        Ok(builder)
    }

    /// POST a JSON body and return the response (for directory endpoints libthreema lacks).
    pub async fn post_json(&self, url: &str, body: &serde_json::Value) -> HttpsResult {
        self.send(HttpsRequest {
            timeout: std::time::Duration::from_secs(20),
            url: url.to_owned(),
            method: HttpsMethod::Post,
            headers: vec![
                libthreema::https::HttpsHeader {
                    name: "content-type".into(),
                    value: "application/json".into(),
                },
                libthreema::https::HttpsHeader {
                    name: "accept".into(),
                    value: "application/json".into(),
                },
            ],
            body: serde_json::to_vec(body)
                .map_err(|e| HttpsError::InvalidRequest(e.to_string()))?,
        })
        .await
    }
}

fn map_error(error: reqwest::Error) -> HttpsError {
    // Never include URLs with query data in errors that may reach logs: strip them.
    let kind: fn(String) -> HttpsError = if error.is_timeout() {
        HttpsError::Timeout
    } else if error.is_builder() || error.is_request() {
        HttpsError::InvalidRequest
    } else if error.is_body() || error.is_decode() {
        HttpsError::InvalidResponse
    } else {
        HttpsError::Unreachable
    };
    kind(error.without_url().to_string())
}

#[cfg(test)]
thread_local! {
    static STALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// From now on this thread's requests get no answer, as on a mobile link that has stalled.
#[cfg(test)]
pub(crate) fn stall_requests() {
    STALLED.set(true);
}

#[cfg(test)]
async fn stalled_if_asked() {
    if STALLED.get() {
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_threema_hosts_have_pins() {
        assert!(pins_for_host("apip.threema.ch").is_some());
        assert!(pins_for_host("ds-apip.threema.ch.").is_some());
        assert!(pins_for_host("threema.com").is_some());
        assert_eq!(pins_for_host("sfu.threema.ch").unwrap().len(), 4);
        assert_eq!(pins_for_host("a.sfu.threema.ch").unwrap().len(), 4);
        assert_eq!(pins_for_host("safe-12.threema.ch").unwrap().len(), 6);
        assert!(pins_for_host("evilthreema.ch").is_none());
        assert!(is_threema_url("wss://rendezvous.threema.ch/abc"));
        assert!(is_threema_url("https://blob-mirror.threema.ch/x"));
        assert!(!is_threema_url("wss://rendezvous.example.com/abc"));
        assert!(!is_threema_url("https://threema.ch.example.com/"));
        assert!(!is_threema_url("https://user@example.com"));
        assert!(!is_threema_url("not a url"));
        assert!(pins_for_host("threema.ch.evil.example").is_none());
        assert!(pins_for_host("example.org").is_none());
    }

    #[test]
    fn a_response_of_a_known_size_needs_no_growing() {
        let mut body = LimitedBody::new(Some(1000), 1000).unwrap();
        assert!(body.bytes.capacity() >= 1000);
        let room = body.bytes.as_ptr();
        body.push(&[1; 400]).unwrap();
        body.push(&[2; 600]).unwrap();
        assert_eq!(body.bytes.as_ptr(), room, "it stayed where it was");
    }

    #[test]
    fn a_response_stops_at_its_limit() {
        // Said up front: not one byte is read.
        assert!(is_too_large(&LimitedBody::new(Some(11), 10).err().unwrap()));
        // Not said, or wrongly: refused once the chunks pass it.
        let mut body = LimitedBody::new(None, 10).unwrap();
        body.push(&[0; 6]).unwrap();
        body.push(&[0; 4]).unwrap();
        assert!(is_too_large(&body.push(&[0]).unwrap_err()));
        assert_eq!(body.bytes.len(), 10);
        let mut body = LimitedBody::new(Some(1), 10).unwrap();
        assert!(is_too_large(&body.push(&[0; 11]).unwrap_err()));
        assert!(!is_too_large(&HttpsError::InvalidResponse("other".into())));
    }

    /// A local server without TLS, as the pinned client reaches none: the limits are the same.
    async fn serve<F, Fut>(handle: F) -> (reqwest::RequestBuilder, reqwest::RequestBuilder)
    where
        F: FnOnce(tokio::net::TcpStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/blob", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle(stream).await;
        });
        let client = reqwest::Client::builder()
            .tls_backend_preconfigured(tls_config().unwrap())
            .build()
            .unwrap();
        (client.get(&url), client.post(&url))
    }

    /// The request's head; the content that came with it stays in `rest`.
    async fn read_head(stream: &mut tokio::net::TcpStream, rest: &mut Vec<u8>) -> String {
        use tokio::io::AsyncReadExt as _;
        loop {
            if let Some(end) = rest.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&rest[..end]).to_lowercase();
                rest.drain(..end + 4);
                return head;
            }
            let mut piece = [0; 4096];
            let read = stream.read(&mut piece).await.unwrap();
            assert_ne!(read, 0);
            rest.extend_from_slice(&piece[..read]);
        }
    }

    async fn read_body(stream: &mut tokio::net::TcpStream, mut got: usize, length: usize) {
        use tokio::io::AsyncReadExt as _;
        let mut piece = vec![0; 1024 * 1024];
        while got < length {
            let read = stream.read(&mut piece).await.unwrap();
            assert_ne!(read, 0);
            got += read;
        }
    }

    const SECOND: std::time::Duration = std::time::Duration::from_secs(1);
    const OK: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";

    #[tokio::test]
    async fn a_download_may_take_longer_than_its_stall_limit() {
        use tokio::io::AsyncWriteExt as _;
        let (get, _) = serve(|mut stream| async move {
            read_head(&mut stream, &mut Vec::new()).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\n")
                .await
                .unwrap();
            for _ in 0..6 {
                stream.write_all(b"x").await.unwrap();
                tokio::time::sleep(SECOND / 2).await;
            }
        })
        .await;
        let started = tokio::time::Instant::now();
        let response = until_stalled(get, Vec::new(), 100, 2 * SECOND, SECOND)
            .await
            .unwrap();
        assert_eq!(response.body, b"xxxxxx");
        assert!(started.elapsed() > 2 * SECOND);
    }

    #[tokio::test]
    async fn a_download_that_stops_moving_ends() {
        use tokio::io::AsyncWriteExt as _;
        let (get, _) = serve(|mut stream| async move {
            read_head(&mut stream, &mut Vec::new()).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nx")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        })
        .await;
        let result = until_stalled(get, Vec::new(), 100, SECOND / 2, SECOND).await;
        assert!(matches!(result, Err(HttpsError::Timeout(_))));
    }

    #[tokio::test]
    async fn an_upload_may_take_longer_than_its_stall_limit() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        const LENGTH: usize = 8 * 1024 * 1024;
        let (_, post) = serve(|mut stream| async move {
            let mut rest = Vec::new();
            let head = read_head(&mut stream, &mut rest).await;
            assert!(
                head.contains(&format!("content-length: {LENGTH}")),
                "{head}"
            );
            let (mut got, mut piece) = (rest.len(), vec![0; 1024 * 1024]);
            while got < LENGTH {
                tokio::time::sleep(SECOND / 2).await;
                let read = stream.read(&mut piece).await.unwrap();
                assert_ne!(read, 0);
                got += read;
            }
            stream.write_all(OK).await.unwrap();
        })
        .await;
        let started = tokio::time::Instant::now();
        let response = until_stalled(post, vec![1; LENGTH], 100, 2 * SECOND, 30 * SECOND)
            .await
            .unwrap();
        assert_eq!(response.body, b"ok");
        assert!(started.elapsed() > 2 * SECOND);
    }

    #[tokio::test]
    async fn an_upload_nobody_takes_ends() {
        let (_, post) = serve(|mut stream| async move {
            read_head(&mut stream, &mut Vec::new()).await;
            std::future::pending::<()>().await;
        })
        .await;
        // More than the system's buffers on both sides take in.
        let body = vec![1; 64 * 1024 * 1024];
        let result = until_stalled(post, body, 100, SECOND, 300 * SECOND);
        let result = tokio::time::timeout(60 * SECOND, result).await.unwrap();
        assert!(matches!(result, Err(HttpsError::Timeout(_))));
    }

    #[tokio::test]
    async fn the_answer_to_an_upload_has_its_own_limit() {
        use tokio::io::AsyncWriteExt as _;
        const LENGTH: usize = 1024;
        let (_, post) = serve(|mut stream| async move {
            let mut rest = Vec::new();
            read_head(&mut stream, &mut rest).await;
            read_body(&mut stream, rest.len(), LENGTH).await;
            tokio::time::sleep(2 * SECOND).await;
            stream.write_all(OK).await.unwrap();
        })
        .await;
        let response = until_stalled(post, vec![1; LENGTH], 100, SECOND, 30 * SECOND).await;
        assert_eq!(response.unwrap().status, 200);

        let (_, post) = serve(|mut stream| async move {
            let mut rest = Vec::new();
            read_head(&mut stream, &mut rest).await;
            read_body(&mut stream, rest.len(), LENGTH).await;
            std::future::pending::<()>().await;
        })
        .await;
        let result = until_stalled(post, vec![1; LENGTH], 100, SECOND, 2 * SECOND);
        let result = tokio::time::timeout(10 * SECOND, result).await.unwrap();
        assert!(matches!(result, Err(HttpsError::Timeout(_))));
    }

    #[test]
    fn redirects_stay_on_threema_hosts() {
        assert!(redirect_allowed("https://ds-blobp-1f.threema.ch/1f00", 0));
        assert!(redirect_allowed("https://threema.com/", MAX_REDIRECTS - 1));
        assert!(!redirect_allowed("https://threema.com/", MAX_REDIRECTS));
        assert!(!redirect_allowed("https://example.com/", 0));
        assert!(!redirect_allowed("https://threema.ch.example.com/", 0));
    }

    #[test]
    fn ipv4_only_drops_the_dual_stack_prefix() {
        use std::sync::atomic::Ordering;
        let urls = [
            "https://ds-apip.threema.ch/identity/fetch_priv",
            "https://ds-blobp-1f.threema.ch/1f00",
            "ds.g-2a.0.threema.ch",
            "https://safe-12.threema.ch/backups/x",
            "https://ds-example.org/",
        ];
        IPV4_ONLY.store(true, Ordering::Relaxed);
        let mapped: Vec<String> = urls.iter().map(|u| server_url(u).into_owned()).collect();
        IPV4_ONLY.store(false, Ordering::Relaxed);
        assert_eq!(
            mapped,
            [
                "https://apip.threema.ch/identity/fetch_priv",
                "https://blobp-1f.threema.ch/1f00",
                "g-2a.0.threema.ch",
                "https://safe-12.threema.ch/backups/x",
                "https://ds-example.org/",
            ]
        );
        assert_eq!(server_url(urls[0]), urls[0]);
    }

    #[test]
    fn spki_hash_of_current_threema_ch_certificate_is_pinned() {
        // Leaf certificate served by apip.threema.ch on 2026-10-21.
        let cert =
            CertificateDer::from(&include_bytes!("../tests/fixtures/threema-ch-leaf.der")[..]);
        let spki = spki_sha256(&cert).unwrap();
        assert_eq!(spki, "vGQZ8hm2h+km+q7rnJ7kF9S17BwSY0rbhwjz6nIupf0=");
        assert!(
            pins_for_host("apip.threema.ch")
                .unwrap()
                .contains(&spki.as_str())
        );
    }
}
