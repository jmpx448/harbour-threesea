// SPDX-License-Identifier: AGPL-3.0-only
#ifndef GIFDECODER_H
#define GIFDECODER_H

#include <QAtomicInt>
#include <QBuffer>
#include <QByteArray>
#include <QElapsedTimer>
#include <QImage>
#include <QObject>
#include <QScopedPointer>
#include <QSize>
#include <QTimer>

#include <functional>

class QImageReader;
class QThread;

// Plays one animated image on a thread of its own, so reading and decoding never hold up the
// interface: decrypting a whole GIF and decoding its frames on the GUI thread, once per tile,
// is what made a grid of favourite GIFs crawl. All decoders share one low-priority thread
// (decoderThread()): many at once then play slower, they don't take more of the phone.
//
// Lives on that thread. Only cancel() and frameTaken() may be called from another; everything
// else goes through queued calls.
class GifDecoder : public QObject
{
    Q_OBJECT

public:
    // `read` is called on the decoder's thread and returns the decrypted file.
    explicit GifDecoder(std::function<QByteArray()> read, QObject *parent = nullptr);
    ~GifDecoder() override;

    // The shared thread, started on first use and stopped when the application quits.
    static QThread *decoderThread();

    // Skip whatever is still queued: a tile scrolled past must not cost a decryption.
    void cancel() { m_cancelled.storeRelease(1); }
    // The frame sent last has arrived. Until then further frames aren't scaled or sent, so a
    // busy interface never finds a queue of them.
    void frameTaken() { m_framesOut.storeRelease(0); }

    // `image` as it is sent to fill `target` (cropped when drawn): scaled down, never up, to
    // what that takes, and premultiplied, as the scene graph wants it. An empty target: full size.
    static QImage scaledFrame(const QImage &image, const QSize &target);
    // What to wait after a frame: GIFs that claim to have no delay play like in a browser.
    static int frameDelay(int claimed);

public slots:
    void load();
    void setPlaying(bool playing);
    void setTargetSize(const QSize &size);

signals:
    // An unusable file (missing, too large, undecodable) just never sends one.
    void frameReady(const QImage &frame);

private:
    void step();
    bool readFrame();
    bool rewind();
    void send();
    void schedule();
    void wipe();

    std::function<QByteArray()> m_read;
    QAtomicInt m_cancelled;
    QAtomicInt m_framesOut;
    QByteArray m_data;
    QBuffer m_buffer;
    QScopedPointer<QImageReader> m_reader;
    QTimer m_timer;
    QElapsedTimer m_sinceFrame;
    QImage m_frame;
    QSize m_target;
    bool m_playing = false;
    bool m_finished = false;
    int m_delay = 0;
    int m_framesThisPass = 0;
    // Further plays after this one; -1: forever.
    int m_loopsLeft = 0;
};

#endif // GIFDECODER_H
