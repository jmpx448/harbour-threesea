// SPDX-License-Identifier: AGPL-3.0-only
// Compiled as C++17 (harbour-threesea.pro, `aec` compiler): webrtc-audio-processing needs it.
#include "echocanceller.h"

#include <atomic>

#include <api/audio/audio_processing.h>

struct EchoCanceller::Private
{
    rtc::scoped_refptr<webrtc::AudioProcessing> apm;
    webrtc::StreamConfig stream{48000, 1};
    std::atomic<int> delay{100};
};

EchoCanceller::EchoCanceller()
    : d(new Private)
{
    d->apm = webrtc::AudioProcessingBuilder().Create();
    webrtc::AudioProcessing::Config config;
    config.echo_canceller.enabled = true;
    config.echo_canceller.mobile_mode = false; // AEC3, as libwebrtc uses on phones too
    config.noise_suppression.enabled = true;
    config.noise_suppression.level = webrtc::AudioProcessing::Config::NoiseSuppression::kHigh;
    config.high_pass_filter.enabled = true;
    config.gain_controller2.enabled = true;
    config.gain_controller2.adaptive_digital.enabled = true;
    d->apm->ApplyConfig(config);
}

EchoCanceller::~EchoCanceller()
{
    delete d;
}

void EchoCanceller::playback(const int16_t *frame)
{
    int16_t unused[FRAME_SAMPLES];
    d->apm->ProcessReverseStream(frame, d->stream, d->stream, unused);
}

void EchoCanceller::capture(int16_t *frame)
{
    d->apm->set_stream_delay_ms(d->delay.load());
    d->apm->ProcessStream(frame, d->stream, d->stream, frame);
}

void EchoCanceller::setDelay(int milliseconds)
{
    d->delay.store(milliseconds);
}
