// SPDX-License-Identifier: AGPL-3.0-only
#ifndef MEDIAIMAGEPROVIDER_H
#define MEDIAIMAGEPROVIDER_H

#include <QQuickImageProvider>

class ThreeseBackend;

// image://threesea/<message id>/thumb, image://threesea/<message id>/file and
// image://threesea/group/<group key> (group pictures), image://threesea/contact/<ID> and
// image://threesea/contact/me (profile pictures): images from the
// encrypted media store, decrypted in memory only. Use with `asynchronous: true` (loads on
// QML's image loader thread) and a `sourceSize` to decode only what is shown. Thread-safe:
// the core's media reads may be called from any thread and nothing here is shared.
class MediaImageProvider : public QQuickImageProvider
{
public:
    explicit MediaImageProvider(const ThreeseBackend *backend);

    QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize) override;

private:
    const ThreeseBackend *m_backend;
};

#endif
