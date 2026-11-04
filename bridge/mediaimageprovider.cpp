// SPDX-License-Identifier: AGPL-3.0-only
#include "mediaimageprovider.h"

#include <QBuffer>
#include <QDebug>
#include <QImageReader>

#include "peerimage.h"
#include "threesebackend.h"

namespace {

// Largest picture decoded at all, and in formats that can't decode scaled down (megapixels).
const qint64 MAX_PIXELS = 100 * 1000 * 1000;
const qint64 MAX_PIXELS_UNSCALED = 30 * 1000 * 1000;

// The size to decode `full` at for a QML `sourceSize`: a 0 (or missing) dimension means
// "unconstrained" (QML sets only sourceSize.width), never upscale.
QSize targetSize(const QSize &full, const QSize &requested)
{
    if (!full.isValid() || full.isEmpty()) {
        return full;
    }
    const int width = requested.width() > 0 ? requested.width() : full.width();
    const int height = requested.height() > 0 ? requested.height() : full.height();
    if (full.width() <= width && full.height() <= height) {
        return full;
    }
    return full.scaled(QSize(width, height), Qt::KeepAspectRatio);
}

}

MediaImageProvider::MediaImageProvider(const ThreeseBackend *backend)
    // Without ForceAsynchronousImageLoading Qt calls a provider on the GUI thread, even for an
    // Image with `asynchronous: true` — so every picture in a chat was decrypted and decoded
    // while the UI waited. The core's media reads are thread-safe (core/src/ffi.rs) and
    // nothing else here is shared, so the image loader thread may do it.
    : QQuickImageProvider(QQuickImageProvider::Image, QQuickImageProvider::ForceAsynchronousImageLoading)
    , m_backend(backend)
{
}

QImage MediaImageProvider::requestImage(const QString &id, QSize *size, const QSize &requestedSize)
{
    // "<message>/<thumb|file>", optionally with "?<anything>" to force a reload.
    // Or "group/<group key>" for a group's picture, "gif/<id>" for a favourite GIF.
    const QString path = id.section(QLatin1Char('?'), 0, 0);
    QByteArray data;
    qint64 message = -1;
    if (path.startsWith(QLatin1String("group/"))) {
        data = m_backend->readPicture(path.mid(6));
    } else if (path.startsWith(QLatin1String("gif/"))) {
        // A favourite GIF's thumbnail ("gif/<id>"). Not every GIF came with one (a sender's
        // message may have none): then its first frame.
        data = m_backend->readPicture(QStringLiteral("gif-thumb:") + path.mid(4));
        if (data.isEmpty()) {
            data = m_backend->readPicture(QStringLiteral("gif:") + path.mid(4));
        }
    } else if (path.startsWith(QLatin1String("contact/"))) {
        // A contact's picture ("contact/<ID>"), or ours ("contact/me").
        const QString who = path.mid(8);
        data = m_backend->readPicture(who == QLatin1String("me") ? who : QStringLiteral("contact:") + who);
    } else {
        const QStringList parts = path.split(QLatin1Char('/'));
        bool ok = false;
        message = parts.value(0).toLongLong(&ok);
        if (!ok || parts.size() != 2) {
            return QImage();
        }
        data = m_backend->readMedia(message, parts.at(1) == QLatin1String("thumb"));
    }
    if (data.isEmpty()) {
        return QImage();
    }
    const QByteArray format = PeerImage::format(data);
    if (format.isEmpty()) {
        qWarning() << "media image" << message << "not in an accepted format";
        data.fill('\0');
        return QImage();
    }
    QBuffer buffer(&data);
    buffer.open(QIODevice::ReadOnly);
    QImageReader reader(&buffer);
    PeerImage::pinTo(reader, format);
    reader.setAutoTransform(true);
    const QSize full = reader.size();
    // Not decoded at all beyond what a phone can hold (a picture's header can claim any size;
    // security audit 2026-10-27, S06): JPEG decodes scaled down, other formats in full.
    const qint64 pixels = static_cast<qint64>(full.width()) * full.height();
    if (!full.isValid() || pixels > MAX_PIXELS
        || (pixels > MAX_PIXELS_UNSCALED && format != QByteArrayLiteral("jpeg"))) {
        qWarning() << "media image" << message << "too large:" << full;
        data.fill('\0');
        return QImage();
    }
    const QSize target = targetSize(full, requestedSize);
    // Decoders that can (JPEG) scale while decoding: camera photos are large.
    if (target != full) {
        reader.setScaledSize(target);
    }
    QImage image = reader.read();
    if (!image.isNull() && target.isValid() && image.size() != target) {
        image = image.scaled(target, Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
    }
    if (image.isNull()) {
        qWarning() << "media image" << message << "not decodable:" << reader.errorString();
    }
    data.fill('\0');
    if (size) {
        *size = image.size();
    }
    return image;
}
