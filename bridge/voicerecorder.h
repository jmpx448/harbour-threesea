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
class VoiceRecorder : public QObject
{
    Q_OBJECT
    // Recording is possible at all (AAC or the Opus fallback).
    Q_PROPERTY(bool available READ available CONSTANT)
    Q_PROPERTY(bool recording READ recording NOTIFY recordingChanged)
    // Milliseconds recorded so far.
    Q_PROPERTY(int duration READ duration NOTIFY durationChanged)

public:
    explicit VoiceRecorder(QObject *parent = nullptr);
    ~VoiceRecorder() override;

    bool available() const { return !m_codec.isEmpty() || m_pcmAvailable; }
    bool recording() const { return m_recording; }
    int duration() const;

    Q_INVOKABLE void start();
    // Stop; `finished` follows with the duration, then fetch it with takeRecording.
    Q_INVOKABLE void stop();
    // Stop and throw the recording away.
    Q_INVOKABLE void cancel();

    // The finished recording (removed from here and from the disk). `pcm`: raw PCM for Opus.
    QByteArray takeRecording(bool *pcm);

signals:
    void recordingChanged();
    void durationChanged();
    void finished(int duration);
    void failed(const QString &error);

private:
    bool usePlatform() const { return !m_codec.isEmpty(); }
    void finish(bool keep);
    void stopped();
    void fail(const QString &error);
    QString outputPath() const;

    QAudioRecorder *m_recorder;
    QString m_codec;
    QString m_container;
    bool m_pcmAvailable = false;
    QAudioInput *m_input = nullptr;
    QByteArray m_pcm;
    QBuffer m_pcmBuffer;
    bool m_recording = false;
    bool m_keep = false;
    bool m_haveRecording = false;
    int m_finalDuration = 0;
    QElapsedTimer m_clock;
    QTimer m_tick;
};

#endif
