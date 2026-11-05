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
    m_spans.clear();
    m_spans.reserve(ranges.size());
    for (const QVariant &value : ranges) {
        const QVariantMap range = value.toMap();
        const int start = range.value(QStringLiteral("start")).toInt();
        m_spans.append({ start, start + range.value(QStringLiteral("length")).toInt() });
    }
    rehighlight();
    emit rangesChanged();
}

void EmojiHighlighter::setEmojiWidth(qreal width)
{
    if (qFuzzyCompare(width, m_emojiWidth)) {
        return;
    }
    m_emojiWidth = width;
    m_spacings.clear();
    rehighlight();
    emit emojiWidthChanged();
}

// Letter spacing (added after each glyph) that makes `emoji` take the emoji width in `font`. Qt
// turns ligatures off in text with letter spacing, which splits emoji the font draws as one
// ligature (a hand and its skin tone): measure the glyphs with some spacing on, too.
qreal EmojiHighlighter::spacingFor(const QString &emoji, const QFont &font)
{
    if (font != m_spacingFont) {
        m_spacings.clear();
        m_spacingFont = font;
    }
    const auto known = m_spacings.constFind(emoji);
    if (known != m_spacings.constEnd()) {
        return known.value();
    }
    const qreal probe = 1;
    QFont measured = font;
    measured.setLetterSpacing(QFont::AbsoluteSpacing, probe);
    QTextLayout layout(emoji, measured);
    layout.beginLayout();
    QTextLine line = layout.createLine();
    layout.endLayout();
    int glyphs = 0;
    for (const QGlyphRun &run : layout.glyphRuns()) {
        glyphs += run.glyphIndexes().size();
    }
    const qreal spacing = glyphs > 0 ? (m_emojiWidth - line.naturalTextWidth()) / glyphs + probe : 0;
    m_spacings.insert(emoji, spacing);
    return spacing;
}

void EmojiHighlighter::highlightBlock(const QString &text)
{
    const int blockStart = currentBlock().position();
    const int blockEnd = blockStart + text.length();
    const QFont font = document()->defaultFont();
    for (const Range &range : m_spans) {
        if (range.end <= blockStart || range.start >= blockEnd) {
            continue;
        }
        const int from = qMax(range.start, blockStart) - blockStart;
        const int length = qMin(range.end, blockEnd) - blockStart - from;
        QTextCharFormat hidden;
        hidden.setForeground(Qt::transparent);
        if (m_emojiWidth > 0) {
            hidden.setFontLetterSpacingType(QFont::AbsoluteSpacing);
            hidden.setFontLetterSpacing(spacingFor(text.mid(from, length), font));
        }
        setFormat(from, length, hidden);
    }
}
