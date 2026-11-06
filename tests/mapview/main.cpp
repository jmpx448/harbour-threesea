// SPDX-License-Identifier: AGPL-3.0-only
// Shows MapViewTest.qml (Silica + the bundled MapboxMap) with the fixture style; argv[1] is the
// directory with the QML and the style, argv[2] the bundled QML import directory. Exit code 1 on
// any QML warning. scripts/test-mapview.sh runs it under tests/mapview/shotcompositor.c.
#include <QFile>
#include <QGuiApplication>
#include <QQmlContext>
#include <QQmlEngine>
#include <QQuickView>
#include <QTimer>
#include <cstdio>

static int problems = 0;

static void onMessage(QtMsgType type, const QMessageLogContext &context, const QString &message)
{
    fprintf(stderr, "%s: %s\n", context.category ? context.category : "", qPrintable(message));
    if ((type == QtWarningMsg || type == QtCriticalMsg) && context.category
        && qstrcmp(context.category, "qml") == 0) {
        ++problems;
    }
}

int main(int argc, char **argv)
{
    qInstallMessageHandler(onMessage);
    QGuiApplication app(argc, argv);
    if (argc < 4) {
        fprintf(stderr, "usage: %s qmldir importdir seconds\n", argv[0]);
        return 2;
    }
    const QString dir = QString::fromLocal8Bit(argv[1]);
    QFile style(dir + "/fixture-style.json");
    if (!style.open(QIODevice::ReadOnly)) {
        return 2;
    }
    // Silica's ApplicationWindow is an item: shown in a view, like sailfishapp does.
    QQuickView view;
    view.engine()->addImportPath(QString::fromLocal8Bit(argv[2]));
    view.rootContext()->setContextProperty("fixtureStyle", QString::fromUtf8(style.readAll()));
    view.setSource(QUrl::fromLocalFile(dir + "/MapViewTest.qml"));
    if (view.status() != QQuickView::Ready) {
        for (const QQmlError &e : view.errors()) {
            fprintf(stderr, "%s\n", qPrintable(e.toString()));
        }
        return 1;
    }
    view.resize(540, 960);
    view.show();
    QTimer::singleShot(atoi(argv[3]) * 1000, &app, [] { QGuiApplication::exit(problems ? 1 : 0); });
    return app.exec();
}
