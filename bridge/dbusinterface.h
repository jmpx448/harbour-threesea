// SPDX-License-Identifier: AGPL-3.0-only
#ifndef DBUSINTERFACE_H
#define DBUSINTERFACE_H

#include <QObject>
#include <QString>
#include <QStringList>

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
// notification by us and kept in the app's private settings; calls without it are ignored
// (security audits 2026-10-27, S07/D04).
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
    // The token for this chat's notification actions; `renew`: a new one (a new notification).
    Q_INVOKABLE QString actionToken(const QString &identity, bool renew = false);

private:
    bool tokenValid(const QString &identity, const QString &token) const;

public slots:
    Q_SCRIPTABLE void openChat(const QString &identity);
    Q_SCRIPTABLE void activate();
    Q_SCRIPTABLE void activate(const QStringList &urls);
    Q_SCRIPTABLE void reply(const QString &identity, const QString &token, const QString &text);
    Q_SCRIPTABLE void markRead(const QString &identity, const QString &token);
    Q_SCRIPTABLE void background();
    // The update notification's action: show the window with the update (ADR 0014).
    Q_SCRIPTABLE void showUpdate();
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
    void activateRequested();
    void showUpdateRequested();
    void openUrlRequested(const QString &url);
#ifdef THREESEA_DEMO
    void demoDataRequested(bool remove);
#endif
};

#endif // DBUSINTERFACE_H
