// SPDX-License-Identifier: AGPL-3.0-only
// The notification actions' tokens: lipstick closes a notification before the action reaches us,
// also when the action is what starts the app, so a token has to outlive its notification for a
// while, and no longer. Exit 0: passed.
#include "dbusinterface.h"

#include <QCoreApplication>
#include <QDateTime>
#include <QFile>
#include <QSettings>
#include <QStringList>
#include <QTemporaryDir>
#include <cstdio>

namespace {

int failures = 0;

const QString chat = QStringLiteral("ABCD1234");
const QString expiryGroup = QStringLiteral("notificationActionTokenExpiry");

void check(bool ok, const char *what)
{
    if (!ok) {
        std::printf("FAILED: %s\n", what);
        ++failures;
    }
}

// Calls the action and says whether it got through.
bool markRead(DBusInterface &dbus, const QString &token)
{
    bool got = false;
    const auto connection = QObject::connect(&dbus, &DBusInterface::markReadRequested, [&got]() { got = true; });
    dbus.markRead(chat, token);
    QObject::disconnect(connection);
    return got;
}

bool reply(DBusInterface &dbus, const QString &token, const QString &text)
{
    bool got = false;
    const auto connection = QObject::connect(&dbus, &DBusInterface::replyRequested, [&got]() { got = true; });
    dbus.reply(chat, token, text);
    QObject::disconnect(connection);
    return got;
}

// Lets the grace of everything that is in one run out.
void expireGraces()
{
    QSettings settings;
    settings.beginGroup(expiryGroup);
    const QStringList hashes = settings.allKeys();
    for (const QString &hash : hashes) {
        settings.setValue(hash, QDateTime::currentMSecsSinceEpoch() - 1);
    }
}

int graces()
{
    QSettings settings;
    settings.beginGroup(expiryGroup);
    return settings.allKeys().size();
}

void reset()
{
    QSettings().clear();
}

void testWhileRunning()
{
    reset();
    DBusInterface dbus;
    const QString token = dbus.actionToken(chat, true);
    check(token.size() == 32, "a token");
    check(!markRead(dbus, QString(32, QLatin1Char('0'))), "another token is ignored");
    check(markRead(dbus, token), "the notification's action gets through");
    check(!markRead(dbus, token), "a token works once");

    // Tapping closes the notification, then the action arrives.
    const QString second = dbus.actionToken(chat, true);
    dbus.releaseToken(chat);
    check(reply(dbus, second, QStringLiteral("hello")), "a reply after the notification closed");
    check(!reply(dbus, second, QStringLiteral("hello")), "the reply used the token up");
    check(graces() == 0, "nothing stays behind after the action");

    const QString third = dbus.actionToken(chat, true);
    dbus.releaseToken(chat);
    check(!reply(dbus, third, QStringLiteral("  ")), "a blank reply sends nothing");
    check(markRead(dbus, third), "a blank reply keeps the token");

    const QString fourth = dbus.actionToken(chat, true);
    dbus.forgetToken(chat);
    check(!markRead(dbus, fourth), "a notification we took away takes its token along");

    dbus.releaseToken(QStringLiteral("EFGH5678"));
    check(graces() == 0, "closing without a token leaves nothing");
}

void testGraceEnds()
{
    reset();
    DBusInterface dbus;
    const QString token = dbus.actionToken(chat, true);
    dbus.releaseToken(chat);
    expireGraces();
    check(!markRead(dbus, token), "past its grace a token counts no more");
    dbus.retainTokens(QStringList());
    check(graces() == 0, "past its grace it is removed");
    check(QSettings().allKeys().filter(QStringLiteral("notificationActionTokens/")).isEmpty(), "with the token");
}

void testNewNotification()
{
    reset();
    DBusInterface dbus;
    const QString first = dbus.actionToken(chat, true);
    const QString renewed = dbus.actionToken(chat, true);
    check(first != renewed, "a new notification gets a new token");
    check(dbus.actionToken(chat, false) == renewed, "a notification taken over keeps its token");

    // A message arrives between the tap and its action.
    dbus.releaseToken(chat);
    check(dbus.actionToken(chat, true) == renewed, "during the grace the new notification shares the token");
    check(graces() == 0, "and the token lives with that notification");
    check(markRead(dbus, renewed), "the tapped action gets through");

    const QString third = dbus.actionToken(chat, true);
    dbus.releaseToken(chat);
    expireGraces();
    check(dbus.actionToken(chat, true) != third, "past the grace a new notification gets a new token");
}

void testColdStart()
{
    reset();
    QString tapped;
    QString shown;
    {
        DBusInterface dbus;
        tapped = dbus.actionToken(chat, true);
        shown = dbus.actionToken(QStringLiteral("g:0011223344556677"), true);
    }
    // The tap removed its notification and is starting us; the action waits in the bus's queue.
    DBusInterface dbus;
    dbus.retainTokens(QStringList{QStringLiteral("g:0011223344556677")});
    check(graces() == 1, "only the chat without a notification goes into grace");
    check(markRead(dbus, tapped), "the action that started the app gets through");
    check(dbus.actionToken(QStringLiteral("g:0011223344556677"), false) == shown, "a shown notification keeps its token");

    // The next start, long after a dismissal.
    const QString dismissed = dbus.actionToken(chat, true);
    {
        DBusInterface earlier;
        earlier.retainTokens(QStringList{QStringLiteral("g:0011223344556677")});
    }
    expireGraces();
    DBusInterface later;
    later.retainTokens(QStringList{QStringLiteral("g:0011223344556677")});
    check(!markRead(later, dismissed), "dismissed before an earlier start: gone");
    check(graces() == 0, "and removed");
}

void testNoPlainChats(const QString &file)
{
    reset();
    {
        QSettings settings;
        settings.setValue(QStringLiteral("notificationTokens/") + chat, QString(32, QLatin1Char('a')));
    }
    DBusInterface dbus;
    check(markRead(dbus, QString(32, QLatin1Char('a'))), "a token from before the hashes still works");
    dbus.actionToken(chat, true);
    dbus.releaseToken(chat);
    QSettings().sync();
    QFile settingsFile(file);
    check(settingsFile.open(QIODevice::ReadOnly), "the settings file");
    check(!settingsFile.readAll().contains(chat.toLatin1()), "the file doesn't name the chat");

    dbus.forgetAllTokens();
    check(QSettings().allKeys().filter(QStringLiteral("notificationActionToken")).isEmpty(), "all gone with the ID");
}

} // namespace

int main(int argc, char **argv)
{
    QCoreApplication app(argc, argv);
    QCoreApplication::setOrganizationName(QStringLiteral("threesea-test"));
    QCoreApplication::setApplicationName(QStringLiteral("tokens"));
    QTemporaryDir dir;
    QSettings::setPath(QSettings::NativeFormat, QSettings::UserScope, dir.path());

    testWhileRunning();
    testGraceEnds();
    testNewNotification();
    testColdStart();
    testNoPlainChats(QSettings().fileName());

    if (failures == 0) {
        std::printf("passed\n");
    }
    return failures == 0 ? 0 : 1;
}
