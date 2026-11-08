// SPDX-License-Identifier: AGPL-3.0-only
//! Video in 1:1 calls (ADR 0012), ported from Android's `VoipVideoParams` and the call's data
//! channel messages (`o2o-call.proto`): the quality profiles and how two of them combine, and
//! the envelopes that announce the camera/microphone state and the profile.

use libthreema::protobuf::{
    common::Resolution,
    o2o_call::{
        CaptureState, Envelope, VideoQualityProfile, capture_state, envelope,
        video_quality_profile::QualityProfile,
    },
};
use prost::Message as _;

/// Lower bounds for raw (unnamed) values, like Android's.
const MIN_BITRATE: u32 = 200;
const MIN_FPS: u32 = 15;
const MIN_WIDTH: u32 = 320;
const MIN_HEIGHT: u32 = 240;

/// What the video sender may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoParams {
    /// `None`: raw values from a peer whose profile we don't know.
    pub profile: Option<QualityProfile>,
    pub bitrate_kbps: u32,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
}

/// The largest remote video picture we decode. Android sends at most the MAX profile (1920x1080,
/// either way up); a VP8 key frame can declare up to 16383x16383 in a few bytes, and decoder and
/// conversion would then ask for gigabytes.
const MAX_REMOTE_VIDEO_SIDE: u32 = 2560;
const MAX_REMOTE_VIDEO_PIXELS: u32 = 2560 * 1440;

/// What to do with a received VP8 frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vp8Verdict {
    /// Decode it.
    Pass,
    /// Waiting for a usable key frame: drop it.
    Drop,
    /// A key frame that is too large or malformed: drop it, and what follows until a good one.
    Refuse,
}

/// Lets only pictures of a sensible size reach the decoder (RFC 6386 9.1: a frame tag whose
/// lowest bit is clear means a key frame, whose bytes 3..6 are the start code and 6..10 the
/// 14-bit width and height, with 2 bits of scaling each). `waiting`: this stream has no accepted
/// key frame yet or lost it; starts out true.
pub fn vp8_filter(frame: &[u8], waiting: &mut bool) -> Vp8Verdict {
    let Some(tag) = frame.first() else {
        return Vp8Verdict::Drop;
    };
    if tag & 1 != 0 {
        return if *waiting {
            Vp8Verdict::Drop
        } else {
            Vp8Verdict::Pass
        };
    }
    let size = match frame {
        [_, _, _, 0x9d, 0x01, 0x2a, w0, w1, h0, h1, ..] => Some((
            u32::from(u16::from_le_bytes([*w0, *w1]) & 0x3fff),
            u32::from(u16::from_le_bytes([*h0, *h1]) & 0x3fff),
        )),
        _ => None,
    };
    let fits = size.is_some_and(|(width, height)| {
        width > 0
            && height > 0
            && width <= MAX_REMOTE_VIDEO_SIDE
            && height <= MAX_REMOTE_VIDEO_SIDE
            && width * height <= MAX_REMOTE_VIDEO_PIXELS
    });
    *waiting = !fits;
    if fits {
        Vp8Verdict::Pass
    } else {
        Vp8Verdict::Refuse
    }
}

impl VideoParams {
    #[must_use]
    pub const fn low() -> Self {
        Self::named(QualityProfile::Low, 400, 20, 960, 540)
    }

    #[must_use]
    pub const fn high() -> Self {
        Self::named(QualityProfile::High, 2000, 25, 1280, 720)
    }

    #[must_use]
    pub const fn max() -> Self {
        Self::named(QualityProfile::Max, 4000, 25, 1920, 1080)
    }

    const fn named(
        profile: QualityProfile,
        bitrate_kbps: u32,
        fps: u32,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            profile: Some(profile),
            bitrate_kbps,
            fps,
            width,
            height,
        }
    }

    /// From Settings → Calls → Video quality: `auto` (high; Android uses low on metered
    /// networks), `low`, `max`.
    #[must_use]
    pub fn from_setting(setting: &str) -> Self {
        match setting {
            "low" => Self::low(),
            "max" => Self::max(),
            _ => Self::high(),
        }
    }

    /// The lower of both (Android's `findCommonProfile`): named profiles by name, MAX only
    /// without a relay; raw values element-wise, clamped to the minimum.
    #[must_use]
    pub fn common(&self, peer: Option<&Self>, relayed: bool) -> Self {
        let Some(peer) = peer else {
            return *self;
        };
        match (self.profile, peer.profile) {
            (Some(own), Some(theirs)) => {
                if own == QualityProfile::Low || theirs == QualityProfile::Low {
                    Self::low()
                } else if own == QualityProfile::High || theirs == QualityProfile::High || relayed {
                    Self::high()
                } else {
                    Self::max()
                }
            }
            _ => Self {
                profile: None,
                bitrate_kbps: self.bitrate_kbps.min(peer.bitrate_kbps).max(MIN_BITRATE),
                fps: self.fps.min(peer.fps).max(MIN_FPS),
                width: self.width.min(peer.width).max(MIN_WIDTH),
                height: self.height.min(peer.height).max(MIN_HEIGHT),
            },
        }
    }

    fn from_message(message: &VideoQualityProfile) -> Self {
        match QualityProfile::try_from(message.profile) {
            Ok(QualityProfile::Low) => Self::low(),
            Ok(QualityProfile::High) => Self::high(),
            Ok(QualityProfile::Max) => Self::max(),
            // An unknown profile: its raw values (Android's fallback).
            Err(_) => Self {
                profile: None,
                bitrate_kbps: message.max_bitrate_kbps,
                fps: message.max_fps,
                width: message.max_resolution.map_or(0, |r| r.width),
                height: message.max_resolution.map_or(0, |r| r.height),
            },
        }
    }
}

/// What the camera or microphone does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureDevice {
    Camera,
    Microphone,
}

/// A message from the peer's data channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelMessage {
    Capture { device: CaptureDevice, on: bool },
    VideoQuality(VideoParams),
}

/// Decode a data channel message; `None` for what we don't know (Android ignores it too).
#[must_use]
pub fn decode(data: &[u8]) -> Option<ChannelMessage> {
    match Envelope::decode(data).ok()?.content? {
        envelope::Content::CaptureStateChange(change) => {
            let device = match capture_state::CaptureDevice::try_from(change.device).ok()? {
                capture_state::CaptureDevice::Camera => CaptureDevice::Camera,
                capture_state::CaptureDevice::Microphone => CaptureDevice::Microphone,
                capture_state::CaptureDevice::ReservedForScreenShare => return None,
            };
            Some(ChannelMessage::Capture {
                device,
                on: change.state == capture_state::Mode::On as i32,
            })
        }
        envelope::Content::VideoQualityProfile(profile) => Some(ChannelMessage::VideoQuality(
            VideoParams::from_message(&profile),
        )),
    }
}

/// "Our camera/microphone is on/off" for the peer.
#[must_use]
pub fn encode_capture(device: CaptureDevice, on: bool) -> Vec<u8> {
    Envelope {
        content: Some(envelope::Content::CaptureStateChange(CaptureState {
            state: if on {
                capture_state::Mode::On
            } else {
                capture_state::Mode::Off
            } as i32,
            device: match device {
                CaptureDevice::Camera => capture_state::CaptureDevice::Camera,
                CaptureDevice::Microphone => capture_state::CaptureDevice::Microphone,
            } as i32,
        })),
        // Unpadded, like Android's 1:1 call messages (its group call messages are padded).
        ..Default::default()
    }
    .encode_to_vec()
}

/// Our video quality profile for the peer.
#[must_use]
pub fn encode_quality(params: &VideoParams) -> Vec<u8> {
    Envelope {
        content: Some(envelope::Content::VideoQualityProfile(
            VideoQualityProfile {
                profile: params.profile.unwrap_or(QualityProfile::High) as i32,
                max_bitrate_kbps: params.bitrate_kbps,
                max_resolution: Some(Resolution {
                    width: params.width,
                    height: params.height,
                }),
                max_fps: params.fps,
            },
        )),
        ..Default::default()
    }
    .encode_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_frame(width: u16, height: u16) -> Vec<u8> {
        let mut frame = vec![0x10, 0, 0, 0x9d, 0x01, 0x2a];
        frame.extend_from_slice(&width.to_le_bytes());
        frame.extend_from_slice(&height.to_le_bytes());
        frame.extend_from_slice(&[0; 20]);
        frame
    }

    const DELTA: &[u8] = &[0x31, 0, 0, 1, 2, 3];

    #[test]
    fn vp8_key_frames_of_a_sensible_size_pass() {
        for (width, height) in [(640, 480), (1920, 1080), (1080, 1920), (2560, 1440), (1, 1)] {
            let mut waiting = true;
            assert_eq!(
                vp8_filter(&key_frame(width, height), &mut waiting),
                Vp8Verdict::Pass,
                "{width}x{height}"
            );
            assert!(!waiting);
            assert_eq!(vp8_filter(DELTA, &mut waiting), Vp8Verdict::Pass);
        }
        // The two upper bits are scaling, not size.
        let mut waiting = true;
        assert_eq!(
            vp8_filter(&key_frame(640 | 0xc000, 480 | 0x4000), &mut waiting),
            Vp8Verdict::Pass
        );
    }

    #[test]
    fn vp8_huge_key_frames_stop_the_stream_until_a_good_one() {
        let mut waiting = false;
        for (width, height) in [
            (16383, 16383),
            (2561, 2),
            (2, 2561),
            (2560, 2560),
            (0, 480),
            (640, 0),
        ] {
            waiting = false;
            assert_eq!(
                vp8_filter(&key_frame(width, height), &mut waiting),
                Vp8Verdict::Refuse,
                "{width}x{height}"
            );
            assert!(waiting);
            assert_eq!(vp8_filter(DELTA, &mut waiting), Vp8Verdict::Drop);
        }
        assert!(waiting);
        assert_eq!(
            vp8_filter(&key_frame(640, 480), &mut waiting),
            Vp8Verdict::Pass
        );
        assert_eq!(vp8_filter(DELTA, &mut waiting), Vp8Verdict::Pass);
    }

    #[test]
    fn vp8_broken_frames_are_dropped() {
        let mut waiting = true;
        // Deltas before any key frame, and nothing at all.
        assert_eq!(vp8_filter(DELTA, &mut waiting), Vp8Verdict::Drop);
        assert_eq!(vp8_filter(&[], &mut waiting), Vp8Verdict::Drop);
        waiting = false;
        assert_eq!(vp8_filter(&[], &mut waiting), Vp8Verdict::Drop);
        assert!(!waiting);
        // A key frame too short to hold a size, or without the start code.
        assert_eq!(
            vp8_filter(&[0x10, 0, 0, 0x9d, 0x01, 0x2a, 1, 1, 1], &mut waiting),
            Vp8Verdict::Refuse
        );
        let mut frame = key_frame(640, 480);
        frame[3] = 0;
        assert_eq!(vp8_filter(&frame, &mut waiting), Vp8Verdict::Refuse);
        assert!(waiting);
    }

    #[test]
    fn common_profile_takes_the_lower_one() {
        let (low, high, max) = (VideoParams::low(), VideoParams::high(), VideoParams::max());
        assert_eq!(high.common(Some(&low), false), low);
        assert_eq!(max.common(Some(&high), false), high);
        assert_eq!(max.common(Some(&max), false), max);
        // MAX only without a relay.
        assert_eq!(max.common(Some(&max), true), high);
        assert_eq!(high.common(None, true), high);
    }

    #[test]
    fn raw_values_are_clamped() {
        let raw = VideoParams {
            profile: None,
            bitrate_kbps: 100,
            fps: 30,
            width: 160,
            height: 1080,
        };
        assert_eq!(
            VideoParams::high().common(Some(&raw), false),
            VideoParams {
                profile: None,
                bitrate_kbps: 200,
                fps: 25,
                width: 320,
                height: 720,
            }
        );
    }

    #[test]
    fn channel_messages_round_trip() {
        let data = encode_capture(CaptureDevice::Camera, true);
        assert_eq!(
            decode(&data),
            Some(ChannelMessage::Capture {
                device: CaptureDevice::Camera,
                on: true
            })
        );
        let data = encode_capture(CaptureDevice::Microphone, false);
        assert_eq!(
            decode(&data),
            Some(ChannelMessage::Capture {
                device: CaptureDevice::Microphone,
                on: false
            })
        );
        let data = encode_quality(&VideoParams::low());
        assert_eq!(
            decode(&data),
            Some(ChannelMessage::VideoQuality(VideoParams::low()))
        );
        assert_eq!(decode(b"\xff\xff"), None);
    }

    #[test]
    fn android_capture_state_bytes() {
        // Android's CaptureState.camera(true): Envelope { capture_state_change: { state: ON,
        // device: CAMERA } } — proto3 leaves the zero device out: field 3, length 2, state=1.
        assert_eq!(
            decode(&[0x1a, 0x02, 0x08, 0x01]),
            Some(ChannelMessage::Capture {
                device: CaptureDevice::Camera,
                on: true
            })
        );
    }
}
