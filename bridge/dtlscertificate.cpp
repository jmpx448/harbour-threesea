// SPDX-License-Identifier: AGPL-3.0-only
#include "dtlscertificate.h"

#include <QRegularExpression>
#include <QStringList>

#include <openssl/crypto.h>
#include <openssl/evp.h>
#include <openssl/pem.h>
#include <openssl/rand.h>
#include <openssl/x509.h>

namespace DtlsCertificate {

namespace {

QByteArray digestOf(X509 *certificate)
{
    unsigned char digest[EVP_MAX_MD_SIZE];
    unsigned int length = 0;
    if (!certificate || X509_digest(certificate, EVP_sha256(), digest, &length) != 1) {
        return QByteArray();
    }
    return QByteArray(reinterpret_cast<const char *>(digest), static_cast<int>(length));
}

QString sdpForm(const QByteArray &digest)
{
    if (digest.isEmpty()) {
        return QString();
    }
    const QByteArray hex = digest.toHex().toUpper();
    QStringList pairs;
    for (int i = 0; i < hex.size(); i += 2) {
        pairs << QString::fromLatin1(hex.mid(i, 2));
    }
    return QStringLiteral("sha-256 ") + pairs.join(QLatin1Char(':'));
}

} // namespace

Certificate generate()
{
    Certificate result;
    EVP_PKEY *key = EVP_EC_gen("P-256");
    X509 *certificate = X509_new();
    BIO *bio = BIO_new(BIO_s_mem());
    if (key && certificate && bio) {
        X509_set_version(certificate, 2);
        unsigned char serial[8];
        if (RAND_bytes(serial, sizeof serial) == 1) {
            BIGNUM *number = BN_bin2bn(serial, sizeof serial, nullptr);
            BN_to_ASN1_INTEGER(number, X509_get_serialNumber(certificate));
            BN_free(number);
        }
        X509_gmtime_adj(X509_getm_notBefore(certificate), -24 * 3600);
        X509_gmtime_adj(X509_getm_notAfter(certificate), 30 * 24 * 3600);
        X509_set_pubkey(certificate, key);
        X509_NAME *name = X509_get_subject_name(certificate);
        X509_NAME_add_entry_by_txt(name, "CN", MBSTRING_ASC, reinterpret_cast<const unsigned char *>("WebRTC"), -1,
                                   -1, 0);
        X509_set_issuer_name(certificate, name);
        if (X509_sign(certificate, key, EVP_sha256()) > 0 && PEM_write_bio_X509(bio, certificate) == 1
            && PEM_write_bio_PrivateKey(bio, key, nullptr, nullptr, 0, nullptr, nullptr) == 1) {
            char *data = nullptr;
            const long size = BIO_get_mem_data(bio, &data);
            result.pem = QByteArray(data, static_cast<int>(size));
            // The memory BIO held the private key: wiped before it is freed.
            OPENSSL_cleanse(data, static_cast<size_t>(size));
            result.digest = digestOf(certificate);
            result.fingerprint = sdpForm(result.digest);
        }
    }
    BIO_free(bio);
    X509_free(certificate);
    EVP_PKEY_free(key);
    if (result.fingerprint.isEmpty()) {
        result = Certificate();
    }
    return result;
}

QString fingerprintOfPem(const QByteArray &pem)
{
    BIO *bio = BIO_new_mem_buf(pem.constData(), pem.size());
    X509 *certificate = bio ? PEM_read_bio_X509(bio, nullptr, nullptr, nullptr) : nullptr;
    const QString result = sdpForm(digestOf(certificate));
    X509_free(certificate);
    BIO_free(bio);
    return result;
}

QString sdpFingerprint(const QString &sdp)
{
    const QRegularExpressionMatch match =
        QRegularExpression(QStringLiteral("a=fingerprint:(\\S+) ([0-9A-Fa-f:]+)")).match(sdp);
    return match.hasMatch() ? match.captured(1).toLower() + QLatin1Char(' ') + match.captured(2).toUpper()
                            : QString();
}

} // namespace DtlsCertificate
