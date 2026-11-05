// SPDX-License-Identifier: AGPL-3.0-only
#ifndef FILETYPE_H
#define FILETYPE_H

#include <QDir>
#include <QFileInfo>
#include <QMimeDatabase>
#include <QString>

// What a file to be sent really is. Headers only: tests/filetype.
namespace FileType {

// By its name, and where the name says nothing of use, by its first bytes. Only then: an .m4a is an
// MP4 inside and a .weba a WebM, and still sound.
inline QString mediaTypeOf(const QFileInfo &file)
{
    // Not in the system's list of endings, which would take it for what it holds.
    if (file.suffix().compare(QLatin1String("weba"), Qt::CaseInsensitive) == 0) {
        return QStringLiteral("audio/webm");
    }
    const QMimeDatabase types;
    const QString named = types.mimeTypeForFile(file).name();
    if (named != QLatin1String("application/octet-stream") && named != QLatin1String("text/plain")) {
        return named;
    }
    const QString read = types.mimeTypeForFile(file, QMimeDatabase::MatchContent).name();
    return read.startsWith(QLatin1String("image/")) || read.startsWith(QLatin1String("video/")) ? read : named;
}

inline bool isPicture(const QString &mediaType)
{
    return mediaType.startsWith(QLatin1String("image/"));
}

// What a file shared from another app goes as.
inline QString sharedKind(const QString &mediaType)
{
    return isPicture(mediaType) ? QStringLiteral("picture")
         : mediaType.startsWith(QLatin1String("video/")) ? QStringLiteral("video")
         : QStringLiteral("file");
}

// Whether `path` lies in `directory`, links followed where the file exists.
inline bool isInside(const QString &path, const QString &directory)
{
    const QFileInfo file(path);
    const QString real = file.canonicalFilePath().isEmpty() ? QDir::cleanPath(file.absoluteFilePath())
                                                            : file.canonicalFilePath();
    const QFileInfo folder(directory);
    const QString within = folder.canonicalFilePath().isEmpty() ? QDir::cleanPath(folder.absoluteFilePath())
                                                                : folder.canonicalFilePath();
    return real.startsWith(within + QLatin1Char('/'));
}

} // namespace FileType

#endif // FILETYPE_H
