// SPDX-License-Identifier: AGPL-3.0-only
#ifndef EMOJIIMAGEPROVIDER_H
#define EMOJIIMAGEPROVIDER_H

#include <QCache>
#include <QImage>
#include <QMutex>
#include <QQuickImageProvider>

// image://emoji/<sheet>,<x>,<y>: one 64x64 Twemoji cell from emojis/<sheet>.png (the sprite sheets
// Threema Libre ships; see scripts/make-emoji-table.py). Decoded sheets are large (about 4 MB), so
// only the cut-out emoji are cached, and the last sheet used.
class EmojiImageProvider : public QQuickImageProvider
{
public:
    EmojiImageProvider();
    QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize) override;

private:
    QMutex m_lock;
    QString m_sheetName;
    QImage m_sheet;
    QCache<QString, QImage> m_emoji;
};

#endif // EMOJIIMAGEPROVIDER_H
