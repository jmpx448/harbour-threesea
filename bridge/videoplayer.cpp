// SPDX-License-Identifier: AGPL-3.0-only
#include "videoplayer.h"

#include <QDebug>
#include <QTimer>

#include "mediareader.h"
#include "threesebackend.h"

VideoPlayer::VideoPlayer(const ThreeseBackend *backend, QObject *parent)
    : QObject(parent)
    , m_backend(backend)
    , m_player(new QMediaPlayer(this))
    , m_reader(new MediaReader(backend->mediaReads(), this))
{
    m_player->setNotifyInterval(250);
    connect(m_player, &QMediaPlayer::stateChanged, this, [this](QMediaPlayer::State state) {
        if (state != QMediaPlayer::PlayingState) {
            m_wantPlaying = false;
        } else if (!m_wantPlaying) {
            QMetaObject::invokeMethod(m_player, "pause", Qt::QueuedConnection);
        }
        emit playingChanged();
    });
    connect(m_player, &QMediaPlayer::mediaStatusChanged, this, [this](QMediaPlayer::MediaStatus status) {
        if (status == QMediaPlayer::EndOfMedia) {
            // Not from inside the player's own notification.
            m_reloadDue = true;
            QTimer::singleShot(0, this, [this]() { reloadAfterEnd(); });
        }
    });
    connect(m_player, &QMediaPlayer::positionChanged, this, &VideoPlayer::positionChanged);
    connect(m_player, &QMediaPlayer::durationChanged, this, &VideoPlayer::durationChanged);
    connect(m_player, static_cast<void (QMediaPlayer::*)(QMediaPlayer::Error)>(&QMediaPlayer::error), this,
            [this](QMediaPlayer::Error) {
                qWarning() << "video playback failed:" << m_player->errorString();
                emit failed(m_player->errorString());
            });
}

bool VideoPlayer::available() const
{
    return m_player->availability() == QMultimedia::Available;
}

bool VideoPlayer::playing() const
{
    return m_player->state() == QMediaPlayer::PlayingState;
}

int VideoPlayer::position() const
{
    return static_cast<int>(m_player->position());
}

int VideoPlayer::duration() const
{
    return static_cast<int>(m_player->duration());
}

bool VideoPlayer::play(int message)
{
    stop();
    if (!available()) {
        return false;
    }
    m_pausedWhileLoading = false;
    setLoading(true);
    const ThreeseBackend *backend = m_backend;
    m_reader->start([backend, message]() { return backend->readMedia(message, false); },
                    [this, message](QByteArray &bytes) { onRead(message, bytes); });
    return true;
}

void VideoPlayer::onRead(int message, QByteArray &bytes)
{
    setLoading(false);
    if (bytes.isEmpty()) {
        qWarning() << "video playback: nothing to read";
        emit failed(QString());
        return;
    }
    m_data.swap(bytes);
    load();
    m_messageId = message;
    emit messageIdChanged();
    if (m_pausedWhileLoading) {
        m_player->pause();
    } else {
        resume();
    }
}

void VideoPlayer::setLoading(bool loading)
{
    if (m_loading != loading) {
        m_loading = loading;
        emit loadingChanged();
    }
}

// A fresh stream over the same bytes. The one it replaces goes only after the player let go of it.
void VideoPlayer::load()
{
    QBuffer *previous = m_buffer;
    m_buffer = new QBuffer(&m_data, this);
    m_buffer->open(QIODevice::ReadOnly);
    m_player->setMedia(QMediaContent(), m_buffer);
    if (previous) {
        previous->deleteLater();
    }
}

void VideoPlayer::reloadAfterEnd()
{
    if (!m_reloadDue) {
        return;
    }
    m_reloadDue = false;
    load();
    // Paused at the start, the first frame instead of black, and the play button works again.
    m_player->pause();
}

void VideoPlayer::pause()
{
    if (m_loading) {
        m_pausedWhileLoading = true;
        return;
    }
    m_wantPlaying = false;
    m_player->pause();
}

void VideoPlayer::resume()
{
    if (m_messageId == -1) {
        return;
    }
    reloadAfterEnd();
    m_wantPlaying = true;
    m_player->play();
}

void VideoPlayer::seek(int position)
{
    reloadAfterEnd();
    m_player->setPosition(position);
    emit seeked(position);
}

void VideoPlayer::stop()
{
    m_reader->cancel();
    setLoading(false);
    m_reloadDue = false;
    m_wantPlaying = false;
    m_player->stop();
    m_player->setMedia(QMediaContent());
    if (m_buffer) {
        m_buffer->deleteLater();
        m_buffer = nullptr;
    }
    // Drop the decrypted video right away.
    m_data.fill('\0');
    m_data.clear();
    if (m_messageId != -1) {
        m_messageId = -1;
        emit messageIdChanged();
    }
}
