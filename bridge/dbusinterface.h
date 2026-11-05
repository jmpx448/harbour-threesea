// SPDX-License-Identifier: AGPL-3.0-only
#ifndef DBUSINTERFACE_H
#define DBUSINTERFACE_H

#include <QObject>
#include <QString>
#include <QStringList>
#include <QTimer>

#include <functional>

// Session-bus entry points for notification actions (lipstick only delivers them via D-Bus):
// "default" calls openChat() so tapping opens the chat, "reply" (an input action: lipstick appends
// the typed text) calls reply(), "read" calls markRead(). Service name follows
// Sailjail's OrganizationName.ApplicationName so the sandbox lets us own it. Links we handle
// (.desktop MimeType x-scheme-handler / x-url-handler) arrive as activate(as) from
// libcontentaction, the launcher calls activate() without arguments. background() starts the app
// without a window (the Full flavour's login service).
// Reply and Mark as read change the chat, and any process on the session bus can call this
// service; lipstick makes the call through a short-lived helper (mliteremoteaction) the sandbox
// can't identify. So these two actions carry a random token per chat, put into the
// notification by us and kept in the app's private settings, under a salted hash of the chat, so
// that the file doesn't list who one chats with; calls without it are ignored, and for a reply the
// window says that it wasn't sent (security audits 2026-10-27, S07/D04). A token lives as long as its notification and a grace period after it:
// lipstick closes the notification first and makes the call afterwards, also when that call starts
// the app. Using it for an action ends it at once. The salt is in the same file and IDs are few
// enough to try them all offline, so the hash only stops casual reading of the file: what limits
// a copy of the file is that it holds just the chats with a token alive at that moment.
class DBusInterface : public QObject
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.threesea.harbourthreesea")

public:
    static const char *serviceName();
    static const char *objectPath();
    static const char *interfaceName();

    explicit DBusInterface(QObject *parent = nullptr);
    bool registerOnBus();
    // Asked first thing for a call that wants the window: true means no window from this process
    // (see AppLifecycle), and the call is dropped.
    void setWindowGuard(std::function<bool()> guard) { m_windowGuard = std::move(guard); }
    // The token for this chat's notification actions; `renew`: a new one (a new notification).
    Q_INVOKABLE QString actionToken(const QString &identity, bool renew = false);
    // The chat's notification closed, by a tap on one of its actions as well: the grace starts.
    Q_INVOKABLE void releaseToken(const QString &identity);
    // We took the chat's notification away ourselves, so no action can be on its way.
    Q_INVOKABLE void forgetToken(const QString &identity);
    // At startup: the chats whose notifications are still there keep their token. For the others
    // the grace starts, unless it already runs: dismissed while we weren't running, or tapped,
    // and that tap is what started us.
    Q_INVOKABLE void retainTokens(const QStringList &identities);
    // The ID was removed from this device.
    Q_INVOKABLE void forgetAllTokens();

private:
    void migrateTokens();
    // Removes tokens past their grace; the timer comes back for the next one.
    void pruneTokens();
    bool tokenValid(const QString &identity, const QString &token) const;
    bool windowRefused() const;

    QTimer m_tokenPrune;
    std::function<bool()> m_windowGuard;

public slots:
    Q_SCRIPTABLE void openChat(const QString &identity);
    Q_SCRIPTABLE void activate();
    Q_SCRIPTABLE void activate(const QStringList &urls);
    Q_SCRIPTABLE void reply(const QString &identity, const QString &token, const QString &text);
    Q_SCRIPTABLE void markRead(const QString &identity, const QString &token);
    // The incoming call's notification. Its token keeps anyone else on the bus from turning the
    // microphone on (key: CallTokenKey).
    Q_SCRIPTABLE void answerCall(const QString &token);
    Q_SCRIPTABLE void declineCall(const QString &token);
    Q_SCRIPTABLE void background();
    // The update notification's action: show the window with the update (ADR 0014).
    Q_SCRIPTABLE void showUpdate();
    // The Threema Web notifications' action: show the window with the sessions page.
    Q_SCRIPTABLE void showWebSessions();
    // The backup notifications' action: show the window with the Threema Safe page.
    Q_SCRIPTABLE void showSafe();
#ifdef THREESEA_DEMO
    // Emulator builds only (`qmake CONFIG+=demo`): fill the store with made-up chats, or take
    // them away again (scripts/emulator-demo.sh). Not compiled into any build for a phone, so
    // the method does not even exist on the bus there.
    Q_SCRIPTABLE void demoData(bool remove);
#endif

signals:
    // Any of the methods above was called (the start was for a reason, see main.cpp).
    void called();
    void backgroundRequested();
    void openChatRequested(const QString &identity);
    void replyRequested(const QString &identity, const QString &text);
    void markReadRequested(const QString &identity);
    void answerCallRequested();
    void declineCallRequested();
    // A reply with nothing in it: nothing to do, and that is done.
    void blankReply();
    // A reply came without a valid token.
    void replyLost();
    void activateRequested();
    void showUpdateRequested();
    void showWebSessionsRequested();
    void showSafeRequested();
    void openUrlRequested(const QString &url);
#ifdef THREESEA_DEMO
    void demoDataRequested(bool remove);
#endif
};

#endif // DBUSINTERFACE_H
