// SPDX-License-Identifier: AGPL-3.0-only
#include "animatedimage.h"

#include <QQuickWindow>
#include <QSGSimpleTextureNode>
#include <QScopedPointer>
#include <QtMath>

#include "gifdecoder.h"
#include "threesebackend.h"

namespace {

// Holds the frame's texture: a new one for every frame, the old one deleted once replaced.
class FrameNode : public QSGSimpleTextureNode
{
public:
    void setFrame(QSGTexture *texture)
    {
        setTexture(texture);
        m_texture.reset(texture);
    }

private:
    QScopedPointer<QSGTexture> m_texture;
};

}

const ThreeseBackend *AnimatedImage::s_backend = nullptr;

AnimatedImage::AnimatedImage(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents);
}

AnimatedImage::~AnimatedImage()
{
    unload();
}

void AnimatedImage::setBackend(const ThreeseBackend *backend)
{
    s_backend = backend;
}

void AnimatedImage::setMessage(int message)
{
    if (message == m_message) {
        return;
    }
    m_message = message;
    emit messageChanged();
    sourceChanged();
}

void AnimatedImage::setFavourite(int favourite)
{
    if (favourite == m_favourite) {
        return;
    }
    m_favourite = favourite;
    emit favouriteChanged();
    sourceChanged();
}

void AnimatedImage::setPlaying(bool playing)
{
    if (playing == m_playing) {
        return;
    }
    m_playing = playing;
    emit playingChanged();
    updateRunning();
}

void AnimatedImage::setOnDemand(bool onDemand)
{
    if (onDemand == m_onDemand) {
        return;
    }
    m_onDemand = onDemand;
    emit onDemandChanged();
    updateRunning();
}

void AnimatedImage::componentComplete()
{
    QQuickItem::componentComplete();
    m_complete = true;
    updateRunning();
}

void AnimatedImage::sourceChanged()
{
    unload();
    updateRunning();
}

void AnimatedImage::load()
{
    if (!s_backend || !window() || (m_message < 0 && m_favourite < 0)) {
        return;
    }
    const ThreeseBackend *backend = s_backend;
    const int message = m_message;
    const int favourite = m_favourite;
    m_decoder = new GifDecoder([backend, message, favourite]() {
        return message >= 0 ? backend->readMedia(message, false)
                            : backend->readPicture(QStringLiteral("gif:%1").arg(favourite));
    });
    m_decoder->moveToThread(GifDecoder::decoderThread());
    const int generation = ++m_generation;
    connect(m_decoder, &GifDecoder::frameReady, this, [this, generation](const QImage &frame) {
        if (generation != m_generation || !m_decoder) {
            return;
        }
        m_frame = frame;
        m_frameChanged = true;
        m_decoder->frameTaken();
        if (!m_ready) {
            m_ready = true;
            emit readyChanged();
        }
        update();
    }, Qt::QueuedConnection);
    QMetaObject::invokeMethod(m_decoder, "setTargetSize", Qt::QueuedConnection, Q_ARG(QSize, targetSize()));
    QMetaObject::invokeMethod(m_decoder, "load", Qt::QueuedConnection);
    m_running = false;
}

void AnimatedImage::unload()
{
    if (m_decoder) {
        m_decoder->cancel();
        m_decoder->deleteLater();
        m_decoder = nullptr;
    }
    m_running = false;
    m_frame = QImage();
    m_frameChanged = false;
    if (m_ready) {
        m_ready = false;
        emit readyChanged();
    }
    update();
}

void AnimatedImage::updateRunning()
{
    if (!m_complete) {
        return;
    }
    if (!m_decoder && (!m_onDemand || shouldRun())) {
        load();
    }
    if (m_decoder && shouldRun() != m_running) {
        m_running = shouldRun();
        QMetaObject::invokeMethod(m_decoder, "setPlaying", Qt::QueuedConnection, Q_ARG(bool, m_running));
    }
}

QSize AnimatedImage::targetSize() const
{
    return QSize(qCeil(width()), qCeil(height()));
}

void AnimatedImage::geometryChanged(const QRectF &newGeometry, const QRectF &oldGeometry)
{
    QQuickItem::geometryChanged(newGeometry, oldGeometry);
    if (m_decoder && newGeometry.size() != oldGeometry.size()) {
        QMetaObject::invokeMethod(m_decoder, "setTargetSize", Qt::QueuedConnection, Q_ARG(QSize, targetSize()));
    }
    update();
}

void AnimatedImage::itemChange(ItemChange change, const ItemChangeData &value)
{
    if (change == ItemVisibleHasChanged || change == ItemSceneChange) {
        updateRunning();
    }
    QQuickItem::itemChange(change, value);
}

QSGNode *AnimatedImage::updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *)
{
    FrameNode *node = static_cast<FrameNode *>(oldNode);
    if (m_frame.isNull() || width() <= 0 || height() <= 0) {
        delete node;
        return nullptr;
    }
    // A new node also after the scene graph was thrown away (the app left the screen).
    const bool fresh = !node;
    if (fresh) {
        node = new FrameNode;
        node->setFiltering(QSGTexture::Linear);
    }
    if (fresh || m_frameChanged) {
        QSGTexture *texture = window()->createTextureFromImage(m_frame);
        if (!texture) {
            delete node;
            return nullptr;
        }
        node->setFrame(texture);
        m_frameChanged = false;
    }
    // PreserveAspectCrop: fill the item, cut off what sticks out (centred).
    const QSizeF frame(m_frame.size());
    const QSizeF shown = QSizeF(width(), height()).scaled(frame, Qt::KeepAspectRatio);
    node->setRect(boundingRect());
    node->setSourceRect(QRectF((frame.width() - shown.width()) / 2, (frame.height() - shown.height()) / 2,
                               shown.width(), shown.height()));
    return node;
}
