// SPDX-License-Identifier: AGPL-3.0-only
#include "voicerecorder.h"

#include <QAudioDeviceInfo>
#include <QAudioEncoderSettings>
#include <QDebug>
#include <QDir>
#include <QFile>
#include <QGuiApplication>
#include <QStandardPaths>
#include <QUrl>

namespace {

// Longest voice message (Threema Android caps by size; 15 minutes stays well below it).
const int MaxDurationMs = 15 * 60 * 1000;
// Shorter is a slip of the finger.
const int MinDurationMs = 1000;

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
        if (m_stop == Stop::ByItself && duration() >= MaxDurationMs) {
            finish(Stop::Limit);
        }
    });
    connect(m_recorder, &QMediaRecorder::stateChanged, this, [this](QMediaRecorder::State state) {
        if (state == QMediaRecorder::StoppedState && m_recording) {
            stopped();
        }
    });
    connect(m_recorder, static_cast<void (QMediaRecorder::*)(QMediaRecorder::Error)>(&QMediaRecorder::error), this,
            [this](QMediaRecorder::Error) { fail(m_recorder->errorString()); });
    // A plain file or the words in memory, with nobody looking: not kept on the off chance.
    connect(qGuiApp, &QGuiApplication::applicationStateChanged, this, [this](Qt::ApplicationState state) {
        if (m_held && state != Qt::ApplicationActive) {
            emit discarded();
            clear();
        }
    });
}

void VoiceRecorder::stopped()
{
    const Stop how = m_stop;
    if (how == Stop::ByItself) {
        m_finalDuration = static_cast<int>(m_clock.elapsed());
    }
    m_recording = false;
    m_tick.stop();
    m_pcmBuffer.close();
    emit recordingChanged();
    emit durationChanged();
    const bool wanted = how != Stop::Discard && m_finalDuration >= MinDurationMs;
    const bool complete = wanted && (usePlatform() ? QFile::exists(outputPath()) : !m_pcm.isEmpty());
    if (complete && how == Stop::Send) {
        m_haveRecording = true;
        emit finished(m_finalDuration);
    } else if (complete && QGuiApplication::applicationState() == Qt::ApplicationActive) {
        m_haveRecording = true;
        setHeld(true);
        return;
    } else if (wanted) {
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
    if (wasRecording) {
        emit recordingChanged();
    }
    emit failed(error);
    clear();
}

void VoiceRecorder::clear()
{
    m_haveRecording = false;
    QFile::remove(outputPath());
    m_pcm.fill('\0');
    m_pcm.clear();
    setHeld(false);
    setOwner(nullptr);
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
    m_recorder->stop();
    if (m_input) {
        m_input->stop();
    }
    QFile::remove(outputPath());
    m_pcm.fill('\0');
}

int VoiceRecorder::duration() const
{
    return m_recording ? static_cast<int>(m_clock.elapsed()) : m_finalDuration;
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
        m_finalDuration = static_cast<int>(m_clock.elapsed());
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
