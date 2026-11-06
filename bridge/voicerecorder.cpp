// SPDX-License-Identifier: AGPL-3.0-only
#include "voicerecorder.h"

#include <QAudioDeviceInfo>
#include <QAudioEncoderSettings>
#include <QDebug>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QStandardPaths>
#include <QUrl>

namespace {

// Longest voice message (Threema Android caps by size; 15 minutes stays well below it).
const int MaxDurationMs = 15 * 60 * 1000;
// Shorter is a slip of the finger.
const int MinDurationMs = 1000;
// A pause is judged by the recorder's time over this long, after this long to settle.
const int PauseWaitMs = 500;
// A paused recorder's time may move this little.
const int PauseToleranceMs = 200;
// What a recorder that reports no time at all has to have recorded for that to mean something.
const int PauseKnownAfterMs = 500;
// How long the platform may take to finish the file.
const int FinalizeWaitMs = 5000;
const int PcmBytesPerMs = 48000 * 2 / 1000;

// GStreamer names AAC "audio/mpeg, mpegversion=(int)4"; other backends say "aac".
// The Opus fallback's input: what the core expects (voice.rs).
QAudioFormat pcmFormat()
{
    QAudioFormat format;
    format.setSampleRate(48000);
    format.setChannelCount(1);
    format.setSampleSize(16);
    format.setSampleType(QAudioFormat::SignedInt);
    format.setByteOrder(QAudioFormat::LittleEndian);
    format.setCodec(QStringLiteral("audio/pcm"));
    return format;
}

bool isAac(const QString &codec)
{
    const QString c = codec.toLower();
    return c.contains(QLatin1String("aac"))
           || (c.startsWith(QLatin1String("audio/mpeg")) && c.contains(QLatin1String("mpegversion=(int)4")));
}

bool isMp4(const QString &container)
{
    const QString c = container.toLower();
    return c.contains(QLatin1String("mp4")) || c.contains(QLatin1String("m4a"))
           || (c.contains(QLatin1String("quicktime")) && c.contains(QLatin1String("iso")));
}

}

VoiceRecorder::VoiceRecorder(QObject *parent)
    : QObject(parent)
    , m_recorder(new QAudioRecorder(this))
    , m_maxDuration(MaxDurationMs)
{
    // A recording left behind by a crash (security audit 2026-10-27, S08): never sent, gone.
    QFile::remove(outputPath());
    const QStringList codecs = m_recorder->supportedAudioCodecs();
    const QStringList containers = m_recorder->supportedContainers();
    // No content data, only what the platform offers: helps when a device lacks AAC.
    qInfo() << "voice recorder codecs:" << codecs << "containers:" << containers;
    for (const QString &codec : codecs) {
        if (isAac(codec)) {
            m_codec = codec;
            break;
        }
    }
    for (const QString &container : containers) {
        if (isMp4(container)) {
            m_container = container;
            break;
        }
    }
    if (m_container.isEmpty()) {
        m_codec.clear();
    }
    const QAudioDeviceInfo device = QAudioDeviceInfo::defaultInputDevice();
    m_pcmAvailable = !device.isNull() && device.isFormatSupported(pcmFormat());
    qInfo() << "voice recorder:" << (usePlatform() ? "platform AAC" : m_pcmAvailable ? "Opus fallback" : "none");

    m_tick.setInterval(200);
    connect(&m_tick, &QTimer::timeout, this, [this]() {
        emit durationChanged();
        if (m_stop == Stop::ByItself && duration() >= m_maxDuration) {
            finish(Stop::Hold);
        }
    });
    m_pauseWait.setSingleShot(true);
    m_pauseWait.setInterval(PauseWaitMs);
    connect(&m_pauseWait, &QTimer::timeout, this, [this]() { pauseSample(m_recorder->duration()); });
    m_finalizeWait.setSingleShot(true);
    m_finalizeWait.setInterval(FinalizeWaitMs);
    connect(&m_finalizeWait, &QTimer::timeout, this, [this]() {
        qWarning() << "voice recorder: the platform did not finish the file, holding the recording";
        // It may be cut off: for the user to listen to, not to go out unseen.
        if (m_stop == Stop::Send) {
            m_stop = Stop::Hold;
        }
        stopped();
    });
    connect(m_recorder, &QMediaRecorder::stateChanged, this, [this](QMediaRecorder::State state) {
        if (state == QMediaRecorder::StoppedState) {
            recorderStopped(m_recorder->status());
        }
    });
    connect(m_recorder, &QMediaRecorder::statusChanged, this, &VoiceRecorder::recorderStatus);
    connect(m_recorder, static_cast<void (QMediaRecorder::*)(QMediaRecorder::Error)>(&QMediaRecorder::error), this,
            [this](QMediaRecorder::Error) { fail(m_recorder->errorString()); });
    // No microphone left open with nothing to show it, and no plain file of someone's words
    // waiting outside the chat page: leaving the app throws away what is running or held, and its
    // owner says so (user decision 2026-11-06). What a tap already sent goes on.
    connect(qGuiApp, &QGuiApplication::applicationStateChanged, this, [this](Qt::ApplicationState state) {
        if (state == Qt::ApplicationActive) {
            return;
        }
        pausePreview();
        const bool sending = m_recording && m_stop == Stop::Send;
        if (sending || (!m_recording && !m_held)) {
            return;
        }
        // Under a second nobody has lost anything.
        const bool something = m_held || duration() >= MinDurationMs;
        if (something) {
            emit discarded();
        }
        cancel();
    });
}

void VoiceRecorder::recorderStopped(QMediaRecorder::Status status)
{
    if (!m_recording) {
        return;
    }
    if (m_stop == Stop::ByItself) {
        m_finalDuration = recordedMs();
        m_stop = Stop::Hold;
    }
    m_pauseWait.stop();
    if (status == QMediaRecorder::LoadedStatus || status == QMediaRecorder::UnloadedStatus) {
        stopped();
    } else {
        m_finalizeWait.start();
    }
}

void VoiceRecorder::recorderStatus(QMediaRecorder::Status status)
{
    if (m_finalizeWait.isActive()
        && (status == QMediaRecorder::LoadedStatus || status == QMediaRecorder::UnloadedStatus)) {
        stopped();
    }
}

void VoiceRecorder::pauseSample(qint64 position)
{
    if (!m_recording || !m_paused || m_stop != Stop::ByItself) {
        return;
    }
    if (m_pausePosition < 0) {
        m_pausePosition = position;
        m_pauseWait.start();
        return;
    }
    const bool goesOn = position - m_pausePosition > PauseToleranceMs;
    const bool noTime = position <= 0 && m_recordedBefore >= PauseKnownAfterMs;
    if (goesOn || noTime) {
        qWarning() << "voice recorder: the platform did not pause, holding the recording";
        finish(Stop::Hold);
    }
}

void VoiceRecorder::stopped()
{
    const Stop how = m_stop;
    if (how == Stop::ByItself) {
        m_finalDuration = recordedMs();
    }
    m_recording = false;
    m_tick.stop();
    m_pauseWait.stop();
    m_finalizeWait.stop();
    setPaused(false);
    m_pcmBuffer.close();
    emit recordingChanged();
    emit durationChanged();
    const bool wanted = how != Stop::Discard && m_finalDuration >= MinDurationMs;
    const bool complete = wanted && (usePlatform() ? QFileInfo(outputPath()).size() > 0 : !m_pcm.isEmpty());
    if (complete && how == Stop::Send) {
        m_haveRecording = true;
        emit finished(m_finalDuration);
    } else if (complete && QGuiApplication::applicationState() == Qt::ApplicationActive) {
        m_haveRecording = true;
        setHeld(true);
        return;
    } else if (wanted) {
        // Also what came to rest only after the app was left: nobody is there to answer.
        emit discarded();
    }
    clear();
}

void VoiceRecorder::fail(const QString &error)
{
    qWarning() << "voice recording failed:" << error;
    const bool wasRecording = m_recording;
    m_recording = false;
    m_tick.stop();
    m_pauseWait.stop();
    m_finalizeWait.stop();
    setPaused(false);
    if (wasRecording) {
        emit recordingChanged();
    }
    emit failed(error);
    clear();
}

void VoiceRecorder::clear()
{
    endPreview();
    m_haveRecording = false;
    QFile::remove(outputPath());
    m_pcm.fill('\0');
    m_pcm.clear();
    setHeld(false);
    setOwner(nullptr);
}

void VoiceRecorder::setPaused(bool paused)
{
    if (m_paused != paused) {
        m_paused = paused;
        emit pausedChanged();
    }
}

void VoiceRecorder::setHeld(bool held)
{
    if (m_held != held) {
        m_held = held;
        emit heldChanged();
    }
}

void VoiceRecorder::setOwner(QObject *owner)
{
    if (m_owner == owner) {
        return;
    }
    disconnect(m_ownerGone);
    m_owner = owner;
    if (m_owner) {
        m_ownerGone = connect(m_owner, &QObject::destroyed, this, [this]() {
            m_owner = nullptr;
            emit ownerChanged();
            cancel();
        });
    }
    emit ownerChanged();
}

QByteArray VoiceRecorder::takeRecording(bool *pcm)
{
    *pcm = !usePlatform();
    QByteArray data;
    if (!m_haveRecording) {
        return data;
    }
    endPreview();
    m_haveRecording = false;
    if (usePlatform()) {
        QFile file(outputPath());
        if (file.open(QIODevice::ReadOnly)) {
            data = file.readAll();
        }
        QFile::remove(outputPath());
    } else {
        data = m_pcm;
        m_pcm.fill('\0');
        m_pcm.clear();
    }
    setHeld(false);
    setOwner(nullptr);
    return data;
}

VoiceRecorder::~VoiceRecorder()
{
    disconnect(m_ownerGone);
    endPreview();
    m_recorder->stop();
    if (m_input) {
        m_input->stop();
    }
    QFile::remove(outputPath());
    m_pcm.fill('\0');
}

int VoiceRecorder::duration() const
{
    return m_recording && m_stop == Stop::ByItself ? recordedMs() : m_finalDuration;
}

int VoiceRecorder::recordedMs() const
{
    return static_cast<int>(m_recordedBefore + (m_paused ? 0 : m_clock.elapsed()));
}

QString VoiceRecorder::outputPath() const
{
    return QStandardPaths::writableLocation(QStandardPaths::CacheLocation) + QStringLiteral("/voice-recording.m4a");
}

void VoiceRecorder::start(QObject *owner)
{
    if (!available() || m_recording || m_held) {
        return;
    }
    m_stop = Stop::ByItself;
    m_haveRecording = false;
    m_finalDuration = 0;
    m_recordedBefore = 0;
    setOwner(owner);
    if (!usePlatform()) {
        m_pcm.clear();
        if (!m_input) {
            m_input = new QAudioInput(QAudioDeviceInfo::defaultInputDevice(), pcmFormat(), this);
            connect(m_input, &QAudioInput::stateChanged, this, [this](QAudio::State state) {
                if (state == QAudio::StoppedState && m_recording) {
                    if (m_input->error() != QAudio::NoError && m_input->error() != QAudio::UnderrunError) {
                        fail(QStringLiteral("audio input error %1").arg(m_input->error()));
                    } else {
                        stopped();
                    }
                }
            });
        }
        m_pcmBuffer.close();
        m_pcmBuffer.setBuffer(&m_pcm);
        m_pcmBuffer.open(QIODevice::WriteOnly);
        m_recording = true;
        m_clock.start();
        m_input->start(&m_pcmBuffer);
        m_tick.start();
        emit recordingChanged();
        emit durationChanged();
        return;
    }
    const QString cache = QStandardPaths::writableLocation(QStandardPaths::CacheLocation);
    QDir().mkpath(cache);
    // The platform encoder writes a plain file there until it is sent or discarded: ours only.
    QFile::setPermissions(cache, QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner);
    QFile::remove(outputPath());
    QAudioEncoderSettings settings;
    settings.setCodec(m_codec);
    settings.setChannelCount(1);
    settings.setBitRate(32000);
    settings.setEncodingMode(QMultimedia::ConstantBitRateEncoding);
    m_recorder->setEncodingSettings(settings, QVideoEncoderSettings(), m_container);
    m_recorder->setOutputLocation(QUrl::fromLocalFile(outputPath()));
    m_recording = true;
    m_clock.start();
    m_recorder->record();
    m_tick.start();
    emit recordingChanged();
    emit durationChanged();
}

void VoiceRecorder::stop()
{
    finish(Stop::Send);
}

void VoiceRecorder::cancel()
{
    if (m_held) {
        clear();
    } else {
        finish(Stop::Discard);
    }
}

void VoiceRecorder::finish(Stop how)
{
    if (!m_recording) {
        return;
    }
    // Stopping takes a moment on the platform's path. Only discarding can overrule what is under way.
    if (m_stop != Stop::ByItself && how != Stop::Discard) {
        return;
    }
    if (m_stop == Stop::ByItself) {
        m_finalDuration = recordedMs();
    }
    m_stop = how;
    // What it ends as follows once the recording is complete (stateChanged).
    if (usePlatform()) {
        m_recorder->stop();
    } else {
        m_input->stop();
        if (m_recording) {
            stopped();
        }
    }
}

void VoiceRecorder::pause()
{
    if (!m_recording || m_paused || m_stop != Stop::ByItself) {
        return;
    }
    m_recordedBefore += m_clock.elapsed();
    setPaused(true);
    emit durationChanged();
    if (usePlatform()) {
        m_pausePosition = -1;
        m_pauseWait.start();
        m_recorder->pause();
    } else if (m_input) {
        m_input->suspend();
    }
}

void VoiceRecorder::resume()
{
    if (!m_recording || !m_paused || m_stop != Stop::ByItself) {
        return;
    }
    m_pauseWait.stop();
    m_clock.start();
    setPaused(false);
    if (usePlatform()) {
        m_recorder->record();
    } else if (m_input) {
        m_input->resume();
    }
}

void VoiceRecorder::hold()
{
    finish(Stop::Hold);
}

void VoiceRecorder::setPreviewing(bool previewing)
{
    if (m_previewing != previewing) {
        m_previewing = previewing;
        emit previewingChanged();
    }
}

int VoiceRecorder::previewPosition() const
{
    if (m_previewPlayer) {
        return static_cast<int>(m_previewPlayer->position());
    }
    if (m_previewOutput) {
        return static_cast<int>(m_previewFrom + m_previewOutput->processedUSecs() / 1000);
    }
    return static_cast<int>(m_previewFrom);
}

void VoiceRecorder::startPreview()
{
    if (!m_held || !m_haveRecording || m_previewing) {
        return;
    }
    setPreviewing(true);
    if (usePlatform()) {
        if (!m_previewPlayer) {
            m_previewPlayer = new QMediaPlayer(this);
            m_previewPlayer->setNotifyInterval(100);
            connect(m_previewPlayer, &QMediaPlayer::positionChanged, this, &VoiceRecorder::previewPositionChanged);
            connect(m_previewPlayer, &QMediaPlayer::stateChanged, this, [this](QMediaPlayer::State state) {
                // Sailfish's resource policy resumes what it paused; only the user starts the sound here.
                if (state == QMediaPlayer::PlayingState && !m_previewing && m_previewPlayer) {
                    QMetaObject::invokeMethod(m_previewPlayer, "pause", Qt::QueuedConnection);
                } else if (state != QMediaPlayer::PlayingState) {
                    setPreviewing(false);
                }
            });
            connect(m_previewPlayer, &QMediaPlayer::mediaStatusChanged, this, [this](QMediaPlayer::MediaStatus status) {
                if (status == QMediaPlayer::EndOfMedia) {
                    previewEnded();
                }
            });
            connect(m_previewPlayer, static_cast<void (QMediaPlayer::*)(QMediaPlayer::Error)>(&QMediaPlayer::error), this,
                    [this](QMediaPlayer::Error error) {
                        qWarning() << "voice recorder: listening failed, code" << static_cast<int>(error);
                        endPreview();
                        emit previewFailed();
                    });
            m_previewPlayer->setMedia(QUrl::fromLocalFile(outputPath()));
        }
        m_previewPlayer->play();
        // Moved before there was a player.
        if (m_previewFrom > 0) {
            m_previewPlayer->setPosition(m_previewFrom);
            m_previewFrom = 0;
        }
        return;
    }
    if (m_previewOutput && m_previewOutput->state() == QAudio::SuspendedState) {
        m_previewOutput->resume();
        return;
    }
    delete m_previewOutput;
    m_previewOutput = new QAudioOutput(pcmFormat(), this);
    m_previewOutput->setNotifyInterval(100);
    connect(m_previewOutput, &QAudioOutput::notify, this, &VoiceRecorder::previewPositionChanged);
    connect(m_previewOutput, &QAudioOutput::stateChanged, this, [this](QAudio::State state) {
        // Idle: everything has been played. Stopped with an error: nothing will be.
        if (state == QAudio::IdleState) {
            previewEnded();
        } else if (state == QAudio::StoppedState && m_previewing && m_previewOutput
                   && m_previewOutput->error() != QAudio::NoError) {
            qWarning() << "voice recorder: listening failed, code" << static_cast<int>(m_previewOutput->error());
            endPreview();
            emit previewFailed();
        }
    });
    m_previewBuffer.close();
    m_previewBuffer.setBuffer(&m_pcm);
    m_previewBuffer.open(QIODevice::ReadOnly);
    m_previewBuffer.seek(m_previewFrom * PcmBytesPerMs);
    m_previewOutput->start(&m_previewBuffer);
    if (m_previewOutput->error() != QAudio::NoError) {
        qWarning() << "voice recorder: listening failed, code" << static_cast<int>(m_previewOutput->error());
        endPreview();
        emit previewFailed();
    }
}

void VoiceRecorder::pausePreview()
{
    if (!m_previewing) {
        return;
    }
    setPreviewing(false);
    if (m_previewPlayer) {
        m_previewPlayer->pause();
    } else if (m_previewOutput) {
        m_previewOutput->suspend();
    }
}

void VoiceRecorder::seekPreview(int position)
{
    if (!m_held) {
        return;
    }
    const qint64 to = qBound(0, position, m_finalDuration);
    if (m_previewPlayer) {
        m_previewPlayer->setPosition(to);
    } else {
        // The output has its own data ahead of the buffer's position: from there anew.
        const bool wasPlaying = m_previewing;
        endPreview();
        m_previewFrom = to;
        if (wasPlaying) {
            startPreview();
        }
    }
    emit previewPositionChanged();
}

void VoiceRecorder::previewEnded()
{
    endPreview();
}

void VoiceRecorder::endPreview()
{
    setPreviewing(false);
    m_previewFrom = 0;
    if (m_previewPlayer) {
        QMediaPlayer *player = m_previewPlayer;
        m_previewPlayer = nullptr;
        player->disconnect(this);
        player->stop();
        // Lets go of the file, which is about to be sent or removed.
        player->setMedia(QMediaContent());
        player->deleteLater();
    }
    if (m_previewOutput) {
        QAudioOutput *output = m_previewOutput;
        m_previewOutput = nullptr;
        output->disconnect(this);
        output->stop();
        output->deleteLater();
    }
    m_previewBuffer.close();
    // Away from the recording, which it only reads.
    m_previewBuffer.setBuffer(nullptr);
    emit previewPositionChanged();
}
