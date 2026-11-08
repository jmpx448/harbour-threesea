// SPDX-License-Identifier: AGPL-3.0-only
#ifndef VIDEOPLAYER_H
#define VIDEOPLAYER_H

#include <QBuffer>
#include <QByteArray>
#include <QMediaPlayer>
#include <QObject>

class MediaReader;
class ThreeseBackend;

// Plays a video message in the app: decrypted by the core into memory and streamed from there
// into QtMultimedia, so no plaintext copy is written (like VoicePlayer). QML shows it with a
// VideoOutput whose source is this object: VideoOutput finds the player through a property
// called `mediaObject` and shows nothing at all for a source without one, a bare QMediaPlayer
// included (no warning either, the sound just plays on over black).
class VideoPlayer : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QObject *mediaObject READ mediaObject CONSTANT)
    Q_PROPERTY(bool available READ available CONSTANT)
    // It is being decrypted (a large file takes seconds); then it starts, or failed() says why not.
    Q_PROPERTY(bool loading READ loading NOTIFY loadingChanged)
    // The message being played or paused (local id), -1 if none.
    Q_PROPERTY(int messageId READ messageId NOTIFY messageIdChanged)
    Q_PROPERTY(bool playing READ playing NOTIFY playingChanged)
    Q_PROPERTY(int position READ position NOTIFY positionChanged)
    Q_PROPERTY(int duration READ duration NOTIFY durationChanged)

public:
    VideoPlayer(const ThreeseBackend *backend, QObject *parent = nullptr);

    QObject *mediaObject() const { return m_player; }
    int messageId() const { return m_messageId; }
    // A media service (GStreamer) exists; otherwise videos are opened in another app.
    bool available() const;
    bool loading() const { return m_loading; }
    bool playing() const;
    int position() const;
    int duration() const;

    // Starts decrypting `message`, and playing when that is done; false if there's no way to play.
    Q_INVOKABLE bool play(int message);
    Q_INVOKABLE void pause();
    Q_INVOKABLE void resume();
    Q_INVOKABLE void stop();
    Q_INVOKABLE void seek(int position);

signals:
    void messageIdChanged();
    void loadingChanged();
    void playingChanged();
    void positionChanged();
    void durationChanged();
    // Somebody moved the position (not the playback going on); milliseconds.
    void seeked(int position);
    void failed(const QString &error);

private:
    void load();
    void reloadAfterEnd();
    void onRead(int message, QByteArray &bytes);
    void setLoading(bool loading);

    const ThreeseBackend *m_backend;
    QMediaPlayer *m_player;
    MediaReader *m_reader;
    bool m_loading = false;
    bool m_pausedWhileLoading = false;
    QByteArray m_data;
    QBuffer *m_buffer = nullptr;
    int m_messageId = -1;
    // Only the user starts the playback (VoicePlayer has the same, and says why).
    bool m_wantPlaying = false;
    // It played to its end, and a stream can't be played or moved in after that: it is loaded
    // again at once, or when somebody wants something from it first.
    bool m_reloadDue = false;
};

#endif // VIDEOPLAYER_H
