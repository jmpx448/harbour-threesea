// SPDX-License-Identifier: AGPL-3.0-only
// Runs one scenario of Scenarios.qml (argv[2]) over the app's QML in the directory argv[1];
// no window system (QT_QPA_PLATFORM=minimal). Exit code 0 when all its checks held.
#include <QDir>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQmlContext>
#include <QImage>
#include <QMouseEvent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTimer>
#include <cstdio>
#include <private/qquickwindow_p.h>
#include <qpa/qwindowsysteminterface.h>
#include <unistd.h>

class Harness : public QObject
{
    Q_OBJECT
public:
    using QObject::QObject;
    // What the window system says when the app is left or comes back.
    Q_INVOKABLE void setActive(bool active)
    {
        QWindowSystemInterface::handleApplicationStateChanged(active ? Qt::ApplicationActive : Qt::ApplicationInactive);
        QWindowSystemInterface::flushWindowSystemEvents();
    }
    // A finger on `item`, at x, y in its own coordinates, however it is turned.
    Q_INVOKABLE void press(QObject *item, qreal x, qreal y) { mouse(item, QEvent::MouseButtonPress, x, y); }
    Q_INVOKABLE void move(QObject *item, qreal x, qreal y) { mouse(item, QEvent::MouseMove, x, y); }
    Q_INVOKABLE void release(QObject *item, qreal x, qreal y) { mouse(item, QEvent::MouseButtonRelease, x, y); }
    Q_INVOKABLE void finish(int code)
    {
        if (m_leaveAtOnce) {
            fflush(nullptr);
            _exit(code);
        }
        QCoreApplication::exit(code);
    }
    void setLeaveAtOnce(bool leave) { m_leaveAtOnce = leave; }

private:
    bool m_leaveAtOnce = false;

    void mouse(QObject *object, QEvent::Type type, qreal x, qreal y)
    {
        auto *item = qobject_cast<QQuickItem *>(object);
        const QPointF point = item->mapToScene(QPointF(x, y));
        QMouseEvent event(type, point, point, point, type == QEvent::MouseMove ? Qt::NoButton : Qt::LeftButton,
                          type == QEvent::MouseButtonRelease ? Qt::NoButton : Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(item->window(), &event);
    }
};

int main(int argc, char **argv)
{
    QGuiApplication app(argc, argv);
    // Wider than high, for the photo editor's scenarios.
    QImage wide(300, 100, QImage::Format_RGB32);
    wide.fill(Qt::darkGreen);
    wide.save(QStringLiteral("/tmp/threesea-wide.png"));
    const QString dir = QString::fromLocal8Bit(argv[1]);
    QQmlApplicationEngine engine;
    engine.addImportPath(dir + QStringLiteral("/imports"));

    QQmlComponent fakesComponent(&engine, QUrl::fromLocalFile(dir + QStringLiteral("/Fakes.qml")));
    QObject *fakes = fakesComponent.create();
    if (!fakes) {
        qCritical("%s", qPrintable(fakesComponent.errorString()));
        return 2;
    }
    for (const char *name : { "backend", "dbus", "lifecycle", "calls", "groupCalls", "voicePlayer", "videoPlayer", "voiceRecorder", "softSound" }) {
        engine.rootContext()->setContextProperty(QString::fromLatin1(name), fakes->property(name));
    }
    Harness harness;
    engine.rootContext()->setContextProperty(QStringLiteral("harness"), &harness);

    engine.load(QUrl::fromLocalFile(dir + QStringLiteral("/harbour-threesea.qml")));
    if (engine.rootObjects().isEmpty()) {
        return 2;
    }
    // As main does once the D-Bus name is ours; this scenario does it itself.
    if (QByteArray(argv[2]) != "notificationsWaitForTheName") {
        QMetaObject::invokeMethod(engine.rootObjects().first(), "nameOwned");
    }
    QQmlComponent scenariosComponent(&engine, QUrl::fromLocalFile(dir + QStringLiteral("/Scenarios.qml")));
    QVariantMap initial;
    QQmlContext context(engine.rootContext());
    context.setContextProperty(QStringLiteral("window"), engine.rootObjects().first());
    context.setContextProperty(QStringLiteral("fakes"), fakes);
    context.setContextProperty(QStringLiteral("scenario"), QString::fromLocal8Bit(argv[2]));
    QObject *scenarios = scenariosComponent.create(&context);
    if (!scenarios) {
        qCritical("%s", qPrintable(scenariosComponent.errorString()));
        return 2;
    }
    // Silica's window is an item here, and only a window polishes and takes touches. Hidden; only for
    // the scenarios of pages that are touched. Qt's normal way out crashes with it in place.
    QQuickWindow touchWindow;
    QTimer polish;
    if (QByteArray(argv[2]).startsWith("touch")) {
        harness.setLeaveAtOnce(true);
        auto *windowItem = qobject_cast<QQuickItem *>(engine.rootObjects().first());
        if (windowItem && !windowItem->window()) {
            windowItem->setParentItem(touchWindow.contentItem());
        }
        windowItem->setSize(QSizeF(540, 960));
        QObject::connect(&polish, &QTimer::timeout, &touchWindow, [&touchWindow] {
            QQuickWindowPrivate::get(&touchWindow)->polishItems();
        });
        polish.start(16);
    }
    QTimer::singleShot(60000, &app, [] { qCritical("scenario timed out"); QCoreApplication::exit(3); });
    return app.exec();
}

#include "main.moc"
