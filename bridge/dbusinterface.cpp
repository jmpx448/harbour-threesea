// SPDX-License-Identifier: AGPL-3.0-only
#include "dbusinterface.h"

#include <QDBusConnection>
#include <QFile>
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

QString DBusInterface::actionToken(const QString &identity, bool renew)
{
    const QString key = QStringLiteral("notificationTokens/") + identity;
    QSettings settings;
    QString token = settings.value(key).toString();
    if (renew || token.isEmpty()) {
        QFile random(QStringLiteral("/dev/urandom"));
        if (!random.open(QIODevice::ReadOnly)) {
            return QString();
        }
        token = QString::fromLatin1(random.read(16).toHex());
        settings.setValue(key, token);
    }
    return token;
}

bool DBusInterface::tokenValid(const QString &identity, const QString &token) const
{
    const QString expected = QSettings().value(QStringLiteral("notificationTokens/") + identity).toString();
    if (expected.size() != 32 || token.size() != expected.size()) {
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
        return;
    }
    emit called();
    emit replyRequested(identity, text);
}

void DBusInterface::markRead(const QString &identity, const QString &token)
{
    if (!tokenValid(identity, token)) {
        qWarning("threesea: notification action without its token ignored");
        return;
    }
    emit called();
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
