// SPDX-License-Identifier: AGPL-3.0-only
// Opens one page of the app (argv[3], as qml/<path>) in the real window on the real Silica, against
// the fake C++ side (make-fakes.py) and the fake core (Results.js), with no window system
// (QT_QPA_PLATFORM=minimal). Exit code 0 when Qt and the QML engine had nothing to say: every
// warning counts (a ReferenceError, a TypeError, a binding loop, a failed import, a missing file).
#include <QCoreApplication>
#include <QGuiApplication>
#include <QImage>
#include <QLoggingCategory>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQmlContext>
#include <QQuickImageProvider>
#include <QQuickWindow>
#include <QRegularExpression>
#include <QTimer>
#include <QTranslator>
#include <QUrl>
#include <cstdio>
#include <unistd.h>
#include <qqml.h>
#include <qpa/qwindowsysteminterface.h>
#include <private/qquickwindow_p.h>

#include "contactlistmodel.h"
#include "emojihighlighter.h"
#include "enterkeyfilter.h"
#include "jsonlistmodel.h"

namespace {

int problems = 0;

// What only this environment says, none of it about the app: no window system, session bus or
// camera here. Keep this short: each entry is something a page could not cause.
const char *const environment[] = {
    // No D-Bus session: Nemo.Notifications, Nemo.DBus, Nemo.KeepAlive, Amber.Mpris.
    "D-Bus",
    "bus connection",
    // No media service without a device or a GStreamer plugin behind it.
    "defaultServiceProvider::requestService",
    "Focus mode selection is not supported",
    // The share plugin's file comes with the package.
    "method will not work",
};

void onMessage(QtMsgType type, const QMessageLogContext &context, const QString &message)
{
    if (type != QtWarningMsg && type != QtCriticalMsg && type != QtFatalMsg) {
        return;
    }
    // No device configuration: no pixel ratio.
    if (qstrcmp(context.category, "org.sailfishos.silica.core") == 0) {
        return;
    }
    for (const char *known : environment) {
        if (message.contains(QLatin1String(known))) {
            return;
        }
    }
    ++problems;
    fprintf(stderr, "%s: %s\n", context.category ? context.category : "", qPrintable(message));
}

// A picture for every image://threesea and image://emoji.
class AnyImage : public QQuickImageProvider
{
public:
    AnyImage() : QQuickImageProvider(QQuickImageProvider::Image) { }
    QImage requestImage(const QString &, QSize *size, const QSize &) override
    {
        QImage image(4, 4, QImage::Format_ARGB32);
        image.fill(Qt::gray);
        if (size) {
            *size = image.size();
        }
        return image;
    }
};

class Harness : public QObject
{
    Q_OBJECT
public:
    using QObject::QObject;
    Q_INVOKABLE void setActive(bool active)
    {
        QWindowSystemInterface::handleApplicationStateChanged(active ? Qt::ApplicationActive : Qt::ApplicationInactive);
        QWindowSystemInterface::flushWindowSystemEvents();
    }
    Q_INVOKABLE void finish() { QCoreApplication::exit(problems > 0 ? 1 : 0); }
    // Without Qt's own way out, which doesn't survive the window of the laid-out round.
    Q_INVOKABLE void finishNow()
    {
        fflush(stderr);
        _exit(problems > 0 ? 1 : 0);
    }
};

}

int main(int argc, char **argv)
{
    qInstallMessageHandler(onMessage);
    QGuiApplication app(argc, argv);
    const QString dir = QString::fromLocal8Bit(argv[1]);
    const QString flavour = QString::fromLocal8Bit(argv[2]);
    const bool layout = argc > 4 && qstrcmp(argv[4], "layout") == 0;

    // The texts as they would be without a translation, for %1 and %n to be there.
    QTranslator texts;
    if (texts.load(dir + QStringLiteral("/texts.qm"))) {
        app.installTranslator(&texts);
    }

    qmlRegisterType<JsonListModel>("harbour.threesea", 1, 0, "JsonListModel");
    qmlRegisterType<EnterKeyFilter>("harbour.threesea", 1, 0, "EnterKeyFilter");
    qmlRegisterType<EmojiHighlighter>("harbour.threesea", 1, 0, "EmojiHighlighter");
    qmlRegisterType(QUrl::fromLocalFile(dir + QStringLiteral("/fakes/AnimatedMedia.qml")), "harbour.threesea", 1, 0, "AnimatedMedia");
    qmlRegisterType(QUrl::fromLocalFile(dir + QStringLiteral("/fakes/CallVideo.qml")), "harbour.threesea", 1, 0, "CallVideo");

    QQmlApplicationEngine engine;
    engine.addImageProvider(QStringLiteral("threesea"), new AnyImage);
    engine.addImageProvider(QStringLiteral("emoji"), new AnyImage);

    ContactListModel contacts;
    contacts.update({ QVariantMap{ { QStringLiteral("identity"), QStringLiteral("ABCD1234") },
                                   { QStringLiteral("displayName"), QStringLiteral("Ada") },
                                   { QStringLiteral("verification"), 1 },
                                   { QStringLiteral("pictureVersion"), 0 },
                                   { QStringLiteral("section"), QStringLiteral("A") } } });
    engine.rootContext()->setContextProperty(QStringLiteral("contactsModel"), &contacts);

    QQmlComponent fakesComponent(&engine, QUrl::fromLocalFile(dir + QStringLiteral("/fakes/Fakes.qml")));
    QObject *fakes = fakesComponent.create();
    if (!fakes) {
        fprintf(stderr, "%s\n", qPrintable(fakesComponent.errorString()));
        return 2;
    }
    for (const char *name : { "backend", "dbus", "lifecycle", "calls", "groupCalls", "voicePlayer", "videoPlayer", "voiceRecorder", "proximity", "softSound" }) {
        engine.rootContext()->setContextProperty(QString::fromLatin1(name), fakes->property(name));
    }
    Harness harness;
    engine.rootContext()->setContextProperty(QStringLiteral("harness"), &harness);

    engine.load(QUrl::fromLocalFile(dir + QStringLiteral("/qml/harbour-threesea.qml")));
    if (engine.rootObjects().isEmpty()) {
        return 2;
    }
    QQmlComponent driverComponent(&engine, QUrl::fromLocalFile(dir + QStringLiteral("/fakes/Driver.qml")));
    QQmlContext context(engine.rootContext());
    context.setContextProperty(QStringLiteral("window"), engine.rootObjects().first());
    context.setContextProperty(QStringLiteral("fakes"), fakes);
    context.setContextProperty(QStringLiteral("flavour"), flavour);
    context.setContextProperty(QStringLiteral("pagePath"), QString::fromLocal8Bit(argv[3]));
    context.setContextProperty(QStringLiteral("layoutCheck"), layout);
    QObject *driver = driverComponent.create(&context);
    if (!driver) {
        fprintf(stderr, "%s\n", qPrintable(driverComponent.errorString()));
        return 2;
    }
    // Silica's window is an item here, and only a window polishes: Columns and lists never lay out
    // without it. This one stays hidden (showing one takes OpenGL); its polishing is all that's
    // needed. Only for the check of where lists come to rest (argv[4]), which ends the run there
    // with finishNow: Qt's normal way out crashes with this window in place.
    QQuickWindow layoutWindow;
    QTimer polish;
    if (layout) {
        auto *windowItem = qobject_cast<QQuickItem *>(engine.rootObjects().first());
        if (windowItem && !windowItem->window()) {
            windowItem->setParentItem(layoutWindow.contentItem());
        }
        QObject::connect(&polish, &QTimer::timeout, &layoutWindow, [&layoutWindow] {
            QQuickWindowPrivate::get(&layoutWindow)->polishItems();
        });
        polish.start(16);
    }
    QTimer::singleShot(30000, &app, [] {
        fprintf(stderr, "timed out\n");
        QCoreApplication::exit(3);
    });
    return app.exec();
}

#include "main.moc"
