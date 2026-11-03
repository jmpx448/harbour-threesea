// SPDX-License-Identifier: AGPL-3.0-only
#ifndef EARPIECEPLAYER_H
#define EARPIECEPLAYER_H

#include <QByteArray>
#include <QObject>
#include <QTimer>

typedef struct _GstElement GstElement;
typedef struct _GstBus GstBus;
typedef struct _GstMessage GstMessage;

// Plays a voice message through the earpiece, the way the call engines reach it: a GStreamer
// pipeline into PulseAudio with the media role "phone", which Sailfish's audio policy routes
// to the earpiece at call volume. Qt's player has no such route — with QMediaPlayer's
// voice-communication role the phone played scrambled silence (user report 2026-10-29), while
// the calls' pipeline is known to work on the same device.
//
// Full flavour only (Lite must not link GStreamer, and its sandbox has no Sensors permission,
// so nothing there puts the phone to the ear). The data stays in memory; the pipeline reads it
// through an appsrc, so the decrypted audio never reaches the disk.
class EarpiecePlayer : public QObject
{
    Q_OBJECT

public:
    explicit EarpiecePlayer(QObject *parent = nullptr);
    ~EarpiecePlayer() override;

    // Play `data` — the voice message as received, or 48 kHz mono S16 PCM when `raw` (the
    // core's Opus decoder, used where the platform has no player) — from `startMs` on, playing
    // or paused, at `speed`. False if the pipeline cannot be built.
    bool start(const QByteArray &data, bool raw, int startMs, bool playing, double speed);
    void pause();
    void resume();
    void stop();
    void seek(int ms);
    void setSpeed(double speed);

    bool playing() const { return m_playing; }
    // Milliseconds; duration is -1 while the pipeline does not know it yet.
    int position() const;
    int duration() const;

signals:
    void positionChanged();
    void playingChanged();
    // The end of the message (not a stop by hand).
    void finished();
    void failed(const QString &error);

private:
    static int onBusMessage(GstBus *bus, GstMessage *message, void *self);
    static void onNeedData(GstElement *source, unsigned length, void *self);
    static int onSeekData(GstElement *source, quint64 offset, void *self);

    void prerolled();
    void setPlaying(bool playing);
    bool sendSeek(int ms);
    void release();

    // The pipeline must reach PAUSED for playback to start; if it never does (no audio
    // route for the "phone" role, a codec the system does not have), this gives up instead
    // of leaving the message standing still.
    QTimer m_watchdog;
    GstElement *m_pipeline = nullptr;
    GstElement *m_source = nullptr;
    unsigned m_busWatch = 0;
    QTimer m_ticker;
    QByteArray m_data;
    // The appsrc's read position, touched only by its callbacks (one streaming thread).
    qint64 m_offset = 0;
    bool m_raw = false;
    bool m_playing = false;
    bool m_prerolled = false;
    bool m_autoplay = false;
    // Added to the pipeline's own position: in raw mode the data is pushed from this point on
    // (no parser that could turn a byte offset into a time), so the pipeline starts at zero.
    int m_baseMs = 0;
    int m_seekMs = 0;
    double m_speed = 1.0;
};

#endif
