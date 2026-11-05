// SPDX-License-Identifier: AGPL-3.0-only
// What a picture carries out of the app through Qt's writers: nothing but its pixels. GIFs (not
// written here) and the browser's pictures have their tests in the core. Exit 0: passed.
#include <QBuffer>
#include <QDirIterator>
#include <QFontDatabase>
#include <QGuiApplication>
#include <QImage>
#include <QRunnable>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QVariantList>
#include <QVariantMap>
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

void wipesOnlyWhatIsItsOwn()
{
    QImage own = halves(false);
    PhotoEdit::wipe(own);
    check(qRed(own.pixel(1, 1)) == 0, "nothing left of the picture");

    const QImage other = halves(false);
    QImage sharing = other;
    PhotoEdit::wipe(sharing);
    check(qRed(other.pixel(1, 1)) == 255, "what is shared stays for whoever shares it");
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

// A line and a text on a white picture, where a preview would have shown them.
void marksLandInPlace()
{
    QImage white(400, 200, QImage::Format_RGB32);
    white.fill(Qt::white);

    QVariantMap line;
    // Across the left part, in fractions of the picture, 2 % of its longest side wide.
    line.insert(QStringLiteral("points"), QVariantList() << 0.1 << 0.5 << 0.4 << 0.5);
    line.insert(QStringLiteral("color"), QStringLiteral("#ff0000"));
    line.insert(QStringLiteral("width"), 0.02);
    QVariantMap dot;
    dot.insert(QStringLiteral("points"), QVariantList() << 0.9 << 0.9);
    dot.insert(QStringLiteral("color"), QStringLiteral("#0000ff"));
    dot.insert(QStringLiteral("width"), 0.05);
    const QVariantList marks = QVariantList() << line << dot;

    const QRectF whole(0, 0, 1, 1);
    const QImage drawn = PhotoEdit::edited(white, whole, 0, marks);
    check(qRed(drawn.pixel(100, 100)) == 255 && qGreen(drawn.pixel(100, 100)) == 0, "the line is where it was drawn");
    check(drawn.pixel(100, 50) == qRgb(255, 255, 255), "and not elsewhere");
    check(qBlue(drawn.pixel(360, 180)) == 255 && qRed(drawn.pixel(360, 180)) == 0, "a single point is a dot");

    // The marks belong to the picture: turned with it, whichever came first.
    const QImage turned = PhotoEdit::edited(white, whole, 90, marks);
    check(turned.size() == QSize(200, 400), "drawn and turned");
    check(qGreen(turned.pixel(100, 100)) == 0 && qRed(turned.pixel(100, 100)) == 255, "the line turned along");

    // Twice the pixels: the same drawing, twice as wide.
    QImage large(800, 400, QImage::Format_RGB32);
    large.fill(Qt::white);
    const QImage drawnLarge = PhotoEdit::edited(large, whole, 0, marks);
    check(qGreen(drawnLarge.pixel(200, 200 + 6)) == 0, "the width follows the picture's size");

    // On a picture with transparency: the marks are drawn, the rest stays see-through.
    const QImage seeThrough = PhotoEdit::edited(halves(true), whole, 0, QVariantList() << line);
    check(seeThrough.hasAlphaChannel(), "drawing keeps transparency");
    check(qAlpha(seeThrough.pixel(2, 2)) == 0, "what wasn't drawn on stays see-through");
    check(qAlpha(seeThrough.pixel(10, 10)) == 255, "what was is not");

    // A palette can't be painted on as it is.
    const QImage indexed = white.convertToFormat(QImage::Format_Indexed8);
    check(qGreen(PhotoEdit::edited(indexed, whole, 0, marks).pixel(100, 100)) == 0, "drawn on a palette picture");

    QVariantMap text;
    text.insert(QStringLiteral("text"), QStringLiteral("HHHH"));
    text.insert(QStringLiteral("x"), 0.5);
    text.insert(QStringLiteral("y"), 0.5);
    text.insert(QStringLiteral("color"), QStringLiteral("#000000"));
    text.insert(QStringLiteral("size"), 0.1);
    text.insert(QStringLiteral("family"), QString());
    text.insert(QStringLiteral("angle"), 0);
    // On the thread pool, where it really runs. The platform plugin here knows no fonts by itself.
    QDirIterator fonts(QStringLiteral("/usr/share/fonts"), QStringList() << QStringLiteral("*.ttf"), QDir::Files,
                       QDirIterator::Subdirectories);
    QString family;
    while (family.isEmpty() && fonts.hasNext()) {
        family = QFontDatabase::applicationFontFamilies(QFontDatabase::addApplicationFont(fonts.next())).value(0);
    }
    text.insert(QStringLiteral("family"), family);
    QImage written;
    class Drawing : public QRunnable
    {
    public:
        Drawing(const QImage &white, const QVariantMap &text, QImage *out) : m_white(white), m_text(text), m_out(out) {}
        void run() override
        {
            *m_out = PhotoEdit::edited(m_white, QRectF(0, 0, 1, 1), 0, QVariantList() << m_text);
        }
        const QImage m_white;
        const QVariantMap m_text;
        QImage *m_out;
    };
    QThreadPool::globalInstance()->start(new Drawing(white, text, &written));
    QThreadPool::globalInstance()->waitForDone();
    check(!family.isEmpty(), "a font to test with");
    check(written != white, "the text is drawn");
    {
        int minX = 400, maxX = -1, minY = 200, maxY = -1;
        for (int y = 0; y < 200; ++y) {
            for (int x = 0; x < 400; ++x) {
                if (written.pixel(x, y) != qRgb(255, 255, 255)) {
                    minX = qMin(minX, x);
                    maxX = qMax(maxX, x);
                    minY = qMin(minY, y);
                    maxY = qMax(maxY, y);
                }
            }
        }
        check(qAbs((minX + maxX) / 2 - 200) < 8, "the text is centred on its place across");
        check(qAbs((minY + maxY) / 2 - 100) < 12, "and down");
    }
}

}

int main(int argc, char **argv)
{
    QGuiApplication app(argc, argv);
    QTemporaryDir dir;
    metadataStaysBehind(dir.path());
    turnsAndCuts();
    wipesOnlyWhatIsItsOwn();
    transparencyStays(dir.path());
    marksLandInPlace();
    if (failures == 0) {
        std::printf("photoedit: passed\n");
    }
    return failures == 0 ? 0 : 1;
}
