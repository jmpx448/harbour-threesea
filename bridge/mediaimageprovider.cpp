// SPDX-License-Identifier: AGPL-3.0-only
#include "mediaimageprovider.h"

#include <QDebug>

#include "peerimage.h"
#include "threesebackend.h"

MediaImageProvider::MediaImageProvider(const ThreeseBackend *backend)
    // Without ForceAsynchronousImageLoading Qt calls a provider on the GUI thread, even for an
    // Image with `asynchronous: true` — so every picture in a chat was decrypted and decoded
    // while the UI waited. The core's media reads are thread-safe (core/src/ffi.rs) and
    // nothing else here is shared, so the image loader thread may do it.
    : QQuickImageProvider(QQuickImageProvider::Image, QQuickImageProvider::ForceAsynchronousImageLoading)
    , m_backend(backend)
{
}

QImage MediaImageProvider::requestImage(const QString &id, QSize *size, const QSize &requestedSize)
{
    // "<message>/<thumb|file>", optionally with "?<anything>" to force a reload.
    // Or "group/<group key>" for a group's picture, "gif/<id>" for a favourite GIF.
    const QString path = id.section(QLatin1Char('?'), 0, 0);
    QByteArray data;
    qint64 message = -1;
    if (path.startsWith(QLatin1String("group/"))) {
        data = m_backend->readPicture(path.mid(6));
    } else if (path.startsWith(QLatin1String("gif/"))) {
        // A favourite GIF's thumbnail ("gif/<id>"). Not every GIF came with one (a sender's
        // message may have none): then its first frame.
        data = m_backend->readPicture(QStringLiteral("gif-thumb:") + path.mid(4));
        if (data.isEmpty()) {
            data = m_backend->readPicture(QStringLiteral("gif:") + path.mid(4));
        }
    } else if (path.startsWith(QLatin1String("contact/"))) {
        // A contact's picture ("contact/<ID>"), or ours ("contact/me").
        const QString who = path.mid(8);
        data = m_backend->readPicture(who == QLatin1String("me") ? who : QStringLiteral("contact:") + who);
    } else {
        const QStringList parts = path.split(QLatin1Char('/'));
        bool ok = false;
        message = parts.value(0).toLongLong(&ok);
        if (!ok || parts.size() != 2) {
            return QImage();
        }
        data = m_backend->readMedia(message, parts.at(1) == QLatin1String("thumb"));
    }
    if (data.isEmpty()) {
        return QImage();
    }
    QString problem;
    const QImage image = PeerImage::decode(data, requestedSize, &problem);
    if (!problem.isEmpty()) {
        // log-ok: our own fixed texts and the image decoder's message, no content
        qWarning() << "media image" << message << problem;
    }
    data.fill('\0');
    if (size) {
        *size = image.size();
    }
    return image;
}
