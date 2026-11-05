// SPDX-License-Identifier: AGPL-3.0-only
#include "photoedit.h"

#include <QFile>
#include <QTransform>
#include <QVector>

namespace PhotoEdit {

QImage withoutMetadata(const QImage &image)
{
    if (image.isNull()) {
        return image;
    }
    // Over the same pixels, then made its own: a new image has no text to take along.
    QImage bare(image.constBits(), image.width(), image.height(), image.bytesPerLine(), image.format());
    bare.setColorTable(image.colorTable());
    return bare.copy();
}

QImage edited(const QImage &image, const QRectF &crop, int rotation)
{
    QImage work = image.convertToFormat(image.hasAlphaChannel() ? QImage::Format_ARGB32 : QImage::Format_RGB32);
    if (rotation % 360 != 0) {
        QTransform turn;
        turn.rotate(rotation);
        work = work.transformed(turn, Qt::SmoothTransformation);
    }
    // In fractions, so it survives any scaling the view did.
    const QRect cut = QRect(qRound(crop.x() * work.width()), qRound(crop.y() * work.height()),
                            qRound(crop.width() * work.width()), qRound(crop.height() * work.height()))
                          .intersected(work.rect());
    if (cut.width() < 1 || cut.height() < 1) {
        return QImage();
    }
    return withoutMetadata(cut == work.rect() ? work : work.copy(cut));
}

QString extensionFor(const QImage &image)
{
    return image.hasAlphaChannel() ? QStringLiteral("png") : QStringLiteral("jpg");
}

bool write(const QImage &image, const QString &path)
{
    const bool written = image.hasAlphaChannel() ? image.save(path, "PNG") : image.save(path, "JPEG", 95);
    if (!written) {
        QFile::remove(path);
    }
    return written;
}

}
