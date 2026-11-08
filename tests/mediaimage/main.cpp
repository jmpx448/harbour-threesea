// SPDX-License-Identifier: AGPL-3.0-only
// PeerImage::decode: a picture lying on its side with EXIF saying so comes out upright, in its own
// proportions, whatever size is asked for. Exit 0: passed.
#include "peerimage.h"

#include <QColor>
#include <QCoreApplication>
#include <cstdio>

namespace {

int failures = 0;

void check(bool ok, const char *what)
{
    if (!ok) {
        std::printf("FAILED: %s\n", what);
        ++failures;
    }
}

// A JPEG of `width` x `height`, its left half red and its right half blue, with the EXIF
// orientation `orientation` (0: no EXIF at all).
QByteArray jpeg(int width, int height, int orientation)
{
    QImage image(width, height, QImage::Format_RGB32);
    for (int y = 0; y < height; ++y) {
        for (int x = 0; x < width; ++x) {
            image.setPixel(x, y, x < width / 2 ? qRgb(255, 0, 0) : qRgb(0, 0, 255));
        }
    }
    QByteArray bytes;
    QBuffer buffer(&bytes);
    buffer.open(QIODevice::WriteOnly);
    image.save(&buffer, "JPEG", 95);
    if (orientation == 0) {
        return bytes;
    }
    // Big-endian TIFF with a single entry: the orientation.
    QByteArray tiff("MM\x00\x2a\x00\x00\x00\x08\x00\x01\x01\x12\x00\x03\x00\x00\x00\x01", 18);
    tiff.append(char(0)).append(char(orientation)).append(char(0)).append(char(0));
    tiff.append(QByteArray(4, '\0'));
    const QByteArray exif = QByteArray("Exif\0\0", 6) + tiff;
    const int length = exif.size() + 2;
    QByteArray segment("\xff\xe1", 2);
    segment.append(char(length >> 8)).append(char(length & 0xff));
    segment.append(exif);
    return bytes.left(2) + segment + bytes.mid(2);
}

QImage decoded(QByteArray bytes, const QSize &requested = QSize())
{
    QString problem;
    const QImage image = PeerImage::decode(bytes, requested, &problem);
    if (image.isNull()) {
        std::printf("no picture: %s\n", qPrintable(problem));
    }
    return image;
}

bool redBlueTopToBottom(const QImage &image)
{
    const QColor top = image.pixelColor(image.width() / 2, image.height() / 8);
    const QColor bottom = image.pixelColor(image.width() / 2, image.height() * 7 / 8);
    return top.red() > 200 && top.blue() < 60 && bottom.blue() > 200 && bottom.red() < 60;
}

}

int main(int argc, char **argv)
{
    QCoreApplication app(argc, argv);

    // As stored.
    QImage image = decoded(jpeg(400, 300, 1));
    check(image.size() == QSize(400, 300), "upright stays as it is");
    image = decoded(jpeg(400, 300, 0));
    check(image.size() == QSize(400, 300), "no EXIF stays as it is");
    image = decoded(jpeg(400, 300, 3), QSize(200, 0));
    check(image.size() == QSize(200, 150), "half a turn keeps its proportions");

    // A quarter turn: the shape of what is shown, not of what is stored.
    image = decoded(jpeg(400, 300, 6));
    check(image.size() == QSize(300, 400), "orientation 6, full size");
    check(redBlueTopToBottom(image), "orientation 6 is really turned, not squashed");
    image = decoded(jpeg(400, 300, 8));
    check(image.size() == QSize(300, 400), "orientation 8, full size");

    // Size asked for by the viewer (both edges) and by a bubble (the width alone).
    image = decoded(jpeg(4000, 3000, 6), QSize(800, 800));
    check(image.size() == QSize(600, 800), "orientation 6 fits the box as shown");
    check(redBlueTopToBottom(image), "orientation 6, scaled: turned, not squashed");
    image = decoded(jpeg(4000, 3000, 6), QSize(300, 0));
    check(image.size() == QSize(300, 400), "orientation 6, width alone");
    image = decoded(jpeg(4000, 3000, 8), QSize(0, 400));
    check(image.size() == QSize(300, 400), "orientation 8, height alone");

    // Never larger than it is.
    image = decoded(jpeg(400, 300, 6), QSize(800, 800));
    check(image.size() == QSize(300, 400), "not scaled up");

    // What may not be decoded says why.
    QByteArray other("not a picture");
    QString problem;
    check(PeerImage::decode(other, QSize(), &problem).isNull() && !problem.isEmpty(), "not a picture");

    std::printf(failures == 0 ? "all passed\n" : "%d FAILED\n", failures);
    return failures == 0 ? 0 : 1;
}
