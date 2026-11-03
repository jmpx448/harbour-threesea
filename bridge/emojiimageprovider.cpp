// SPDX-License-Identifier: AGPL-3.0-only
#include "emojiimageprovider.h"

#include <QMutexLocker>
#include <QStringList>

#include <sailfishapp.h>

namespace {
const int Cell = 64;
}

EmojiImageProvider::EmojiImageProvider()
    // The sheet and the cache are behind the mutex, so the image loader thread can cut the
    // emoji instead of the GUI thread (which is what Qt does for a provider without this).
    : QQuickImageProvider(QQuickImageProvider::Image, QQuickImageProvider::ForceAsynchronousImageLoading)
    , m_emoji(1000)
{
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
            if (sheet != m_sheetName) {
                m_sheet = QImage(SailfishApp::pathTo(QStringLiteral("emojis/") + sheet + QStringLiteral(".png")).toLocalFile());
                m_sheetName = sheet;
            }
            if (m_sheet.isNull()) {
                return QImage();
            }
            emoji = m_sheet.copy(x, y, Cell, Cell);
            m_emoji.insert(id, new QImage(emoji));
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
