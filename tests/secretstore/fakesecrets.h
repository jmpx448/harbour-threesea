// SPDX-License-Identifier: AGPL-3.0-only
// Sailfish Secrets as far as bridge/secretstore.cpp uses it, over an in-memory daemon the test
// controls (FakeDaemon). The app builds against the real headers, so anything secretstore.cpp
// starts to use that is missing here fails to build this test rather than going untested.
// Like the real library, a request finishes later, from the event loop, never inside
// startRequest(); and the daemon refuses to overwrite a secret.
#ifndef FAKESECRETS_H
#define FAKESECRETS_H

#include <QByteArray>
#include <QMap>
#include <QObject>
#include <QSet>
#include <QString>
#include <QStringList>

namespace Sailfish {
namespace Secrets {

class Result
{
public:
    enum ResultCode { Succeeded = 0, Pending = 1, Failed = 2 };
    enum ErrorCode {
        NoError = 0,
        DaemonError = 5,
        InvalidSecretError = 40,
        InvalidCollectionError = 43,
        CollectionAlreadyExistsError = 46,
        SecretAlreadyExistsError = 47,
        DatabaseError = 72,
    };
    Result(ResultCode code = Succeeded) : m_code(code) {}
    Result(ErrorCode error, const QString &message) : m_code(Failed), m_error(error), m_message(message) {}
    ResultCode code() const { return m_code; }
    ErrorCode errorCode() const { return m_error; }
    QString errorMessage() const { return m_message; }

private:
    ResultCode m_code;
    ErrorCode m_error = NoError;
    QString m_message;
};

class Secret
{
public:
    static const QString TypeBlob;

    class Identifier
    {
    public:
        Identifier() = default;
        Identifier(const QString &name, const QString &collectionName, const QString &storagePluginName)
            : m_name(name), m_collection(collectionName), m_plugin(storagePluginName)
        {
        }
        QString name() const { return m_name; }
        QString collectionName() const { return m_collection; }
        QString storagePluginName() const { return m_plugin; }

    private:
        QString m_name;
        QString m_collection;
        QString m_plugin;
    };

    Secret() = default;
    explicit Secret(const Identifier &identifier) : m_identifier(identifier) {}
    Identifier identifier() const { return m_identifier; }
    QByteArray data() const { return m_data; }
    void setData(const QByteArray &data) { m_data = data; }
    QString type() const { return m_type; }
    void setType(const QString &type) { m_type = type; }

private:
    Identifier m_identifier;
    QByteArray m_data;
    QString m_type;
};

class SecretManager : public QObject
{
    Q_OBJECT
public:
    enum UserInteractionMode { PreventInteraction = 0, SystemInteraction, ApplicationInteraction };
    enum AccessControlMode { OwnerOnlyMode = 0, SystemAccessControlMode, NoAccessControlMode };
    enum DeviceLockUnlockSemantic { DeviceLockKeepUnlocked = 0, DeviceLockVerifyLock, DeviceLockRelock };
    static const QString DefaultAuthenticationPluginName;
    static const QString DefaultEncryptedStoragePluginName;
    explicit SecretManager(QObject *parent = nullptr) : QObject(parent) {}
};

class Request : public QObject
{
    Q_OBJECT
public:
    enum Status { Inactive = 0, Active, Finished };
    explicit Request(QObject *parent = nullptr) : QObject(parent) {}
    SecretManager *manager() const { return m_manager; }
    void setManager(SecretManager *manager) { m_manager = manager; }
    Status status() const { return m_status; }
    Result result() const { return m_result; }
    void startRequest();

signals:
    void statusChanged();

protected:
    // What the daemon answers, worked out when the request finishes.
    virtual Result run() = 0;

private:
    Q_INVOKABLE void finish();

    SecretManager *m_manager = nullptr;
    Status m_status = Inactive;
    Result m_result;
};

class CreateCollectionRequest : public Request
{
    Q_OBJECT
public:
    enum CollectionLockType { DeviceLock = 0, CustomLock };
    explicit CreateCollectionRequest(QObject *parent = nullptr) : Request(parent) {}
    void setCollectionName(const QString &name) { m_name = name; }
    void setAccessControlMode(SecretManager::AccessControlMode mode) { m_access = mode; }
    void setCollectionLockType(CollectionLockType type) { m_lock = type; }
    void setDeviceLockUnlockSemantic(SecretManager::DeviceLockUnlockSemantic semantic) { m_unlock = semantic; }
    void setStoragePluginName(const QString &) {}
    void setEncryptionPluginName(const QString &) {}
    void setAuthenticationPluginName(const QString &) {}

protected:
    Result run() override;

private:
    QString m_name;
    SecretManager::AccessControlMode m_access = SecretManager::NoAccessControlMode;
    CollectionLockType m_lock = CustomLock;
    SecretManager::DeviceLockUnlockSemantic m_unlock = SecretManager::DeviceLockRelock;
};

class StoredSecretRequest : public Request
{
    Q_OBJECT
public:
    explicit StoredSecretRequest(QObject *parent = nullptr) : Request(parent) {}
    void setIdentifier(const Secret::Identifier &identifier) { m_identifier = identifier; }
    void setUserInteractionMode(SecretManager::UserInteractionMode) {}
    Secret secret() const { return m_secret; }

protected:
    Result run() override;

private:
    Secret::Identifier m_identifier;
    Secret m_secret;
};

class StoreSecretRequest : public Request
{
    Q_OBJECT
public:
    enum SecretStorageType { CollectionSecret = 0, StandaloneDeviceLockSecret, StandaloneCustomLockSecret };
    explicit StoreSecretRequest(QObject *parent = nullptr) : Request(parent) {}
    void setSecretStorageType(SecretStorageType type) { m_type = type; }
    void setUserInteractionMode(SecretManager::UserInteractionMode) {}
    void setSecret(const Secret &secret) { m_secret = secret; }

protected:
    Result run() override;

private:
    SecretStorageType m_type = StandaloneCustomLockSecret;
    Secret m_secret;
};

class DeleteSecretRequest : public Request
{
    Q_OBJECT
public:
    explicit DeleteSecretRequest(QObject *parent = nullptr) : Request(parent) {}
    void setIdentifier(const Secret::Identifier &identifier) { m_identifier = identifier; }
    void setUserInteractionMode(SecretManager::UserInteractionMode) {}

protected:
    Result run() override;

private:
    Secret::Identifier m_identifier;
};

} // namespace Secrets
} // namespace Sailfish

// The daemon behind the fake: what it holds, and what the test makes go wrong.
struct FakeDaemon
{
    bool collectionExists = false;
    // Whether the collection was made as the app needs it: owner only, device lock, kept
    // unlocked (the app starts in the background, before anyone unlocks anything).
    bool collectionAsExpected = false;
    QMap<QString, QByteArray> secrets;
    // An error for the next request of that kind on that secret name (or the collection, "").
    QMap<QString, int> failCreate;
    QMap<QString, int> failRead;
    QMap<QString, int> failWrite;
    QMap<QString, int> failRemove;
    // Every request in order: "create", "read <name>", "write <name>", "remove <name>".
    QStringList log;
    int pending = 0;

    static FakeDaemon &instance();
    void reset() { *this = FakeDaemon(); }
};

#endif // FAKESECRETS_H
