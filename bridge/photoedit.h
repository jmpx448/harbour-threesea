// SPDX-License-Identifier: AGPL-3.0-only
#ifndef PHOTOEDIT_H
#define PHOTOEDIT_H

#include <QImage>
#include <QRectF>
#include <QString>

// What pictures on their way out have in common. Plain functions on images, for the thread pool.
namespace PhotoEdit {

// The same pixels with nothing else. A QImage carries the text of what it was read from (a JPEG's
// comments, a PNG's text chunks) through scaling and copying, and Qt's writers write it out again.
QImage withoutMetadata(const QImage &image);

// The picture turned by `rotation` (0, 90, 180, 270 degrees clockwise) and cut to `crop`, a
// rectangle in fractions (0..1) of the turned picture. Null if nothing is left of it.
QImage edited(const QImage &image, const QRectF &crop, int rotation);

// "png" for a picture with transparency, which a JPEG would flatten, else "jpg".
QString extensionFor(const QImage &image);
// Written in that format, at a high quality: what is sent is encoded once more (FilePreparer).
bool write(const QImage &image, const QString &path);

}

#endif // PHOTOEDIT_H
