// SPDX-License-Identifier: AGPL-3.0-only
//! The SDP of a group call: the SFU never sends one, each client builds the "remote offer" from
//! the join response and the participants it knows, and answers it locally (Android's
//! `GroupCallSessionDescription`). Both sides derive the MIDs from participant IDs.

use libthreema::protobuf::group_call::sfu_http_response::join::RtpHeaderExtensionIds;

/// The SDP token characters, in the order the MIDs are generated.
const SDP_TOKENS: &[u8] =
    b"!#$%&'*+-.0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ^_`abcdefghijklmnopqrstuvwxyz{|}~";
const MIDS_PER_PARTICIPANT: usize = 8;

/// The `n`th MID: every 1-character token, then every 2-character token (left-major).
fn mid(n: usize) -> Option<String> {
    let count = SDP_TOKENS.len();
    if n < count {
        return Some((SDP_TOKENS[n] as char).to_string());
    }
    let n = n - count;
    (n < count * count).then(|| {
        format!(
            "{}{}",
            SDP_TOKENS[n / count] as char,
            SDP_TOKENS[n % count] as char
        )
    })
}

/// A participant's MIDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mids {
    pub microphone: String,
    pub camera: String,
    pub data: String,
}

impl Mids {
    /// `None` beyond the 790 participants the MID space has room for.
    #[must_use]
    pub fn of(participant: u32) -> Option<Self> {
        let first = usize::try_from(participant)
            .ok()?
            .checked_mul(MIDS_PER_PARTICIPANT)?;
        Some(Self {
            microphone: mid(first)?,
            camera: mid(first + 1)?,
            data: mid(first + MIDS_PER_PARTICIPANT - 1)?,
        })
    }
}

const MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
const RTP_STREAM_ID: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
const REPAIRED_RTP_STREAM_ID: &str = "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id";
const ABSOLUTE_SEND_TIME: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
const TRANSPORT_CC: &str =
    "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
const VIDEO_ORIENTATION: &str = "urn:3gpp:video-orientation";
const TIME_OFFSET: &str = "urn:ietf:params:rtp-hdrext:toffset";

/// RTP header extension IDs per media (from the join response, else Android's fallbacks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extensions {
    pub microphone: Vec<(u32, &'static str)>,
    pub camera: Vec<(u32, &'static str)>,
}

impl Extensions {
    #[must_use]
    pub fn from_join(ids: Option<&RtpHeaderExtensionIds>) -> Self {
        let Some(ids) = ids else {
            return Self {
                microphone: vec![(1, MID), (4, ABSOLUTE_SEND_TIME), (5, TRANSPORT_CC)],
                camera: vec![
                    (1, MID),
                    (2, RTP_STREAM_ID),
                    (3, REPAIRED_RTP_STREAM_ID),
                    (4, ABSOLUTE_SEND_TIME),
                    (5, TRANSPORT_CC),
                    (11, VIDEO_ORIENTATION),
                    (12, TIME_OFFSET),
                ],
            };
        };
        Self {
            microphone: vec![
                (ids.mid, MID),
                (ids.absolute_send_time, ABSOLUTE_SEND_TIME),
                (ids.transport_wide_congestion_control_01, TRANSPORT_CC),
            ],
            camera: vec![
                (ids.mid, MID),
                (ids.rtp_stream_id, RTP_STREAM_ID),
                (ids.repaired_rtp_stream_id, REPAIRED_RTP_STREAM_ID),
                (ids.absolute_send_time, ABSOLUTE_SEND_TIME),
                (ids.transport_wide_congestion_control_01, TRANSPORT_CC),
                (ids.video_orientation, VIDEO_ORIENTATION),
                (ids.time_offset, TIME_OFFSET),
            ],
        }
    }
}

/// The camera's simulcast layers (Android's and Desktop's `CAMERA_SEND_ENCODINGS`).
pub const CAMERA_RIDS: [&str; 3] = ["l", "m", "h"];

/// Opus as the SFU expects it (also patched into our local answer).
pub const OPUS_FMTP: &str = "minptime=10;useinbandfec=1;usedtx=1";

/// What the SFU told us when joining.
#[derive(Debug, Clone)]
pub struct Session {
    pub local: u32,
    pub ice_ufrag: String,
    pub ice_pwd: String,
    /// Raw SHA-256 of the SFU's DTLS certificate.
    pub dtls_fingerprint: Vec<u8>,
    pub extensions: Extensions,
}

/// The remote offer of a call, regenerated whenever participants come or go. m-lines are never
/// removed or reordered: participants who left stay as inactive lines.
#[derive(Debug)]
pub struct Description {
    session: Session,
    version: u64,
    /// Remote participants in the order first seen.
    order: Vec<u32>,
}

impl Description {
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self {
            session,
            version: 1,
            order: Vec::new(),
        }
    }

    /// The remote offer for these (currently present) remote participants.
    pub fn remote_offer(&mut self, present: &[u32]) -> String {
        for participant in present {
            if *participant != self.session.local && !self.order.contains(participant) {
                self.order.push(*participant);
            }
        }
        let mut bundle = Vec::new();
        let mut media = Vec::new();
        if let Some(mids) = Mids::of(self.session.local) {
            bundle.extend([
                mids.microphone.clone(),
                mids.camera.clone(),
                mids.data.clone(),
            ]);
            media.extend(self.rtp_lines(Kind::Audio, Direction::Local, true, &mids.microphone));
            media.extend(self.rtp_lines(Kind::Video, Direction::Local, true, &mids.camera));
            media.extend([
                "m=application 9 UDP/DTLS/SCTP webrtc-datachannel".to_owned(),
                "c=IN IP4 0.0.0.0".to_owned(),
                format!("a=mid:{}", mids.data),
                "a=sctp-port:5000".to_owned(),
                "a=max-message-size:131072".to_owned(),
            ]);
        }
        for participant in &self.order {
            let Some(mids) = Mids::of(*participant) else {
                continue;
            };
            let active = present.contains(participant);
            bundle.extend([mids.microphone.clone(), mids.camera.clone()]);
            media.extend(self.rtp_lines(Kind::Audio, Direction::Remote, active, &mids.microphone));
            media.extend(self.rtp_lines(Kind::Video, Direction::Remote, active, &mids.camera));
        }
        let fingerprint = self
            .session
            .dtls_fingerprint
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(":");
        let mut lines = vec![
            "v=0".to_owned(),
            format!("o=- 1 {} IN IP4 127.0.0.1", self.version),
            "s=-".to_owned(),
            "t=0 0".to_owned(),
            format!("a=group:BUNDLE {}", bundle.join(" ")),
            "a=msid-semantic: WMS *".to_owned(),
            format!("a=ice-ufrag:{}", self.session.ice_ufrag),
            format!("a=ice-pwd:{}", self.session.ice_pwd),
            "a=ice-options:trickle".to_owned(),
            "a=ice-lite".to_owned(),
            format!("a=fingerprint:sha-256 {fingerprint}"),
            "a=setup:passive".to_owned(),
        ];
        self.version += 1;
        lines.extend(media);
        lines.push(String::new());
        lines.join("\r\n")
    }

    fn rtp_lines(&self, kind: Kind, direction: Direction, active: bool, mid: &str) -> Vec<String> {
        let (payload_types, extensions) = match kind {
            Kind::Audio => ("111", &self.session.extensions.microphone),
            Kind::Video => ("96 97", &self.session.extensions.camera),
        };
        // From the SFU's side: it receives our media and sends the others'.
        let direction_line = match (active, direction) {
            (false, _) => "inactive",
            (true, Direction::Local) => "recvonly",
            (true, Direction::Remote) => "sendonly",
        };
        let kind_name = match kind {
            Kind::Audio => "audio",
            Kind::Video => "video",
        };
        let mut lines = vec![
            format!(
                "m={kind_name} {} UDP/TLS/RTP/SAVPF {payload_types}",
                if active { 9 } else { 0 }
            ),
            "c=IN IP4 0.0.0.0".to_owned(),
            "a=rtcp:9 IN IP4 0.0.0.0".to_owned(),
            format!("a=mid:{mid}"),
        ];
        lines.extend(
            extensions
                .iter()
                .map(|(id, uri)| format!("a=extmap:{id} {uri}")),
        );
        lines.push(format!("a={direction_line}"));
        lines.push("a=rtcp-mux".to_owned());
        match kind {
            Kind::Audio => lines.extend([
                "a=rtpmap:111 opus/48000/2".to_owned(),
                "a=rtcp-fb:111 transport-cc".to_owned(),
                format!("a=fmtp:111 {OPUS_FMTP}"),
            ]),
            Kind::Video => {
                lines.push("a=rtcp-rsize".to_owned());
                lines.push("a=rtpmap:96 VP8/90000".to_owned());
                lines.extend(
                    ["transport-cc", "ccm fir", "nack", "nack pli", "goog-remb"]
                        .iter()
                        .map(|feedback| format!("a=rtcp-fb:96 {feedback}")),
                );
                lines.push("a=rtpmap:97 rtx/90000".to_owned());
                lines.push("a=fmtp:97 apt=96".to_owned());
                if active && direction == Direction::Local {
                    lines.extend(CAMERA_RIDS.iter().map(|rid| format!("a=rid:{rid} recv")));
                    lines.push(format!("a=simulcast:recv {}", CAMERA_RIDS.join(";")));
                }
            }
        }
        lines
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Audio,
    Video,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Local,
    Remote,
}

/// Our local answer with the SFU's Opus parameters (Android's `patchLocalDescription`).
#[must_use]
pub fn patch_local_answer(sdp: &str) -> String {
    sdp.split("\r\n")
        .map(|line| {
            if line.starts_with("a=fmtp:111 ") {
                format!("a=fmtp:111 {OPUS_FMTP}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mids_like_android() {
        assert_eq!(
            Mids::of(0),
            Some(Mids {
                microphone: "!".into(),
                camera: "#".into(),
                data: "+".into(),
            })
        );
        let one = Mids::of(1).unwrap();
        assert_eq!((one.microphone.as_str(), one.camera.as_str()), ("-", "."));
        // Participant 9 starts at MID 72 ("x"), its data MID is 79, the first 2-character one.
        let nine = Mids::of(9).unwrap();
        assert_eq!((nine.microphone.as_str(), nine.data.as_str()), ("x", "!!"));
        assert!(Mids::of(789).is_some());
        assert!(Mids::of(790).is_none());
    }

    fn session() -> Session {
        Session {
            local: 1,
            ice_ufrag: "ufrag".into(),
            ice_pwd: "pwd".into(),
            dtls_fingerprint: vec![0xab, 0x01],
            extensions: Extensions::from_join(None),
        }
    }

    #[test]
    fn remote_offer_keeps_lines_of_participants_who_left() {
        let mut description = Description::new(session());
        let first = description.remote_offer(&[0]);
        assert!(first.contains("a=group:BUNDLE - . 5 ! #\r\n"), "{first}");
        assert!(first.contains("a=fingerprint:sha-256 AB:01\r\n"));
        assert!(first.contains("a=ice-lite\r\n"));
        // Our camera takes the three layers.
        assert!(first.contains("a=simulcast:recv l;m;h\r\n"));
        assert_eq!(first.matches("a=sendonly").count(), 2);
        // Participant 0 left, 2 came: 0's lines stay, inactive, before 2's.
        let second = description.remote_offer(&[2]);
        assert!(second.contains("o=- 1 2 IN IP4"));
        assert!(
            second.contains("a=group:BUNDLE - . 5 ! # 6 7\r\n"),
            "{second}"
        );
        assert_eq!(second.matches("a=inactive").count(), 2);
        assert!(second.contains("m=audio 0 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=rtcp:9 IN IP4 0.0.0.0\r\na=mid:!\r\n"));
        assert!(second.ends_with("\r\n"));
    }

    #[test]
    fn opus_fmtp_is_patched() {
        let answer = "v=0\r\na=fmtp:111 minptime=10;useinbandfec=1\r\na=mid:-\r\n";
        assert_eq!(
            patch_local_answer(answer),
            "v=0\r\na=fmtp:111 minptime=10;useinbandfec=1;usedtx=1\r\na=mid:-\r\n"
        );
    }
}
