// SPDX-License-Identifier: AGPL-3.0-only
#include "fakesecrets.h"

namespace Sailfish {
namespace Secrets {

const QString Secret::TypeBlob = QStringLiteral("Blob");
const QString SecretManager::DefaultAuthenticationPluginName = QStringLiteral("authentication");
const QString SecretManager::DefaultEncryptedStoragePluginName = QStringLiteral("encryptedstorage");

namespace {

// The error set for this request, if any (it applies once).
bool failing(QMap<QString, int> &failures, const QString &name, Result *result)
{
    if (!failures.contains(name)) {
        return false;
    }
    *result = Result(static_cast<Result::ErrorCode>(failures.take(name)), QStringLiteral("made to fail"));
    return true;
}

} // namespace

void Request::startRequest()
{
    m_status = Active;
    ++FakeDaemon::instance().pending;
    QMetaObject::invokeMethod(this, "finish", Qt::QueuedConnection);
}

void Request::finish()
{
    --FakeDaemon::instance().pending;
    m_result = run();
    m_status = Finished;
    emit statusChanged();
}

Result CreateCollectionRequest::run()
{
    FakeDaemon &daemon = FakeDaemon::instance();
    daemon.log << QStringLiteral("create");
    Result result;
    if (failing(daemon.failCreate, QString(), &result)) {
        return result;
    }
    if (daemon.collectionExists) {
        return Result(Result::CollectionAlreadyExistsError, QStringLiteral("exists"));
    }
    daemon.collectionExists = true;
    daemon.collectionAsExpected = m_access == SecretManager::OwnerOnlyMode && m_lock == DeviceLock
        && m_unlock == SecretManager::DeviceLockKeepUnlocked && !m_name.isEmpty();
    return Result();
}

Result StoredSecretRequest::run()
{
    FakeDaemon &daemon = FakeDaemon::instance();
    const QString name = m_identifier.name();
    daemon.log << QStringLiteral("read ") + name;
    if (daemon.collectionLocked && m_mode == SecretManager::PreventInteraction) {
        return Result(static_cast<Result::ErrorCode>(daemon.lockedAnswer), QStringLiteral("locked"));
    }
    if (daemon.collectionLocked) {
        ++daemon.prompts;
    }
    Result result;
    if (failing(daemon.failRead, name, &result)) {
        return result;
    }
    daemon.collectionLocked = false;
    if (!daemon.collectionExists) {
        return Result(Result::InvalidCollectionError, QStringLiteral("no collection"));
    }
    if (!daemon.secrets.contains(name)) {
        return Result(Result::InvalidSecretError, QStringLiteral("no such secret"));
    }
    m_secret = Secret(m_identifier);
    m_secret.setData(daemon.secrets.value(name));
    return Result();
}

Result StoreSecretRequest::run()
{
    FakeDaemon &daemon = FakeDaemon::instance();
    const QString name = m_secret.identifier().name();
    daemon.log << QStringLiteral("write ") + name;
    Result result;
    if (failing(daemon.failWrite, name, &result)) {
        return result;
    }
    if (!daemon.collectionExists || m_type != CollectionSecret) {
        return Result(Result::InvalidCollectionError, QStringLiteral("no collection"));
    }
    if (daemon.secrets.contains(name)) {
        return Result(Result::SecretAlreadyExistsError, QStringLiteral("exists"));
    }
    daemon.secrets.insert(name, m_secret.data());
    return Result();
}

Result DeleteSecretRequest::run()
{
    FakeDaemon &daemon = FakeDaemon::instance();
    const QString name = m_identifier.name();
    daemon.log << QStringLiteral("remove ") + name;
    Result result;
    if (failing(daemon.failRemove, name, &result)) {
        return result;
    }
    if (!daemon.secrets.remove(name)) {
        return Result(Result::InvalidSecretError, QStringLiteral("no such secret"));
    }
    return Result();
}

} // namespace Secrets
} // namespace Sailfish

FakeDaemon &FakeDaemon::instance()
{
    static FakeDaemon daemon;
    return daemon;
}
