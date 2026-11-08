// SPDX-License-Identifier: AGPL-3.0-only
#include "secretstore.h"

#include "threesea_core.h"

#include <QFile>
#include <QLoggingCategory>
#include <QTimer>

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

// What Secrets answered, for support: which step, and its error code.
QString secretsCode(const char *step, const Result &result)
{
    // log-ok: the secrets daemon's own text about its request, no user data in it
    qWarning("threesea: Sailfish Secrets: %s: %s", step, qPrintable(result.errorMessage()));
    return QStringLiteral("%1 %2").arg(QLatin1String(step)).arg(static_cast<int>(result.errorCode()));
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
    m_askedAgain = false;
    ensureCollection();
}

void SecretStore::askAgainIfDismissed(int delay, std::function<bool()> mayAsk)
{
    m_askAgainAfter = delay;
    m_mayAskAgain = std::move(mayAsk);
}

void SecretStore::replaceKey()
{
    m_allowCreate = true;
    m_replaceKey = true;
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
            emit failed(QStringLiteral("secrets"), secretsCode("collection", result));
            return;
        }
        if (m_replaceKey) {
            m_replaceKey = false;
            // A wrapped key goes the same way: the passphrase belonged to the removed ID. Either
            // may be absent, so what counts is that neither can be fetched afterwards: the old
            // key must not stay behind while a new one works.
            remove(StagingName, [this](bool) {
                remove(SecretName, [this](bool) {
                    fetch(StagingName, [this](const Fetched &staged) {
                        const bool stagedFound = staged.found;
                        fetch(SecretName, [this, stagedFound](const Fetched &key) {
                            if (stagedFound || key.found) {
                                emit failed(QStringLiteral("oldKeyStored"));
                                return;
                            }
                            storeNewKey();
                        });
                    });
                });
            });
            return;
        }
        fetchKey();
    });
}

bool SecretStore::readFailed(const Fetched &fetched)
{
    // With no database there is nothing to protect: on to a new key, and writing it shows what's
    // wrong. Also when a secret that isn't there were reported under another code than we expect.
    if (m_allowCreate || fetched.missing) {
        return false;
    }
    emit failed(QStringLiteral("secrets"), fetched.code);
    return true;
}

void SecretStore::fetchKey()
{
    // After every boot the collection is locked and reading brings up the system's prompt, in the
    // system's words (ADR 0010). Trying first without it says whether that is about to happen, so
    // that the app can say what the prompt is for.
    fetch(SecretName, [this](const Fetched &key) {
        if (!key.locked) {
            keyFetched(key);
            return;
        }
        emit confirmationNeeded();
        askForKey();
    }, false);
}

void SecretStore::askForKey()
{
    fetch(SecretName, [this](const Fetched &key) {
        if (key.dismissed && !m_askedAgain && m_mayAskAgain && m_mayAskAgain()) {
            m_askedAgain = true;
            qWarning("threesea: the prompt for the database key went away, asking once more");
            QTimer::singleShot(m_askAgainAfter, this, [this]() { askForKey(); });
            return;
        }
        keyFetched(key);
    });
}

void SecretStore::keyFetched(const Fetched &key)
{
    if (key.found) {
        useKey(key.data, true);
        return;
    }
    if (readFailed(key)) {
        return;
    }
    // Interrupted while replacing it: the staging copy is the current one.
    fetch(StagingName, [this](const Fetched &staged) {
        if (!staged.found) {
            if (!readFailed(staged)) {
                useKey(QByteArray(), false);
            }
            return;
        }
        const QByteArray data = staged.data;
        qWarning("threesea: finishing an interrupted database key replacement");
        write(SecretName, data, [this, data](bool ok) {
            if (ok) {
                remove(StagingName, [](bool) {});
            }
            useKey(data, true);
        });
    });
}

void SecretStore::fetch(const QString &name, std::function<void(const Fetched &)> done, bool mayAsk)
{
    auto *request = new StoredSecretRequest(this);
    request->setManager(m_manager);
    request->setIdentifier(secretId(name));
    request->setUserInteractionMode(mayAsk ? SecretManager::SystemInteraction : SecretManager::PreventInteraction);
    whenFinished(request, [done, mayAsk](StoredSecretRequest *r) {
        const Result result = r->result();
        Fetched fetched;
        fetched.found = result.code() == Result::Succeeded;
        if (fetched.found) {
            fetched.data = r->secret().data();
        } else {
            // Not InvalidCollectionError: the daemon also answers that when its metadata query
            // fails, and ensureCollection has made the collection by now.
            fetched.missing = result.errorCode() == Result::InvalidSecretError;
            fetched.locked = !mayAsk && (result.errorCode() == Result::CollectionIsLockedError
                                         || result.errorCode() == Result::OperationRequiresUserInteraction);
            fetched.dismissed = result.errorCode() == Result::InteractionViewUserCanceledError;
            if (!fetched.missing && !fetched.locked) {
                fetched.code = secretsCode("read", result);
            }
        }
        done(fetched);
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
        emit failed(QStringLiteral("keyMissing"));
    }
}

void SecretStore::store(const QByteArray &data)
{
    // Nothing is deleted until it is known what is current: the staging copy may be the only one.
    fetch(SecretName, [this, data](const Fetched &current) {
        if (current.found) {
            replaceWith(data);
        } else if (!current.missing) {
            emit stored(false);
        } else {
            // An earlier replacement stopped between its two writes: the staging copy is current.
            fetch(StagingName, [this, data](const Fetched &staged) {
                if (staged.found) {
                    write(SecretName, staged.data, [this, data](bool ok) {
                        if (ok) {
                            replaceWith(data);
                        } else {
                            emit stored(false);
                        }
                    });
                } else if (staged.missing) {
                    replaceWith(data);
                } else {
                    emit stored(false);
                }
            });
        }
    });
}

void SecretStore::replaceWith(const QByteArray &data)
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
                write(SecretName, data, [this, data](bool ok) {
                    if (ok) {
                        remove(StagingName, [](bool) {});
                        emit stored(true);
                    } else {
                        settleFailedReplacement(data);
                    }
                });
            });
        });
    });
}

void SecretStore::settleFailedReplacement(const QByteArray &data)
{
    // The answer has to be what the next start will do (fetchKey: the key if there is one, else
    // the staging copy), or the user is told one thing and gets the other.
    fetch(SecretName, [this, data](const Fetched &current) {
        if (current.found && current.data == data) {
            remove(StagingName, [](bool) {});
            emit stored(true);
        } else if (current.found) {
            // The old one could not be removed, and stays in force.
            remove(StagingName, [this](bool) { emit stored(false); });
        } else if (current.missing) {
            qWarning("threesea: database key replaced in its staging copy only");
            emit stored(true);
        } else {
            // Can't tell. The copy stays, as it may be the only one.
            emit stored(false);
        }
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
        emit failed(QStringLiteral("noRandomness"));
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
            emit failed(QStringLiteral("secrets"), secretsCode("store", r->result()));
        }
        key.fill('\0');
    });
}
