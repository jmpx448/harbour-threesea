// SPDX-License-Identifier: AGPL-3.0-only
// Shows MapViewTest.qml: the app's ThreemaMap (qml/components, the bundled MapboxMap) on a Silica
// page. argv[1]: the directory with the QML and the fixture style; argv[2]: the bundled QML import
// directory; argv[3]: seconds to run; argv[4]: "fixture" for the offline style, else the style's
// URL; argv[5], if given: the cache database (else in memory, as for a private chat). Exit code 1 on
// any QML warning. scripts/test-mapview.sh runs it under tests/mapview/shotcompositor.c, with
// MapLibre's THREESEA_MAP_* variables set or not per case.
#include <QFile>
#include <QGuiApplication>
#include <QQmlContext>
#include <QQmlEngine>
#include <QQuickView>
#include <QTimer>
#include <QVariantMap>
#include <cstdio>

#include "maplogfilter.h"

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
    // The bridge's second guard: a record in MapLibre's own form loses its text.
    const QString unpatched = withoutMapDetails(
        QStringLiteral("[ ERROR ]  \"{Worker}[Style]: Failed to load tile 14/8600/5750=>14 for source tiles\""));
    if (unpatched != QStringLiteral("MapLibre: ERROR (text withheld)")) {
        fprintf(stderr, "maplogfilter let through: %s\n", qPrintable(unpatched));
        return 1;
    }
    qInstallMessageHandler(onMessage);
    QGuiApplication app(argc, argv);
    if (argc < 5) {
        fprintf(stderr, "usage: %s qmldir importdir seconds fixture|style-url [cache]\n", argv[0]);
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
    const QString styleArg = QString::fromLocal8Bit(argv[4]);
    const bool fixture = styleArg == QLatin1String("fixture");
    view.rootContext()->setContextProperty("testStyleJson", fixture ? QString::fromUtf8(style.readAll()) : QString());
    view.rootContext()->setContextProperty("testStyleUrl", fixture ? QString() : styleArg);
    view.rootContext()->setContextProperty("testPrivate", argc < 6);
    // The one property of the bridge the map reads.
    QVariantMap backend;
    backend.insert("mapCache", argc >= 6 ? QString::fromLocal8Bit(argv[5]) : QString());
    view.rootContext()->setContextProperty("backend", backend);
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
