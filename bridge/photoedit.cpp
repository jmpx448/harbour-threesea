// SPDX-License-Identifier: AGPL-3.0-only
#include "photoedit.h"

#include <QColor>
#include <QFile>
#include <QFont>
#include <QFontMetricsF>
#include <QPainter>
#include <QPen>
#include <QPolygonF>
#include <QTransform>
#include <QVariantMap>

namespace {

void drawMarks(QImage &image, const QVariantList &marks)
{
    const qreal longest = qMax(image.width(), image.height());
    QPainter painter(&image);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setRenderHint(QPainter::TextAntialiasing);
    for (const QVariant &value : marks) {
        const QVariantMap mark = value.toMap();
        const QVariantList points = mark.value(QStringLiteral("points")).toList();
        if (points.size() < 2) {
            continue;
        }
        QPolygonF line;
        for (int i = 0; i + 1 < points.size(); i += 2) {
            line << QPointF(points.at(i).toReal() * image.width(), points.at(i + 1).toReal() * image.height());
        }
        painter.setPen(QPen(QColor(mark.value(QStringLiteral("color")).toString()),
                            qMax<qreal>(1, mark.value(QStringLiteral("width")).toReal() * longest), Qt::SolidLine,
                            Qt::RoundCap, Qt::RoundJoin));
        if (line.size() == 1) {
            painter.drawPoint(line.first());
        } else {
            painter.drawPolyline(line);
        }
    }
    for (const QVariant &value : marks) {
        const QVariantMap mark = value.toMap();
        const QString text = mark.value(QStringLiteral("text")).toString();
        if (text.isEmpty()) {
            continue;
        }
        QFont font(mark.value(QStringLiteral("family")).toString());
        font.setPixelSize(qMax(1, qRound(mark.value(QStringLiteral("size")).toReal() * longest)));
        const QFontMetricsF metrics(font);
        painter.save();
        painter.translate(mark.value(QStringLiteral("x")).toReal() * image.width(),
                          mark.value(QStringLiteral("y")).toReal() * image.height());
        painter.rotate(mark.value(QStringLiteral("angle")).toReal());
        painter.setFont(font);
        painter.setPen(QColor(mark.value(QStringLiteral("color")).toString()));
        painter.drawText(QPointF(-metrics.width(text) / 2, metrics.ascent() - metrics.height() / 2), text);
        painter.restore();
    }
}

}

namespace PhotoEdit {

QImage withoutMetadata(const QImage &image)
{
    if (image.isNull()) {
        return image;
    }
    // Over the same pixels, then made its own: a new image has no text to take along.
    QImage bare(image.constBits(), image.width(), image.height(), image.bytesPerLine(), image.format());
    // Only where there is one: setting it would make the wrapper copy the pixels first.
    if (image.colorCount() > 0) {
        bare.setColorTable(image.colorTable());
    }
    return bare.copy();
}

void wipe(QImage &image)
{
    if (!image.isNull() && image.isDetached()) {
        image.fill(0u);
    }
}

QImage edited(const QImage &image, const QRectF &crop, int rotation, const QVariantList &marks)
{
    QImage work = image.convertToFormat(image.hasAlphaChannel() ? QImage::Format_ARGB32 : QImage::Format_RGB32);
    if (!marks.isEmpty()) {
        drawMarks(work, marks);
    }
    if (rotation % 360 != 0) {
        QTransform turn;
        turn.rotate(rotation);
        QImage turned = work.transformed(turn, Qt::SmoothTransformation);
        wipe(work);
        work = turned;
    }
    // In fractions, so it survives any scaling the view did.
    const QRect cut = QRect(qRound(crop.x() * work.width()), qRound(crop.y() * work.height()),
                            qRound(crop.width() * work.width()), qRound(crop.height() * work.height()))
                          .intersected(work.rect());
    if (cut.width() < 1 || cut.height() < 1) {
        wipe(work);
        return QImage();
    }
    const QImage result = withoutMetadata(cut == work.rect() ? work : work.copy(cut));
    wipe(work);
    return result;
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
