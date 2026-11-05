// SPDX-License-Identifier: AGPL-3.0-only
#ifndef EMOJIIMAGEPROVIDER_H
#define EMOJIIMAGEPROVIDER_H

#include <QCache>
#include <QImage>
#include <QMutex>
#include <QQuickImageProvider>
#include <QTimer>

// image://emoji/<sheet>,<x>,<y>: one 64x64 Twemoji cell from emojis/<sheet>.png (the sprite sheets
// Threema Libre ships; see scripts/make-emoji-table.py). The cut-out emoji stay cached. Decoded
// sheets are large (about 1 MB, paletted) and go again shortly after the last emoji cut from one:
// the picker goes from sheet to sheet with nearly every cell, up to eleven of them in one category,
// and decoding the PNG anew each time was what made its tiles appear one by one.
class EmojiImageProvider : public QQuickImageProvider
{
public:
    EmojiImageProvider();
    QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize) override;

private:
    void dropSheets();

    QMutex m_lock;
    QCache<QString, QImage> m_sheets;
    QCache<QString, QImage> m_emoji;
    // Lives on the GUI thread, where the provider is made; goes first when the provider does.
    QTimer m_sheetsIdle;
};

#endif // EMOJIIMAGEPROVIDER_H
