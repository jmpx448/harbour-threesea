// SPDX-License-Identifier: AGPL-3.0-only
#ifndef VIDEOTRANSCODER_H
#define VIDEOTRANSCODER_H

#include <QString>

// Scales a video down before sending, like Android's "Video size": the longest edge becomes
// 480 or 848 px (Android's VideoConfig) at its bitrate, the audio track is copied as it is
// (Android copies it too, so nothing is lost twice).
//
// Full flavour only: it needs the system's GStreamer. It runs on the file preparer's worker
// thread and blocks, and it gives up quietly — a phone without an H.264 encoder (or a
// container we cannot read) sends the video unchanged.
class VideoTranscoder
{
public:
    // The longest edge Android uses per setting (1 small, 2 medium; anything else: unchanged).
    static int edgeForSetting(int setting);
    // Bitrate (bit/s) that goes with that edge.
    static int bitrateForSetting(int setting);

    // Writes the scaled video to `target` (an .mp4 path) and returns true. False means the
    // video is to be sent as it is: no encoder, an unreadable container, or an error (then
    // `target` is removed again). Whether it is too large is the caller's question.
    static bool scaleDown(const QString &source, int setting, const QString &target);
};

#endif
