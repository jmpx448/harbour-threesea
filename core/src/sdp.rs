// SPDX-License-Identifier: AGPL-3.0-only
//! SDP rules of Threema calls (ADR 0012), ported from Android's `SdpPatcher`
//! (`app/.../voip/util/SdpPatcher.java`), so both sides agree on what a call uses:
//!
//! - Audio: Opus only, mono, constant bitrate (`cbr=1`, which also turns voice activity
//!   detection off: no bitrate leaks what is said).
//! - Video and the data channel section stay; unknown media sections are dropped.
//! - RTP header extensions: off for audio-only calls; with video, only encrypted ones
//!   (RFC 6904, `urn:ietf:params:rtp-hdrext:encrypt`), never audio levels or frame marking,
//!   IDs renumbered in local offers, the remote offer's extensions replayed in local answers.

use std::collections::HashMap;

/// Which SDP is patched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdpKind {
    RemoteOffer,
    RemoteAnswer,
    LocalOffer,
    LocalAnswer,
}

/// RTP header extensions (Android: off for audio calls, one- and two-byte headers with video).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderExtensions {
    Disable,
    OneByteOnly,
    OneAndTwoByte,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SdpError {
    #[error("invalid SDP: {0}")]
    Invalid(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Global,
    Audio,
    Video,
    DataChannel,
    Unknown,
}

enum Action {
    Accept,
    Reject,
    Rewrite(String),
}

/// One call's patcher: a local answer replays the extensions of the remote offer patched before.
pub struct SdpPatcher {
    extensions: HeaderExtensions,
    remote_offer_extensions: HashMap<Section, Vec<String>>,
}

struct Context {
    kind: SdpKind,
    opus: String,
    section: Section,
    next_id: u32,
    max_id: u32,
    ids: HashMap<String, u32>,
}

impl SdpPatcher {
    #[must_use]
    pub fn new(extensions: HeaderExtensions) -> Self {
        Self {
            extensions,
            remote_offer_extensions: HashMap::new(),
        }
    }

    /// Apply the rules to `sdp` (lines end with CRLF afterwards).
    ///
    /// # Errors
    /// Without an Opus payload type, or with too many header extensions in a local offer.
    pub fn patch(&mut self, kind: SdpKind, sdp: &str) -> Result<String, SdpError> {
        let opus = sdp
            .lines()
            .find_map(|line| {
                let rest = line.strip_prefix("a=rtpmap:")?;
                let (payload_type, codec) = rest.split_once(' ')?;
                codec.starts_with("opus").then(|| payload_type.to_owned())
            })
            .ok_or(SdpError::Invalid("a=rtpmap: [...] opus not found"))?;
        let mut context = Context {
            kind,
            opus,
            section: Section::Global,
            next_id: 1,
            max_id: match self.extensions {
                HeaderExtensions::Disable => 0,
                HeaderExtensions::OneByteOnly => 14,
                HeaderExtensions::OneAndTwoByte => 255,
            },
            ids: HashMap::new(),
        };
        let mut out = String::with_capacity(sdp.len());
        let mut skipping = false;
        for line in sdp.lines() {
            let line = line.trim_end_matches('\r');
            if skipping && !line.starts_with("m=") {
                continue;
            }
            skipping = false;
            let before = context.section;
            let action = if line.starts_with("m=") {
                Self::section_line(&mut context, line)?
            } else {
                match context.section {
                    Section::Global => self.rtp_attributes(line),
                    Section::Audio => self.audio_line(&mut context, line)?,
                    Section::Video => self.video_line(&mut context, line)?,
                    Section::DataChannel => Action::Accept,
                    Section::Unknown => Action::Reject,
                }
            };
            match action {
                Action::Accept => {
                    out.push_str(line);
                    out.push_str("\r\n");
                }
                Action::Rewrite(text) => {
                    out.push_str(&text);
                    out.push_str("\r\n");
                }
                // A rejected media section goes with all its lines.
                Action::Reject => skipping = before != context.section,
            }
        }
        Ok(out)
    }

    fn section_line(context: &mut Context, line: &str) -> Result<Action, SdpError> {
        if let Some(rest) = line.strip_prefix("m=audio ") {
            context.section = Section::Audio;
            let mut parts = rest.splitn(3, ' ');
            let (Some(port), Some(proto), Some(payload_types)) =
                (parts.next(), parts.next(), parts.next())
            else {
                return Err(SdpError::Invalid("audio media line"));
            };
            if !payload_types.split(' ').any(|pt| pt == context.opus) {
                return Err(SdpError::Invalid(
                    "Opus payload type not in the audio media description",
                ));
            }
            return Ok(Action::Rewrite(format!(
                "m=audio {port} {proto} {}",
                context.opus
            )));
        }
        if line.starts_with("m=video") {
            context.section = Section::Video;
            return Ok(Action::Accept);
        }
        if line.starts_with("m=application") && line.contains("DTLS/SCTP") {
            context.section = Section::DataChannel;
            return Ok(Action::Accept);
        }
        context.section = Section::Unknown;
        Ok(Action::Reject)
    }

    fn rtp_attributes(&self, line: &str) -> Action {
        if self.extensions != HeaderExtensions::OneAndTwoByte
            && line.starts_with("a=extmap-allow-mixed")
        {
            return Action::Reject;
        }
        Action::Accept
    }

    fn audio_line(&mut self, context: &mut Context, line: &str) -> Result<Action, SdpError> {
        if let Some(rest) = line.strip_prefix("a=rtpmap:")
            && rest.contains(' ')
        {
            let payload_type = rest.split(' ').next().unwrap_or_default();
            return Ok(if payload_type == context.opus {
                Action::Accept
            } else {
                Action::Reject
            });
        }
        if let Some(rest) = line.strip_prefix("a=fmtp:")
            && let Some((payload_type, params)) = rest.split_once(' ')
            && !params.contains(' ')
        {
            if payload_type != context.opus {
                return Ok(Action::Reject);
            }
            let mut rewritten = format!("a=fmtp:{} ", context.opus);
            for param in params.split(';') {
                let key = param.split('=').next().unwrap_or_default();
                if !param.is_empty() && !["stereo", "sprop-stereo", "cbr"].contains(&key) {
                    rewritten.push_str(param);
                    rewritten.push(';');
                }
            }
            rewritten.push_str("stereo=0;sprop-stereo=0;cbr=1");
            return Ok(Action::Rewrite(rewritten));
        }
        self.media_line(context, line)
    }

    fn video_line(&mut self, context: &mut Context, line: &str) -> Result<Action, SdpError> {
        self.media_line(context, line)
    }

    /// What audio and video share: the replay after `a=mid:` and the extension lines.
    fn media_line(&mut self, context: &mut Context, line: &str) -> Result<Action, SdpError> {
        if context.kind == SdpKind::LocalAnswer && line.starts_with("a=mid:") {
            let mut lines = vec![line.to_owned()];
            lines.extend(
                self.remote_offer_extensions
                    .remove(&context.section)
                    .unwrap_or_default(),
            );
            return Ok(Action::Rewrite(lines.join("\r\n")));
        }
        if let Some(rest) = line.strip_prefix("a=extmap:")
            && let Some((_, uri_and_attributes)) = rest.split_once(' ')
        {
            return self.extension_line(context, line, uri_and_attributes);
        }
        Ok(self.rtp_attributes(line))
    }

    fn extension_line(
        &mut self,
        context: &mut Context,
        line: &str,
        uri_and_attributes: &str,
    ) -> Result<Action, SdpError> {
        const ENCRYPT: &str = "urn:ietf:params:rtp-hdrext:encrypt";
        if self.extensions == HeaderExtensions::Disable
            || uri_and_attributes.contains("urn:ietf:params:rtp-hdrext:ssrc-audio-level")
            || uri_and_attributes.contains("urn:ietf:params:rtp-hdrext:csrc-audio-level")
            || uri_and_attributes
                .contains("http://tools.ietf.org/html/draft-ietf-avtext-framemarking-07")
        {
            return Ok(Action::Reject);
        }
        let encrypted = uri_and_attributes.starts_with(ENCRYPT);
        Ok(match context.kind {
            SdpKind::RemoteOffer => {
                if !encrypted {
                    return Ok(Action::Reject);
                }
                if matches!(context.section, Section::Audio | Section::Video) {
                    self.remote_offer_extensions
                        .entry(context.section)
                        .or_default()
                        .push(line.to_owned());
                }
                Action::Accept
            }
            SdpKind::RemoteAnswer if encrypted => Action::Accept,
            SdpKind::LocalOffer if !encrypted => {
                let id = match context.ids.get(uri_and_attributes) {
                    Some(id) => *id,
                    None => {
                        if context.next_id > context.max_id {
                            return Err(SdpError::Invalid("RTP extension IDs exhausted"));
                        }
                        let id = context.next_id;
                        context.next_id += 1;
                        // 15 is reserved in one-byte headers.
                        if context.next_id == 15 {
                            context.next_id += 1;
                        }
                        context.ids.insert(uri_and_attributes.to_owned(), id);
                        id
                    }
                };
                Action::Rewrite(format!("a=extmap:{id} {ENCRYPT} {uri_and_attributes}"))
            }
            _ => Action::Reject,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Vectors from upstream `SdpPatcherTest.java`.
    use super::*;

    const ALL: [SdpKind; 4] = [
        SdpKind::LocalOffer,
        SdpKind::LocalAnswer,
        SdpKind::RemoteOffer,
        SdpKind::RemoteAnswer,
    ];

    fn patch(kind: SdpKind, sdp: &str) -> Result<String, SdpError> {
        SdpPatcher::new(HeaderExtensions::Disable).patch(kind, sdp)
    }

    #[test]
    fn force_cbr() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n\
                   a=fmtp:111 minptime=10;cbr=0;useinbandfec=1\r\n";
        let expected = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n\
                        a=fmtp:111 minptime=10;useinbandfec=1;stereo=0;sprop-stereo=0;cbr=1\r\n";
        for kind in ALL {
            assert_eq!(patch(kind, sdp).unwrap(), expected);
        }
    }

    #[test]
    fn invalid_and_foreign_fmtp() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n\
                   a=fmtp:111 minptime=10;cbr0;useinbandfec=1\r\na=fmtp:1337 cat=yes;duck=no\r\n";
        let expected = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n\
                        a=fmtp:111 minptime=10;cbr0;useinbandfec=1;stereo=0;sprop-stereo=0;cbr=1\r\n";
        for kind in ALL {
            assert_eq!(patch(kind, sdp).unwrap(), expected);
        }
    }

    #[test]
    fn opus_is_required() {
        let without_opus = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 popus/48000/2\r\n\
                            a=fmtp:111 minptime=10;cbr0;useinbandfec=1\r\n";
        let without_payload_type =
            "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 1337\r\na=rtpmap:111 opus/48000/2\r\n";
        for kind in ALL {
            assert!(patch(kind, without_opus).is_err());
            assert!(patch(kind, without_payload_type).is_err());
        }
    }

    #[test]
    fn unknown_media_dropped_data_channel_kept() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n\
                   m=everything in plain text over the wire kthx\r\na=plaintext OH YES YES YES\r\n\
                   a=moar-plaintext\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
                   a=sctp-port:5000\r\nm=the-train-protocol\r\na=choo-chooo\r\n";
        let expected = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n\
                        m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=sctp-port:5000\r\n";
        for kind in ALL {
            assert_eq!(patch(kind, sdp).unwrap(), expected);
        }
    }

    #[test]
    fn no_header_extensions() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n\
                   a=extmap:6 urn:ietf:params:rtp-hdrext:encrypt 6-1\r\n\
                   a=extmap:5 urn:ietf:params:rtp-hdrext:encrypt 5\r\n";
        let expected = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n";
        for kind in ALL {
            assert_eq!(patch(kind, sdp).unwrap(), expected);
        }
    }

    #[test]
    fn one_byte_ids_reassigned_in_local_offers() {
        let ids = [
            "6-1",
            "6-2",
            "5",
            "3",
            "1",
            "7",
            "8",
            "9",
            "11",
            "10",
            "12",
            "15",
            "19",
            "1337387126438213678123681273618",
        ];
        let mut sdp =
            String::from("v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n");
        let mut expected = sdp.clone();
        for (index, id) in ids.iter().enumerate() {
            let original = id.split('-').next().unwrap();
            sdp.push_str(&format!("a=extmap:{original} {id}\r\n"));
            expected.push_str(&format!(
                "a=extmap:{} urn:ietf:params:rtp-hdrext:encrypt {id}\r\n",
                index + 1
            ));
        }
        let mut patcher = SdpPatcher::new(HeaderExtensions::OneByteOnly);
        assert_eq!(patcher.patch(SdpKind::LocalOffer, &sdp).unwrap(), expected);
        // A 15th one doesn't fit.
        sdp.push_str("a=extmap:20 20\r\n");
        assert!(
            SdpPatcher::new(HeaderExtensions::OneByteOnly)
                .patch(SdpKind::LocalOffer, &sdp)
                .is_err()
        );
    }

    #[test]
    fn remote_offer_extensions_replayed_in_local_answer() {
        let offer = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\na=rtpmap:111 opus/48000/2\r\n\
                     a=extmap:1 urn:ietf:params:rtp-hdrext:encrypt urn:x\r\na=extmap:2 urn:plain\r\n";
        let mut patcher = SdpPatcher::new(HeaderExtensions::OneAndTwoByte);
        let patched_offer = patcher.patch(SdpKind::RemoteOffer, offer).unwrap();
        assert!(patched_offer.contains("encrypt urn:x") && !patched_offer.contains("urn:plain"));
        let answer = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\na=rtpmap:111 opus/48000/2\r\n\
                      a=extmap:3 urn:whatever\r\n";
        let patched_answer = patcher.patch(SdpKind::LocalAnswer, answer).unwrap();
        assert_eq!(
            patched_answer,
            "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\n\
             a=extmap:1 urn:ietf:params:rtp-hdrext:encrypt urn:x\r\na=rtpmap:111 opus/48000/2\r\n"
        );
    }
}
