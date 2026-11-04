// SPDX-License-Identifier: AGPL-3.0-only
#include "dbusinterface.h"

#include <QDBusConnection>
#include <QCryptographicHash>
#include <QDateTime>
#include <QFile>
#include <QMessageAuthenticationCode>
#include <QSet>
#include <QSettings>
#include <QDebug>

const char *DBusInterface::serviceName()
{
    return "org.threesea.harbour-threesea";
}

const char *DBusInterface::objectPath()
{
    return "/";
}

const char *DBusInterface::interfaceName()
{
    return "org.threesea.harbourthreesea";
}

DBusInterface::DBusInterface(QObject *parent)
    : QObject(parent)
{
    m_tokenPrune.setSingleShot(true);
    connect(&m_tokenPrune, &QTimer::timeout, this, &DBusInterface::pruneTokens);
    migrateTokens();
}

bool DBusInterface::registerOnBus()
{
    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.registerObject(QString::fromLatin1(objectPath()), this, QDBusConnection::ExportScriptableSlots)) {
        qWarning("threesea: could not register D-Bus object");
        return false;
    }
    if (!bus.registerService(QString::fromLatin1(serviceName()))) {
        qWarning("threesea: could not own D-Bus name (another instance running?)");
        return false;
    }
    return true;
}

void DBusInterface::openChat(const QString &identity)
{
    emit called();
    emit openChatRequested(identity);
}

void DBusInterface::activate()
{
    emit called();
    emit activateRequested();
}

void DBusInterface::showUpdate()
{
    emit called();
    emit activateRequested();
    emit showUpdateRequested();
}

void DBusInterface::activate(const QStringList &urls)
{
    emit called();
    emit activateRequested();
    for (const QString &url : urls) {
        emit openUrlRequested(url);
    }
}

namespace {

const QString tokenGroup = QStringLiteral("notificationActionTokens");
// When the token of a closed notification stops counting (ms since the epoch, under the same hash).
const QString expiryGroup = QStringLiteral("notificationActionTokenExpiry");
const QString tokenSaltKey = QStringLiteral("notificationActionTokenSalt");
// Until 0.2.0: tokens under the chat's plain ID or group key.
const QString oldTokenGroup = QStringLiteral("notificationTokens");

// How long a token outlives its notification. Tapping Reply or Mark as read makes lipstick close
// the notification first and call us afterwards, through a helper process, or after starting us;
// the wait for a device unlock can come in between, which may be a night. A token of a notification
// still showing lives for as long as that, so this adds little, and the app lock holds back what
// the action asks for (the window).
const qint64 tokenGraceMs = 12 * 60 * 60 * 1000;

QByteArray randomBytes(int count)
{
    QFile random(QStringLiteral("/dev/urandom"));
    if (!random.open(QIODevice::ReadOnly)) {
        return QByteArray();
    }
    const QByteArray bytes = random.read(count);
    return bytes.size() == count ? bytes : QByteArray();
}

// What a chat's token is kept under: a keyed hash of the chat, so the settings file doesn't list
// who one chats with. Empty without randomness for a salt.
QString chatHash(const QString &identity)
{
    QSettings settings;
    QByteArray salt = QByteArray::fromHex(settings.value(tokenSaltKey).toString().toLatin1());
    if (salt.size() != 32) {
        salt = randomBytes(32);
        if (salt.isEmpty()) {
            return QString();
        }
        settings.setValue(tokenSaltKey, QString::fromLatin1(salt.toHex()));
    }
    const QByteArray hash = QMessageAuthenticationCode::hash(identity.toUtf8(), salt, QCryptographicHash::Sha256);
    return QString::fromLatin1(hash.toHex());
}

QString tokenKey(const QString &hash)
{
    return tokenGroup + QLatin1Char('/') + hash;
}

QString expiryKey(const QString &hash)
{
    return expiryGroup + QLatin1Char('/') + hash;
}

} // namespace

void DBusInterface::migrateTokens()
{
    QSettings settings;
    settings.beginGroup(oldTokenGroup);
    const QStringList identities = settings.allKeys();
    if (identities.isEmpty()) {
        return;
    }
    QStringList tokens;
    for (const QString &identity : identities) {
        tokens << settings.value(identity).toString();
    }
    settings.endGroup();
    for (int i = 0; i < identities.size(); ++i) {
        const QString hash = chatHash(identities.at(i));
        if (!hash.isEmpty()) {
            settings.setValue(tokenKey(hash), tokens.at(i));
        }
    }
    settings.remove(oldTokenGroup);
}

QString DBusInterface::actionToken(const QString &identity, bool renew)
{
    const QString hash = chatHash(identity);
    if (hash.isEmpty()) {
        return QString();
    }
    QSettings settings;
    const qint64 expiry = settings.value(expiryKey(hash)).toLongLong();
    const qint64 now = QDateTime::currentMSecsSinceEpoch();
    // While the last notification's grace runs a tap on it may still be on its way, so the new
    // notification shares its token instead of cutting that action off.
    const bool inGrace = expiry > now;
    const bool expired = expiry != 0 && !inGrace;
    QString token = settings.value(tokenKey(hash)).toString();
    if (token.isEmpty() || expired || (renew && !inGrace)) {
        const QByteArray bytes = randomBytes(16);
        if (bytes.isEmpty()) {
            return QString();
        }
        token = QString::fromLatin1(bytes.toHex());
        settings.setValue(tokenKey(hash), token);
    }
    settings.remove(expiryKey(hash));
    return token;
}

void DBusInterface::releaseToken(const QString &identity)
{
    const QString hash = chatHash(identity);
    QSettings settings;
    if (!hash.isEmpty() && settings.contains(tokenKey(hash))) {
        settings.setValue(expiryKey(hash), QDateTime::currentMSecsSinceEpoch() + tokenGraceMs);
        pruneTokens();
    }
}

void DBusInterface::forgetToken(const QString &identity)
{
    const QString hash = chatHash(identity);
    if (!hash.isEmpty()) {
        QSettings settings;
        settings.remove(tokenKey(hash));
        settings.remove(expiryKey(hash));
    }
}

void DBusInterface::retainTokens(const QStringList &identities)
{
    QSet<QString> keep;
    for (const QString &identity : identities) {
        keep.insert(chatHash(identity));
    }
    QSettings settings;
    settings.beginGroup(tokenGroup);
    const QStringList hashes = settings.allKeys();
    settings.endGroup();
    const qint64 expiry = QDateTime::currentMSecsSinceEpoch() + tokenGraceMs;
    for (const QString &hash : hashes) {
        if (keep.contains(hash)) {
            settings.remove(expiryKey(hash));
        } else if (!settings.contains(expiryKey(hash))) {
            settings.setValue(expiryKey(hash), expiry);
        }
    }
    pruneTokens();
}

void DBusInterface::pruneTokens()
{
    QSettings settings;
    settings.beginGroup(expiryGroup);
    const QStringList hashes = settings.allKeys();
    settings.endGroup();
    const qint64 now = QDateTime::currentMSecsSinceEpoch();
    qint64 next = 0;
    for (const QString &hash : hashes) {
        const qint64 expiry = settings.value(expiryKey(hash)).toLongLong();
        if (expiry <= now) {
            settings.remove(tokenKey(hash));
            settings.remove(expiryKey(hash));
        } else if (next == 0 || expiry < next) {
            next = expiry;
        }
    }
    if (next != 0) {
        m_tokenPrune.start(int(qBound(qint64(1000), next - now, tokenGraceMs)));
    } else {
        m_tokenPrune.stop();
    }
}

void DBusInterface::forgetAllTokens()
{
    QSettings settings;
    settings.remove(tokenGroup);
    settings.remove(expiryGroup);
    settings.remove(tokenSaltKey);
    m_tokenPrune.stop();
}

bool DBusInterface::tokenValid(const QString &identity, const QString &token) const
{
    const QString hash = chatHash(identity);
    if (hash.isEmpty()) {
        return false;
    }
    QSettings settings;
    const QString expected = settings.value(tokenKey(hash)).toString();
    const qint64 expiry = settings.value(expiryKey(hash)).toLongLong();
    if (expected.size() != 32 || token.size() != expected.size()
        || (expiry != 0 && expiry <= QDateTime::currentMSecsSinceEpoch())) {
        return false;
    }
    // Compared in full (no early exit).
    int difference = 0;
    for (int i = 0; i < expected.size(); ++i) {
        difference |= expected.at(i).unicode() ^ token.at(i).unicode();
    }
    return difference == 0;
}

void DBusInterface::reply(const QString &identity, const QString &token, const QString &text)
{
    if (!tokenValid(identity, token)) {
        qWarning("threesea: notification action without its token ignored");
        // Someone may have typed this: the only copy of the words ends here. Not the caller's
        // chat or text: without the token nothing says they are real.
        if (!text.trimmed().isEmpty()) {
            emit replyLost();
        }
        return;
    }
    emit called();
    // A blank reply sends nothing, so it doesn't use the token up.
    if (text.trimmed().isEmpty()) {
        return;
    }
    forgetToken(identity);
    emit replyRequested(identity, text);
}

void DBusInterface::markRead(const QString &identity, const QString &token)
{
    if (!tokenValid(identity, token)) {
        qWarning("threesea: notification action without its token ignored");
        return;
    }
    emit called();
    forgetToken(identity);
    emit markReadRequested(identity);
}

#ifdef THREESEA_DEMO
void DBusInterface::demoData(bool remove)
{
    emit called();
    emit demoDataRequested(remove);
}
#endif

void DBusInterface::background()
{
    emit called();
    emit backgroundRequested();
}
