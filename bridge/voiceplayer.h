// SPDX-License-Identifier: AGPL-3.0-only
#ifndef VOICEPLAYER_H
#define VOICEPLAYER_H

#include <QAudioOutput>
#include <QBuffer>
#include <QMediaPlayer>
#include <QObject>
#include <QTimer>

class EarpiecePlayer;
class SoftSound;
class ThreeseBackend;

// Plays voice messages (one at a time) from memory: the decrypted audio never touches the disk.
// The platform player handles AAC and Opus; without one (e.g. the SDK emulator) Opus messages
// are decoded by the core and played as PCM.
//
// Routes: the loudspeaker through Qt, the earpiece (phone at the ear) through a GStreamer
// pipeline with the PulseAudio role "phone", like the call engines — the only route that
// reaches the earpiece on Sailfish, and Full-flavour only (EarpiecePlayer).
//
// Only the user starts the sound. Sailfish's resource policy pauses a QMediaPlayer when something
// else takes the audio and resumes it by itself when it comes back; here that stays a pause.
// Nothing lingers either: closing the window ends the playback (main.cpp), the chat drops a paused
// message when it is left, and a message left paused for long goes by itself.
class VoicePlayer : public QObject
{
    Q_OBJECT
    // The message being played or paused (local id), -1 if none.
    Q_PROPERTY(int messageId READ messageId NOTIFY messageIdChanged)
    // A message is loaded or the chime before the next one is sounding: the system's media
    // controls stay up (MediaSession.qml).
    Q_PROPERTY(bool active READ active NOTIFY activeChanged)
    Q_PROPERTY(bool chiming READ chiming NOTIFY activeChanged)
    Q_PROPERTY(bool playing READ playing NOTIFY playingChanged)
    // Milliseconds.
    Q_PROPERTY(int position READ position NOTIFY positionChanged)
    Q_PROPERTY(int duration READ duration NOTIFY durationChanged)
    // Phone-to-the-ear mode: the sound goes to the earpiece at call volume; switching reroutes
    // the running playback in place. Only when earpieceAvailable.
    Q_PROPERTY(bool earpiece READ earpiece WRITE setEarpiece NOTIFY earpieceChanged)
    Q_PROPERTY(bool earpieceAvailable READ earpieceAvailable CONSTANT)
    // Playback speed (Threema Android's 1×, 1.5×, 2×), kept for the next message and the next
    // start of the app.
    Q_PROPERTY(qreal speed READ speed WRITE setSpeed NOTIFY speedChanged)

public:
    VoicePlayer(const ThreeseBackend *backend, QObject *parent = nullptr);

    int messageId() const { return m_messageId; }
    bool active() const { return m_active; }
    bool chiming() const { return m_chimePending; }
    bool playing() const;
    int position() const;
    int duration() const;
    bool earpiece() const { return m_earpiece; }
    void setEarpiece(bool earpiece);
    bool earpieceAvailable() const;
    qreal speed() const { return m_speed; }
    void setSpeed(qreal speed);

    // Play `message` (resume if it is the paused one); false if it can't be played.
    Q_INVOKABLE bool play(int message);
    // Carry on with the paused message.
    Q_INVOKABLE void resume();
    Q_INVOKABLE void pause();
    Q_INVOKABLE void stop();
    Q_INVOKABLE void seek(int position);
    // A soft chime (generated, no asset) through the route in use, as a separator between two
    // voice messages played one after the other; chimeDone() when it has sounded, also when
    // in-app sounds are off and nothing sounded at all.
    Q_INVOKABLE void chime();

signals:
    void messageIdChanged();
    void activeChanged();
    void playingChanged();
    void positionChanged();
    void durationChanged();
    // Somebody moved the position (not the playback going on); milliseconds.
    void seeked(int position);
    void earpieceChanged();
    void speedChanged();
    // The message played to its natural end (not stopped by hand): the chat may queue the
    // next voice message.
    void finished(int message);
    void chimeDone();
    void failed(const QString &error);

private:
    void release();
    void releaseRoute();
    bool startRoute(int startMs, bool autoplay);
    void startPcmOutput(qint64 startByte, bool autoplay);
    void onEarpieceFinished();
    void fallBackToLoudspeaker();
    void onChimeEnded();
    void updateActive();

    const ThreeseBackend *m_backend;
    QMediaPlayer *m_player;
    QBuffer *m_buffer = nullptr;
    QByteArray m_data;
    int m_messageId = -1;
    QAudioOutput *m_output = nullptr;
    // The loaded message is PCM decoded by the core, not the data as it was received.
    bool m_pcm = false;
    bool m_earpiece = false;
    bool m_atEnd = false;
    // What the user asked of QMediaPlayer, against its own resuming.
    bool m_wantPlaying = false;
    bool m_active = false;
    // Between two messages of a queue, or moving to another one: no gap for the media controls.
    bool m_holdActive = false;
    bool m_chimePending = false;
    QTimer m_pausedTooLong;
    qreal m_speed = 1.0;
    EarpiecePlayer *m_earpiecePlayer = nullptr;
    // The chime: through the loudspeaker, or through the earpiece's pipeline while at the ear.
    SoftSound *m_chime;
    EarpiecePlayer *m_chimePlayer = nullptr;
    QByteArray m_chimeAtEar;
};

#endif
