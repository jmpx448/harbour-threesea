// SPDX-License-Identifier: AGPL-3.0-only
#ifndef ANIMATEDIMAGE_H
#define ANIMATEDIMAGE_H

#include <QImage>
#include <QQuickItem>

class GifDecoder;
class ThreeseBackend;

// Plays an animated image (GIF) of a file message or a favourite: decrypted by the core into
// memory and decoded from there on GifDecoder's thread, so no plaintext copy is written (like
// VideoPlayer) and the interface doesn't wait for it. QML's AnimatedImage can't do this (no
// image providers for animations). Drawn like an Image with PreserveAspectCrop.
class AnimatedImage : public QQuickItem
{
    Q_OBJECT
    Q_PROPERTY(int message READ message WRITE setMessage NOTIFY messageChanged)
    // Or a favourite GIF (its id).
    Q_PROPERTY(int favourite READ favourite WRITE setFavourite NOTIFY favouriteChanged)
    Q_PROPERTY(bool playing READ playing WRITE setPlaying NOTIFY playingChanged)
    // Don't read the file until asked to play: for a caller with a thumbnail to show until then.
    // Otherwise the first frame shows even while not playing.
    Q_PROPERTY(bool onDemand READ onDemand WRITE setOnDemand NOTIFY onDemandChanged)
    // The first frame is decoded and drawn.
    Q_PROPERTY(bool ready READ ready NOTIFY readyChanged)

public:
    explicit AnimatedImage(QQuickItem *parent = nullptr);
    ~AnimatedImage() override;

    // Set once before QML creates any item.
    static void setBackend(const ThreeseBackend *backend);

    int message() const { return m_message; }
    void setMessage(int message);
    int favourite() const { return m_favourite; }
    void setFavourite(int favourite);
    bool playing() const { return m_playing; }
    void setPlaying(bool playing);
    bool onDemand() const { return m_onDemand; }
    void setOnDemand(bool onDemand);
    bool ready() const { return m_ready; }

signals:
    void messageChanged();
    void favouriteChanged();
    void playingChanged();
    void onDemandChanged();
    void readyChanged();

protected:
    void componentComplete() override;
    void geometryChanged(const QRectF &newGeometry, const QRectF &oldGeometry) override;
    void itemChange(ItemChange change, const ItemChangeData &value) override;
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *data) override;

private:
    void sourceChanged();
    void load();
    void unload();
    void updateRunning();
    bool shouldRun() const { return m_playing && isVisible(); }
    QSize targetSize() const;

    static const ThreeseBackend *s_backend;
    int m_message = -1;
    int m_favourite = -1;
    bool m_playing = false;
    bool m_onDemand = false;
    bool m_ready = false;
    bool m_running = false;
    bool m_complete = false;
    // Which decoder a queued frame is from: one that was replaced may still have some on the way.
    int m_generation = 0;
    GifDecoder *m_decoder = nullptr;
    QImage m_frame;
    bool m_frameChanged = false;
};

#endif // ANIMATEDIMAGE_H
