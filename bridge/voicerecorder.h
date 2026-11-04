// SPDX-License-Identifier: AGPL-3.0-only
#ifndef VOICERECORDER_H
#define VOICERECORDER_H

#include <QAudioInput>
#include <QAudioRecorder>
#include <QBuffer>
#include <QElapsedTimer>
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
// way (the length limit, the platform taking the microphone) waits as `held` while the app is in
// front, and is thrown away otherwise.
class VoiceRecorder : public QObject
{
    Q_OBJECT
    // Recording is possible at all (AAC or the Opus fallback).
    Q_PROPERTY(bool available READ available CONSTANT)
    Q_PROPERTY(bool recording READ recording NOTIFY recordingChanged)
    // Stopped by itself and kept: its owner offers to send or discard it.
    Q_PROPERTY(bool held READ held NOTIFY heldChanged)
    // Who started the running or held recording; null if there is none.
    Q_PROPERTY(QObject *owner READ owner NOTIFY ownerChanged)
    // Milliseconds recorded so far.
    Q_PROPERTY(int duration READ duration NOTIFY durationChanged)

public:
    explicit VoiceRecorder(QObject *parent = nullptr);
    ~VoiceRecorder() override;

    bool available() const { return !m_codec.isEmpty() || m_pcmAvailable; }
    bool recording() const { return m_recording; }
    bool held() const { return m_held; }
    QObject *owner() const { return m_owner; }
    int duration() const;

    Q_INVOKABLE void start(QObject *owner);
    // Stop to send; `finished` follows with the duration, then fetch it with takeRecording during
    // that signal: what isn't taken is gone afterwards, as is anything under a second.
    Q_INVOKABLE void stop();
    // Stop if running and throw the recording away, held or not.
    Q_INVOKABLE void cancel();

    // The finished or held recording (removed from here and from the disk). `pcm`: raw PCM for Opus.
    QByteArray takeRecording(bool *pcm);

signals:
    void recordingChanged();
    void heldChanged();
    void ownerChanged();
    void durationChanged();
    void finished(int duration);
    // Something nobody cancelled is gone, because no one could be asked about it.
    void discarded();
    void failed(const QString &error);

private:
    // Why the recording stops; ByItself: nobody here asked for it.
    enum class Stop { ByItself, Send, Limit, Discard };

    bool usePlatform() const { return !m_codec.isEmpty(); }
    void finish(Stop how);
    void stopped();
    void fail(const QString &error);
    // Removes what was recorded and ends the recording's time with its owner.
    void clear();
    void setHeld(bool held);
    void setOwner(QObject *owner);
    QString outputPath() const;

    QAudioRecorder *m_recorder;
    QString m_codec;
    QString m_container;
    bool m_pcmAvailable = false;
    QAudioInput *m_input = nullptr;
    QByteArray m_pcm;
    QBuffer m_pcmBuffer;
    bool m_recording = false;
    bool m_held = false;
    Stop m_stop = Stop::ByItself;
    QObject *m_owner = nullptr;
    QMetaObject::Connection m_ownerGone;
    bool m_haveRecording = false;
    int m_finalDuration = 0;
    QElapsedTimer m_clock;
    QTimer m_tick;
};

#endif
