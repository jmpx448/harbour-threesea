// SPDX-License-Identifier: AGPL-3.0-only
#ifndef EMOJIHIGHLIGHTER_H
#define EMOJIHIGHLIGHTER_H

#include <QPointer>
#include <QQuickTextDocument>
#include <QSyntaxHighlighter>
#include <QVariantList>

// Threema's emoji pictures in a plain text editor (Settings → Appearance → Emoji style): the
// editor keeps its plain text (cursor, copy and paste, mentions unchanged); this highlighter makes
// the system's emoji glyphs in `ranges` transparent, and QML draws the pictures over them.
class EmojiHighlighter : public QSyntaxHighlighter
{
    Q_OBJECT
    // The editor's `textDocument` (a TextArea's `_editor.textDocument`).
    Q_PROPERTY(QObject *textDocument READ textDocument WRITE setTextDocument NOTIFY textDocumentChanged)
    // `[{start, length}]` in UTF-16 units of the editor's text (from Format.js emojiRanges).
    Q_PROPERTY(QVariantList ranges READ ranges WRITE setRanges NOTIFY rangesChanged)
    // The width every emoji takes in the text, in pixels (the picture and some room on both
    // sides); 0 keeps the glyphs' own widths. The system draws some emoji as several glyphs
    // (a hand and its skin tone), so the letter spacing is worked out per emoji.
    Q_PROPERTY(qreal emojiWidth READ emojiWidth WRITE setEmojiWidth NOTIFY emojiWidthChanged)

public:
    explicit EmojiHighlighter(QObject *parent = nullptr);

    QObject *textDocument() const { return m_textDocument; }
    void setTextDocument(QObject *document);
    QVariantList ranges() const { return m_ranges; }
    void setRanges(const QVariantList &ranges);
    qreal emojiWidth() const { return m_emojiWidth; }
    void setEmojiWidth(qreal width);

signals:
    void textDocumentChanged();
    void rangesChanged();
    void emojiWidthChanged();

protected:
    void highlightBlock(const QString &text) override;

private:
    QPointer<QQuickTextDocument> m_textDocument;
    void attach();
    QVariantList m_ranges;
    qreal m_emojiWidth = 0;
};

#endif // EMOJIHIGHLIGHTER_H
