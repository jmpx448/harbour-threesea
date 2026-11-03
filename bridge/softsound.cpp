// SPDX-License-Identifier: AGPL-3.0-only
#include "softsound.h"

#include <cmath>

namespace {

const int Rate = 48000;

QAudioFormat pcmFormat()
{
    QAudioFormat format;
    format.setSampleRate(Rate);
    format.setChannelCount(1);
    format.setSampleSize(16);
    format.setSampleType(QAudioFormat::SignedInt);
    format.setByteOrder(QAudioFormat::LittleEndian);
    format.setCodec(QStringLiteral("audio/pcm"));
    return format;
}

} // namespace

SoftSound::SoftSound(QObject *parent)
    : QObject(parent)
{
}

// C5 with its fifth a little quieter, a slow attack and a long ring-out, quiet: the first
// version (a sharp two-tone chime at four times this level) was loud and aggressive on the
// phone (user report 2026-10-29).
QByteArray SoftSound::samples(qreal gain)
{
    const int count = Rate * 450 / 1000;
    const double tones[] = {523.25, 783.99};
    const double gains[] = {0.045, 0.016};
    const double attack = 0.025 * Rate;
    QByteArray data(count * 2, '\0');
    qint16 *samples = reinterpret_cast<qint16 *>(data.data());
    for (int i = 0; i < count; ++i) {
        const double t = double(i) / Rate;
        // Raised-cosine attack (no click), exponential decay, faded to zero at the end.
        const double rise = i < attack ? 0.5 - 0.5 * std::cos(M_PI * i / attack) : 1.0;
        const double envelope = rise * std::exp(-t * 5.0) * (1.0 - double(i) / count);
        double value = 0.0;
        for (int tone = 0; tone < 2; ++tone) {
            value += std::sin(2.0 * M_PI * tones[tone] * t) * gains[tone];
        }
        samples[i] = static_cast<qint16>(qBound(-1.0, value * envelope * gain, 1.0) * 32767.0);
    }
    return data;
}

void SoftSound::play()
{
    if (m_output) {
        return; // already sounding
    }
    if (m_data.isEmpty()) {
        m_data = samples();
    }
    m_buffer = new QBuffer(&m_data, this);
    m_buffer->open(QIODevice::ReadOnly);
    m_output = new QAudioOutput(pcmFormat(), this);
    m_output->setCategory(QStringLiteral("media"));
    connect(m_output, &QAudioOutput::stateChanged, this, [this](QAudio::State state) {
        if (state == QAudio::IdleState || state == QAudio::StoppedState) {
            QMetaObject::invokeMethod(this, "cleanUp", Qt::QueuedConnection);
        }
    });
    m_output->start(m_buffer);
}

void SoftSound::cleanUp()
{
    if (!m_output) {
        return;
    }
    m_output->stop();
    m_output->deleteLater();
    m_output = nullptr;
    m_buffer->close();
    m_buffer->deleteLater();
    m_buffer = nullptr;
    emit done();
}
