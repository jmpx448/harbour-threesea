// SPDX-License-Identifier: AGPL-3.0-only
#ifndef DTLSCERTIFICATE_H
#define DTLSCERTIFICATE_H

#include <QByteArray>
#include <QString>

// DTLS certificates for the call engines (ADR 0012). Threema's libwebrtc uses ECDSA certificates
// and, as DTLS client, offers only ECDSA cipher suites, so GStreamer's default RSA certificate
// fails ("no shared cipher") whenever we are the DTLS server; and webrtcbin neither lets us pick
// one before it writes its SDP nor checks the peer's against the SDP. Hence our own.
namespace DtlsCertificate {

struct Certificate
{
    // Certificate and private key, PEM (for dtlssrtpdec's "pem").
    QByteArray pem;
    // SHA-256 of the certificate: raw, and as in an SDP a=fingerprint line ("sha-256 AB:CD:…").
    QByteArray digest;
    QString fingerprint;
    bool isNull() const { return pem.isEmpty(); }
};

// A fresh self-signed ECDSA P-256 certificate (new per call, like libwebrtc).
Certificate generate();

// "sha-256 AB:CD:…" of a PEM certificate (e.g. webrtcbin's remote-certificate); empty on failure.
QString fingerprintOfPem(const QByteArray &pem);

// The (first) fingerprint of an SDP, normalized like Certificate::fingerprint.
QString sdpFingerprint(const QString &sdp);

} // namespace DtlsCertificate

#endif // DTLSCERTIFICATE_H
