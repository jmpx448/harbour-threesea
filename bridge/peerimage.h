// SPDX-License-Identifier: AGPL-3.0-only
#ifndef PEERIMAGE_H
#define PEERIMAGE_H

#include <QByteArray>
#include <QImageReader>

// Pictures from other people are shown without anyone tapping them, so the bytes alone must not
// choose among every image plugin installed on the phone. The format comes from the first bytes,
// whatever the sender's label says, and only these four get a reader. `pinTo` keeps Qt from
// asking any other plugin, the way it would by itself when a format's own decoder says no.
namespace PeerImage {

// "jpeg", "png", "gif" or "webp"; empty for anything else.
inline QByteArray format(const QByteArray &bytes)
{
    if (bytes.startsWith(QByteArrayLiteral("\xff\xd8\xff"))) {
        return QByteArrayLiteral("jpeg");
    }
    if (bytes.startsWith(QByteArrayLiteral("\x89PNG\r\n\x1a\n"))) {
        return QByteArrayLiteral("png");
    }
    if (bytes.startsWith(QByteArrayLiteral("GIF87a")) || bytes.startsWith(QByteArrayLiteral("GIF89a"))) {
        return QByteArrayLiteral("gif");
    }
    if (bytes.startsWith(QByteArrayLiteral("RIFF")) && bytes.mid(8, 4) == QByteArrayLiteral("WEBP")) {
        return QByteArrayLiteral("webp");
    }
    return QByteArray();
}

inline void pinTo(QImageReader &reader, const QByteArray &imageFormat)
{
    reader.setFormat(imageFormat);
    reader.setAutoDetectImageFormat(false);
}

}

#endif
