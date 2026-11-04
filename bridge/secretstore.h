// SPDX-License-Identifier: AGPL-3.0-only
#ifndef SECRETSTORE_H
#define SECRETSTORE_H

#include <functional>

#include <QByteArray>
#include <QObject>

namespace Sailfish { namespace Secrets { class SecretManager; } }

// Holds the 32-byte database key in Sailfish Secrets (owner-only collection, unlocked with the
// device lock). The key never touches disk outside the secrets daemon's encrypted storage. With
// the app lock's key encryption (ADR 0009) it holds the key encrypted with the passphrase.
class SecretStore : public QObject
{
    Q_OBJECT

public:
    explicit SecretStore(QObject *parent = nullptr);
    ~SecretStore() override;

    // Asynchronously fetch the key, or create and store a new one if `allowCreate` (i.e. there
    // is no database yet). Emits exactly one of keyReady / failed.
    void loadKey(bool allowCreate);

    // Replace what is stored (the key, or the wrapped key); emits stored(ok).
    void store(const QByteArray &data);

    // The old key goes for good and a new one takes its place: the ID was removed and its
    // database with it (ADR 0018). Emits keyReady or failed.
    void replaceKey();

signals:
    void keyReady(QByteArray key);
    // The key is encrypted with the app lock passphrase; `keyboard`: typed, not digits.
    void wrappedKeyReady(QByteArray blob, bool keyboard);
    // `kind`: what the UI tells apart (secrets: Secrets answered with an error, keyMissing: a
    // database exists and its key is really not there). `detail`: a code for support.
    void failed(const QString &kind, const QString &detail = QString());
    void stored(bool ok);

private:
    struct Fetched
    {
        bool found = false;
        // Secrets says it isn't there; anything else that went wrong says nothing about that.
        bool missing = false;
        QByteArray data;
        QString code;
    };

    void ensureCollection();
    void fetchKey();
    bool readFailed(const Fetched &fetched);
    void useKey(const QByteArray &key, bool found);
    void fetch(const QString &name, std::function<void(const Fetched &)> done);
    void write(const QString &name, const QByteArray &data, std::function<void(bool)> done);
    void remove(const QString &name, std::function<void(bool)> done);
    void storeNewKey();

    Sailfish::Secrets::SecretManager *m_manager;
    bool m_allowCreate = false;
    bool m_replaceKey = false;
};

#endif // SECRETSTORE_H
