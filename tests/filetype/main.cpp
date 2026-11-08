// SPDX-License-Identifier: AGPL-3.0-only
// FileType: sound stays sound whatever container it is in, a picture is found under any name, and
// the app's own folders are told from the rest. Exit 0: passed.
#include "filetype.h"

#include <QFile>
#include <QTemporaryDir>
#include <cstdio>

namespace {

int failures = 0;

void check(const QString &actual, const char *expected, const char *what)
{
    if (actual != QLatin1String(expected)) {
        std::printf("FAILED: %s: %s\n", what, qPrintable(actual));
        ++failures;
    }
}

void check(bool ok, const char *what)
{
    if (!ok) {
        std::printf("FAILED: %s\n", what);
        ++failures;
    }
}

const QByteArray isoMp4 = QByteArray("\0\0\0\x18" "ftypisom\0\0\2\0isommp42", 24) + QByteArray(64, '\0');
const QByteArray webm = QByteArray("\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\x82\x84webm", 16) + QByteArray(64, '\0');
const QByteArray jpeg = QByteArray("\xff\xd8\xff\xe0\0\x10JFIF\0\1\1\0\0\1\0\1\0\0", 20) + QByteArray(64, '\0');

QString kindOf(const QTemporaryDir &dir, const char *name, const QByteArray &bytes)
{
    const QString path = dir.path() + QLatin1Char('/') + QLatin1String(name);
    QFile file(path);
    file.open(QIODevice::WriteOnly);
    file.write(bytes);
    file.close();
    return FileType::sharedKind(FileType::mediaTypeOf(QFileInfo(path)));
}

} // namespace

int main()
{
    QTemporaryDir dir;
    check(kindOf(dir, "sound.m4a", isoMp4), "file", "an .m4a with an ISO brand");
    check(kindOf(dir, "sound.weba", webm), "file", "a .weba");
    check(kindOf(dir, "film.mp4", isoMp4), "video", "an .mp4");
    check(kindOf(dir, "noending", isoMp4), "video", "an MP4 without an ending");
    check(kindOf(dir, "photo.jpg", jpeg), "picture", "a .jpg");
    check(kindOf(dir, "photo.txt", jpeg), "picture", "a JPEG called .txt");
    check(kindOf(dir, "photo.bin", jpeg), "picture", "a JPEG called .bin");
    check(kindOf(dir, "noending2", jpeg), "picture", "a JPEG without an ending");
    check(kindOf(dir, "drawing.svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), "picture", "an SVG");
    check(kindOf(dir, "words.txt", "only words\n"), "file", "a text");
    check(kindOf(dir, "paper.pdf", "%PDF-1.4\n"), "file", "a PDF");

    QDir(dir.path()).mkpath(QStringLiteral("data/capture"));
    const QString data = dir.path() + QStringLiteral("/data");
    QFile inside(data + QStringLiteral("/capture/a.jpg"));
    inside.open(QIODevice::WriteOnly);
    inside.close();
    QFile::link(data + QStringLiteral("/capture/a.jpg"), dir.path() + QStringLiteral("/link.jpg"));
    check(FileType::isInside(data + QStringLiteral("/capture/a.jpg"), data), "a file in the folder");
    check(FileType::isInside(data + QStringLiteral("/gone.jpg"), data), "one that isn't there");
    check(FileType::isInside(data + QStringLiteral("/../data/capture/a.jpg"), data), "by way of ..");
    check(FileType::isInside(dir.path() + QStringLiteral("/link.jpg"), data), "by way of a link");
    check(!FileType::isInside(dir.path() + QStringLiteral("/photo.jpg"), data), "a file beside it");
    check(!FileType::isInside(data + QStringLiteral("2/a.jpg"), data), "a folder of a like name");
    return failures == 0 ? 0 : 1;
}
