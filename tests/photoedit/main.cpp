// SPDX-License-Identifier: AGPL-3.0-only
// What a picture carries out of the app: nothing but its pixels, whichever way it goes. Exit 0:
// passed.
#include <QBuffer>
#include <QGuiApplication>
#include <QImage>
#include <QTemporaryDir>
#include <cstdio>

#include "filepreparer.h"
#include "photoedit.h"

namespace {

int failures = 0;

void check(bool ok, const char *what)
{
    if (!ok) {
        std::printf("FAILED: %s\n", what);
        ++failures;
    }
}

const char Secret[] = "written-where-the-photo-was-taken";

QByteArray encoded(const QImage &image, const char *format)
{
    QByteArray bytes;
    QBuffer buffer(&bytes);
    buffer.open(QIODevice::WriteOnly);
    image.save(&buffer, format, 90);
    return bytes;
}

// A picture with text in its file, the kind Qt's readers hand on.
QImage withText(QImage::Format format)
{
    QImage image(64, 48, format);
    image.fill(qRgba(10, 200, 30, format == QImage::Format_ARGB32 ? 128 : 255));
    image.setText(QStringLiteral("Comment"), QString::fromLatin1(Secret));
    return image;
}

// Runs a file through what prepares every chosen picture for sending.
QByteArray prepared(const QString &path)
{
    QVariantMap request;
    request.insert(QStringLiteral("rendering"), QStringLiteral("media"));
    FilePreparer preparer(path, request);
    QByteArray result;
    QObject::connect(&preparer, &FilePreparer::prepared,
                     [&result](const QVariantMap &, const QByteArray &data, const QByteArray &, const QString &) {
                         result = data;
                     });
    preparer.setAutoDelete(false);
    preparer.run();
    return result;
}

void metadataStaysBehind(const QString &dir)
{
    // First that the pictures below really have something to lose.
    const QByteArray jpeg = encoded(withText(QImage::Format_RGB32), "JPEG");
    const QByteArray png = encoded(withText(QImage::Format_ARGB32), "PNG");
    check(jpeg.contains(Secret), "a JPEG written from a QImage with text keeps it");
    check(png.contains(Secret), "a PNG written from a QImage with text keeps it");

    const QImage bare = PhotoEdit::withoutMetadata(withText(QImage::Format_ARGB32));
    check(bare.textKeys().isEmpty(), "withoutMetadata leaves no text");
    check(bare.pixel(3, 3) == qRgba(10, 200, 30, 128), "withoutMetadata keeps the pixels");
    check(bare.format() == QImage::Format_ARGB32, "withoutMetadata keeps the format");
    check(!encoded(bare, "PNG").contains(Secret), "nothing of the text in the PNG");

    QImage indexed = withText(QImage::Format_RGB32).convertToFormat(QImage::Format_Indexed8);
    const QImage bareIndexed = PhotoEdit::withoutMetadata(indexed);
    check(bareIndexed.pixel(3, 3) == indexed.pixel(3, 3), "withoutMetadata keeps a palette");

    // The way a chosen picture goes out.
    const QString jpegPath = dir + QStringLiteral("/in.jpg");
    const QString pngPath = dir + QStringLiteral("/in.png");
    check(withText(QImage::Format_RGB32).save(jpegPath, "JPEG", 90), "test JPEG written");
    check(withText(QImage::Format_ARGB32).save(pngPath, "PNG"), "test PNG written");
    const QByteArray sentJpeg = prepared(jpegPath);
    const QByteArray sentPng = prepared(pngPath);
    check(!sentJpeg.isEmpty() && !sentJpeg.contains(Secret), "FilePreparer: nothing of a JPEG's comment");
    check(!sentPng.isEmpty() && !sentPng.contains(Secret), "FilePreparer: nothing of a PNG's text");
}

}

int main(int argc, char **argv)
{
    QGuiApplication app(argc, argv);
    QTemporaryDir dir;
    metadataStaysBehind(dir.path());
    if (failures == 0) {
        std::printf("photoedit: passed\n");
    }
    return failures == 0 ? 0 : 1;
}
