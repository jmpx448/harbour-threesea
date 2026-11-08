// SPDX-License-Identifier: AGPL-3.0-only
#include "emojiimageprovider.h"

#include <QMutexLocker>
#include <QStringList>

#include <sailfishapp.h>

namespace {
const int Cell = 64;
// In bytes: the largest category (people) has eleven sheets of about 1 MB each, and goes through
// all of them from its first screen on, so any less would decode each of them again and again.
const int SheetsCost = 12 * 1024 * 1024;
const int SheetsIdleMs = 5000;
}

EmojiImageProvider::EmojiImageProvider()
    // The sheet and the cache are behind the mutex, so the image loader thread can cut the
    // emoji instead of the GUI thread (which is what Qt does for a provider without this).
    : QQuickImageProvider(QQuickImageProvider::Image, QQuickImageProvider::ForceAsynchronousImageLoading)
    , m_sheets(SheetsCost)
    , m_emoji(1000)
{
    m_sheetsIdle.setSingleShot(true);
    m_sheetsIdle.setInterval(SheetsIdleMs);
    QObject::connect(&m_sheetsIdle, &QTimer::timeout, [this]() { dropSheets(); });
}

void EmojiImageProvider::dropSheets()
{
    // On the GUI thread, which must not wait for a PNG being decoded: then try again later.
    if (!m_lock.tryLock()) {
        m_sheetsIdle.start();
        return;
    }
    m_sheets.clear();
    m_lock.unlock();
}

QImage EmojiImageProvider::requestImage(const QString &id, QSize *size, const QSize &requestedSize)
{
    QImage emoji;
    {
        QMutexLocker locker(&m_lock);
        if (QImage *cached = m_emoji.object(id)) {
            emoji = *cached;
        } else {
            const QStringList parts = id.split(QLatin1Char(','));
            bool okX = false, okY = false;
            const int x = parts.value(1).toInt(&okX);
            const int y = parts.value(2).toInt(&okY);
            const QString sheet = parts.value(0);
            // Only our own sheet names: letters, a dash and a number.
            if (parts.size() != 3 || !okX || !okY || sheet.contains(QLatin1Char('/')) || sheet.contains(QLatin1Char('.'))) {
                return QImage();
            }
            QImage decoded;
            if (QImage *cached = m_sheets.object(sheet)) {
                decoded = *cached;
            } else {
                decoded = QImage(SailfishApp::pathTo(QStringLiteral("emojis/") + sheet + QStringLiteral(".png")).toLocalFile());
                if (decoded.isNull()) {
                    return QImage();
                }
                m_sheets.insert(sheet, new QImage(decoded), decoded.byteCount());
            }
            emoji = decoded.copy(x, y, Cell, Cell);
            m_emoji.insert(id, new QImage(emoji));
            // We are on the image loader's thread; the timer belongs to the GUI thread.
            QMetaObject::invokeMethod(&m_sheetsIdle, "start", Qt::QueuedConnection);
        }
    }
    if (size) {
        *size = emoji.size();
    }
    if (requestedSize.isValid() && requestedSize != emoji.size()) {
        const QSize target(requestedSize.width() > 0 ? requestedSize.width() : requestedSize.height(),
                           requestedSize.height() > 0 ? requestedSize.height() : requestedSize.width());
        return emoji.scaled(target, Qt::KeepAspectRatio, Qt::SmoothTransformation);
    }
    return emoji;
}
