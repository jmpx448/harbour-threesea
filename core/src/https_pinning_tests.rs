// SPDX-License-Identifier: AGPL-3.0-only
//! The certificate checks refuse what they exist to refuse (CLAUDE.md §7.5), under a throwaway CA
//! made here and with real handshakes against a TLS server on this machine. Nothing leaves it: host
//! names go to the loopback address by the test's own resolver.
//!
//! Each refusal is told by its kind, and for a handshake by what the server hears, so that none
//! passes for some other reason; and what the same paths accept is tested beside it.

use std::{
    io::{Read as _, Write as _},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::Mutex,
    time::Duration,
};

use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, PublicKeyData,
    SignatureAlgorithm,
};
use rustls::{
    AlertDescription, CertificateError, ClientConfig, ClientConnection, Error, RootCertStore,
    ServerConfig, ServerConnection,
    client::WebPkiServerVerifier,
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime},
};

use super::*;

/// Only so that a defect can't hang the test run: nothing here waits for time to pass.
const NOT_FOR_EVER: Duration = Duration::from_secs(300);

struct Ca {
    issuer: Issuer<'static, KeyPair>,
    der: CertificateDer<'static>,
}

impl Ca {
    fn new() -> Self {
        Self::named("Test CA")
    }

    /// One the tests' clients don't trust.
    fn foreign() -> Self {
        Self::named("Foreign CA")
    }

    fn named(name: &str) -> Self {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.distinguished_name.push(DnType::CommonName, name);
        let der = params.self_signed(&key).unwrap().der().clone();
        Self {
            issuer: Issuer::new(params, key),
            der,
        }
    }

    fn roots(&self) -> RootCertStore {
        let mut roots = RootCertStore::empty();
        roots.add(self.der.clone()).unwrap();
        roots
    }

    /// A certificate for `host` with its own key, and that key.
    fn leaf(&self, host: &str) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        let key = KeyPair::generate().unwrap();
        let cert = CertificateParams::new(vec![host.to_owned()])
            .unwrap()
            .signed_by(&key, &self.issuer)
            .unwrap();
        (
            cert.der().clone(),
            PrivateKeyDer::try_from(key.serialize_der()).unwrap(),
        )
    }

    /// A certificate for `host` with the public key the real server had on the day of the
    /// fixture: a pinned one. Nobody here has its private key, so it can't complete a handshake,
    /// but the check of the certificate alone can be run on it.
    fn leaf_with_pinned_key(&self, host: &str) -> CertificateDer<'static> {
        CertificateParams::new(vec![host.to_owned()])
            .unwrap()
            .signed_by(&PinnedKey::of_the_fixture(), &self.issuer)
            .unwrap()
            .der()
            .clone()
    }
}

/// The P-256 public key of the Threema server's certificate kept in the fixtures.
struct PinnedKey(Vec<u8>);

impl PinnedKey {
    fn of_the_fixture() -> Self {
        let cert =
            CertificateDer::from(&include_bytes!("../tests/fixtures/threema-ch-leaf.der")[..]);
        let cert = webpki::EndEntityCert::try_from(&cert).unwrap();
        let spki = cert.subject_public_key_info();
        // An uncompressed P-256 point closes the SubjectPublicKeyInfo.
        let point = &spki.as_ref()[spki.as_ref().len() - 65..];
        assert_eq!(point[0], 4);
        Self(point.to_vec())
    }
}

impl PublicKeyData for PinnedKey {
    fn der_bytes(&self) -> &[u8] {
        &self.0
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &rcgen::PKCS_ECDSA_P256_SHA256
    }
}

/// The first pin of `threema.ch`, the one of the fixture.
const THE_FIXTURES_PIN: &str = "vGQZ8hm2h+km+q7rnJ7kF9S17BwSY0rbhwjz6nIupf0=";

fn webpki_of(ca: &Ca) -> Arc<WebPkiServerVerifier> {
    WebPkiServerVerifier::builder_with_provider(
        Arc::new(ca.roots()),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .unwrap()
}

fn name(host: &str) -> ServerName<'static> {
    ServerName::try_from(host.to_owned()).unwrap()
}

fn pinning(ca: &Ca, cert: &CertificateDer<'_>, host: &str) -> Result<(), Error> {
    pinning_at(ca, cert, host, UnixTime::now())
}

fn pinning_at(ca: &Ca, cert: &CertificateDer<'_>, host: &str, now: UnixTime) -> Result<(), Error> {
    PinningVerifier {
        webpki: webpki_of(ca),
    }
    .verify_server_cert(cert, &[], &name(host), &[], now)
    .map(|_| ())
}

const PIN_MISMATCH: Error =
    Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure);

#[test]
fn a_valid_chain_with_a_pinned_key_passes() {
    let ca = Ca::new();
    let cert = ca.leaf_with_pinned_key("apip.threema.ch");
    assert_eq!(spki_sha256(&cert).unwrap(), THE_FIXTURES_PIN);
    pinning(&ca, &cert, "apip.threema.ch").unwrap();
}

#[test]
fn a_valid_chain_with_another_key_is_refused_on_every_threema_domain() {
    let ca = Ca::new();
    for (domain, _) in PINS {
        for host in [(*domain).to_owned(), format!("host.{domain}")] {
            let (cert, _) = ca.leaf(&host);
            assert_eq!(pinning(&ca, &cert, &host), Err(PIN_MISMATCH), "{host}");
        }
    }
}

/// The most specific domain's pins are the only ones that count.
#[test]
fn a_pin_of_another_threema_domain_does_not_count() {
    let ca = Ca::new();
    for host in [
        "sfu.threema.ch",
        "x.sfu.threema.ch",
        "test.threema.ch",
        "threema.com",
    ] {
        let cert = ca.leaf_with_pinned_key(host);
        assert_eq!(pinning(&ca, &cert, host), Err(PIN_MISMATCH), "{host}");
    }
}

/// A pinned key says nothing without a chain to a root and the right name, within its dates.
#[test]
fn a_pinned_key_does_not_replace_the_certificate_check() {
    let ca = Ca::new();
    let host = "apip.threema.ch";

    let foreign = Ca::foreign().leaf_with_pinned_key(host);
    assert_eq!(spki_sha256(&foreign).unwrap(), THE_FIXTURES_PIN);
    assert_eq!(
        pinning(&ca, &foreign, host),
        Err(Error::InvalidCertificate(CertificateError::UnknownIssuer))
    );

    let another_host = ca.leaf_with_pinned_key("threema.ch.example.org");
    assert!(matches!(
        pinning(&ca, &another_host, host),
        Err(Error::InvalidCertificate(
            CertificateError::NotValidForNameContext { .. } | CertificateError::NotValidForName
        ))
    ));

    // rcgen's default dates end in 4096, the last year it can write here being a long way off.
    let cert = ca.leaf_with_pinned_key(host);
    let later = UnixTime::since_unix_epoch(Duration::from_secs(4_200_000_000_000));
    assert!(matches!(
        pinning_at(&ca, &cert, host, later),
        Err(Error::InvalidCertificate(
            CertificateError::Expired | CertificateError::ExpiredContext { .. }
        ))
    ));
}

#[test]
fn other_hosts_are_refused_whatever_they_show() {
    let ca = Ca::new();
    let not_ours = Error::General("Not a Threema host".into());
    for host in ["example.org", "evilthreema.ch", "threema.ch.example.org"] {
        let cert = ca.leaf_with_pinned_key(host);
        assert_eq!(pinning(&ca, &cert, host), Err(not_ours.clone()), "{host}");
    }
    let cert = ca.leaf_with_pinned_key("127.0.0.1");
    let address = ServerName::try_from("127.0.0.1").unwrap();
    let verifier = PinningVerifier {
        webpki: webpki_of(&ca),
    };
    assert_eq!(
        verifier
            .verify_server_cert(&cert, &[], &address, &[], UnixTime::now())
            .map(|_| ()),
        Err(not_ours)
    );
}

fn tofu(
    ca: &Ca,
    cert: &CertificateDer<'_>,
    host: &str,
    expected: Option<&str>,
) -> (Result<(), Error>, Option<String>) {
    let seen = Arc::new(Mutex::new(None));
    let result = TofuVerifier {
        webpki: webpki_of(ca),
        expected: expected.map(str::to_owned),
        seen: Arc::clone(&seen),
    }
    .verify_server_cert(cert, &[], &name(host), &[], UnixTime::now())
    .map(|_| ());
    let seen = seen.lock().unwrap().clone();
    (result, seen)
}

/// Threema's hosts have their pins; what the user allowed for another server doesn't apply.
#[test]
fn trust_on_first_use_is_never_for_threema_hosts() {
    let ca = Ca::new();
    let refused = Error::General("Threema host without its pins".into());
    for host in [
        "threema.ch",
        "apip.threema.ch",
        "sfu.threema.ch",
        "threema.com",
    ] {
        let cert = ca.leaf_with_pinned_key(host);
        for expected in [None, Some(THE_FIXTURES_PIN)] {
            assert_eq!(
                tofu(&ca, &cert, host, expected),
                (Err(refused.clone()), None),
                "{host}"
            );
        }
    }
}

#[test]
fn trust_on_first_use_follows_what_was_pinned() {
    let ca = Ca::new();
    let host = "web.example.org";
    let (cert, _) = ca.leaf(host);
    let key = spki_sha256(&cert).unwrap();

    assert_eq!(tofu(&ca, &cert, host, None), (Ok(()), Some(key.clone())));
    assert_eq!(
        tofu(&ca, &cert, host, Some(&key)),
        (Ok(()), Some(key.clone()))
    );
    // What it showed is kept: the user is told the key changed.
    assert_eq!(
        tofu(&ca, &cert, host, Some(THE_FIXTURES_PIN)),
        (Err(PIN_MISMATCH), Some(key))
    );
}

/// Trusted on first use still means a valid certificate, and one that isn't leaves no key to pin.
#[test]
fn trust_on_first_use_keeps_the_certificate_check() {
    let ca = Ca::new();
    let host = "web.example.org";
    let (foreign, _) = Ca::foreign().leaf(host);
    let key = spki_sha256(&foreign).unwrap();
    for expected in [None, Some(key.as_str())] {
        assert_eq!(
            tofu(&ca, &foreign, host, expected),
            (
                Err(Error::InvalidCertificate(CertificateError::UnknownIssuer)),
                None
            )
        );
    }
    let (another_host, _) = ca.leaf("other.example.org");
    assert!(tofu(&ca, &another_host, host, None).0.is_err());
}

/// A TLS server on this machine: what became of the handshake on its side, and whether any data
/// came after it.
struct Server {
    address: SocketAddr,
    outcome: std::thread::JoinHandle<ServerSide>,
}

/// How the handshake ended on the server, and the data it got after it.
type ServerSide = (Result<(), Error>, Vec<u8>);

impl Server {
    fn start(cert: CertificateDer<'static>, key: PrivateKeyDer<'static>) -> Self {
        let config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let outcome = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(NOT_FOR_EVER)).unwrap();
            let mut tls = ServerConnection::new(Arc::new(config)).unwrap();
            while tls.is_handshaking() {
                if let Err(error) = tls.complete_io(&mut socket) {
                    let error = error
                        .get_ref()
                        .and_then(|inner| inner.downcast_ref::<Error>())
                        .cloned()
                        .unwrap_or_else(|| Error::General(error.to_string()));
                    return (Err(error), Vec::new());
                }
            }
            let mut stream = rustls::Stream::new(&mut tls, &mut socket);
            let mut got = vec![0; 64];
            let length = stream.read(&mut got).unwrap_or(0);
            got.truncate(length);
            // An echo, for whoever got this far to be sure of it.
            let _ = stream.write_all(&got);
            let _ = stream.flush();
            (Ok(()), got)
        });
        Self { address, outcome }
    }

    fn outcome(self) -> ServerSide {
        self.outcome.join().unwrap()
    }
}

/// A TLS client at the given host name, whose address is the server's: the handshake, and with it
/// done a few bytes there and back. What the client says, and what the server says.
fn handshake(config: ClientConfig, host: &str, server: Server) -> (Result<(), Error>, ServerSide) {
    let mut socket = TcpStream::connect(server.address).unwrap();
    socket.set_read_timeout(Some(NOT_FOR_EVER)).unwrap();
    let mut tls = ClientConnection::new(Arc::new(config), name(host)).unwrap();
    let mut result = Ok(());
    while tls.is_handshaking() {
        if let Err(error) = tls.complete_io(&mut socket) {
            result = Err(error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<Error>())
                .cloned()
                .unwrap_or_else(|| Error::General(error.to_string())));
            break;
        }
    }
    if result.is_ok() {
        let mut stream = rustls::Stream::new(&mut tls, &mut socket);
        stream.write_all(b"hello").unwrap();
        stream.flush().unwrap();
        let mut back = [0; 5];
        stream.read_exact(&mut back).unwrap();
        assert_eq!(&back, b"hello");
    }
    drop(socket);
    (result, server.outcome())
}

/// The same server and client, with the key allowed: what the refusals below are measured against.
#[test]
fn a_handshake_with_the_expected_key_goes_through() {
    let ca = Ca::new();
    let host = "web.example.org";
    let (cert, key) = ca.leaf(host);
    let pin = spki_sha256(&cert).unwrap();
    let config = tofu_tls_config_with(ca.roots(), Some(pin), Arc::new(Mutex::new(None))).unwrap();
    let (client, server) = handshake(config, host, Server::start(cert, key));
    assert_eq!(client, Ok(()));
    assert_eq!(server, (Ok(()), b"hello".to_vec()));
}

/// A server for a Threema host with a certificate from a trusted root, as a CA that has gone wrong
/// could issue: the client hangs up at the pin, and no data was sent.
#[test]
fn a_handshake_with_a_key_that_is_not_pinned_is_refused() {
    let ca = Ca::new();
    let host = "apip.threema.ch";
    let (cert, key) = ca.leaf(host);
    let config = tls_config_with(ca.roots()).unwrap();
    let (client, server) = handshake(config, host, Server::start(cert, key));
    assert_eq!(client, Err(PIN_MISMATCH));
    assert_eq!(
        server,
        (
            Err(Error::AlertReceived(AlertDescription::AccessDenied)),
            Vec::new()
        )
    );
}

/// What the app itself runs with knows no root but the Web's.
#[test]
fn a_handshake_with_a_certificate_from_an_unknown_root_is_refused() {
    let host = "apip.threema.ch";
    let (cert, key) = Ca::foreign().leaf(host);
    let (client, server) = handshake(tls_config().unwrap(), host, Server::start(cert, key));
    assert_eq!(
        client,
        Err(Error::InvalidCertificate(CertificateError::UnknownIssuer))
    );
    assert_eq!(
        server,
        (
            Err(Error::AlertReceived(AlertDescription::UnknownCA)),
            Vec::new()
        )
    );
}

/// The app's configuration for a server the user allowed, and a certificate that isn't valid.
#[test]
fn a_handshake_of_trust_on_first_use_with_an_unknown_root_is_refused() {
    let host = "web.example.org";
    let (cert, key) = Ca::foreign().leaf(host);
    let seen = Arc::new(Mutex::new(None));
    let config = tofu_tls_config(None, Arc::clone(&seen)).unwrap();
    let (client, server) = handshake(config, host, Server::start(cert, key));
    assert_eq!(
        client,
        Err(Error::InvalidCertificate(CertificateError::UnknownIssuer))
    );
    assert_eq!(server.1, Vec::<u8>::new());
    assert_eq!(*seen.lock().unwrap(), None);
}

/// Every name goes to the local server, and only to it.
struct ToTheServer(SocketAddr);

impl reqwest::dns::Resolve for ToTheServer {
    fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let address = self.0;
        Box::pin(async move {
            let addresses: reqwest::dns::Addrs = Box::new(std::iter::once(address));
            Ok(addresses)
        })
    }
}

/// The client every request goes through: no request reaches a server that fails the pin.
#[tokio::test(flavor = "current_thread")]
async fn a_request_goes_nowhere_when_the_pin_fails() {
    let ca = Ca::new();
    let host = "apip.threema.ch";
    let (cert, key) = ca.leaf(host);
    let server = Server::start(cert, key);
    let client = HttpsClient::with(
        tls_config_with(ca.roots()).unwrap(),
        Arc::new(ToTheServer(server.address)),
    )
    .unwrap();
    let url = format!("https://{host}:{}/", server.address.port());
    let answer = client
        .send(HttpsRequest {
            timeout: NOT_FOR_EVER,
            url,
            method: HttpsMethod::Get,
            headers: Vec::new(),
            body: b"a secret".to_vec(),
        })
        .await;
    assert!(
        matches!(answer, Err(HttpsError::Unreachable(_))),
        "{answer:?}"
    );
    assert_eq!(
        server.outcome(),
        (
            Err(Error::AlertReceived(AlertDescription::AccessDenied)),
            Vec::new()
        )
    );
}
