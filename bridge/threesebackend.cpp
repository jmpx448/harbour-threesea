// SPDX-License-Identifier: AGPL-3.0-only
#include "threesebackend.h"

#include <QCoreApplication>
#include <QBuffer>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDateTime>
#include <QImage>
#include <QImageReader>
#include <QGuiApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocale>
#include <QMetaObject>
#include <QNetworkConfigurationManager>
#include <QLoggingCategory>
#include <QSettings>
#include <QStandardPaths>
#include <QThreadPool>
#include <QTransform>
#include <QUrl>

#include <threesea_core.h>

#include "contactlistmodel.h"
#include "coreresult.h"
#include "endofprocess.h"
#include "filepreparer.h"
#include "photoedit.h"
#include "voicerecorder.h"
#include "secretstore.h"

namespace {

QString takeCoreString(char *s)
{
    const QString result = QString::fromUtf8(s);
    threesea_string_free(s);
    return result;
}

QString dataDir()
{
    return QStandardPaths::writableLocation(QStandardPaths::AppDataLocation);
}

// In the settings from the moment the core has wiped until the new key and database are there.
// Found at a start: the app died in between, and goes on with the rest. The value is whether
// linked devices remain.
const QString RemovalPendingKey = QStringLiteral("removalPending");

QString databasePath()
{
    return dataDir() + QStringLiteral("/threesea.db");
}

// KEY=value files like /etc/sailfish-release and /etc/hw-release.
// A function run on the thread pool.
class Task : public QRunnable
{
public:
    explicit Task(std::function<void()> work) : m_work(std::move(work)) {}
    void run() override { m_work(); }

private:
    std::function<void()> m_work;
};

QString releaseValue(const QString &file, const QString &key)
{
    QSettings release(file, QSettings::IniFormat);
    const QString value = release.value(key).toString();
    return value.isEmpty() ? QStringLiteral("unknown") : value;
}

} // namespace

ThreeseBackend::ThreeseBackend(QJSEngine *engine)
    : QObject(engine)
    , m_engine(engine)
    , m_secrets(new SecretStore(this))
    , m_contacts(new ContactListModel(this))
    , m_voiceRecorder(new VoiceRecorder(this))
{
    loadBubbleSettings();
    m_unreadTimer.setSingleShot(true);
    m_unreadTimer.setInterval(50);
    connect(&m_unreadTimer, &QTimer::timeout, this, [this]() {
        QJSValue none;
        call(QStringLiteral("unreadSummary"), QVariantMap(), none);
    });
    connect(m_secrets, &SecretStore::keyReady, this, &ThreeseBackend::startCore);
    connect(m_secrets, &SecretStore::wrappedKeyReady, this, [this](QByteArray blob, bool keyboard) {
        Q_UNUSED(keyboard)
        m_wrappedKey = blob;
        m_state = QStringLiteral("locked");
        emit dataEncryptionChanged();
        emit stateChanged();
    });
    m_mediaReads.setMaxThreadCount(1);
    connect(this, &ThreeseBackend::keyWorkDone, this, &ThreeseBackend::onKeyWorkDone, Qt::QueuedConnection);
    m_network = new QNetworkConfigurationManager(this);
    connect(m_network, &QNetworkConfigurationManager::configurationChanged, this, &ThreeseBackend::reportNetwork);
    connect(m_network, &QNetworkConfigurationManager::onlineStateChanged, this, &ThreeseBackend::reportNetwork);
    m_networkWakeTimer.setSingleShot(true);
    m_networkWakeTimer.setInterval(2000);
    connect(&m_networkWakeTimer, &QTimer::timeout, this, [this]() {
        if (m_state == QLatin1String("ready")) {
            emit networkChangedSettled();
        }
    });
    connect(m_secrets, &SecretStore::failed, this, &ThreeseBackend::fail);
    // The core's own mark: killed after it began to wipe, before the bridge heard of it.
    const int removalMark = threesea_data_removal_mark(dataDir().toUtf8().constData());
    if (removalMark >= 0 && !QSettings().contains(RemovalPendingKey)) {
        QSettings settings;
        settings.setValue(RemovalPendingKey, removalMark == 1);
        settings.sync();
    }
    if (QSettings().contains(RemovalPendingKey)) {
        m_removingData = true;
        m_state = QStringLiteral("removing");
        QMetaObject::invokeMethod(this, "finishDataRemoval", Qt::QueuedConnection);
    } else {
        m_secrets->loadKey(!QFile::exists(databasePath()));
    }
}

void ThreeseBackend::prepareToEnd()
{
    m_ending = true;
}

bool ThreeseBackend::shutdown()
{
    m_dbKey.fill('\0');
    // What the closing pages handed over is on its way through a FilePreparer. A photo takes about
    // a second; this and the rest stay under the 15 s after which the Full flavour's update kills
    // the process.
    deliverUnderWay(this, [this]() { return !m_filesPreparing.isEmpty(); }, 3000);
    // A reader of a large media file holds this until it has gone through the whole file, which is
    // outside every limit of the stop. Not having it means leaving with the core still running,
    // which the store takes like any crash.
    if (!m_coreLock.tryLockForWrite(2000)) {
        qWarning("threesea: a media read holds up the end");
        // The core goes on until the process ends, so what it has been given can still get there.
        deliverUnderWay(this, [this]() { return !m_mustArrive.isEmpty(); }, 3000);
        tellWhatIsLost();
        return false;
    }
    bool clean = true;
    if (m_core) {
        clean = threesea_core_quit(m_core);
        m_core = nullptr;
    }
    m_coreLock.unlock();
    // Its last answers: they say what got there.
    QCoreApplication::sendPostedEvents(this, QEvent::MetaCall);
    tellWhatIsLost();
    // Without the core lock: a task reading media would wait for it. Scrypt takes seconds, and a
    // task left running would report to this object, which goes with the engine. The application
    // would wait for these in its destructor as well, however long a video being scaled takes.
    const bool workersDone = QThreadPool::globalInstance()->waitForDone(2000);
    // A read still on its turn finds no core now and ends at once.
    const bool readsDone = m_mediaReads.waitForDone(1000);
    return workersDone && readsDone && clean;
}

// The window has gone, so this says it itself. Its text has no words of the user's in it.
void ThreeseBackend::tellWhatIsLost()
{
    if (m_mustArrive.isEmpty()) {
        return;
    }
    qWarning("threesea: ended with %d files or drafts unfinished", m_mustArrive.size());
    //% "Threesea ended before it was done"
    const QString summary = qtTrId("threesea-la-ended-early");
    //% "Your last files or draft did not reach the chat. Add them again there."
    const QString body = qtTrId("threesea-la-ended-early-body");
    QDBusMessage notify = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.Notifications"), QStringLiteral("/org/freedesktop/Notifications"),
        QStringLiteral("org.freedesktop.Notifications"), QStringLiteral("Notify"));
    QVariantMap hints;
    hints.insert(QStringLiteral("x-nemo-preview-summary"), summary);
    hints.insert(QStringLiteral("x-nemo-preview-body"), body);
    notify << QStringLiteral("Threesea") << 0u << QStringLiteral("harbour-threesea") << summary << body
           << QStringList() << hints << -1;
    QDBusConnection::sessionBus().call(notify, QDBus::Block, 1000);
}

ThreeseBackend::~ThreeseBackend()
{
    // They read through this object and the core.
    m_mediaReads.waitForDone();
    // Worker tasks report to this object; shutdown() has seen them off on the usual way out.
    QThreadPool::globalInstance()->waitForDone();
    m_dbKey.fill('\0');
    QWriteLocker locker(&m_coreLock);
    if (m_core) {
        // Blocks until the core thread has stopped; no callbacks afterwards.
        threesea_core_stop(m_core);
        m_core = nullptr;
    }
}

QByteArray ThreeseBackend::readMedia(qint64 message, bool thumbnail) const
{
    QReadLocker locker(&m_coreLock);
    if (!m_core) {
        return QByteArray();
    }
    size_t length = 0;
    uint8_t *buffer = threesea_media_read(m_core, message, thumbnail, &length);
    if (!buffer) {
        return QByteArray();
    }
    QByteArray data(reinterpret_cast<const char *>(buffer), static_cast<int>(length));
    threesea_buffer_free(buffer, length);
    return data;
}

QString ThreeseBackend::coreVersion() const
{
    return takeCoreString(threesea_core_version());
}

QString ThreeseBackend::startConfigJson() const
{
    const QLocale locale = QLocale::system();
    const QStringList parts = locale.name().split(QLatin1Char('_'));
    QJsonObject config;
    config.insert(QStringLiteral("dataDir"), dataDir());
    config.insert(QStringLiteral("appVersion"), QCoreApplication::applicationVersion());
    config.insert(QStringLiteral("language"), parts.value(0));
    config.insert(QStringLiteral("country"), parts.value(1));
    config.insert(QStringLiteral("model"), releaseValue(QStringLiteral("/etc/hw-release"), QStringLiteral("NAME")));
    config.insert(QStringLiteral("osVersion"),
                  releaseValue(QStringLiteral("/etc/sailfish-release"), QStringLiteral("VERSION_ID")));
    config.insert(QStringLiteral("picturesDir"),
                  QStandardPaths::writableLocation(QStandardPaths::PicturesLocation));
    config.insert(QStringLiteral("calls"), callsSupported());
    config.insert(QStringLiteral("logLevel"), logLevel());
#ifdef THREESEA_UPDATE_CHECK
    config.insert(QStringLiteral("updateCheck"), true);
#endif
    return QString::fromUtf8(QJsonDocument(config).toJson(QJsonDocument::Compact));
}

void ThreeseBackend::unlockDatabase(const QString &passphrase, const QJSValue &callback)
{
    if (m_state != QLatin1String("locked")) {
        // Nothing to open any more; the page waiting for this answer must not wait forever.
        if (callback.isCallable()) {
            QJSValue(callback).call(QJSValueList() << false);
        }
        return;
    }
    const quint64 id = m_nextId++;
    m_keyWork.insert(id, [this, callback](const QByteArray &key) {
        const bool ok = key.size() == 32;
        if (ok) {
            m_unlockedAtStart = true;
            m_state = QStringLiteral("starting");
            emit stateChanged();
            startCore(key);
        }
        if (callback.isCallable()) {
            QJSValue(callback).call(QJSValueList() << ok);
        }
    });
    QByteArray blob = m_wrappedKey;
    QByteArray pass = passphrase.toUtf8();
    // scrypt takes seconds: off the GUI thread.
    QThreadPool::globalInstance()->start(new Task([this, id, blob, pass]() mutable {
        size_t length = 0;
        uint8_t *key = threesea_key_unwrap(reinterpret_cast<const uint8_t *>(blob.constData()),
                                           static_cast<size_t>(blob.size()), pass.constData(), &length);
        pass.fill('\0');
        QByteArray result;
        if (key) {
            result = QByteArray(reinterpret_cast<const char *>(key), static_cast<int>(length));
            threesea_buffer_free(key, length);
        }
        emit keyWorkDone(id, result);
    }));
}

void ThreeseBackend::setKeyEncryption(bool encrypt, const QString &passphrase, bool keyboard,
                                      const QJSValue &callback)
{
    if (m_dbKey.size() != 32) {
        if (callback.isCallable()) {
            QJSValue(callback).call(QJSValueList() << false);
        }
        return;
    }
    const quint64 id = m_nextId++;
    m_keyWork.insert(id, [this, callback](const QByteArray &data) {
        auto finish = [callback](bool ok) {
            if (callback.isCallable()) {
                QJSValue(callback).call(QJSValueList() << ok);
            }
        };
        if (data.isEmpty()) {
            finish(false);
            return;
        }
        // One store at a time (settings change rarely).
        auto *connection = new QMetaObject::Connection;
        const bool wrapped = data.size() != 32;
        *connection = connect(m_secrets, &SecretStore::stored, this, [this, finish, connection, data, wrapped](bool ok) {
            QObject::disconnect(*connection);
            delete connection;
            if (ok) {
                // The encrypted key is kept to check the passphrase later (it is safe to hold).
                m_wrappedKey = wrapped ? data : QByteArray();
                emit dataEncryptionChanged();
            }
            finish(ok);
        });
        m_secrets->store(data);
    });
    if (!encrypt) {
        emit keyWorkDone(id, m_dbKey);
        return;
    }
    QByteArray key = m_dbKey;
    QByteArray pass = passphrase.toUtf8();
    QThreadPool::globalInstance()->start(new Task([this, id, key, pass, keyboard]() mutable {
        size_t length = 0;
        uint8_t *blob = threesea_key_wrap(reinterpret_cast<const uint8_t *>(key.constData()),
                                          static_cast<size_t>(key.size()), pass.constData(), keyboard, &length);
        key.fill('\0');
        pass.fill('\0');
        QByteArray result;
        if (blob) {
            result = QByteArray(reinterpret_cast<const char *>(blob), static_cast<int>(length));
            threesea_buffer_free(blob, length);
        }
        emit keyWorkDone(id, result);
    }));
}

// Settings → Media: auto-download depends on Wi-Fi (or Ethernet) versus mobile data.
void ThreeseBackend::reportNetwork()
{
    bool unmetered = false;
    QStringList active;
    for (const QNetworkConfiguration &config : m_network->allConfigurations(QNetworkConfiguration::Active)) {
        active.append(config.identifier());
        const auto bearer = config.bearerType();
        if (bearer == QNetworkConfiguration::BearerWLAN || bearer == QNetworkConfiguration::BearerEthernet) {
            unmetered = true;
        }
    }
    // A socket on the old network hangs without a word, and a wait for the next attempt goes on
    // although the network is back. Only what is in use counts: a Wi-Fi scan's findings are
    // changes too. Together with the core's echo this tells within seconds.
    active.sort();
    if (active != m_activeNetworks) {
        m_activeNetworks = active;
        m_networkWakeTimer.start();
    }
    if (m_state != QLatin1String("ready")) {
        return;
    }
    QVariantMap args;
    args.insert(QStringLiteral("wifi"), unmetered);
    call(QStringLiteral("setNetwork"), args);
}

void ThreeseBackend::checkPassphrase(const QString &passphrase, const QJSValue &callback)
{
    const quint64 id = m_nextId++;
    m_keyWork.insert(id, [callback](const QByteArray &key) {
        if (callback.isCallable()) {
            QJSValue(callback).call(QJSValueList() << (key.size() == 32));
        }
    });
    QByteArray blob = m_wrappedKey;
    QByteArray pass = passphrase.toUtf8();
    QThreadPool::globalInstance()->start(new Task([this, id, blob, pass]() mutable {
        size_t length = 0;
        uint8_t *key = threesea_key_unwrap(reinterpret_cast<const uint8_t *>(blob.constData()),
                                           static_cast<size_t>(blob.size()), pass.constData(), &length);
        pass.fill('\0');
        QByteArray result;
        if (key) {
            // Only its length is looked at.
            result = QByteArray(static_cast<int>(length), '\0');
            threesea_buffer_free(key, length);
        }
        emit keyWorkDone(id, result);
    }));
}

void ThreeseBackend::onKeyWorkDone(quint64 id, const QByteArray &result)
{
    if (m_ending) {
        return;
    }
    auto work = m_keyWork.take(id);
    if (work) {
        work(result);
    }
}

static QString captureDir()
{
    return QStandardPaths::writableLocation(QStandardPaths::CacheLocation) + QStringLiteral("/capture");
}

void ThreeseBackend::removeAllData(const QString &pin)
{
    if (!m_core || m_removingData) {
        return;
    }
    m_removingData = true;
    m_stateBeforeRemoval = m_state;
    m_state = QStringLiteral("removing");
    emit stateChanged();
    // The PIN first: a refusal must leave a running call and a recording alone. `unlock` without
    // `privateChats` only checks it; with no lock there is nothing to check.
    request(QStringLiteral("unlock"), QVariantMap{{QStringLiteral("pin"), pin}},
            [this, pin](bool ok, const QVariantMap &result, const QString &kind) {
                if (!ok && kind != QLatin1String("lockNeeded")) {
                    dataRemovalRefused(result, kind);
                    return;
                }
                // First, so that their last commands reach the core ahead of the removal.
                emit dataRemovalStarted();
                requestDataRemoval(pin);
            });
}

void ThreeseBackend::dataRemovalRefused(const QVariantMap &error, const QString &kind)
{
    m_removingData = false;
    m_state = m_stateBeforeRemoval;
    emit stateChanged();
    emit dataRemovalFailed(kind, error.value(QStringLiteral("message")).toString());
}

void ThreeseBackend::requestDataRemoval(const QString &pin)
{
    // The core stops what runs, leaves the device group (that can take a while) and wipes.
    request(QStringLiteral("deleteAllData"), QVariantMap{{QStringLiteral("pin"), pin}},
            [this](bool ok, const QVariantMap &result, const QString &kind) {
                if (!ok) {
                    dataRemovalRefused(result, kind);
                    return;
                }
                QSettings settings;
                settings.setValue(RemovalPendingKey, result.value(QStringLiteral("linkedDevicesRemain")).toBool());
                settings.sync();
                stopCore();
                m_discardCoreMessages = true;
                // After everything the old core has posted.
                QMetaObject::invokeMethod(this, "finishDataRemoval", Qt::QueuedConnection);
            });
}

void ThreeseBackend::retryDataRemoval()
{
    if (!dataRemovalStuck()) {
        return;
    }
    m_state = QStringLiteral("removing");
    emit stateChanged();
    QMetaObject::invokeMethod(this, "finishDataRemoval", Qt::QueuedConnection);
}

void ThreeseBackend::stopCore()
{
    QWriteLocker locker(&m_coreLock);
    if (m_core) {
        // Blocks until the core thread has stopped; no callbacks afterwards.
        threesea_core_stop(m_core);
        m_core = nullptr;
    }
}

void ThreeseBackend::finishDataRemoval()
{
    // The core's mark brings this back at the next start.
    if (m_ending) {
        return;
    }
    // A new core that died on its start, when trying again.
    stopCore();
    m_discardCoreMessages = false;
    // Listeners close what still shows or plays something of the ID.
    if (!m_dataRemovedSent) {
        m_dataRemovedSent = true;
        emit dataRemoved(QSettings().value(RemovalPendingKey).toBool());
    }

    m_unreadTimer.stop();
    m_callbacks.clear();
    m_mustArrive.clear();
    m_nativeCallbacks.clear();
    m_statusRequests.clear();
    m_contactRequests.clear();
    m_keyWork.clear();
    m_tempSources.clear();
    m_chatTones.clear();
    m_firstLiveId = m_nextId;
    m_identity.clear();
    m_unreadCount = 0;
    m_unreadFrom.clear();
    m_unreadChat.clear();
    m_connectionState = QStringLiteral("disconnected");
    m_contacts->update(QVariantList());
    m_dbKey.fill('\0');
    m_dbKey.clear();
    m_wrappedKey.clear();
    m_unlockedAtStart = false;

    // All of them, the device's own too: chat IDs in the notification tokens, the address book
    // opt-in (the next ID is asked again), the wallpaper. Most getters read them live, so the
    // signals below bring the UI along.
    {
        QSettings settings;
        const QVariant pending = settings.value(RemovalPendingKey);
        settings.clear();
        settings.setValue(RemovalPendingKey, pending);
        settings.sync();
    }
    loadBubbleSettings();
    applyQtLogLevel();
    QGuiApplication::setQuitOnLastWindowClosed(!keepRunning());

    // Captures, a voice recording: all this app keeps there is plain files of the ID.
    QDir(QStandardPaths::writableLocation(QStandardPaths::CacheLocation)).removeRecursively();
    const bool removed = threesea_data_remove(dataDir().toUtf8().constData());
    // On a full disk the mark and the clearing above may have stayed in memory, and the core's
    // mark file is gone now: with the space just freed they reach the disk before anything else.
    QSettings().sync();

    emit unreadCountChanged();
    emit unreadFromChanged();
    emit unreadChatChanged();
    emit connectionStateChanged();
    emit keepRunningChanged();
    emit logLevelChanged();
    emit notificationSettingsChanged();
    emit addressBookSyncedAtChanged();
    emit dataEncryptionChanged();
    if (!removed) {
        // A new key couldn't open what stayed. The user can try again, or the next start does.
        fail(QStringLiteral("removingData"));
        return;
    }
    m_state = QStringLiteral("starting");
    emit stateChanged();
    m_secrets->replaceKey();
}

void ThreeseBackend::startCore(QByteArray key)
{
    m_dbKey = key;
    QDir().mkpath(dataDir());
    // In-app captures left behind (e.g. the app was closed before sending): never kept.
    QDir(captureDir()).removeRecursively();
    QWriteLocker locker(&m_coreLock);
    m_core = threesea_core_start(startConfigJson().toUtf8().constData(),
                                 reinterpret_cast<const uint8_t *>(key.constData()),
                                 static_cast<size_t>(key.size()), &ThreeseBackend::onCoreMessage, this);
    key.fill('\0');
    locker.unlock();
    if (!m_core) {
        // If the core could still say why, its `fatal` follows.
        fail(QStringLiteral("coreStart"));
        return;
    }
    QJSValue none;
    call(QStringLiteral("setImageSize"), QVariantMap{{QStringLiteral("side"), imageSize()}}, none);
}

void ThreeseBackend::fail(const QString &kind, const QString &detail)
{
    m_errorKind = kind;
    m_errorText = detail.isEmpty() ? kind : kind + QLatin1Char(' ') + detail;
    qWarning("threesea: startup failed: %s", qPrintable(m_errorText)); // log-ok: the app's own codes and core error kinds
    setUpgrading(false);
    m_state = QStringLiteral("error");
    // Nothing runs, so nothing is there to keep running for, and with the process staying the next
    // launch would only show this window again: closing it must end the process.
    QGuiApplication::setQuitOnLastWindowClosed(true);
    emit stateChanged();
}

void ThreeseBackend::retryStart()
{
    if (m_state != QLatin1String("error") || m_removingData) {
        return;
    }
    // A core can be there: one that came up but whose status failed.
    stopCore();
    m_discardCoreMessages = true;
    m_errorKind.clear();
    m_errorText.clear();
    m_state = QStringLiteral("starting");
    QGuiApplication::setQuitOnLastWindowClosed(!keepRunning());
    emit stateChanged();
    // After everything the old core has posted.
    QMetaObject::invokeMethod(this, "startAgain", Qt::QueuedConnection);
}

void ThreeseBackend::startAgain()
{
    if (m_ending) {
        return;
    }
    m_discardCoreMessages = false;
    m_callbacks.clear();
    m_mustArrive.clear();
    m_nativeCallbacks.clear();
    m_statusRequests.clear();
    m_contactRequests.clear();
    m_firstLiveId = m_nextId;
    m_dbKey.fill('\0');
    m_dbKey.clear();
    m_wrappedKey.clear();
    m_unlockedAtStart = false;
    m_secrets->loadKey(!QFile::exists(databasePath()));
}

void ThreeseBackend::setUpgrading(bool upgrading)
{
    if (upgrading != m_upgrading) {
        m_upgrading = upgrading;
        emit upgradingChanged();
    }
}

void ThreeseBackend::failFromCore(const QJsonObject &error)
{
    const QString kind = error.value(QStringLiteral("kind")).toString();
    // The store's kinds have a code as their message; an internal error's could say anything.
    fail(kind, kind == QLatin1String("internal") ? QString() : error.value(QStringLiteral("message")).toString());
}

void ThreeseBackend::onCoreMessage(void *userData, const char *json)
{
    // Core thread: copy and hand over to the GUI thread.
    auto *self = static_cast<ThreeseBackend *>(userData);
    QMetaObject::invokeMethod(self, "handleMessage", Qt::QueuedConnection, Q_ARG(QByteArray, QByteArray(json)));
}

void ThreeseBackend::sendFile(const QString &identity, const QString &path, bool asPhoto, const QString &caption,
                              const QJSValue &callback)
{
    const quint64 id = m_nextId++;
    if (m_core || m_ending) {
        m_mustArrive.insert(id);
    }
    // Picker results are file:// URLs.
    const QString localPath = path.startsWith(QLatin1String("file://")) ? QUrl(path).toLocalFile() : path;
    m_tempSources.insert(id, localPath);
    if (!m_core) {
        answerNoCore(callback, id);
        return;
    }
    if (callback.isCallable()) {
        m_callbacks.insert(id, callback);
    }
    QVariantMap request;
    request.insert(QStringLiteral("id"), static_cast<double>(id));
    request.insert(QStringLiteral("cmd"), QStringLiteral("sendFile"));
    request.insert(QStringLiteral("identity"), identity);
    request.insert(QStringLiteral("caption"), caption);
    request.insert(QStringLiteral("rendering"), asPhoto ? QStringLiteral("media") : QStringLiteral("file"));
    m_filesPreparing.insert(id, identity);
    auto *preparer = new FilePreparer(localPath, request);
    connect(preparer, &FilePreparer::prepared, this, &ThreeseBackend::filePrepared, Qt::QueuedConnection);
    QThreadPool::globalInstance()->start(preparer);
}


QString ThreeseBackend::captureFile(const QString &extension) const
{
    QDir().mkpath(captureDir());
    // Plain files until they are stored encrypted: ours only.
    QFile::setPermissions(captureDir(), QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner);
    return captureDir() + QStringLiteral("/capture-%1.%2")
            .arg(QDateTime::currentMSecsSinceEpoch())
            .arg(extension);
}

QString ThreeseBackend::editPhoto(const QString &path, const QRectF &crop, int rotation) const
{
    const QString local = path.startsWith(QLatin1String("file://")) ? QUrl(path).toLocalFile() : path;
    // With the EXIF rotation applied, like the preview (Qt Quick does it) and FilePreparer:
    // without it a camera photo would be cut in the wrong place and come out sideways.
    QImageReader reader(local);
    reader.setAutoTransform(true);
    QImage image = reader.read();
    if (image.isNull()) {
        qWarning("photo could not be read for editing");
        return QString();
    }
    if (rotation % 360 != 0) {
        QTransform turn;
        turn.rotate(rotation);
        image = image.transformed(turn, Qt::SmoothTransformation);
    }
    // The crop comes in fractions of the turned picture, so it survives any scaling the view did.
    const QRect cut = QRect(qRound(crop.x() * image.width()), qRound(crop.y() * image.height()),
                            qRound(crop.width() * image.width()), qRound(crop.height() * image.height()))
            .intersected(image.rect());
    if (cut.width() < 1 || cut.height() < 1) {
        return QString();
    }
    if (cut != image.rect()) {
        image = image.copy(cut);
    }
    const QString out = captureFile(QStringLiteral("jpg"));
    // High quality: what is sent is re-encoded once more by FilePreparer.
    if (!image.save(out, "JPEG", 95)) {
        qWarning("edited photo could not be written");
        return QString();
    }
    return out;
}

void ThreeseBackend::removeTempSources(quint64 id, const QJSValue &callbackAnswer)
{
    // A callback that took a refused file back says so by returning its path. Anything else, such
    // as the error of a function whose page has gone, keeps nothing: nothing stays for nobody.
    const QString answer = callbackAnswer.isString() ? callbackAnswer.toString() : QString();
    const QString kept = answer.startsWith(QLatin1String("file://")) ? QUrl(answer).toLocalFile() : answer;
    const QStringList paths = m_tempSources.values(id);
    m_tempSources.remove(id);
    for (const QString &path : paths) {
        if (path != kept) {
            removeCapture(path);
        }
    }
}

void ThreeseBackend::lockWithFilesOnTheirWay()
{
    QVariantMap args;
    if (!m_filesPreparing.isEmpty()) {
        QVariantMap files;
        for (const QString &identity : m_filesPreparing) {
            files.insert(identity, files.value(identity).toInt() + 1);
        }
        args.insert(QStringLiteral("pendingFiles"), files);
    }
    call(QStringLiteral("lock"), args);
}

bool ThreeseBackend::removeCapture(const QString &path) const
{
    const QString local = path.startsWith(QLatin1String("file://")) ? QUrl(path).toLocalFile() : path;
    const QFileInfo info(local);
    return info.absolutePath() == QDir(captureDir()).absolutePath() && QFile::remove(local);
}

QVariantList ThreeseBackend::alertTones() const
{
    // The tones Jolla's sound settings offer (the same folder on every device).
    QDir dir(QStringLiteral("/usr/share/sounds/jolla-ringtones/stereo"));
    QVariantList tones;
    for (const QFileInfo &file : dir.entryInfoList(QStringList() << QStringLiteral("*.ogg") << QStringLiteral("*.wav")
                                                                << QStringLiteral("*.mp3"),
                                                  QDir::Files, QDir::Name)) {
        QVariantMap tone;
        tone.insert(QStringLiteral("path"), file.absoluteFilePath());
        tone.insert(QStringLiteral("title"), toneTitle(file.absoluteFilePath()));
        tones.append(tone);
    }
    return tones;
}

QString ThreeseBackend::toneTitle(const QString &path) const
{
    // "lime-tone-1.ogg" -> "Lime tone 1" (music files: their file name the same way).
    QString title = QFileInfo(path).completeBaseName();
    title.replace(QLatin1Char('-'), QLatin1Char(' ')).replace(QLatin1Char('_'), QLatin1Char(' '));
    if (!title.isEmpty()) {
        title[0] = title.at(0).toUpper();
    }
    return title;
}

QString ThreeseBackend::chatTone(const QString &identity) const
{
    return m_chatTones.value(identity).toString();
}

void ThreeseBackend::setChatTone(const QString &identity, const QString &path)
{
    if (path.isEmpty()) {
        m_chatTones.remove(identity);
    } else {
        m_chatTones.insert(identity, path);
    }
    call(QStringLiteral("setChatTone"),
         QVariantMap{{QStringLiteral("identity"), identity}, {QStringLiteral("tone"), path}}, QJSValue());
    emit notificationSettingsChanged();
}

void ThreeseBackend::refreshChatTones()
{
    request(QStringLiteral("chatTones"), QVariantMap(), [this](bool ok, const QVariantMap &result, const QString &) {
        if (ok) {
            m_chatTones = result.value(QStringLiteral("tones")).toMap();
        }
    });
}

void ThreeseBackend::importOldChatTones()
{
    // Until 0.2.0 the tones lay in the plain settings file, under the chat's ID or group key.
    const QString group = QStringLiteral("chatTones");
    QSettings settings;
    settings.beginGroup(group);
    QVariantMap tones;
    for (const QString &identity : settings.allKeys()) {
        tones.insert(identity, settings.value(identity));
    }
    settings.endGroup();
    if (tones.isEmpty()) {
        return;
    }
    request(QStringLiteral("importChatTones"), QVariantMap{{QStringLiteral("tones"), tones}},
            [group](bool ok, const QVariantMap &, const QString &) {
                if (ok) {
                    QSettings().remove(group);
                }
            });
}

void ThreeseBackend::addGifFavourite(const QString &path, const QJSValue &callback)
{
    if (!m_core) {
        answerNoCore(callback);
        return;
    }
    const quint64 id = m_nextId++;
    if (callback.isCallable()) {
        m_callbacks.insert(id, callback);
    }
    QVariantMap request;
    request.insert(QStringLiteral("id"), static_cast<double>(id));
    request.insert(QStringLiteral("cmd"), QStringLiteral("addGifFavourite"));
    // Prepared like a GIF sent as media: size and first-frame thumbnail; the core checks it is one.
    request.insert(QStringLiteral("rendering"), QStringLiteral("media"));
    const QString localPath = path.startsWith(QLatin1String("file://")) ? QUrl(path).toLocalFile() : path;
    auto *preparer = new FilePreparer(localPath, request);
    connect(preparer, &FilePreparer::prepared, this, &ThreeseBackend::filePrepared, Qt::QueuedConnection);
    QThreadPool::globalInstance()->start(preparer);
}

void ThreeseBackend::sendVideo(const QString &identity, const QString &path, const QVariant &frame,
                               double seconds, const QString &caption, const QJSValue &callback,
                               bool keepMetadata)
{
    const quint64 id = m_nextId++;
    if (m_core || m_ending) {
        m_mustArrive.insert(id);
    }
    const QString localPath = path.startsWith(QLatin1String("file://")) ? QUrl(path).toLocalFile() : path;
    m_tempSources.insert(id, localPath);
    if (!m_core) {
        answerNoCore(callback, id);
        return;
    }
    if (callback.isCallable()) {
        m_callbacks.insert(id, callback);
    }
    QVariantMap request;
    request.insert(QStringLiteral("id"), static_cast<double>(id));
    request.insert(QStringLiteral("cmd"), QStringLiteral("sendFile"));
    request.insert(QStringLiteral("identity"), identity);
    request.insert(QStringLiteral("caption"), caption);
    request.insert(QStringLiteral("rendering"), QStringLiteral("media"));
    if (seconds > 0) {
        request.insert(QStringLiteral("duration"), seconds);
    }
    if (keepMetadata) {
        request.insert(QStringLiteral("keepMetadata"), true);
    }
    // Where a scaled-down copy may go (Settings → Media & storage → Video size). Nothing is
    // written unless the video is really scaled; the file goes once the core has read it.
    const QString scratch = captureFile(QStringLiteral("mp4"));
    m_tempSources.insert(id, scratch);
    m_filesPreparing.insert(id, identity);
    auto *preparer = new FilePreparer(localPath, request, scratch);
    preparer->setVideoFrame(frame.value<QImage>());
    connect(preparer, &FilePreparer::prepared, this, &ThreeseBackend::filePrepared, Qt::QueuedConnection);
    QThreadPool::globalInstance()->start(preparer);
}

VoiceRecorder *ThreeseBackend::voiceRecorder() const
{
    return m_voiceRecorder;
}

QByteArray ThreeseBackend::readPicture(const QString &name) const
{
    QReadLocker locker(&m_coreLock);
    if (!m_core) {
        return QByteArray();
    }
    size_t length = 0;
    uint8_t *buffer = threesea_picture_read(m_core, name.toUtf8().constData(), &length);
    if (!buffer) {
        return QByteArray();
    }
    QByteArray data(reinterpret_cast<const char *>(buffer), static_cast<int>(length));
    threesea_buffer_free(buffer, length);
    return data;
}

void ThreeseBackend::setGroupPicture(const QString &key, const QString &path, const QJSValue &callback)
{
    QVariantMap request;
    request.insert(QStringLiteral("cmd"), QStringLiteral("setGroupPicture"));
    request.insert(QStringLiteral("key"), key);
    sendSquarePicture(path, request, callback);
}

void ThreeseBackend::setProfilePicture(const QString &path, const QJSValue &callback)
{
    QVariantMap request;
    request.insert(QStringLiteral("cmd"), QStringLiteral("setProfilePicture"));
    sendSquarePicture(path, request, callback, true);
}

void ThreeseBackend::sendSquarePicture(const QString &path, QVariantMap request, const QJSValue &callback,
                                       bool safeCopy)
{
    const QString localPath = path.startsWith(QLatin1String("file://")) ? QUrl(path).toLocalFile() : path;
    QImageReader reader(localPath);
    reader.setAutoTransform(true);
    const QSize full = reader.size();
    // Decode close to the target size (camera photos are large), then crop the centre square.
    if (full.isValid() && qMin(full.width(), full.height()) > 1024) {
        reader.setScaledSize(full.scaled(1024, 1024, Qt::KeepAspectRatioByExpanding));
    }
    QImage image = reader.read();
    if (image.isNull()) {
        if (callback.isCallable()) {
            QJSValue(callback).call(QJSValueList() << false << QJSValue() << QStringLiteral("internal")
                                                   << reader.errorString());
        }
        return;
    }
    const int side = qMin(image.width(), image.height());
    image = image.copy((image.width() - side) / 2, (image.height() - side) / 2, side, side)
                .scaled(512, 512, Qt::IgnoreAspectRatio, Qt::SmoothTransformation)
                .convertToFormat(QImage::Format_RGB32);
    image = PhotoEdit::withoutMetadata(image);
    QByteArray jpeg;
    QBuffer buffer(&jpeg);
    buffer.open(QIODevice::WriteOnly);
    image.save(&buffer, "JPEG", 85);
    // Threema Safe's format (safe-backup.schema.yml): JPEG, q=60, at most 400×400, no metadata
    // (Qt writes what the image still carries: none, since withoutMetadata above).
    QByteArray safeJpeg;
    if (safeCopy) {
        QBuffer safeBuffer(&safeJpeg);
        safeBuffer.open(QIODevice::WriteOnly);
        image.scaled(400, 400, Qt::IgnoreAspectRatio, Qt::SmoothTransformation).save(&safeBuffer, "JPEG", 60);
    }

    const quint64 id = m_nextId++;
    if (callback.isCallable()) {
        m_callbacks.insert(id, callback);
    }
    request.insert(QStringLiteral("id"), static_cast<double>(id));
    filePrepared(request, jpeg, safeJpeg, QString());
}

namespace {

QString decodeQrImage(QImage image)
{
    if (image.isNull()) {
        return QString();
    }
    // Enough resolution for a phone-sized code, fast enough for every frame we look at.
    if (qMax(image.width(), image.height()) > 900) {
        image = image.scaled(900, 900, Qt::KeepAspectRatio, Qt::SmoothTransformation);
    }
    image = image.convertToFormat(QImage::Format_Grayscale8);
    QByteArray grey;
    grey.reserve(image.width() * image.height());
    for (int y = 0; y < image.height(); ++y) {
        grey.append(reinterpret_cast<const char *>(image.constScanLine(y)), image.width());
    }
    char *text = threesea_qr_decode(reinterpret_cast<const uint8_t *>(grey.constData()),
                                    static_cast<size_t>(image.width()), static_cast<size_t>(image.height()));
    return takeCoreString(text);
}

}

void ThreeseBackend::decodeQr(const QVariant &image, const QJSValue &callback)
{
    const quint64 id = m_nextId++;
    m_keyWork.insert(id, [callback](const QByteArray &text) {
        if (callback.isCallable()) {
            QJSValue(callback).call(QJSValueList() << QString::fromUtf8(text));
        }
    });
    // Several frames a second: not on the thread that draws the viewfinder.
    const QImage frame = image.value<QImage>();
    QThreadPool::globalInstance()->start(new Task([this, id, frame]() {
        emit keyWorkDone(id, decodeQrImage(frame).toUtf8());
    }));
}

QString ThreeseBackend::decodeQrFile(const QString &path) const
{
    const QString localPath = path.startsWith(QLatin1String("file://")) ? QUrl(path).toLocalFile() : path;
    QImageReader reader(localPath);
    reader.setAutoTransform(true);
    return decodeQrImage(reader.read());
}

QByteArray ThreeseBackend::decodeVoice(qint64 message) const
{
    QReadLocker locker(&m_coreLock);
    if (!m_core) {
        return QByteArray();
    }
    size_t length = 0;
    uint8_t *buffer = threesea_voice_decode(m_core, message, &length);
    if (!buffer) {
        return QByteArray();
    }
    QByteArray data(reinterpret_cast<const char *>(buffer), static_cast<int>(length));
    threesea_buffer_free(buffer, length);
    return data;
}

void ThreeseBackend::sendVoice(const QString &identity, int durationMs, const QJSValue &callback)
{
    bool pcm = false;
    QByteArray data = m_voiceRecorder->takeRecording(&pcm);
    const quint64 id = m_nextId++;
    if (callback.isCallable()) {
        m_callbacks.insert(id, callback);
    }
    QVariantMap request;
    request.insert(QStringLiteral("id"), static_cast<double>(id));
    request.insert(QStringLiteral("cmd"), QStringLiteral("sendFile"));
    request.insert(QStringLiteral("identity"), identity);
    // Like Threema Android: an audio file rendered as media is a voice message.
    request.insert(QStringLiteral("rendering"), QStringLiteral("media"));
    request.insert(QStringLiteral("mediaType"), QStringLiteral("audio/aac"));
    request.insert(QStringLiteral("name"),
                   QStringLiteral("voice-%1.aac").arg(QDateTime::currentDateTime().toString(QStringLiteral("yyyyMMdd-HHmmss"))));
    request.insert(QStringLiteral("duration"), durationMs / 1000.0);
    if (pcm) {
        // The core makes Ogg Opus of it (and sets type, name and exact duration).
        request.insert(QStringLiteral("encode"), QStringLiteral("opus"));
    }
    filePrepared(request, data, QByteArray(), data.isEmpty() ? QStringLiteral("empty recording") : QString());
    data.fill('\0');
}

void ThreeseBackend::filePrepared(const QVariantMap &request, const QByteArray &data, const QByteArray &thumbnail,
                                  const QString &error)
{
    const quint64 id = request.value(QStringLiteral("id")).toULongLong();
    m_filesPreparing.remove(id);
    if (id < m_firstLiveId) {
        return;
    }
    if (!error.isEmpty()) {
        // Whoever asked has gone; it stays among what is lost.
        if (m_ending) {
            return;
        }
        m_mustArrive.remove(id);
        // Nothing was sent, so nothing will answer and clean up after it.
        QJSValue answer;
        QJSValue callback = m_callbacks.take(id);
        if (callback.isCallable()) {
            answer = callback.call(QJSValueList() << false << QJSValue() << QStringLiteral("internal") << error);
        }
        removeTempSources(id, answer);
        return;
    }
    if (!m_core) {
        answerNoCore(m_callbacks.take(id), id);
        return;
    }
    QByteArray json = QJsonDocument(QJsonObject::fromVariantMap(request)).toJson(QJsonDocument::Compact);
    threesea_core_send_data(m_core, json.constData(), reinterpret_cast<const uint8_t *>(data.constData()),
                            static_cast<size_t>(data.size()), reinterpret_cast<const uint8_t *>(thumbnail.constData()),
                            static_cast<size_t>(thumbnail.size()));
}

bool ThreeseBackend::callsSupported()
{
#ifdef THREESEA_FULL
    return true;
#else
    return false;
#endif
}

void ThreeseBackend::request(const QString &command, const QVariantMap &args, NativeCallback done)
{
    if (!m_core) {
        if (done) {
            done(false, QVariantMap(), QStringLiteral("internal"));
        }
        return;
    }
    const quint64 id = m_nextId;
    if (done) {
        m_nativeCallbacks.insert(id, done);
    }
    call(command, args, QJSValue());
}

void ThreeseBackend::answerNoCore(const QJSValue &callback, quint64 id)
{
    if (!callback.isCallable()) {
        removeTempSources(id);
        return;
    }
    // Later, like any answer of the core: callers go on after the call before they expect one.
    QJSValue answer = callback;
    QTimer::singleShot(0, this, [this, answer, id]() mutable {
        QJSValueList arguments;
        arguments << QJSValue(false);
        arguments << m_engine->toScriptValue(QVariantMap());
        arguments << QJSValue(QStringLiteral("internal"));
        arguments << QJSValue(QStringLiteral("the app's core is not running"));
        removeTempSources(id, answer.call(arguments));
    });
}

void ThreeseBackend::call(const QString &command, const QVariantMap &args, const QJSValue &callback)
{
    if (!m_core) {
        answerNoCore(callback);
        return;
    }
    const quint64 id = m_nextId++;
    QJsonObject request = QJsonObject::fromVariantMap(args);
    // The command's arguments and the request's own fields share one object, so an argument
    // called "id" or "cmd" would be overwritten here and the core would refuse the whole
    // request ("invalid request"). Name such an argument differently (e.g. `device`).
    if (request.contains(QStringLiteral("id")) || request.contains(QStringLiteral("cmd"))) {
        qWarning("threesea: argument named id or cmd in command %s", qPrintable(command));
    }
    request.insert(QStringLiteral("id"), static_cast<double>(id));
    request.insert(QStringLiteral("cmd"), command);
    if (m_ending && command == QLatin1String("setDraft")) {
        m_mustArrive.insert(id);
    }
    if (command == QLatin1String("status")) {
        m_statusRequests.insert(id);
    } else if (command == QLatin1String("contacts")) {
        m_contactRequests.insert(id);
    }
    if (callback.isCallable()) {
        m_callbacks.insert(id, callback);
    }
    QByteArray json = QJsonDocument(request).toJson(QJsonDocument::Compact);
    threesea_core_send(m_core, json.constData());
    json.fill('\0'); // may contain a license key or password
    // Private chats' tones are known only while those chats are readable.
    if (command == QLatin1String("unlock") || command == QLatin1String("lock")
        || command == QLatin1String("setPrivate")) {
        refreshChatTones();
    }
}

void ThreeseBackend::refresh()
{
    QJSValue none;
    call(QStringLiteral("status"), QVariantMap(), none);
}

bool ThreeseBackend::notificationsEnabled() const
{
    return QSettings().value(QStringLiteral("notifications"), true).toBool();
}

void ThreeseBackend::loadBubbleSettings()
{
    const QSettings settings;
    const bool threema = settings.value(QStringLiteral("threemaEmoji"), true).toBool();
    const bool bigger = settings.value(QStringLiteral("biggerEmojis"), true).toBool();
    const int size = qBound(0, settings.value(QStringLiteral("chatFontSize"), 1).toInt(), 2);
    const bool play = settings.value(QStringLiteral("autoPlayGifs"), true).toBool();
    if (threema != m_threemaEmoji) {
        m_threemaEmoji = threema;
        emit threemaEmojiChanged();
    }
    if (bigger != m_biggerEmojis) {
        m_biggerEmojis = bigger;
        emit biggerEmojisChanged();
    }
    if (size != m_chatFontSize) {
        m_chatFontSize = size;
        emit chatFontSizeChanged();
    }
    if (play != m_autoPlayGifs) {
        m_autoPlayGifs = play;
        emit autoPlayGifsChanged();
    }
}

void ThreeseBackend::setThreemaEmoji(bool threema)
{
    if (threema != m_threemaEmoji) {
        QSettings().setValue(QStringLiteral("threemaEmoji"), threema);
        m_threemaEmoji = threema;
        emit threemaEmojiChanged();
    }
}

void ThreeseBackend::setBiggerEmojis(bool bigger)
{
    if (bigger != m_biggerEmojis) {
        QSettings().setValue(QStringLiteral("biggerEmojis"), bigger);
        m_biggerEmojis = bigger;
        emit biggerEmojisChanged();
    }
}

void ThreeseBackend::setChatFontSize(int size)
{
    size = qBound(0, size, 2);
    if (size != m_chatFontSize) {
        QSettings().setValue(QStringLiteral("chatFontSize"), size);
        m_chatFontSize = size;
        emit chatFontSizeChanged();
    }
}

void ThreeseBackend::setAutoPlayGifs(bool play)
{
    if (play != m_autoPlayGifs) {
        QSettings().setValue(QStringLiteral("autoPlayGifs"), play);
        m_autoPlayGifs = play;
        emit autoPlayGifsChanged();
    }
}

void ThreeseBackend::setNotificationsEnabled(bool enabled)
{
    if (enabled != notificationsEnabled()) {
        QSettings().setValue(QStringLiteral("notifications"), enabled);
        emit notificationSettingsChanged();
    }
}

bool ThreeseBackend::notificationPreview() const
{
    return QSettings().value(QStringLiteral("notificationPreview"), true).toBool();
}

void ThreeseBackend::setNotificationPreview(bool preview)
{
    if (preview != notificationPreview()) {
        QSettings().setValue(QStringLiteral("notificationPreview"), preview);
        emit notificationSettingsChanged();
    }
}

namespace {
QString storedLogLevel()
{
    const QString level = QSettings().value(QStringLiteral("logLevel")).toString();
    return level == QLatin1String("info") || level == QLatin1String("debug") ? level : QStringLiteral("warn");
}
}

QString ThreeseBackend::logLevel() const
{
    return storedLogLevel();
}

void ThreeseBackend::setImageSize(int side)
{
    QSettings().setValue(QStringLiteral("imageSize"), side);
    if (m_core) {
        QJSValue none;
        call(QStringLiteral("setImageSize"), QVariantMap{{QStringLiteral("side"), side}}, none);
    }
    emit notificationSettingsChanged();
}

void ThreeseBackend::setLogLevel(const QString &level)
{
    if (level == logLevel()) {
        return;
    }
    QSettings().setValue(QStringLiteral("logLevel"), level);
    applyQtLogLevel();
    QJSValue none;
    call(QStringLiteral("setLogLevel"), QVariantMap{{QStringLiteral("level"), logLevel()}}, none);
    emit logLevelChanged();
}

// Warnings always.
void ThreeseBackend::applyQtLogLevel()
{
    const QString level = storedLogLevel();
    const QString info = level != QLatin1String("warn") ? QStringLiteral("true") : QStringLiteral("false");
    const QString debug = level == QLatin1String("debug") ? QStringLiteral("true") : QStringLiteral("false");
    // Only what we and the QML write: `*.debug=true` also turns on Qt's own categories, and
    // the scene graph alone then writes a line per frame — the log and the support package
    // drown in it and the app gets slower while someone is looking for a problem.
    QLoggingCategory::setFilterRules(QStringLiteral("*.info=false\n*.debug=false\n"
                                                    "default.info=%1\ndefault.debug=%2\n"
                                                    "qml.info=%1\nqml.debug=%2")
                                         .arg(info, debug));
}

bool ThreeseBackend::keepRunning() const
{
    return QSettings().value(QStringLiteral("keepRunning"), true).toBool();
}

void ThreeseBackend::setKeepRunning(bool keep)
{
    if (keep == keepRunning()) {
        return;
    }
    QSettings().setValue(QStringLiteral("keepRunning"), keep);
    QGuiApplication::setQuitOnLastWindowClosed(!keep);
    emit keepRunningChanged();
}

QObject *ThreeseBackend::contacts() const
{
    return m_contacts;
}

void ThreeseBackend::refreshUnread()
{
    if (!m_unreadTimer.isActive()) {
        m_unreadTimer.start();
    }
}

void ThreeseBackend::refreshContacts()
{
    QJSValue none;
    call(QStringLiteral("contacts"), QVariantMap(), none);
}

void ThreeseBackend::handleMessage(const QByteArray &json)
{
    if (m_discardCoreMessages) {
        return;
    }
    const QJsonObject message = QJsonDocument::fromJson(json).object();
    if (m_ending) {
        // Nobody is left to tell, and only whether it got there counts.
        if (message.contains(QStringLiteral("reply")) && message.value(QStringLiteral("ok")).toBool()) {
            const quint64 id = static_cast<quint64>(message.value(QStringLiteral("reply")).toDouble());
            m_mustArrive.remove(id);
            removeTempSources(id);
        }
        return;
    }

    if (message.contains(QStringLiteral("event"))) {
        const QString name = message.value(QStringLiteral("event")).toString();
        if (name == QLatin1String("started")) {
            setUpgrading(false);
            if (m_removingData) {
                // New database, new key, a core running on them: nothing left to finish.
                QSettings settings;
                settings.remove(RemovalPendingKey);
                settings.sync();
                m_removingData = false;
                m_dataRemovedSent = false;
                QGuiApplication::setQuitOnLastWindowClosed(!keepRunning());
            }
            refresh();
        } else if (name == QLatin1String("upgrading")) {
            setUpgrading(true);
        } else if (name == QLatin1String("fatal")) {
            failFromCore(message);
        } else if (name == QLatin1String("licenseChanged")) {
            refresh();
        } else if (name == QLatin1String("contactsChanged")) {
            refreshContacts();
        } else if (name == QLatin1String("chatDeleted")) {
            // A group's or list's tone may have gone with it.
            refreshChatTones();
        } else if (name == QLatin1String("connectionState")) {
            m_connectionState = message.value(QStringLiteral("state")).toString();
            emit connectionStateChanged();
        }
        // The cover's count and name, whether or not a chat list is looking (it only reloads when
        // seen): what brings or takes unread chats, and what renames them.
        if (name == QLatin1String("messageAdded") || name == QLatin1String("started")
            || name == QLatin1String("contactsChanged") || name == QLatin1String("groupsChanged")
            || name == QLatin1String("chatsChanged") || name == QLatin1String("privateChanged")
            || name == QLatin1String("chatDeleted")) {
            refreshUnread();
        }
        emit coreEvent(name, message.toVariantMap());
        return;
    }

    const quint64 id = static_cast<quint64>(message.value(QStringLiteral("reply")).toDouble());
    m_mustArrive.remove(id);
    const bool ok = message.value(QStringLiteral("ok")).toBool();
    const QJsonObject result = message.value(QStringLiteral("result")).toObject();
    const QJsonObject error = message.value(QStringLiteral("error")).toObject();

    // Status replies drive the onboarding state machine. Only those: message replies have a
    // (delivery) `state` too.
    const bool statusReply = m_statusRequests.remove(id) && !m_removingData;
    if (statusReply && !ok) {
        // The core is up and can't read what it needs to go on. Only at the start: a status that
        // fails later must not take a running app to the error page.
        if (m_state == QLatin1String("starting")) {
            failFromCore(error);
        }
    } else if (statusReply) {
        const QString previous = m_state;
        m_state = result.value(QStringLiteral("state")).toString();
        m_identity = result.value(QStringLiteral("identity")).toString();
        if (previous == QLatin1String("error")) {
            // The core could read after all. It runs now, so the window closing must not end it.
            m_errorKind.clear();
            m_errorText.clear();
            QGuiApplication::setQuitOnLastWindowClosed(!keepRunning());
        }
        emit stateChanged();
        if (m_state == QLatin1String("ready") && previous != m_state) {
            refreshContacts();
            reportNetwork();
            importOldChatTones();
            refreshChatTones();
        } else if (m_state != QLatin1String("ready")) {
            m_chatTones.clear();
        }
    }
    // Conversation list and unreadSummary replies keep the unread total and the chat on the cover
    // current. The core counts the chat list in every one of these lists, so the archive's and
    // the private chats' own replies don't change them.
    if (ok && result.contains(QStringLiteral("unreadTotal"))) {
        const int unread = result.value(QStringLiteral("unreadTotal")).toInt();
        if (unread != m_unreadCount) {
            m_unreadCount = unread;
            emit unreadCountChanged();
        }
        const QString from = result.value(QStringLiteral("unreadFrom")).toString();
        if (from != m_unreadFrom) {
            m_unreadFrom = from;
            emit unreadFromChanged();
        }
        const QString chat = result.value(QStringLiteral("unreadChat")).toString();
        if (chat != m_unreadChat) {
            m_unreadChat = chat;
            emit unreadChatChanged();
        }
    }
    // Contact list replies feed the shared model. By request, not by the look of the reply: the
    // summaries of a backup import and a Safe restore have a `contacts` of their own.
    if (m_contactRequests.remove(id) && ok) {
        m_contacts->update(result.value(QStringLiteral("contacts")).toArray().toVariantList());
    }

    const NativeCallback native = m_nativeCallbacks.take(id);
    if (native) {
        // On failure: the error's `kind` and `message`.
        native(ok, ok ? result.toVariantMap() : error.toVariantMap(), error.value(QStringLiteral("kind")).toString());
    }
    QJSValue answer;
    QJSValue callback = m_callbacks.take(id);
    if (callback.isCallable()) {
        QJSValueList arguments;
        arguments << QJSValue(ok);
        arguments << coreResultForScript(m_engine, message.value(QStringLiteral("result")));
        arguments << QJSValue(error.value(QStringLiteral("kind")).toString());
        arguments << QJSValue(error.value(QStringLiteral("message")).toString());
        answer = callback.call(arguments);
    }
    // What the core has now read into its own store; a refused file only if nobody took it back.
    removeTempSources(id, ok ? QJSValue() : answer);
}
