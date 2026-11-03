// SPDX-License-Identifier: AGPL-3.0-only
#include "filepreparer.h"

#include <QBuffer>
#include <QFileInfo>
#include <QImage>
#include <QImageReader>
#include <QMimeDatabase>
#include <QPainter>
#include <QSettings>
#ifdef THREESEA_FULL
#include "videotranscoder.h"
#endif

namespace {

const int ThumbnailSide = 512;

QImage limited(const QImage &image, int side)
{
    if (image.width() <= side && image.height() <= side) {
        return image;
    }
    return image.scaled(side, side, Qt::KeepAspectRatio, Qt::SmoothTransformation);
}

// The frame comes letterboxed (fitted into a square, transparent around it): cut that off.
QImage opaquePart(const QImage &image)
{
    const QImage argb = image.convertToFormat(QImage::Format_ARGB32);
    int left = argb.width(), right = -1, top = argb.height(), bottom = -1;
    for (int y = 0; y < argb.height(); ++y) {
        const QRgb *line = reinterpret_cast<const QRgb *>(argb.constScanLine(y));
        for (int x = 0; x < argb.width(); ++x) {
            if (qAlpha(line[x]) > 0) {
                left = qMin(left, x);
                right = qMax(right, x);
                top = qMin(top, y);
                bottom = qMax(bottom, y);
            }
        }
    }
    if (right < left || bottom < top) {
        return QImage();
    }
    return argb.copy(left, top, right - left + 1, bottom - top + 1);
}

QByteArray encode(const QImage &image, const char *format, int quality)
{
    QByteArray bytes;
    QBuffer buffer(&bytes);
    buffer.open(QIODevice::WriteOnly);
    image.save(&buffer, format, quality);
    return bytes;
}

// The thumbnail is always a JPEG: flatten transparency onto white.
QImage jpegThumbnail(const QImage &image)
{
    QImage thumbnail(limited(image, ThumbnailSide).size(), QImage::Format_RGB32);
    thumbnail.fill(Qt::white);
    QPainter painter(&thumbnail);
    painter.setRenderHint(QPainter::SmoothPixmapTransform);
    painter.drawImage(thumbnail.rect(), image);
    return thumbnail;
}

}

FilePreparer::FilePreparer(const QString &path, const QVariantMap &request, const QString &scratchPath)
    : m_path(path)
    , m_request(request)
    , m_scratchPath(scratchPath)
{
}

void FilePreparer::run()
{
    QVariantMap request = m_request;
    const QFileInfo info(m_path);
    if (!info.isFile()) {
        emit prepared(request, QByteArray(), QByteArray(), QStringLiteral("not a file"));
        return;
    }
    const QString mediaType = QMimeDatabase().mimeTypeForFile(info).name();
    request.insert(QStringLiteral("name"), info.fileName());
    request.insert(QStringLiteral("mediaType"), mediaType);

    // Videos as media (Threema shows them with their thumbnail and plays them), scaled down
    // where Settings → Media & storage → Video size asks for it and the phone can.
    const QImage videoFrame = m_videoFrame.isNull() ? QImage() : opaquePart(m_videoFrame);
    if (mediaType.startsWith(QLatin1String("video/")) && !videoFrame.isNull()
            && request.value(QStringLiteral("rendering")).toString() == QLatin1String("media")) {
        QString path = m_path;
        int width = videoFrame.width();
        int height = videoFrame.height();
#ifdef THREESEA_FULL
        const int videoSize = QSettings().value(QStringLiteral("videoSize"), 0).toInt();
        const int edge = VideoTranscoder::edgeForSetting(videoSize);
        if (edge > 0 && !m_scratchPath.isEmpty() && qMax(width, height) > edge
                && VideoTranscoder::scaleDown(m_path, videoSize, m_scratchPath)) {
            path = m_scratchPath;
            const qreal factor = qreal(edge) / qMax(width, height);
            // Even numbers: encoders work on pairs of lines.
            width = 2 * qRound(width * factor / 2);
            height = 2 * qRound(height * factor / 2);
            request.insert(QStringLiteral("mediaType"), QStringLiteral("video/mp4"));
            request.insert(QStringLiteral("name"), info.completeBaseName() + QStringLiteral(".mp4"));
        }
#endif
        request.insert(QStringLiteral("path"), path);
        request.insert(QStringLiteral("width"), width);
        request.insert(QStringLiteral("height"), height);
        const QImage frame = limited(videoFrame, ThumbnailSide).convertToFormat(QImage::Format_RGB32);
        emit prepared(request, QByteArray(), encode(frame, "JPEG", 80), QString());
        return;
    }

    // GIFs as media (Android's animated images): sent unchanged, the first frame as thumbnail.
    if (mediaType == QLatin1String("image/gif")
            && request.value(QStringLiteral("rendering")).toString() == QLatin1String("media")) {
        QImageReader reader(m_path);
        const QImage first = reader.read();
        if (!first.isNull()) {
            request.insert(QStringLiteral("path"), m_path);
            request.insert(QStringLiteral("width"), first.width());
            request.insert(QStringLiteral("height"), first.height());
            emit prepared(request, QByteArray(), encode(jpegThumbnail(first), "JPEG", 80), QString());
            return;
        }
    }

    const bool photo = request.value(QStringLiteral("rendering")).toString() == QLatin1String("media")
                       && (mediaType == QLatin1String("image/jpeg") || mediaType == QLatin1String("image/png")
                           || mediaType == QLatin1String("image/webp") || mediaType == QLatin1String("image/bmp"));
    if (!photo) {
        // Sent as it is.
        request.insert(QStringLiteral("rendering"), QStringLiteral("file"));
        request.insert(QStringLiteral("path"), m_path);
        emit prepared(request, QByteArray(), QByteArray(), QString());
        return;
    }

    QImageReader reader(m_path);
    reader.setAutoTransform(true); // apply the EXIF rotation before EXIF is dropped
    QImage image = reader.read();
    if (image.isNull()) {
        emit prepared(request, QByteArray(), QByteArray(), reader.errorString());
        return;
    }
    // Settings → Media & storage → Image size (Android's: default 1600 px; 0: original).
    const int maxSide = QSettings().value(QStringLiteral("imageSize"), 1600).toInt();
    if (maxSide > 0) {
        image = limited(image, maxSide);
    }
    const bool transparent = image.hasAlphaChannel();
    const QByteArray data = transparent ? encode(image, "PNG", -1) : encode(image, "JPEG", 85);
    request.insert(QStringLiteral("mediaType"),
                   transparent ? QStringLiteral("image/png") : QStringLiteral("image/jpeg"));
    if (!transparent) {
        QString name = info.completeBaseName() + QStringLiteral(".jpg");
        request.insert(QStringLiteral("name"), name);
    }
    request.insert(QStringLiteral("width"), image.width());
    request.insert(QStringLiteral("height"), image.height());

    emit prepared(request, data, encode(jpegThumbnail(image), "JPEG", 80), QString());
}
