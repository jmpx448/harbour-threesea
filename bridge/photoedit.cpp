// SPDX-License-Identifier: AGPL-3.0-only
#include "photoedit.h"

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

}
