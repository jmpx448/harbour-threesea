// SPDX-License-Identifier: AGPL-3.0-only
#include "voiceplayer.h"

#include <QDebug>
#include <QSettings>

#include <cmath>

#include "softsound.h"
#include "threesebackend.h"
#ifdef THREESEA_FULL
#include "earpieceplayer.h"
#endif

namespace {

const int BytesPerMs = 48000 * 2 / 1000;

// A paused message is dropped after this long: its decrypted audio stays in memory and the
// system's media controls stay up until then.
const int PausedTooLongMs = 10 * 60 * 1000;

QAudioFormat pcmFormat(qreal speed)
{
    QAudioFormat format;
    // A higher rate plays faster (and higher); the platform route stretches with scaletempo
    // where the system has it.
    format.setSampleRate(static_cast<int>(qRound(48000 * (speed > 0 ? speed : 1.0))));
    format.setChannelCount(1);
    format.setSampleSize(16);
    format.setSampleType(QAudioFormat::SignedInt);
    format.setByteOrder(QAudioFormat::LittleEndian);
    format.setCodec(QStringLiteral("audio/pcm"));
    return format;
}

// The speeds the chat offers (Threema Android's).
qreal clampSpeed(qreal speed)
{
    if (speed >= 2.0) {
        return 2.0;
    }
    if (speed >= 1.5) {
        return 1.5;
    }
    return 1.0;
}

} // namespace

VoicePlayer::VoicePlayer(const ThreeseBackend *backend, QObject *parent)
    : QObject(parent)
    , m_backend(backend)
    , m_player(new QMediaPlayer(this))
    , m_chime(new SoftSound(this))
{
    connect(m_chime, &SoftSound::done, this, &VoicePlayer::onChimeEnded);
    m_pausedTooLong.setSingleShot(true);
    m_pausedTooLong.setInterval(PausedTooLongMs);
    connect(&m_pausedTooLong, &QTimer::timeout, this, &VoicePlayer::stop);
    connect(this, &VoicePlayer::playingChanged, this, [this]() {
        if (m_messageId == -1 || playing()) {
            m_pausedTooLong.stop();
        } else {
            m_pausedTooLong.start();
        }
    });
    m_speed = clampSpeed(QSettings().value(QStringLiteral("voiceSpeed"), 1.0).toReal());
    m_player->setNotifyInterval(100);
    connect(m_player, &QMediaPlayer::stateChanged, this, [this](QMediaPlayer::State state) {
        if (state != QMediaPlayer::PlayingState) {
            m_wantPlaying = false;
        } else if (!m_wantPlaying) {
            QMetaObject::invokeMethod(m_player, "pause", Qt::QueuedConnection);
        }
        emit playingChanged();
    });
    connect(m_player, &QMediaPlayer::positionChanged, this, &VoicePlayer::positionChanged);
    connect(m_player, &QMediaPlayer::durationChanged, this, &VoicePlayer::durationChanged);
    connect(m_player, &QMediaPlayer::mediaStatusChanged, this, [this](QMediaPlayer::MediaStatus status) {
        if (status == QMediaPlayer::EndOfMedia) {
            m_atEnd = true;
            stop();
        } else if (status == QMediaPlayer::LoadedMedia || status == QMediaPlayer::BufferedMedia) {
            // Some backends forget the rate while nothing is loaded.
            m_player->setPlaybackRate(m_speed);
        }
    });
    connect(m_player, static_cast<void (QMediaPlayer::*)(QMediaPlayer::Error)>(&QMediaPlayer::error), this,
            [this](QMediaPlayer::Error) {
                qWarning() << "voice playback failed:" << m_player->errorString();
                emit failed(m_player->errorString());
                stop();
            });
#ifdef THREESEA_FULL
    m_earpiecePlayer = new EarpiecePlayer(this);
    connect(m_earpiecePlayer, &EarpiecePlayer::positionChanged, this, &VoicePlayer::positionChanged);
    connect(m_earpiecePlayer, &EarpiecePlayer::playingChanged, this, &VoicePlayer::playingChanged);
    connect(m_earpiecePlayer, &EarpiecePlayer::finished, this, &VoicePlayer::onEarpieceFinished);
    connect(m_earpiecePlayer, &EarpiecePlayer::failed, this, [this](const QString &error) {
        // The earpiece did not take the sound (no route for the "phone" role, a codec the
        // system lacks): the message keeps playing on the loudspeaker where it stood, rather
        // than standing still at the user's ear.
        if (m_earpiece && m_messageId != -1 && !m_data.isEmpty()) {
            qWarning() << "voice at the ear failed, playing on the loudspeaker:" << error;
            fallBackToLoudspeaker();
            return;
        }
        emit failed(error);
        stop();
    });
#endif
}

bool VoicePlayer::earpieceAvailable() const
{
    return m_earpiecePlayer != nullptr;
}

bool VoicePlayer::playing() const
{
#ifdef THREESEA_FULL
    if (m_earpiece && m_earpiecePlayer && m_messageId != -1) {
        return m_earpiecePlayer->playing();
    }
#endif
    if (m_output) {
        return m_output->state() == QAudio::ActiveState;
    }
    return m_player->state() == QMediaPlayer::PlayingState;
}

int VoicePlayer::position() const
{
#ifdef THREESEA_FULL
    if (m_earpiece && m_earpiecePlayer && m_messageId != -1) {
        return m_earpiecePlayer->position();
    }
#endif
    if (m_output && m_buffer) {
        return static_cast<int>(m_buffer->pos() / BytesPerMs);
    }
    return static_cast<int>(m_player->position());
}

int VoicePlayer::duration() const
{
#ifdef THREESEA_FULL
    if (m_earpiece && m_earpiecePlayer && m_messageId != -1) {
        const int known = m_earpiecePlayer->duration();
        if (known > 0) {
            return known;
        }
    }
#endif
    if (m_pcm) {
        return m_data.size() / BytesPerMs;
    }
    return static_cast<int>(m_player->duration());
}

bool VoicePlayer::play(int message)
{
    if (message == m_messageId && !m_data.isEmpty()) {
#ifdef THREESEA_FULL
        if (m_earpiece && m_earpiecePlayer) {
            m_earpiecePlayer->resume();
            return true;
        }
#endif
        if (m_output) {
            m_output->resume();
            emit playingChanged();
        } else {
            m_wantPlaying = true;
            m_player->play();
        }
        return true;
    }
    m_holdActive = true;
    stop();
    m_holdActive = false;
    // Without a platform player (the SDK emulator) the core decodes Opus for us.
    m_pcm = m_player->availability() != QMultimedia::Available;
    m_data = m_pcm ? m_backend->decodeVoice(message) : m_backend->readMedia(message, false);
    if (m_data.isEmpty()) {
        if (m_pcm) {
            qWarning() << "voice playback: no media player and not an Opus message";
        }
        updateActive();
        return false;
    }
    m_messageId = message;
    emit messageIdChanged();
    emit durationChanged();
    if (!startRoute(0, true)) {
        release();
        return false;
    }
    updateActive();
    return true;
}

void VoicePlayer::resume()
{
    if (m_messageId != -1) {
        play(m_messageId);
    }
}

// Start (or restart) the loaded message on the route in use, from `startMs`.
bool VoicePlayer::startRoute(int startMs, bool autoplay)
{
#ifdef THREESEA_FULL
    if (m_earpiece && m_earpiecePlayer) {
        return m_earpiecePlayer->start(m_data, m_pcm, startMs, autoplay, m_speed);
    }
#endif
    m_buffer = new QBuffer(&m_data, this);
    m_buffer->open(QIODevice::ReadOnly);
    if (m_pcm) {
        startPcmOutput(static_cast<qint64>(startMs) * BytesPerMs, autoplay);
        return true;
    }
    m_player->setMedia(QMediaContent(), m_buffer);
    m_player->setPlaybackRate(m_speed);
    if (startMs > 0) {
        m_player->setPosition(startMs);
    }
    m_wantPlaying = autoplay;
    if (autoplay) {
        m_player->play();
    } else {
        m_player->pause();
    }
    return true;
}

void VoicePlayer::startPcmOutput(qint64 startByte, bool autoplay)
{
    m_buffer->seek(qBound<qint64>(0, startByte & ~1LL, m_data.size()));
    m_output = new QAudioOutput(pcmFormat(m_speed), this);
    m_output->setCategory(QStringLiteral("media"));
    m_output->setNotifyInterval(100);
    connect(m_output, &QAudioOutput::notify, this, &VoicePlayer::positionChanged);
    connect(m_output, &QAudioOutput::stateChanged, this, [this](QAudio::State state) {
        emit playingChanged();
        // Idle: the buffer has run dry, i.e. the end.
        if (state == QAudio::IdleState) {
            m_atEnd = true;
            QMetaObject::invokeMethod(this, "stop", Qt::QueuedConnection);
        }
    });
    m_output->start(m_buffer);
    if (!autoplay) {
        m_output->suspend();
    }
}

// The earpiece route is gone: carry on through the loudspeaker from where it stood.
void VoicePlayer::fallBackToLoudspeaker()
{
    const int at = position();
    m_earpiece = false;
    emit earpieceChanged();
    releaseRoute();
    if (!startRoute(at, true)) {
        release();
        return;
    }
    emit positionChanged();
    emit playingChanged();
}

void VoicePlayer::setEarpiece(bool earpiece)
{
    if (m_earpiece == earpiece || (earpiece && !earpieceAvailable())) {
        return;
    }
    m_earpiece = earpiece;
    emit earpieceChanged();
    if (m_messageId == -1 || m_data.isEmpty()) {
        return;
    }
    // Reroute the loaded message: same position, same play/pause state. Everything of the old
    // route goes first — a second reader on the same buffer played scrambled audio.
    const bool wasPlaying = playing();
    const int at = position();
    releaseRoute();
    if (!startRoute(at, wasPlaying)) {
        // No earpiece route on this device: stay on the loudspeaker instead of going silent.
        if (m_earpiece) {
            qWarning() << "voice at the ear: no route, staying on the loudspeaker";
            fallBackToLoudspeaker();
            return;
        }
        release();
        return;
    }
    emit positionChanged();
    emit playingChanged();
}

void VoicePlayer::setSpeed(qreal speed)
{
    const qreal wanted = clampSpeed(speed);
    if (qFuzzyCompare(wanted, m_speed)) {
        return;
    }
    m_speed = wanted;
    QSettings().setValue(QStringLiteral("voiceSpeed"), m_speed);
    emit speedChanged();
    if (m_messageId == -1 || m_data.isEmpty()) {
        return;
    }
#ifdef THREESEA_FULL
    if (m_earpiece && m_earpiecePlayer) {
        m_earpiecePlayer->setSpeed(m_speed);
        return;
    }
#endif
    if (m_pcm) {
        // The rate is part of the output's format: start it again where it stands.
        const bool wasPlaying = playing();
        const int at = position();
        releaseRoute();
        startRoute(at, wasPlaying);
        emit playingChanged();
        return;
    }
    m_player->setPlaybackRate(m_speed);
}

void VoicePlayer::chime()
{
    m_chimePending = true;
    updateActive();
    if (m_backend && !m_backend->inAppSounds()) {
        onChimeEnded(); // the queue goes on without a sound
        return;
    }
#ifdef THREESEA_FULL
    if (m_earpiece && earpieceAvailable()) {
        if (!m_chimePlayer) {
            m_chimePlayer = new EarpiecePlayer(this);
            connect(m_chimePlayer, &EarpiecePlayer::finished, this, &VoicePlayer::onChimeEnded);
            connect(m_chimePlayer, &EarpiecePlayer::failed, this, &VoicePlayer::onChimeEnded);
        }
        // Against the ear the same level is much louder.
        if (m_chimeAtEar.isEmpty()) {
            m_chimeAtEar = SoftSound::samples(0.5);
        }
        if (m_chimePlayer->start(m_chimeAtEar, true, 0, true, 1.0)) {
            return;
        }
    }
#endif
    m_chime->play();
}

void VoicePlayer::onChimeEnded()
{
    // Stopped meanwhile: the queue ends here, the sound itself just dies away.
    if (!m_chimePending) {
        return;
    }
    // Still pending while the next message starts, so the media controls see no gap.
    m_holdActive = true;
    emit chimeDone();
    m_holdActive = false;
    m_chimePending = false;
    updateActive();
}

void VoicePlayer::updateActive()
{
    const bool active = m_messageId != -1 || m_chimePending;
    if (!m_holdActive && active != m_active) {
        m_active = active;
        emit activeChanged();
    }
}

void VoicePlayer::pause()
{
    // Nothing is sounding but the chime: pausing that ends the queue.
    if (m_messageId == -1) {
        stop();
        return;
    }
#ifdef THREESEA_FULL
    if (m_earpiece && m_earpiecePlayer) {
        m_earpiecePlayer->pause();
        return;
    }
#endif
    if (m_output) {
        m_output->suspend();
        emit playingChanged();
    } else {
        m_wantPlaying = false;
        m_player->pause();
    }
}

void VoicePlayer::seek(int position)
{
#ifdef THREESEA_FULL
    if (m_earpiece && m_earpiecePlayer) {
        m_earpiecePlayer->seek(position);
        emit seeked(position);
        return;
    }
#endif
    if (m_output && m_buffer) {
        m_buffer->seek(qBound<qint64>(0, static_cast<qint64>(position) * BytesPerMs & ~1LL, m_data.size()));
        emit positionChanged();
    } else {
        m_player->setPosition(position);
    }
    emit seeked(position);
}

void VoicePlayer::onEarpieceFinished()
{
    m_atEnd = true;
    stop();
}

void VoicePlayer::stop()
{
    // Reached the natural end (not stopped by hand): tell the chat, which may queue the
    // next voice message, after everything is released.
    const int ended = m_atEnd ? m_messageId : -1;
    m_atEnd = false;
    // Stopped by hand, or another message comes: the chime's queue is over.
    if (ended == -1) {
        m_chimePending = false;
    }
    const bool held = m_holdActive;
    m_holdActive = true;
    release();
    if (ended != -1) {
        emit finished(ended);
    }
    m_holdActive = held;
    updateActive();
}

// Everything of the route in use; the loaded message (m_data, m_messageId) stays.
void VoicePlayer::releaseRoute()
{
#ifdef THREESEA_FULL
    if (m_earpiecePlayer) {
        m_earpiecePlayer->stop();
    }
#endif
    if (m_output) {
        m_output->stop();
        m_output->deleteLater();
        m_output = nullptr;
        emit playingChanged();
    }
    m_wantPlaying = false;
    m_player->stop();
    m_player->setMedia(QMediaContent());
    if (m_buffer) {
        m_buffer->close();
        m_buffer->deleteLater();
        m_buffer = nullptr;
    }
}

void VoicePlayer::release()
{
    releaseRoute();
    // Wiped only here: nothing reads the bytes any more (the pipeline's threads are joined).
    m_data.fill('\0');
    m_data.clear();
    m_pcm = false;
    m_pausedTooLong.stop();
    if (m_messageId != -1) {
        m_messageId = -1;
        emit messageIdChanged();
    }
    emit positionChanged();
    updateActive();
}
