// SPDX-License-Identifier: AGPL-3.0-only
#ifndef PHOTOEDIT_H
#define PHOTOEDIT_H

#include <QImage>

// What pictures on their way out have in common. Plain functions on images, for the thread pool.
namespace PhotoEdit {

// The same pixels with nothing else. A QImage carries the text of what it was read from (a JPEG's
// comments, a PNG's text chunks) through scaling and copying, and Qt's writers write it out again.
QImage withoutMetadata(const QImage &image);

}

#endif // PHOTOEDIT_H
