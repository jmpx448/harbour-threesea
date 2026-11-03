// SPDX-License-Identifier: AGPL-3.0-only
#include "emojihighlighter.h"

#include <QQuickTextDocument>
#include <QGlyphRun>
#include <QTextCharFormat>
#include <QTextDocument>
#include <QTextLayout>
#include <QTimer>

// Qt 5.6's QSyntaxHighlighter(QObject *) dereferences the parent, and QML creates items without
// one: start without a document, set the parent after.
EmojiHighlighter::EmojiHighlighter(QObject *parent)
    : QSyntaxHighlighter(static_cast<QTextDocument *>(nullptr))
{
    setParent(parent);
}

void EmojiHighlighter::setTextDocument(QObject *document)
{
    QQuickTextDocument *textDocument = qobject_cast<QQuickTextDocument *>(document);
    if (textDocument == m_textDocument) {
        return;
    }
    m_textDocument = textDocument;
    // Once the editor is fully set up.
    QTimer::singleShot(0, this, &EmojiHighlighter::attach);
    emit textDocumentChanged();
}

void EmojiHighlighter::attach()
{
    setDocument(m_textDocument ? m_textDocument->textDocument() : nullptr);
}

void EmojiHighlighter::setRanges(const QVariantList &ranges)
{
    if (ranges == m_ranges) {
        return;
    }
    m_ranges = ranges;
    rehighlight();
    emit rangesChanged();
}

void EmojiHighlighter::setEmojiWidth(qreal width)
{
    if (qFuzzyCompare(width, m_emojiWidth)) {
        return;
    }
    m_emojiWidth = width;
    rehighlight();
    emit emojiWidthChanged();
}

// Letter spacing (added after each glyph) that makes `emoji` take `width` pixels in `font`. Qt
// turns ligatures off in text with letter spacing, which splits emoji the font draws as one
// ligature (a hand and its skin tone): measure the glyphs with some spacing on, too.
static qreal spacingFor(const QString &emoji, QFont font, qreal width)
{
    const qreal probe = 1;
    font.setLetterSpacing(QFont::AbsoluteSpacing, probe);
    QTextLayout layout(emoji, font);
    layout.beginLayout();
    QTextLine line = layout.createLine();
    layout.endLayout();
    int glyphs = 0;
    for (const QGlyphRun &run : layout.glyphRuns()) {
        glyphs += run.glyphIndexes().size();
    }
    return glyphs > 0 ? (width - line.naturalTextWidth()) / glyphs + probe : 0;
}

void EmojiHighlighter::highlightBlock(const QString &text)
{
    const int blockStart = currentBlock().position();
    const int blockEnd = blockStart + text.length();
    const QFont font = document()->defaultFont();
    for (const QVariant &value : m_ranges) {
        const QVariantMap range = value.toMap();
        const int start = range.value(QStringLiteral("start")).toInt();
        const int end = start + range.value(QStringLiteral("length")).toInt();
        if (end <= blockStart || start >= blockEnd) {
            continue;
        }
        const int from = qMax(start, blockStart) - blockStart;
        const int length = qMin(end, blockEnd) - blockStart - from;
        QTextCharFormat hidden;
        hidden.setForeground(Qt::transparent);
        if (m_emojiWidth > 0) {
            hidden.setFontLetterSpacingType(QFont::AbsoluteSpacing);
            hidden.setFontLetterSpacing(spacingFor(text.mid(from, length), font, m_emojiWidth));
        }
        setFormat(from, length, hidden);
    }
}
