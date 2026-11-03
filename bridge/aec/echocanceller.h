// SPDX-License-Identifier: AGPL-3.0-only
#ifndef ECHOCANCELLER_H
#define ECHOCANCELLER_H

#include <cstddef>
#include <cstdint>

// Echo cancellation, noise suppression, high-pass filter and automatic gain for calls (ADR 0012),
// with the bundled webrtc-audio-processing (third_party/webrtc-audio-processing, BSD): what
// libwebrtc does inside Android's calls. Built as C++17 on its own (abseil), so this header
// carries no webrtc types. Audio: 48 kHz mono S16, frames of FRAME_SAMPLES (10 ms). playback()
// and capture() may run on different threads (the library locks internally).
class EchoCanceller
{
public:
    static const std::size_t FRAME_SAMPLES = 480;

    EchoCanceller();
    ~EchoCanceller();
    EchoCanceller(const EchoCanceller &) = delete;
    EchoCanceller &operator=(const EchoCanceller &) = delete;

    // What the speaker plays (the far end), before it plays.
    void playback(const int16_t *frame);
    // What the microphone picked up: cleaned in place.
    void capture(int16_t *frame);
    // Estimated time from playback() to the echo reaching capture() (a hint, AEC3 adapts).
    void setDelay(int milliseconds);

private:
    struct Private;
    Private *d;
};

#endif // ECHOCANCELLER_H
