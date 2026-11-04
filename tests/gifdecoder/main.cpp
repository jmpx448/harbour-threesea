// SPDX-License-Identifier: AGPL-3.0-only
// GifDecoder: what used to hold up the interface happens on its own thread, GIFs play and stop
// as they say, and a file that claims too much is refused. Exit 0: passed.
#include "gifdecoder.h"
#include "peerimage.h"

#include <QAtomicInt>
#include <QBuffer>
#include <QCoreApplication>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QImage>
#include <QThread>
#include <QTimer>
#include <cstdio>

namespace {

int failures = 0;

void check(bool ok, const char *what)
{
    if (!ok) {
        std::printf("FAILED: %s\n", what);
        ++failures;
    }
}

void little(QByteArray &out, int value)
{
    out.append(char(value & 0xff));
    out.append(char((value >> 8) & 0xff));
}

// A GIF with literal 8-bit LZW codes: a clear code often enough keeps the table from growing.
// `loop`: Netscape's "forever" extension; without it a GIF plays once.
QByteArray gif(int width, int height, int frames, bool loop, int delayCs = 2)
{
    QByteArray out("GIF89a");
    little(out, width);
    little(out, height);
    out.append(char(0xf6)).append(char(0)).append(char(0));
    for (int i = 0; i < 128; ++i) {
        out.append(char(i * 2)).append(char(i * 2)).append(char(i * 2));
    }
    if (loop) {
        out.append("\x21\xff\x0bNETSCAPE2.0\x03\x01", 16);
        out.append(char(0)).append(char(0)).append(char(0));
    }
    for (int frame = 0; frame < frames; ++frame) {
        out.append("\x21\xf9\x04", 3).append(char(0));
        little(out, delayCs);
        out.append(char(0)).append(char(0)).append(char(0x2c));
        little(out, 0);
        little(out, 0);
        little(out, width);
        little(out, height);
        out.append(char(0)).append(char(7));
        QByteArray codes;
        for (int i = 0; i < width * height; ++i) {
            if (i % 100 == 0) {
                codes.append(char(128));
            }
            codes.append(char((i + frame * 10) % 128));
        }
        codes.append(char(129));
        for (int at = 0; at < codes.size(); at += 255) {
            const QByteArray block = codes.mid(at, 255);
            out.append(char(block.size())).append(block);
        }
        out.append(char(0));
    }
    out.append(char(0x3b));
    return out;
}

// Runs a decoder on the shared thread and counts what arrives on this one.
struct Run
{
    explicit Run(const QByteArray &data, bool takeFrames = true)
        : take(takeFrames)
    {
        const QByteArray file = data;
        QAtomicInt *reads = &readCount;
        QThread **thread = &readThread;
        decoder = new GifDecoder([file, reads, thread]() {
            reads->ref();
            *thread = QThread::currentThread();
            return file;
        });
        decoder->moveToThread(GifDecoder::decoderThread());
        QObject::connect(decoder, &GifDecoder::frameReady, &context, [this](const QImage &image) {
            ++frames;
            last = image;
            if (take) {
                decoder->frameTaken();
            }
        }, Qt::QueuedConnection);
    }
    ~Run()
    {
        decoder->cancel();
        decoder->deleteLater();
    }
    void load() { QMetaObject::invokeMethod(decoder, "load", Qt::QueuedConnection); }
    void play(bool on) { QMetaObject::invokeMethod(decoder, "setPlaying", Qt::QueuedConnection, Q_ARG(bool, on)); }
    void target(const QSize &size)
    {
        QMetaObject::invokeMethod(decoder, "setTargetSize", Qt::QueuedConnection, Q_ARG(QSize, size));
    }

    QObject context;
    GifDecoder *decoder = nullptr;
    bool take = true;
    int frames = 0;
    QImage last;
    QAtomicInt readCount;
    QThread *readThread = nullptr;
};

void wait(int ms)
{
    QEventLoop loop;
    QTimer::singleShot(ms, &loop, &QEventLoop::quit);
    loop.exec();
}

void scaling()
{
    QImage image(400, 200, QImage::Format_ARGB32);
    image.fill(Qt::red);
    check(GifDecoder::scaledFrame(image, QSize(100, 100)).size() == QSize(200, 100), "covers the target, cropped later");
    check(GifDecoder::scaledFrame(image, QSize(800, 800)).size() == QSize(400, 200), "never scaled up");
    check(GifDecoder::scaledFrame(image, QSize()).size() == QSize(400, 200), "no target: full size");
    check(GifDecoder::scaledFrame(image, QSize(100, 300)).size() == QSize(400, 200),
          "a long frame keeps its proportions rather than being stretched");
    check(GifDecoder::scaledFrame(image, QSize(100, 100)).format() == QImage::Format_ARGB32_Premultiplied,
          "premultiplied, also when scaled");
    check(GifDecoder::scaledFrame(image, QSize()).format() == QImage::Format_ARGB32_Premultiplied,
          "premultiplied, also at full size");
}

void delays()
{
    check(GifDecoder::frameDelay(0) == 100, "no delay plays at 10 frames a second");
    check(GifDecoder::frameDelay(10) == 100, "a delay nobody means likewise");
    check(GifDecoder::frameDelay(40) == 40, "a real delay is kept");
}

void firstFrameWithoutPlaying()
{
    Run run(gif(16, 16, 5, true));
    run.load();
    wait(300);
    check(run.frames == 1, "the first frame comes without playing, and only that");
    check(run.last.size() == QSize(16, 16), "at its size");
    check(run.readThread && run.readThread != QThread::currentThread(), "read off the caller's thread");
}

void looping()
{
    Run run(gif(16, 16, 3, true));
    run.load();
    run.play(true);
    wait(600);
    check(run.frames > 3, "forever goes on past the last frame");
    const int playing = run.frames;
    run.play(false);
    wait(100);
    const int paused = run.frames;
    wait(200);
    check(playing > 0 && run.frames == paused, "paused means no more frames");
}

void playsOnce()
{
    Run run(gif(16, 16, 3, false));
    run.load();
    run.play(true);
    wait(600);
    check(run.frames == 3, "without the loop extension: once");
}

void singleFrame()
{
    Run run(gif(16, 16, 1, true));
    run.load();
    run.play(true);
    wait(400);
    check(run.frames == 1, "one frame has nothing to animate, whatever the file says");
}

void refused()
{
    // Really that large: a valid file only the limit can be what stops (a logical screen far
    // larger than its frames proves nothing, Qt's reader disbelieves those by itself).
    Run tooLarge(gif(5000, 5000, 1, false));
    tooLarge.load();
    Run garbage(QByteArray(1000, 'x'));
    garbage.load();
    Run empty{QByteArray()};
    empty.load();
    wait(1500);
    check(tooLarge.frames == 0, "a frame larger than a phone should hold is refused");
    check(garbage.frames == 0, "not an image");
    check(empty.frames == 0, "nothing to read");
}

// A picture GifDecoder isn't for: PNG's reader is built into Qt, JPEG's a plugin like GIF's.
QByteArray encoded(const char *format)
{
    QImage image(2, 2, QImage::Format_RGB32);
    image.fill(Qt::red);
    QByteArray out;
    QBuffer buffer(&out);
    buffer.open(QIODevice::WriteOnly);
    image.save(&buffer, format);
    return out;
}

void onlyTheAcceptedFormats()
{
    const QByteArray pbm("P1\n2 2\n1 0\n0 1\n");
    {
        // Without the pin Qt finds a reader for it (the bytes are valid): the test means something.
        QByteArray bytes = pbm;
        QBuffer buffer(&bytes);
        buffer.open(QIODevice::ReadOnly);
        check(!QImageReader(&buffer).read().isNull(), "Qt reads a PBM when left to itself");
    }
    check(PeerImage::format(pbm).isEmpty(), "PBM isn't accepted");
    check(PeerImage::format(QByteArray("II*\0", 4)).isEmpty(), "TIFF isn't accepted");
    check(PeerImage::format("<svg xmlns='http://www.w3.org/2000/svg'/>").isEmpty(), "SVG isn't accepted");
    check(PeerImage::format(QByteArray("RIFF\0\0\0\0WAVE", 12)).isEmpty(), "a WAV isn't a WebP");
    check(PeerImage::format("\xff\xd8\xff\xe0") == "jpeg", "JPEG");
    check(PeerImage::format(encoded("PNG")) == "png", "PNG");
    check(PeerImage::format(encoded("JPEG")) == "jpeg", "a JPEG as Qt writes it");
    check(PeerImage::format(gif(1, 1, 1, false)) == "gif", "GIF");
    check(PeerImage::format(QByteArray("RIFF\0\0\0\0WEBPVP8 ", 16)) == "webp", "WebP");

    // Pinned, a reader takes its own format and no other.
    for (const char *format : {"PNG", "JPEG"}) {
        QByteArray bytes = encoded(format);
        QBuffer buffer(&bytes);
        buffer.open(QIODevice::ReadOnly);
        QImageReader reader(&buffer);
        PeerImage::pinTo(reader, PeerImage::format(bytes));
        check(reader.size() == QSize(2, 2) && !reader.read().isNull(), "reads in its own format");
    }
    {
        QByteArray bytes = pbm;
        QBuffer buffer(&bytes);
        buffer.open(QIODevice::ReadOnly);
        QImageReader reader(&buffer);
        PeerImage::pinTo(reader, "png");
        check(reader.read().isNull(), "a pinned reader asks no other format");
    }

    Run picture(encoded("PNG"));
    picture.load();
    wait(500);
    check(picture.frames == 0, "the GIF decoder takes GIFs only");
}

void cancelledBeforeLoading()
{
    Run run(gif(16, 16, 2, true));
    run.decoder->cancel();
    run.load();
    wait(100);
    check(run.readCount.load() == 0, "a tile scrolled past costs no decryption");
    check(run.frames == 0, "and sends nothing");
}

void unconsumedFramesDontPileUp()
{
    Run run(gif(16, 16, 3, true), false);
    run.load();
    run.play(true);
    wait(400);
    check(run.frames == 1, "nothing more until the last frame was taken");
    run.decoder->frameTaken();
    wait(200);
    check(run.frames == 2, "then the next, and no queue of them");
}

void targetChangesWhilePaused()
{
    Run run(gif(64, 64, 2, false));
    run.load();
    wait(200);
    check(run.last.size() == QSize(64, 64), "full size before the size is known");
    run.target(QSize(16, 16));
    wait(200);
    check(run.last.size() == QSize(16, 16), "a paused GIF follows its item's size");
}

}

int main(int argc, char **argv)
{
    QCoreApplication app(argc, argv);
    scaling();
    delays();
    firstFrameWithoutPlaying();
    looping();
    playsOnce();
    singleFrame();
    refused();
    onlyTheAcceptedFormats();
    cancelledBeforeLoading();
    unconsumedFramesDontPileUp();
    targetChangesWhilePaused();
    std::printf(failures ? "%d FAILED\n" : "gifdecoder: all passed\n", failures);
    return failures ? 1 : 0;
}
