// SPDX-License-Identifier: AGPL-3.0-only
// Runs Cases.qml's checks of the text helpers (Format.js and others) on the real QML engine, and
// looks at what a real Text makes of the tags those helpers write. Exit 0: passed.
#include <QCoreApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QGuiApplication>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QColor>
#include <QTextCursor>
#include <QTextDocument>
#include <QTextLayout>
#include <QtQuick/private/qquicktext_p_p.h>
#include <cstdio>

namespace {

int failures = 0;

// The formats a real Text in StyledText gives `html`, from its private layout (nothing can be drawn
// here to look at).
QVector<QTextLayout::FormatRange> formatsOf(const QString &html, QQmlEngine &engine)
{
    QQmlComponent component(&engine);
    component.setData("import QtQuick 2.6\nText { textFormat: Text.StyledText }", QUrl());
    QScopedPointer<QObject> text(component.create());
    if (!text) {
        std::printf("FAILED: no Text: %s\n", qPrintable(component.errorString()));
        ++failures;
        return {};
    }
    text->setProperty("text", html);
    auto *d = static_cast<QQuickTextPrivate *>(QObjectPrivate::get(text.data()));
    return d->layout.formats();
}

// The character format before `position` in rich text, which is what a Text's document holds.
QTextCharFormat formatAt(const QString &html, int position)
{
    QTextDocument document;
    document.setHtml(html);
    QTextCursor cursor(&document);
    cursor.setPosition(position);
    return cursor.charFormat();
}

void checkTags(QQmlEngine &engine)
{
    const auto bold = formatsOf(QStringLiteral("a<b>b</b>c"), engine);
    const auto italic = formatsOf(QStringLiteral("a<i>b</i>c"), engine);
    const auto one = [](const QVector<QTextLayout::FormatRange> &formats, int start, int length) {
        for (const QTextLayout::FormatRange &range : formats) {
            if (range.start == start && range.length == length) {
                return range.format;
            }
        }
        return QTextCharFormat();
    };
    const struct {
        bool ok;
        const char *what;
    } checks[] = {
        { one(bold, 1, 1).fontWeight() == QFont::Bold, "<b> is bold" },
        { one(italic, 1, 1).fontItalic(), "<i> is italic" },
        // Qt 5.6's StyledText knows no tag for this, hence markup with it goes to RichText.
        { [&]() {
              for (const QTextLayout::FormatRange &range : formatsOf(QStringLiteral("a<s>b</s>c"), engine)) {
                  if (range.format.fontStrikeOut()) {
                      return false;
                  }
              }
              return true;
          }(), "StyledText still has no strikethrough: Format.js could do without rich text" },
        { formatAt(QStringLiteral("a<s>b</s>c"), 2).fontStrikeOut(), "rich text: <s> is struck through" },
        { !formatAt(QStringLiteral("a<s>b</s>c"), 3).fontStrikeOut(), "rich text: <s> ends" },
        { formatAt(QStringLiteral("<b>b<s>c</s></b>"), 2).fontStrikeOut()
              && formatAt(QStringLiteral("<b>b<s>c</s></b>"), 2).fontWeight() == QFont::Bold, "rich text: <s> inside <b> is both" },
        { formatAt(QStringLiteral("<a style=\"color:#102030\" href=\"x\">b</a><s>c</s>"), 1).foreground().color() == QColor(0x10, 0x20, 0x30),
          "rich text: a link has the colour it is given" },
    };
    for (const auto &check : checks) {
        if (!check.ok) {
            std::printf("FAILED: %s\n", check.what);
            ++failures;
        }
    }
}

}

int main(int argc, char **argv)
{
    QGuiApplication app(argc, argv);
    QQmlEngine engine;
    checkTags(engine);

    QQmlComponent component(&engine, QUrl::fromLocalFile(QDir::current().absoluteFilePath(QStringLiteral("Cases.qml"))));
    QObject *cases = component.create();
    if (!cases) {
        std::printf("FAILED: %s\n", qPrintable(component.errorString()));
        return 1;
    }
    QElapsedTimer clock;
    clock.start();
    while (!cases->property("done").toBool() && clock.elapsed() < 20000) {
        QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
    }
    if (!cases->property("done").toBool()) {
        std::printf("FAILED: the cases didn't finish\n");
        ++failures;
    }
    const QStringList failed = cases->property("failed").toStringList();
    for (const QString &failure : failed) {
        std::printf("FAILED: %s\n", qPrintable(failure));
        ++failures;
    }
    std::printf("cases run: %d\n", cases->property("count").toInt());
    if (failures) {
        std::printf("text helpers: %d failed\n", failures);
        return 1;
    }
    std::printf("text helpers: passed\n");
    return 0;
}
