// SPDX-License-Identifier: AGPL-3.0-only
#ifndef VOICERECORDER_H
#define VOICERECORDER_H

#include <QAudioInput>
#include <QAudioOutput>
#include <QAudioRecorder>
#include <QBuffer>
#include <QElapsedTimer>
#include <QMediaPlayer>
#include <QObject>
#include <QTimer>

// Records voice messages. Preferably with the platform's encoder (on phones AAC through
// gst-droid), like Threema Android: AAC in MP4, mono, 32 kbit/s. Where there is none (e.g. the
// SDK emulator), raw 16-bit mono PCM at 48 kHz is recorded into memory and the core encodes it as
// Opus (royalty-free). An AAC recording lives in the app's private cache only until it has been
// handed to the core (which stores it encrypted).
//
// A recording belongs to whoever started it (the chat page): only that one sends or discards it,
// and it goes when its owner goes. Only the owner's tap sends: a recording that stopped any other
// way (the length limit, the platform taking the microphone) waits as `held`, a plain file like a
// running one, until the owner's tap, the app lock or the window's closing. Losing focus (a system
// dialog too) stops a running one and holds it; leaving the app (appLeft: its cover shows, or the
// display goes off) throws away a running or held recording at once (`discarded`): no plain file
// outlives the chat page.
//
// A running recording can pause and go on. A held one can be listened to before it goes: the
// platform's MP4 is only playable once it is finished, so listening means stopping, and what was
// held can't go on being recorded. Qt says "paused" at once whether the pipeline did or not, so the
// recorder's own time tells: if it goes on (gst-droid is only known from the phone), or there is
// none to go by, the recording stops and is held, which keeps what was said.
//
// On the platform's path the file is complete only after "stopped": Qt reports that at once, while
// the file is still being finished. Nothing happens to the recording before its status has come to
// rest; if it never does, the recording is held and never goes out by itself.
class VoiceRecorder : public QObject
{
    Q_OBJECT
    // Recording is possible at all (AAC or the Opus fallback).
    Q_PROPERTY(bool available READ available CONSTANT)
    Q_PROPERTY(bool recording READ recording NOTIFY recordingChanged)
    // Running, but not taking anything in at the moment.
    Q_PROPERTY(bool paused READ paused NOTIFY pausedChanged)
    // Stopped and kept: its owner offers to send or discard it.
    Q_PROPERTY(bool held READ held NOTIFY heldChanged)
    // The held recording is sounding.
    Q_PROPERTY(bool previewing READ previewing NOTIFY previewingChanged)
    // Milliseconds into the held recording.
    Q_PROPERTY(int previewPosition READ previewPosition NOTIFY previewPositionChanged)
    // Who started the running or held recording; null if there is none.
    Q_PROPERTY(QObject *owner READ owner NOTIFY ownerChanged)
    // Milliseconds recorded so far.
    Q_PROPERTY(int duration READ duration NOTIFY durationChanged)

public:
    explicit VoiceRecorder(QObject *parent = nullptr);
    ~VoiceRecorder() override;

    bool available() const { return !m_codec.isEmpty() || m_pcmAvailable; }
    bool recording() const { return m_recording; }
    bool paused() const { return m_paused; }
    bool held() const { return m_held; }
    bool previewing() const { return m_previewing; }
    int previewPosition() const;
    QObject *owner() const { return m_owner; }
    int duration() const;

    Q_INVOKABLE void start(QObject *owner);
    // Stop to send; `finished` follows with the duration, then fetch it with takeRecording during
    // that signal: what isn't taken is gone afterwards, as is anything under a second.
    Q_INVOKABLE void stop();
    // Stop if running and throw the recording away, held or not.
    Q_INVOKABLE void cancel();
    Q_INVOKABLE void pause();
    Q_INVOKABLE void resume();
    // Stop and keep it, for listening. Under a second there is nothing to keep.
    Q_INVOKABLE void hold();
    // The app was left, not only its focus lost: what runs or is held goes; a tap's send goes on.
    Q_INVOKABLE void appLeft();
    // Listen to the held recording, from where it stands; at its end it goes back to the start.
    Q_INVOKABLE void startPreview();
    Q_INVOKABLE void pausePreview();
    Q_INVOKABLE void seekPreview(int position);

    // The finished or held recording (removed from here and from the disk). `pcm`: raw PCM for Opus.
    QByteArray takeRecording(bool *pcm);

signals:
    void recordingChanged();
    void pausedChanged();
    void heldChanged();
    void previewingChanged();
    void previewPositionChanged();
    void ownerChanged();
    void durationChanged();
    void finished(int duration);
    // Something nobody cancelled is gone: the app was left, or the platform left nothing usable to
    // hold.
    void discarded();
    void failed(const QString &error);
    // The recording can't be listened to; it is still there to send.
    void previewFailed();

private:
    friend class VoiceRecorderTest;

    // Why the recording stops; ByItself: nobody here asked for it. Hold: to wait for an answer.
    enum class Stop { ByItself, Send, Hold, Discard };

    bool usePlatform() const { return !m_codec.isEmpty(); }
    void finish(Stop how);
    void stopped();
    // The platform says it stopped, with its status at that moment; then each later status.
    void recorderStopped(QMediaRecorder::Status status);
    void recorderStatus(QMediaRecorder::Status status);
    // The platform recorder's time, half a second apart after a pause.
    void pauseSample(qint64 position);
    void fail(const QString &error);
    // Removes what was recorded and ends the recording's time with its owner.
    void clear();
    void setPaused(bool paused);
    void setHeld(bool held);
    void setPreviewing(bool previewing);
    // Nothing sounds and nothing holds on to what was recorded.
    void endPreview();
    void previewEnded();
    int recordedMs() const;
    void setOwner(QObject *owner);
    QString outputPath() const;

    QAudioRecorder *m_recorder;
    int m_maxDuration;
    QString m_codec;
    QString m_container;
    bool m_pcmAvailable = false;
    QAudioInput *m_input = nullptr;
    QByteArray m_pcm;
    QBuffer m_pcmBuffer;
    bool m_recording = false;
    bool m_paused = false;
    // What was recorded before the running stretch.
    qint64 m_recordedBefore = 0;
    QTimer m_pauseWait;
    qint64 m_pausePosition = -1;
    QTimer m_finalizeWait;
    bool m_held = false;
    // Left (appLeft) and not in front again since: nothing is held for an answer.
    bool m_left = false;
    bool m_previewing = false;
    // Only for the platform's file: the core's fallback has PCM in memory, for the output.
    QMediaPlayer *m_previewPlayer = nullptr;
    QAudioOutput *m_previewOutput = nullptr;
    QBuffer m_previewBuffer;
    qint64 m_previewFrom = 0;
    Stop m_stop = Stop::ByItself;
    QObject *m_owner = nullptr;
    QMetaObject::Connection m_ownerGone;
    bool m_haveRecording = false;
    int m_finalDuration = 0;
    QElapsedTimer m_clock;
    QTimer m_tick;
};

#endif
