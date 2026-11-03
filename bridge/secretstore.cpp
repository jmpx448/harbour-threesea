// SPDX-License-Identifier: AGPL-3.0-only
#include "secretstore.h"

#include "threesea_core.h"

#include <QFile>
#include <QLoggingCategory>

#include <Sailfish/Secrets/createcollectionrequest.h>
#include <Sailfish/Secrets/deletesecretrequest.h>
#include <Sailfish/Secrets/result.h>
#include <Sailfish/Secrets/secret.h>
#include <Sailfish/Secrets/secretmanager.h>
#include <Sailfish/Secrets/storedsecretrequest.h>
#include <Sailfish/Secrets/storesecretrequest.h>

using namespace Sailfish::Secrets;

namespace {

const QString CollectionName = QStringLiteral("harbourthreesea");
const QString SecretName = QStringLiteral("databasekey");
// Secrets can't overwrite a secret: replacing goes through this copy (see SecretStore::store).
const QString StagingName = QStringLiteral("databasekey-new");

// Runs `done` once the request has finished, then deletes the request.
template <typename R, typename F>
void whenFinished(R *request, F done)
{
    QObject::connect(request, &Request::statusChanged, request, [request, done]() mutable {
        if (request->status() == Request::Finished) {
            done(request);
            request->deleteLater();
        }
    });
    request->startRequest();
}

Secret::Identifier secretId(const QString &name = SecretName)
{
    return Secret::Identifier(name, CollectionName, SecretManager::DefaultEncryptedStoragePluginName);
}

} // namespace

SecretStore::SecretStore(QObject *parent)
    : QObject(parent)
    , m_manager(new SecretManager(this))
{
}

SecretStore::~SecretStore() = default;

void SecretStore::loadKey(bool allowCreate)
{
    m_allowCreate = allowCreate;
    ensureCollection();
}

void SecretStore::ensureCollection()
{
    auto *request = new CreateCollectionRequest(this);
    request->setManager(m_manager);
    request->setCollectionName(CollectionName);
    request->setAccessControlMode(SecretManager::OwnerOnlyMode);
    request->setCollectionLockType(CreateCollectionRequest::DeviceLock);
    request->setDeviceLockUnlockSemantic(SecretManager::DeviceLockKeepUnlocked);
    request->setStoragePluginName(SecretManager::DefaultEncryptedStoragePluginName);
    request->setEncryptionPluginName(SecretManager::DefaultEncryptedStoragePluginName);
    request->setAuthenticationPluginName(SecretManager::DefaultAuthenticationPluginName);
    whenFinished(request, [this](CreateCollectionRequest *r) {
        const Result result = r->result();
        if (result.code() == Result::Failed && result.errorCode() != Result::CollectionAlreadyExistsError) {
            emit failed(QStringLiteral("collection: ") + result.errorMessage());
            return;
        }
        fetchKey();
    });
}

void SecretStore::fetchKey()
{
    fetch(SecretName, [this](bool found, const QByteArray &data) {
        if (found) {
            useKey(data, true);
            return;
        }
        // Interrupted while replacing it: the staging copy is the current one.
        fetch(StagingName, [this](bool found, const QByteArray &data) {
            if (!found) {
                useKey(QByteArray(), false);
                return;
            }
            qWarning("threesea: finishing an interrupted database key replacement");
            write(SecretName, data, [this, data](bool ok) {
                if (ok) {
                    remove(StagingName, [](bool) {});
                }
                useKey(data, true);
            });
        });
    });
}

void SecretStore::fetch(const QString &name, std::function<void(bool, const QByteArray &)> done)
{
    auto *request = new StoredSecretRequest(this);
    request->setManager(m_manager);
    request->setIdentifier(secretId(name));
    request->setUserInteractionMode(SecretManager::SystemInteraction);
    whenFinished(request, [done](StoredSecretRequest *r) {
        done(r->result().code() == Result::Succeeded, r->secret().data());
    });
}

void SecretStore::write(const QString &name, const QByteArray &data, std::function<void(bool)> done)
{
    Secret secret(secretId(name));
    secret.setData(data);
    secret.setType(Secret::TypeBlob);
    auto *request = new StoreSecretRequest(this);
    request->setManager(m_manager);
    request->setSecretStorageType(StoreSecretRequest::CollectionSecret);
    request->setUserInteractionMode(SecretManager::SystemInteraction);
    request->setSecret(secret);
    whenFinished(request, [done](StoreSecretRequest *r) {
        if (r->result().code() != Result::Succeeded) {
            qWarning("threesea: storing a secret failed: %s", qPrintable(r->result().errorMessage()));
        }
        done(r->result().code() == Result::Succeeded);
    });
}

void SecretStore::remove(const QString &name, std::function<void(bool)> done)
{
    auto *request = new DeleteSecretRequest(this);
    request->setManager(m_manager);
    request->setIdentifier(secretId(name));
    request->setUserInteractionMode(SecretManager::SystemInteraction);
    whenFinished(request, [done](DeleteSecretRequest *r) { done(r->result().code() == Result::Succeeded); });
}

void SecretStore::useKey(const QByteArray &key, bool found)
{
    const int wrapped = threesea_key_wrapped(reinterpret_cast<const uint8_t *>(key.constData()),
                                             static_cast<size_t>(key.size()));
    if (found && key.size() == 32) {
        emit keyReady(key);
    } else if (found && wrapped >= 0) {
        emit wrappedKeyReady(key, wrapped == 1);
    } else if (m_allowCreate) {
        storeNewKey();
    } else {
        // A database exists but its key is gone: never silently create a new one.
        emit failed(QStringLiteral("database key missing"));
    }
}

void SecretStore::store(const QByteArray &data)
{
    // Secrets refuses to overwrite. So: the new data as the staging copy, then replace the key
    // with it, then drop the copy; at every point one valid copy exists (fetchKey finishes an
    // interrupted run).
    remove(StagingName, [this, data](bool) {
        write(StagingName, data, [this, data](bool staged) {
            if (!staged) {
                emit stored(false);
                return;
            }
            remove(SecretName, [this, data](bool) {
                write(SecretName, data, [this](bool ok) {
                    if (ok) {
                        remove(StagingName, [](bool) {});
                    }
                    // Not ok: the staging copy stays and is used on the next start.
                    emit stored(ok);
                });
            });
        });
    });
}

void SecretStore::storeNewKey()
{
    QFile random(QStringLiteral("/dev/urandom"));
    QByteArray key;
    if (random.open(QIODevice::ReadOnly)) {
        key = random.read(32);
    }
    if (key.size() != 32) {
        emit failed(QStringLiteral("no randomness"));
        return;
    }
    Secret secret(secretId());
    secret.setData(key);
    secret.setType(Secret::TypeBlob);

    auto *request = new StoreSecretRequest(this);
    request->setManager(m_manager);
    request->setSecretStorageType(StoreSecretRequest::CollectionSecret);
    request->setUserInteractionMode(SecretManager::SystemInteraction);
    request->setSecret(secret);
    whenFinished(request, [this, key](StoreSecretRequest *r) mutable {
        if (r->result().code() == Result::Succeeded) {
            emit keyReady(key);
        } else {
            emit failed(QStringLiteral("store: ") + r->result().errorMessage());
        }
        key.fill('\0');
    });
}
