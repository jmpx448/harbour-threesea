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

// Left half red, right half blue; with `transparent` the top half of it see-through.
QImage halves(bool transparent)
{
    QImage image(40, 20, transparent ? QImage::Format_ARGB32 : QImage::Format_RGB32);
    for (int y = 0; y < image.height(); ++y) {
        for (int x = 0; x < image.width(); ++x) {
            const QRgb colour = x < 20 ? qRgb(255, 0, 0) : qRgb(0, 0, 255);
            image.setPixel(x, y, transparent && y < 10 ? qRgba(qRed(colour), 0, qBlue(colour), 0) : colour);
        }
    }
    return image;
}

void turnsAndCuts()
{
    const QImage whole = PhotoEdit::edited(halves(false), QRectF(0, 0, 1, 1), 0);
    check(whole.size() == QSize(40, 20), "whole picture keeps its size");

    // Turned clockwise, the left half comes to the top.
    const QImage turned = PhotoEdit::edited(halves(false), QRectF(0, 0, 1, 1), 90);
    check(turned.size() == QSize(20, 40), "turned sideways");
    check(qRed(turned.pixel(10, 5)) == 255 && qBlue(turned.pixel(10, 35)) == 255, "turned clockwise");

    // The cut is in fractions of the turned picture.
    const QImage cut = PhotoEdit::edited(halves(false), QRectF(0, 0, 1, 0.5), 90);
    check(cut.size() == QSize(20, 20) && qBlue(cut.pixel(10, 10)) == 0, "cut after turning");

    check(PhotoEdit::edited(halves(false), QRectF(0, 0, 0, 0), 0).isNull(), "nothing left: nothing");
}

void transparencyStays(const QString &dir)
{
    const QImage result = PhotoEdit::edited(halves(true), QRectF(0, 0, 1, 1), 90);
    check(result.hasAlphaChannel(), "editing keeps transparency");
    check(PhotoEdit::extensionFor(result) == QLatin1String("png"), "a transparent picture is a PNG");
    check(PhotoEdit::extensionFor(halves(false)) == QLatin1String("jpg"), "an opaque one a JPEG");

    const QString path = dir + QStringLiteral("/edited.png");
    check(PhotoEdit::write(result, path), "edited picture written");
    const QImage back(path);
    // The top half, now on the right: see-through; the other half opaque.
    check(qAlpha(back.pixel(15, 20)) == 0, "see-through after writing");
    check(qAlpha(back.pixel(5, 20)) == 255, "opaque after writing");

    check(!PhotoEdit::write(result, dir + QStringLiteral("/no/such/folder/edited.png")), "failing to write says so");

    QImage marked = halves(false);
    marked.setText(QStringLiteral("Comment"), QString::fromLatin1(Secret));
    check(PhotoEdit::edited(marked, QRectF(0, 0, 1, 1), 0).textKeys().isEmpty(), "edited: no text left");
    check(PhotoEdit::edited(marked, QRectF(0, 0, 0.5, 1), 90).textKeys().isEmpty(), "edited and cut: no text left");
}

}

int main(int argc, char **argv)
{
    QGuiApplication app(argc, argv);
    QTemporaryDir dir;
    metadataStaysBehind(dir.path());
    turnsAndCuts();
    transparencyStays(dir.path());
    if (failures == 0) {
        std::printf("photoedit: passed\n");
    }
    return failures == 0 ? 0 : 1;
}
