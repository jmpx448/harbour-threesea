// SPDX-License-Identifier: AGPL-3.0-only
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QGuiApplication>
#include <QQmlContext>
#include <QQmlEngine>
#include <QQuickView>
#include <QScopedPointer>
#include <QSettings>
#include <QSocketNotifier>
#include <QStandardPaths>
#include <QTimer>
#include <QtQml>

#include <csignal>
#include <sys/socket.h>
#include <unistd.h>

#include <sailfishapp.h>

#include "animatedimage.h"
#include "applifecycle.h"
#include "callmanager.h"
#include "callvideo.h"
#include "groupcallmanager.h"
#include "dbusinterface.h"
#include "emojiimageprovider.h"
#include "emojihighlighter.h"
#include "endofprocess.h"
#include "enterkeyfilter.h"
#include "jsonlistmodel.h"
#include "mediaimageprovider.h"
#include "proximitymonitor.h"
#include "softsound.h"
#include "offlinenetwork.h"
#include "threesebackend.h"
#include <QEvent>
#include <functional>

#include "videoplayer.h"
#include "voiceplayer.h"
#include "voicerecorder.h"

#include <threesea_core.h>

namespace {

// Qt and QML warnings also land in the in-app log (Settings → Advanced → Log), beside the
// core's own lines; the previous handler keeps them going to the journal as before.
QtMessageHandler previousMessageHandler = nullptr;

void forwardMessageToLog(QtMsgType type, const QMessageLogContext &context, const QString &message)
{
    // Only warnings and worse: Qt's own debug/info stream is a firehose of internals that
    // would drown the core's lines (and the journal keeps carrying everything anyway).
    if (type != QtDebugMsg && type != QtInfoMsg) {
        threesea_core_log("warn", message.toUtf8().constData());
    }
    if (previousMessageHandler) {
        previousMessageHandler(type, context, message);
    }
}

// SIGTERM (a package update stops the running instance, rpm/harbour-threesea.spec) quits
// through the event loop, so the core thread stops and the store closes like on a normal
// quit. The handler only writes to a socket (async-signal-safe); Qt reads it.
int termSocket[2] = {-1, -1};

void onTerm(int)
{
    const char byte = 1;
    if (::write(termSocket[0], &byte, 1) < 0) {
        // Nothing to do in a signal handler.
    }
}

void quitOnTerm(QObject *parent)
{
    if (::socketpair(AF_UNIX, SOCK_STREAM, 0, termSocket) != 0) {
        return;
    }
    QSocketNotifier *notifier = new QSocketNotifier(termSocket[1], QSocketNotifier::Read, parent);
    QObject::connect(notifier, &QSocketNotifier::activated, parent, [notifier]() {
        notifier->setEnabled(false);
        char byte;
        if (::read(termSocket[1], &byte, 1) < 0) {
            // Quitting regardless.
        }
        qWarning("terminated: quitting");
        QCoreApplication::quit();
    });
    struct sigaction action = {};
    action.sa_handler = onTerm;
    sigemptyset(&action.sa_mask);
    action.sa_flags = SA_RESTART;
    ::sigaction(SIGTERM, &action, nullptr);
}

// Whether an instance owns the name (possibly without a window, see below).
bool instanceRunning()
{
    QDBusConnection bus = QDBusConnection::sessionBus();
    return bus.interface() && bus.interface()->isServiceRegistered(QString::fromLatin1(DBusInterface::serviceName()));
}

// Asks the running instance to show itself. Says whether it answered.
bool activateRunningInstance()
{
    QDBusMessage call = QDBusMessage::createMethodCall(
        QString::fromLatin1(DBusInterface::serviceName()), QString::fromLatin1(DBusInterface::objectPath()),
        QString::fromLatin1(DBusInterface::interfaceName()), QStringLiteral("activate"));
    return QDBusConnection::sessionBus().call(call, QDBus::Block, 3000).type() == QDBusMessage::ReplyMessage;
}

} // namespace

// Calls back when the watched window is closed by the user.
class CloseWatcher : public QObject
{
public:
    CloseWatcher(std::function<void()> closed, QObject *parent)
        : QObject(parent)
        , m_closed(std::move(closed))
    {
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override
    {
        if (event->type() == QEvent::Close) {
            m_closed();
        }
        return QObject::eventFilter(watched, event);
    }

private:
    std::function<void()> m_closed;
};

int main(int argc, char *argv[])
{
    QScopedPointer<QGuiApplication> app(SailfishApp::application(argc, argv));
    previousMessageHandler = qInstallMessageHandler(forwardMessageToLog);
    app->setOrganizationName(QStringLiteral("org.threesea"));
    app->setApplicationName(QStringLiteral("harbour-threesea"));
    app->setApplicationVersion(QStringLiteral(APP_VERSION));
    // Sailjail only lets us write in ~/.config/<org>/<app>/, but QSettings() files go to
    // ~/.config/<org>/<app>.conf: device-local settings (background running, notifications)
    // were silently lost on every restart. Keep them inside the allowed directory.
    QSettings::setPath(QSettings::NativeFormat, QSettings::UserScope,
                       QStandardPaths::writableLocation(QStandardPaths::AppConfigLocation));

    // One process for the store: two would log in with the same ID and throw each other out, and
    // each would empty the other's captures. So this one ends also if the other is busy and doesn't
    // answer in time; it may still act on the call.
    if (instanceRunning()) {
        return activateRunningInstance() ? 0 : 1;
    }
    quitOnTerm(app.data());
    ThreeseBackend::applyQtLogLevel();

    // Keep running when the window is closed, so the chat server connection stays up and
    // messages keep arriving (Harbour-compatible: same process, no daemon). Opening the app
    // again shows the window of this process (activateRunningInstance above). Users can turn
    // it off in Settings ("Keep running in the background").
    const bool keepRunning = QSettings().value(QStringLiteral("keepRunning"), true).toBool();
    app->setQuitOnLastWindowClosed(!keepRunning);

    qmlRegisterType<JsonListModel>("harbour.threesea", 1, 0, "JsonListModel");
    qmlRegisterType<EnterKeyFilter>("harbour.threesea", 1, 0, "EnterKeyFilter");
    qmlRegisterType<EmojiHighlighter>("harbour.threesea", 1, 0, "EmojiHighlighter");
    qmlRegisterType<AnimatedImage>("harbour.threesea", 1, 0, "AnimatedMedia");
    qmlRegisterType<CallVideo>("harbour.threesea", 1, 0, "CallVideo");

    QScopedPointer<QQuickView> view(SailfishApp::createView());
    // Outlives the engine (destroyed after main returns).
    static OfflineNetworkFactory offlineNetwork;
    view->engine()->setNetworkAccessManagerFactory(&offlineNetwork);
    ThreeseBackend *backend = new ThreeseBackend(view->engine());
    view->rootContext()->setContextProperty(QStringLiteral("backend"), backend);
    AnimatedImage::setBackend(backend);
    view->engine()->addImageProvider(QStringLiteral("threesea"), new MediaImageProvider(backend));
    view->engine()->addImageProvider(QStringLiteral("emoji"), new EmojiImageProvider);
    view->rootContext()->setContextProperty(QStringLiteral("voiceRecorder"), backend->voiceRecorder());
    VoicePlayer *voicePlayer = new VoicePlayer(backend, view->engine());
    view->rootContext()->setContextProperty(QStringLiteral("voicePlayer"), voicePlayer);
    view->rootContext()->setContextProperty(QStringLiteral("proximity"), new ProximityMonitor(view->engine()));
    // Android's in-app sound: a message arriving in the chat the user is reading.
    view->rootContext()->setContextProperty(QStringLiteral("softSound"), new SoftSound(view->engine()));
    VideoPlayer *videoPlayer = new VideoPlayer(backend, view->engine());
    view->rootContext()->setContextProperty(QStringLiteral("videoPlayer"), videoPlayer);
    // Closing the window means done with it, also when the process stays for the connection.
    // The close itself, not visibleChanged: the window also goes invisible when it is minimized,
    // and playing on from the cover is what the system's media controls are for.
    // A recording goes too: with the window closed nothing shows that the microphone is on.
    VoiceRecorder *voiceRecorder = backend->voiceRecorder();
    view->installEventFilter(new CloseWatcher([voicePlayer, videoPlayer, voiceRecorder]() {
        voicePlayer->stop();
        videoPlayer->stop();
        voiceRecorder->cancel();
    }, view->engine()));

    CallManager *calls = new CallManager(backend, view->engine());
    view->rootContext()->setContextProperty(QStringLiteral("calls"), calls);
    GroupCallManager *groupCalls = new GroupCallManager(backend, view->engine());
    view->rootContext()->setContextProperty(QStringLiteral("groupCalls"), groupCalls);
    // One call has the microphone (like Android): both pipelines would capture it, and a group
    // would hear the private call. Here, not in the pages' buttons: the cover and a notification
    // get past those.
    QObject::connect(calls, &CallManager::aboutToTakeMicrophone, groupCalls, &GroupCallManager::leave);
    QObject::connect(groupCalls, &GroupCallManager::aboutToTakeMicrophone, calls, &CallManager::endCallWithMicrophone);
    // Without a window there is no cover and no call page to show that the microphone is on.
    view->installEventFilter(new CloseWatcher([calls, groupCalls]() {
        calls->endCallWithMicrophone();
        groupCalls->leave();
    }, view->engine()));
    // Removing the ID: nothing of it goes on playing, recording or calling, from the moment the
    // PIN is accepted (the device group alone can take a minute) and in any case once it happens.
    const auto stopPlayers = [voicePlayer, videoPlayer, voiceRecorder]() {
        voicePlayer->stop();
        videoPlayer->stop();
        voiceRecorder->cancel();
    };
    QObject::connect(backend, &ThreeseBackend::dataRemovalStarted, view->engine(), [calls, groupCalls, stopPlayers]() {
        if (calls->state() == QLatin1String("incoming")) {
            calls->reject();
        } else {
            calls->endCallWithMicrophone();
        }
        groupCalls->leave();
        stopPlayers();
    });
    QObject::connect(backend, &ThreeseBackend::dataRemoved, view->engine(), [calls, groupCalls, stopPlayers]() {
        calls->forget();
        groupCalls->forget();
        stopPlayers();
    });
    // An incoming call brings the window up (also when it was closed, the Full flavour's background run).
    QObject::connect(calls, &CallManager::incoming, view.data(), [&view]() {
        view->showFullScreen();
        view->raise();
        view->requestActivate();
    });

    DBusInterface *dbus = new DBusInterface(view->engine());
    view->rootContext()->setContextProperty(QStringLiteral("dbus"), dbus);
    QObject::connect(dbus, &DBusInterface::activateRequested, view.data(), [&view]() {
        view->showFullScreen();
        view->raise();
        view->requestActivate();
    });
    QObject::connect(dbus, &DBusInterface::openChatRequested, view.data(), [&view]() {
        if (!view->isVisible()) {
            view->showFullScreen();
        }
    });

#ifdef THREESEA_DEMO
    // The emulator's demo chats: asked for over D-Bus, so the interface has nothing of them.
    QObject::connect(dbus, &DBusInterface::demoDataRequested, backend, [backend](bool remove) {
        backend->request(QStringLiteral("demoData"), {{QStringLiteral("remove"), remove}},
                         [](bool ok, const QVariantMap &, const QString &kind) {
            if (ok) {
                qWarning("threesea: demo data done");
            } else {
                qWarning("threesea: demo data failed: %s", qPrintable(kind));
            }
        });
    });
#endif

    // Every launch comes through D-Bus activation (the .desktop's X-Maemo-Service, notification
    // actions, links, sharing), and the call says whether a window is wanted: a notification's
    // Reply is not, and the Full flavour's login service calls background() (Sailjail only runs
    // the .desktop's exact Exec line, so it can't pass an argument). So the window is shown by
    // those calls, not here; without any call (started from a shell) it shows after a moment.
    QTimer showUnlessCalled;
    showUnlessCalled.setSingleShot(true);
    QObject::connect(&showUnlessCalled, &QTimer::timeout, view.data(), [&view]() { view->show(); });
    QObject::connect(dbus, &DBusInterface::called, &showUnlessCalled, &QTimer::stop);
    AppLifecycle *lifecycle = new AppLifecycle(view.data(), [backend]() { return backend->keepRunning(); },
                                               InstallCheck(SailfishApp::pathToMainQml().toLocalFile().toLocal8Bit()),
                                               view->engine());
    view->rootContext()->setContextProperty(QStringLiteral("lifecycle"), lifecycle);
    dbus->setWindowGuard([lifecycle]() { return lifecycle->refuseWindow(); });
    QObject::connect(dbus, &DBusInterface::backgroundRequested, lifecycle, &AppLifecycle::leaveIfUnwanted);

    view->setSource(SailfishApp::pathToMainQml());
    showUnlessCalled.start(2000);
    // Own the D-Bus name only now: when the launcher started us via D-Bus activation, its
    // activate()/openChat() call is queued by the bus until then and must reach the loaded UI.
    if (!dbus->registerOnBus()) {
        // Another instance got the name while this one was loading. The core starts in the event
        // loop, so nothing of the store has been touched here.
        if (instanceRunning()) {
            activateRunningInstance();
        }
        return 1;
    }
    const int result = app->exec();

    // The event loop has ended, so the window doesn't draw any more. The pages go first, as they
    // hand the core what they still hold (bridge/endofprocess.h). The name stays until the
    // process is gone: nothing starts beside a core that is still stopping, and a start the bus
    // began while this one holds the launcher's lock would stay pending for minutes. A call in this
    // time gets no answer.
    if (!endOfProcess(*view, *backend)) {
        // The core or a worker task is stuck and still reports to what the destructors would
        // free. The store takes this like any crash.
        qWarning("threesea: ending without waiting for the core");
        ::_exit(result);
    }
    return result;
}
