// SPDX-License-Identifier: AGPL-3.0-only
// Runs one scenario of Scenarios.qml (argv[2]) over the app's QML in the directory argv[1];
// no window system (QT_QPA_PLATFORM=minimal). Exit code 0 when all its checks held.
#include <QDir>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQmlContext>
#include <QTimer>
#include <qpa/qwindowsysteminterface.h>

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
    Q_INVOKABLE void finish(int code) { QCoreApplication::exit(code); }
};

int main(int argc, char **argv)
{
    QGuiApplication app(argc, argv);
    const QString dir = QString::fromLocal8Bit(argv[1]);
    QQmlApplicationEngine engine;
    engine.addImportPath(dir + QStringLiteral("/imports"));

    QQmlComponent fakesComponent(&engine, QUrl::fromLocalFile(dir + QStringLiteral("/Fakes.qml")));
    QObject *fakes = fakesComponent.create();
    if (!fakes) {
        qCritical("%s", qPrintable(fakesComponent.errorString()));
        return 2;
    }
    for (const char *name : { "backend", "dbus", "calls", "groupCalls", "voicePlayer", "videoPlayer", "softSound" }) {
        engine.rootContext()->setContextProperty(QString::fromLatin1(name), fakes->property(name));
    }
    Harness harness;
    engine.rootContext()->setContextProperty(QStringLiteral("harness"), &harness);

    engine.load(QUrl::fromLocalFile(dir + QStringLiteral("/harbour-threesea.qml")));
    if (engine.rootObjects().isEmpty()) {
        return 2;
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
    QTimer::singleShot(60000, &app, [] { qCritical("scenario timed out"); QCoreApplication::exit(3); });
    return app.exec();
}

#include "main.moc"
