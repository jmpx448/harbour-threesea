// SPDX-License-Identifier: AGPL-3.0-only
#ifndef PEERIMAGE_H
#define PEERIMAGE_H

#include <QBuffer>
#include <QByteArray>
#include <QImage>
#include <QImageIOHandler>
#include <QImageReader>
#include <QSize>
#include <QString>

// Pictures from other people are shown without anyone tapping them, so the bytes alone must not
// choose among every image plugin installed on the phone. The format comes from the first bytes,
// whatever the sender's label says, and only these four get a reader. `pinTo` keeps Qt from
// asking any other plugin, the way it would by itself when a format's own decoder says no.
namespace PeerImage {

// "jpeg", "png", "gif" or "webp"; empty for anything else.
inline QByteArray format(const QByteArray &bytes)
{
    if (bytes.startsWith(QByteArrayLiteral("\xff\xd8\xff"))) {
        return QByteArrayLiteral("jpeg");
    }
    if (bytes.startsWith(QByteArrayLiteral("\x89PNG\r\n\x1a\n"))) {
        return QByteArrayLiteral("png");
    }
    if (bytes.startsWith(QByteArrayLiteral("GIF87a")) || bytes.startsWith(QByteArrayLiteral("GIF89a"))) {
        return QByteArrayLiteral("gif");
    }
    if (bytes.startsWith(QByteArrayLiteral("RIFF")) && bytes.mid(8, 4) == QByteArrayLiteral("WEBP")) {
        return QByteArrayLiteral("webp");
    }
    return QByteArray();
}

inline void pinTo(QImageReader &reader, const QByteArray &imageFormat)
{
    reader.setFormat(imageFormat);
    reader.setAutoDetectImageFormat(false);
}

// Largest picture decoded at all, and in formats that can't decode scaled down (pixels).
const qint64 MaxPixels = 100 * 1000 * 1000;
const qint64 MaxPixelsUnscaled = 30 * 1000 * 1000;

// The size to decode `full` at for a QML `sourceSize`: a 0 (or missing) dimension means
// "unconstrained" (QML sets only sourceSize.width), never upscale.
inline QSize targetSize(const QSize &full, const QSize &requested)
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

// The picture in `data`, upright as its EXIF says, fitted into `requested` (see targetSize). Null
// if it can't or mustn't be decoded: then `problem` says why, for the log.
inline QImage decode(QByteArray &data, const QSize &requested, QString *problem)
{
    const QByteArray imageFormat = format(data);
    if (imageFormat.isEmpty()) {
        *problem = QStringLiteral("not in an accepted format");
        return QImage();
    }
    QBuffer buffer(&data);
    buffer.open(QIODevice::ReadOnly);
    QImageReader reader(&buffer);
    pinTo(reader, imageFormat);
    reader.setAutoTransform(true);
    // As stored: a picture taken upright usually lies on its side, with EXIF saying so. The reader
    // turns it only while reading, and its scaled size counts as stored too.
    const QSize stored = reader.size();
    // Not decoded at all beyond what a phone can hold (a picture's header can claim any size;
    // security audit 2026-10-27, S06): JPEG decodes scaled down, other formats in full.
    const qint64 pixels = static_cast<qint64>(stored.width()) * stored.height();
    if (!stored.isValid() || pixels > MaxPixels
        || (pixels > MaxPixelsUnscaled && imageFormat != QByteArrayLiteral("jpeg"))) {
        *problem = QStringLiteral("too large");
        return QImage();
    }
    const bool turned = reader.transformation() & QImageIOHandler::TransformationRotate90;
    const QSize shown = turned ? stored.transposed() : stored;
    const QSize target = targetSize(shown, requested);
    // Decoders that can (JPEG) scale while decoding: camera photos are large.
    if (target != shown) {
        reader.setScaledSize(turned ? target.transposed() : target);
    }
    QImage image = reader.read();
    if (image.isNull()) {
        *problem = QStringLiteral("not decodable: ") + reader.errorString();
    } else if (target.isValid() && image.size() != target) {
        image = image.scaled(target, Qt::KeepAspectRatio, Qt::SmoothTransformation);
    }
    return image;
}

}

#endif
