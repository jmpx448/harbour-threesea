// SPDX-License-Identifier: AGPL-3.0-only
#include "gifdecoder.h"

#include <QCoreApplication>
#include <QDebug>
#include <QImageReader>
#include <QThread>

#include "peerimage.h"

namespace {
// Largest GIF frame decoded (pixels).
const qint64 MAX_FRAME_PIXELS = 16 * 1000 * 1000;
// A delay this short means "not set"; browsers play those at 10 frames a second, so do we.
const int MIN_FRAME_DELAY_MS = 20;
const int UNSET_FRAME_DELAY_MS = 100;
const QByteArray GIF = QByteArrayLiteral("gif");
}

GifDecoder::GifDecoder(std::function<QByteArray()> read, QObject *parent)
    : QObject(parent)
    , m_read(std::move(read))
    , m_buffer(this)
    , m_timer(this)
{
    m_timer.setSingleShot(true);
    connect(&m_timer, &QTimer::timeout, this, &GifDecoder::step);
}

GifDecoder::~GifDecoder()
{
    wipe();
}

QThread *GifDecoder::decoderThread()
{
    static QThread *thread = nullptr;
    if (!thread) {
        thread = new QThread(QCoreApplication::instance());
        thread->setObjectName(QStringLiteral("GifDecoder"));
        // Before anything the decoders read from goes away with the application.
        QObject::connect(QCoreApplication::instance(), &QCoreApplication::aboutToQuit, thread, []() {
            thread->quit();
            thread->wait();
        }, Qt::DirectConnection);
        thread->start(QThread::LowPriority);
    }
    return thread;
}

QImage GifDecoder::scaledFrame(const QImage &image, const QSize &target)
{
    const QImage frame = image.convertToFormat(QImage::Format_ARGB32_Premultiplied);
    if (target.isEmpty()) {
        return frame;
    }
    const QSize size = frame.size().scaled(target, Qt::KeepAspectRatioByExpanding);
    // Only ever down, in both directions, so the picture keeps its proportions.
    if (size.width() >= frame.width() || size.height() >= frame.height()) {
        return frame;
    }
    return frame.scaled(size, Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
}

int GifDecoder::frameDelay(int claimed)
{
    return claimed < MIN_FRAME_DELAY_MS ? UNSET_FRAME_DELAY_MS : claimed;
}

void GifDecoder::load()
{
    if (m_cancelled.loadAcquire()) {
        return;
    }
    m_data = m_read();
    if (m_data.isEmpty() || m_cancelled.loadAcquire()) {
        wipe();
        return;
    }
    if (PeerImage::format(m_data) != GIF) {
        qWarning("animated image is not a GIF");
        wipe();
        return;
    }
    // Every frame is decoded at full size: not beyond what a phone holds (a GIF's header can
    // claim any size; security audit 2026-10-27, S06).
    {
        QBuffer probe(&m_data);
        probe.open(QIODevice::ReadOnly);
        QImageReader reader(&probe);
        PeerImage::pinTo(reader, GIF);
        const QSize size = reader.size();
        if (!size.isValid() || static_cast<qint64>(size.width()) * size.height() > MAX_FRAME_PIXELS) {
            qWarning() << "animated image too large:" << size;
            wipe();
            return;
        }
    }
    m_buffer.setBuffer(&m_data);
    m_buffer.open(QIODevice::ReadOnly);
    if (!rewind()) {
        wipe();
        return;
    }
    m_loopsLeft = m_reader->loopCount();
    // The first frame shows whether playing or not; playing goes on from there.
    if (!readFrame()) {
        wipe();
        return;
    }
    send();
    schedule();
}

void GifDecoder::setPlaying(bool playing)
{
    m_playing = playing;
    if (playing) {
        schedule();
    } else {
        m_timer.stop();
    }
}

void GifDecoder::setTargetSize(const QSize &size)
{
    if (size == m_target) {
        return;
    }
    m_target = size;
    // A paused or finished GIF has no next frame that would pick this up.
    if (!m_frame.isNull() && !m_cancelled.loadAcquire()) {
        m_framesOut.storeRelease(0);
        send();
    }
}

void GifDecoder::step()
{
    if (m_cancelled.loadAcquire() || !m_reader) {
        return;
    }
    if (!readFrame()) {
        // The end of a pass. A file with one frame (or none readable) has nothing to animate.
        const bool again = m_framesThisPass > 1 && m_loopsLeft != 0;
        if (!again || !rewind() || !readFrame()) {
            m_finished = true;
            return;
        }
        if (m_loopsLeft > 0) {
            --m_loopsLeft;
        }
    }
    send();
    schedule();
}

bool GifDecoder::readFrame()
{
    QImage next;
    if (!m_reader->canRead() || !m_reader->read(&next) || next.isNull()) {
        return false;
    }
    ++m_framesThisPass;
    m_frame = next;
    m_delay = frameDelay(m_reader->nextImageDelay());
    m_sinceFrame.start();
    return true;
}

bool GifDecoder::rewind()
{
    if (!m_buffer.seek(0)) {
        return false;
    }
    m_reader.reset(new QImageReader(&m_buffer));
    PeerImage::pinTo(*m_reader, GIF);
    m_framesThisPass = 0;
    return m_reader->canRead();
}

void GifDecoder::send()
{
    if (!m_framesOut.testAndSetAcquire(0, 1)) {
        return;
    }
    emit frameReady(scaledFrame(m_frame, m_target));
}

void GifDecoder::schedule()
{
    if (!m_playing || m_finished || !m_reader || m_timer.isActive()) {
        return;
    }
    const qint64 elapsed = m_sinceFrame.isValid() ? m_sinceFrame.elapsed() : 0;
    m_timer.start(qMax<qint64>(0, m_delay - elapsed));
}

void GifDecoder::wipe()
{
    m_timer.stop();
    m_reader.reset();
    m_buffer.close();
    m_buffer.setBuffer(nullptr);
    m_frame = QImage();
    m_data.fill('\0');
    m_data.clear();
}
