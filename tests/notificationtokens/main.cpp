// SPDX-License-Identifier: AGPL-3.0-only
// The notification actions' tokens: lipstick closes a notification before the action reaches us,
// also when the action is what starts the app, so a token has to outlive its notification for a
// while, and no longer. An action can arrive before the core runs, and then waits for it.
// Exit 0: passed.
#include "dbusinterface.h"

#include <QCoreApplication>
#include <QDateTime>
#include <QFile>
#include <QJSEngine>
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

// How many replies were lost on a token since the last look.
int lostReplies = 0;

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
    check(!markRead(dbus, QString()), "an empty token is ignored");
    check(!markRead(dbus, token.left(31)), "a token cut short is ignored");
    check(!markRead(dbus, token + QLatin1Char('0')), "a token too long is ignored");
    check(!reply(dbus, QString(), QStringLiteral("hello")), "no reply without a token");
    // Another chat's token opens only that chat.
    const QString other = dbus.actionToken(QStringLiteral("EFGH5678"), true);
    check(!markRead(dbus, other), "another chat's token is ignored");
    check(!reply(dbus, other, QStringLiteral("hello")), "another chat's token sends no reply");
    dbus.forgetToken(QStringLiteral("EFGH5678"));
    check(markRead(dbus, token), "the notification's action gets through");
    check(!markRead(dbus, token), "a token works once");

    // Tapping closes the notification, then the action arrives.
    const QString second = dbus.actionToken(chat, true);
    dbus.releaseToken(chat);
    check(reply(dbus, second, QStringLiteral("hello")), "a reply after the notification closed");
    check(!reply(dbus, second, QStringLiteral("hello")), "the reply used the token up");
    check(graces() == 0, "nothing stays behind after the action");
    lostReplies = 0;
    QObject::connect(&dbus, &DBusInterface::replyLost, [] { ++lostReplies; });
    check(!reply(dbus, second, QStringLiteral("hello")), "again without a token");
    check(lostReplies == 1, "a reply without its token is said");
    check(!reply(dbus, second, QStringLiteral("  ")), "a blank one without a token");
    check(lostReplies == 1, "losing a blank reply loses nothing");
    check(!markRead(dbus, second), "mark as read without a token");
    check(lostReplies == 1, "nor does a mark as read");

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

// What arrives before the core runs waits for it (qml/components/NotificationActions.js).
void testActionsWaitForTheCore()
{
    QFile file(QStringLiteral(NOTIFICATION_ACTIONS_JS));
    check(file.open(QIODevice::ReadOnly), "the actions' script");
    // Only QML knows about the pragma.
    const QString script = QString::fromUtf8(file.readAll()).replace(QStringLiteral(".pragma library"), QString());
    QJSEngine engine;
    const QJSValue loaded = engine.evaluate(script);
    check(!loaded.isError(), "the script loads");
    // The identities and texts released in this state, in one string.
    const auto release = [&engine](const char *state) {
        return engine.evaluate(QStringLiteral("release('%1').map(function(a) {"
                                              " return a.identity + ':' + (a.text === undefined ? '' : a.text) }).join()")
                                   .arg(QLatin1String(state))).toString();
    };
    const auto replyWaiting = [&engine]() { return engine.evaluate(QStringLiteral("replyWaiting()")).toBool(); };

    check(!replyWaiting(), "nothing waits at first");
    engine.evaluate(QStringLiteral("add({ identity: 'ABCD1234' })"));
    check(!replyWaiting(), "mark as read is no reply");
    engine.evaluate(QStringLiteral("add({ identity: 'EFGH5678', text: 'hello' })"));
    check(replyWaiting(), "a reply waits");
    check(release("starting").isEmpty(), "no core while starting");
    check(release("locked").isEmpty(), "no core before the passphrase");
    check(release("starting").isEmpty(), "no core right after the passphrase");
    check(replyWaiting(), "still waiting");
    check(release("ready") == QLatin1String("ABCD1234:,EFGH5678:hello"), "the core takes them in order");
    check(release("ready").isEmpty(), "and only once");
    check(!replyWaiting(), "nothing waits any more");

    engine.evaluate(QStringLiteral("add({ identity: 'ABCD1234', text: 'hello' })"));
    check(release("needsLicense") == QLatin1String("ABCD1234:hello"), "the core answers for itself in other states");

    engine.evaluate(QStringLiteral("add({ identity: 'ABCD1234', text: 'hello' })"));
    check(release("error").isEmpty(), "no core after a failed start");
    check(engine.evaluate(QStringLiteral("repliesToKeep().length")).toInt() == 1, "a reply to keep");
    check(!engine.evaluate(QStringLiteral("allRepliesKept()")).toBool(), "without its copy");
    engine.evaluate(QStringLiteral("repliesToKeep()[0].kept = true"));
    check(engine.evaluate(QStringLiteral("repliesToKeep().length")).toInt() == 0, "nothing left to keep");
    check(engine.evaluate(QStringLiteral("allRepliesKept()")).toBool(), "with its copy");
    check(engine.evaluate(QStringLiteral("drop()")).toBool(), "the failed start lost a reply");
    check(!engine.evaluate(QStringLiteral("drop()")).toBool(), "and nothing is left");
    engine.evaluate(QStringLiteral("add({ identity: 'ABCD1234' })"));
    check(!engine.evaluate(QStringLiteral("drop()")).toBool(), "losing a mark as read loses no reply");
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
    testActionsWaitForTheCore();

    if (failures == 0) {
        std::printf("passed\n");
    }
    return failures == 0 ? 0 : 1;
}
