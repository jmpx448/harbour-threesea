// SPDX-License-Identifier: AGPL-3.0-only
//! 1:1 calls (ADR 0012): the signalling messages' JSON and the call state, without I/O.
//!
//! Upstream: Android's `VoipCall*Data` (JSON) and `VoipStateService`/`CallState`. The media
//! (WebRTC) runs in the bridge on GStreamer; this decides what a signal means for the call and
//! which signals to send. One call at a time, like Android: a second offer gets "busy".

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::providers::CallSignalKind;

/// Why a call was rejected (`rejectReason` of `call-answer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    Unknown = 0,
    Busy = 1,
    Timeout = 2,
    Rejected = 3,
    Disabled = 4,
    OffHours = 5,
}

impl RejectReason {
    fn from_code(code: u64) -> Self {
        match code {
            1 => Self::Busy,
            2 => Self::Timeout,
            3 => Self::Rejected,
            4 => Self::Disabled,
            5 => Self::OffHours,
            _ => Self::Unknown,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Busy => "busy",
            Self::Timeout => "timeout",
            Self::Rejected => "rejected",
            Self::Disabled => "disabled",
            Self::OffHours => "offHours",
        }
    }
}

/// An ICE candidate as Threema sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub candidate: String,
    #[serde(default)]
    pub sdp_mid: Option<String>,
    #[serde(default, rename = "sdpMLineIndex")]
    pub sdp_m_line_index: Option<u32>,
    #[serde(default)]
    pub ufrag: Option<String>,
}

/// A decoded call signal.
#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    Offer {
        call_id: u32,
        sdp: String,
        video: bool,
    },
    Accept {
        call_id: u32,
        sdp: String,
        video: bool,
    },
    Reject {
        call_id: u32,
        reason: RejectReason,
    },
    Candidates {
        call_id: u32,
        candidates: Vec<Candidate>,
    },
    Hangup {
        call_id: u32,
    },
    Ringing {
        call_id: u32,
    },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid call message: {0}")]
pub struct SignalError(&'static str);

fn call_id(value: &Value) -> Result<u32, SignalError> {
    // "Assume 0 if not set"; it must fit an unsigned 32-bit integer.
    match value.get("callId") {
        None | Some(Value::Null) => Ok(0),
        Some(id) => id
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
            .ok_or(SignalError("callId")),
    }
}

fn has_video(value: &Value) -> bool {
    value["features"].get("video").is_some()
}

impl Signal {
    /// Decode a received signal (hangup and ringing may be empty).
    ///
    /// # Errors
    /// Bad JSON, or fields the spec requires.
    pub fn decode(kind: CallSignalKind, data: &[u8]) -> Result<Self, SignalError> {
        let value: Value = if data.iter().all(u8::is_ascii_whitespace) {
            json!({})
        } else {
            serde_json::from_slice(data).map_err(|_| SignalError("JSON"))?
        };
        let call_id = call_id(&value)?;
        Ok(match kind {
            CallSignalKind::Offer => {
                let offer = &value["offer"];
                if offer["sdpType"] != "offer" {
                    return Err(SignalError("offer sdpType"));
                }
                Self::Offer {
                    call_id,
                    sdp: offer["sdp"]
                        .as_str()
                        .ok_or(SignalError("offer sdp"))?
                        .to_owned(),
                    video: has_video(&value),
                }
            }
            CallSignalKind::Answer => match value["action"].as_u64() {
                Some(1) => {
                    let answer = &value["answer"];
                    if answer["sdpType"] != "answer" {
                        return Err(SignalError("answer sdpType"));
                    }
                    Self::Accept {
                        call_id,
                        sdp: answer["sdp"]
                            .as_str()
                            .ok_or(SignalError("answer sdp"))?
                            .to_owned(),
                        video: has_video(&value),
                    }
                }
                Some(0) => Self::Reject {
                    call_id,
                    reason: RejectReason::from_code(value["rejectReason"].as_u64().unwrap_or(0)),
                },
                _ => return Err(SignalError("action")),
            },
            CallSignalKind::IceCandidates => {
                if value["removed"] == true {
                    return Err(SignalError("removed candidates"));
                }
                let candidates: Vec<Candidate> =
                    serde_json::from_value(value["candidates"].clone())
                        .map_err(|_| SignalError("candidates"))?;
                if candidates.is_empty() {
                    return Err(SignalError("no candidates"));
                }
                Self::Candidates {
                    call_id,
                    candidates,
                }
            }
            CallSignalKind::Hangup => Self::Hangup { call_id },
            CallSignalKind::Ringing => Self::Ringing { call_id },
        })
    }

    /// The message type and JSON body to send.
    #[must_use]
    pub fn encode(&self) -> (CallSignalKind, Vec<u8>) {
        let features = |video: bool| {
            if video {
                json!({ "video": null })
            } else {
                json!({})
            }
        };
        let (kind, value) = match self {
            Self::Offer {
                call_id,
                sdp,
                video,
            } => {
                let mut value = json!({
                    "callId": call_id,
                    "offer": { "sdpType": "offer", "sdp": sdp },
                });
                if *video {
                    value["features"] = features(true);
                }
                (CallSignalKind::Offer, value)
            }
            Self::Accept {
                call_id,
                sdp,
                video,
            } => {
                let mut value = json!({
                    "callId": call_id,
                    "action": 1,
                    "answer": { "sdpType": "answer", "sdp": sdp },
                });
                if *video {
                    value["features"] = features(true);
                }
                (CallSignalKind::Answer, value)
            }
            Self::Reject { call_id, reason } => (
                CallSignalKind::Answer,
                json!({ "callId": call_id, "action": 0, "rejectReason": *reason as u8 }),
            ),
            Self::Candidates {
                call_id,
                candidates,
            } => (
                CallSignalKind::IceCandidates,
                json!({ "callId": call_id, "removed": false, "candidates": candidates }),
            ),
            Self::Hangup { call_id } => (CallSignalKind::Hangup, json!({ "callId": call_id })),
            Self::Ringing { call_id } => (CallSignalKind::Ringing, json!({ "callId": call_id })),
        };
        (kind, value.to_string().into_bytes())
    }

    #[must_use]
    pub fn call_id(&self) -> u32 {
        match self {
            Self::Offer { call_id, .. }
            | Self::Accept { call_id, .. }
            | Self::Reject { call_id, .. }
            | Self::Candidates { call_id, .. }
            | Self::Hangup { call_id }
            | Self::Ringing { call_id } => *call_id,
        }
    }
}

/// Where the current call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// We called; waiting for the answer (`ringing`: their phone rings).
    Outgoing { ringing: bool },
    /// They called; waiting for the user.
    Incoming,
    /// Accepted; WebRTC connects / is connected.
    Active,
}

/// The one call in progress.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub peer: String,
    pub call_id: u32,
    pub phase: Phase,
    /// We called.
    pub outgoing: bool,
    pub video: bool,
    /// When it was offered and when it was accepted (ms).
    pub started_at: i64,
    pub accepted_at: Option<i64>,
}

/// What the app has to do because of a signal or a user action.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Send this signal to `peer`.
    Send { peer: String, signal: Signal },
    /// Show the incoming call (ring) with the remote offer for the media engine.
    Incoming {
        peer: String,
        call_id: u32,
        sdp: String,
        video: bool,
    },
    /// The peer's phone rings.
    Ringing,
    /// The peer accepted: give the media engine their answer. `video`: both sides take video.
    RemoteAnswer { sdp: String, video: bool },
    /// Candidates for the media engine.
    RemoteCandidates(Vec<Candidate>),
    /// The call is over: stop the media, show it in the chat.
    Ended { record: CallRecord },
    /// Show this call in the chat; the call in progress, if any, goes on.
    RecordOnly { record: CallRecord },
}

/// How a call went, for the chat (Android shows these as status messages).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallRecord {
    pub peer: String,
    pub call_id: u32,
    pub outgoing: bool,
    /// `finished`, `missed`, `rejected`, `busy`, `aborted`, …
    pub result: String,
    /// Seconds talked (finished calls).
    pub duration: Option<i64>,
    pub at: i64,
}

/// The calls of this device (one at a time).
#[derive(Debug, Default)]
pub struct Calls {
    pub current: Option<Call>,
}

impl Calls {
    /// A signal from `peer` at `now` (ms).
    pub fn on_signal(&mut self, peer: &str, signal: Signal, now: i64) -> Vec<Effect> {
        let matches_current = self.is_current(peer, signal.call_id());
        match signal {
            Signal::Offer {
                call_id,
                sdp,
                video,
            } => {
                if let Some(call) = &self.current {
                    if call.peer == peer && call.call_id == call_id {
                        return Vec::new(); // repeated
                    }
                    // One call at a time; the user still learns that this one tried (Android).
                    return vec![
                        Effect::Send {
                            peer: peer.to_owned(),
                            signal: Signal::Reject {
                                call_id,
                                reason: RejectReason::Busy,
                            },
                        },
                        missed_call(peer, call_id, now),
                    ];
                }
                self.current = Some(Call {
                    peer: peer.to_owned(),
                    call_id,
                    phase: Phase::Incoming,
                    outgoing: false,
                    video,
                    started_at: now,
                    accepted_at: None,
                });
                vec![
                    Effect::Send {
                        peer: peer.to_owned(),
                        signal: Signal::Ringing { call_id },
                    },
                    Effect::Incoming {
                        peer: peer.to_owned(),
                        call_id,
                        sdp,
                        video,
                    },
                ]
            }
            Signal::Ringing { .. } if matches_current => {
                if let Some(call) = self.current.as_mut()
                    && let Phase::Outgoing { ringing } = &mut call.phase
                {
                    *ringing = true;
                    return vec![Effect::Ringing];
                }
                Vec::new()
            }
            Signal::Accept { sdp, video, .. } if matches_current => {
                let Some(call) = self.current.as_mut() else {
                    return Vec::new();
                };
                if !matches!(call.phase, Phase::Outgoing { .. }) {
                    return Vec::new();
                }
                call.phase = Phase::Active;
                call.accepted_at = Some(now);
                call.video &= video;
                vec![Effect::RemoteAnswer {
                    sdp,
                    video: call.video,
                }]
            }
            Signal::Reject { reason, .. } if matches_current => {
                let result = match reason {
                    RejectReason::Busy => "busy",
                    RejectReason::Timeout => "unanswered",
                    RejectReason::Disabled => "disabled",
                    _ => "rejected",
                };
                self.end(result, now)
            }
            Signal::Candidates { candidates, .. } if matches_current => {
                vec![Effect::RemoteCandidates(candidates)]
            }
            Signal::Hangup { .. } if matches_current => {
                let result = match self.current.as_ref().map(|c| c.phase) {
                    Some(Phase::Incoming) => "missed",
                    Some(Phase::Outgoing { .. }) => "aborted",
                    _ => "finished",
                };
                self.end(result, now)
            }
            // A hangup for a call we never saw (its offer was dropped): a missed call, whatever
            // else is going on. The app sorts out the calls it does know (`on_call_signal`).
            Signal::Hangup { call_id } => vec![missed_call(peer, call_id, now)],
            _ => Vec::new(),
        }
    }

    /// The user calls `peer`; `sdp`: the media engine's offer.
    pub fn start(
        &mut self,
        peer: &str,
        call_id: u32,
        sdp: String,
        video: bool,
        now: i64,
    ) -> Vec<Effect> {
        if self.current.is_some() {
            return Vec::new();
        }
        self.current = Some(Call {
            peer: peer.to_owned(),
            call_id,
            phase: Phase::Outgoing { ringing: false },
            outgoing: true,
            video,
            started_at: now,
            accepted_at: None,
        });
        vec![Effect::Send {
            peer: peer.to_owned(),
            signal: Signal::Offer {
                call_id,
                sdp,
                video,
            },
        }]
    }

    /// The user accepts the incoming call; `sdp`: the media engine's answer.
    pub fn accept(&mut self, sdp: String, video: bool, now: i64) -> Vec<Effect> {
        let Some(call) = self.current.as_mut() else {
            return Vec::new();
        };
        if call.phase != Phase::Incoming {
            return Vec::new();
        }
        call.phase = Phase::Active;
        call.accepted_at = Some(now);
        call.video &= video;
        vec![Effect::Send {
            peer: call.peer.clone(),
            signal: Signal::Accept {
                call_id: call.call_id,
                sdp,
                video: call.video,
            },
        }]
    }

    /// The user declines, or nobody answers in time (`timeout`).
    pub fn reject(&mut self, timeout: bool, now: i64) -> Vec<Effect> {
        let Some(call) = self.current.as_ref() else {
            return Vec::new();
        };
        if call.phase != Phase::Incoming {
            return Vec::new();
        }
        let mut effects = vec![Effect::Send {
            peer: call.peer.clone(),
            signal: Signal::Reject {
                call_id: call.call_id,
                reason: if timeout {
                    RejectReason::Timeout
                } else {
                    RejectReason::Rejected
                },
            },
        }];
        effects.extend(self.end(if timeout { "missed" } else { "rejected" }, now));
        effects
    }

    /// The user hangs up (or cancels an outgoing call).
    pub fn hang_up(&mut self, now: i64) -> Vec<Effect> {
        let Some(call) = self.current.as_ref() else {
            return Vec::new();
        };
        if call.phase == Phase::Incoming {
            return self.reject(false, now);
        }
        let mut effects = vec![Effect::Send {
            peer: call.peer.clone(),
            signal: Signal::Hangup {
                call_id: call.call_id,
            },
        }];
        let result = if call.phase == Phase::Active {
            "finished"
        } else {
            "aborted"
        };
        effects.extend(self.end(result, now));
        effects
    }

    /// This is the call in progress.
    #[must_use]
    pub fn is_current(&self, peer: &str, call_id: u32) -> bool {
        self.current
            .as_ref()
            .is_some_and(|call| call.peer == peer && call.call_id == call_id)
    }

    /// Nobody answered within the ring time (Android: 60 s): an incoming call is rejected with
    /// "timeout" (missed), an outgoing one is hung up (unanswered).
    pub fn time_out(&mut self, now: i64) -> Vec<Effect> {
        let Some(call) = self.current.as_ref() else {
            return Vec::new();
        };
        match call.phase {
            Phase::Incoming => self.reject(true, now),
            Phase::Outgoing { .. } => {
                let mut effects = vec![Effect::Send {
                    peer: call.peer.clone(),
                    signal: Signal::Hangup {
                        call_id: call.call_id,
                    },
                }];
                effects.extend(self.end("unanswered", now));
                effects
            }
            Phase::Active => Vec::new(),
        }
    }

    /// Our own ICE candidates for the peer.
    pub fn local_candidates(&self, candidates: Vec<Candidate>) -> Vec<Effect> {
        let Some(call) = self.current.as_ref() else {
            return Vec::new();
        };
        vec![Effect::Send {
            peer: call.peer.clone(),
            signal: Signal::Candidates {
                call_id: call.call_id,
                candidates,
            },
        }]
    }

    fn end(&mut self, result: &str, now: i64) -> Vec<Effect> {
        let Some(call) = self.current.take() else {
            return Vec::new();
        };
        vec![Effect::Ended {
            record: CallRecord {
                outgoing: call.outgoing,
                peer: call.peer,
                call_id: call.call_id,
                result: result.to_owned(),
                duration: call.accepted_at.map(|at| (now - at).max(0) / 1000),
                at: call.started_at,
            },
        }]
    }
}

fn missed_call(peer: &str, call_id: u32, now: i64) -> Effect {
    Effect::RecordOnly {
        record: CallRecord {
            peer: peer.to_owned(),
            call_id,
            outgoing: false,
            result: "missed".into(),
            duration: None,
            at: now,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_json_like_android() {
        let signal = Signal::Offer {
            call_id: 42,
            sdp: "v=0".into(),
            video: false,
        };
        let (kind, data) = signal.encode();
        assert_eq!(kind, CallSignalKind::Offer);
        let value: Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(
            value,
            json!({"callId": 42, "offer": {"sdpType": "offer", "sdp": "v=0"}})
        );
        assert_eq!(Signal::decode(kind, &data).unwrap(), signal);

        // Android's video feature: `"features": {"video": null}`
        let android =
            br#"{"callId":7,"offer":{"sdpType":"offer","sdp":"x"},"features":{"video":null}}"#;
        assert_eq!(
            Signal::decode(CallSignalKind::Offer, android).unwrap(),
            Signal::Offer {
                call_id: 7,
                sdp: "x".into(),
                video: true
            }
        );
        assert!(
            Signal::decode(
                CallSignalKind::Offer,
                br#"{"offer":{"sdpType":"answer","sdp":"x"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn answers_candidates_and_empty_bodies() {
        let reject = br#"{"callId":1,"action":0,"rejectReason":1}"#;
        assert_eq!(
            Signal::decode(CallSignalKind::Answer, reject).unwrap(),
            Signal::Reject {
                call_id: 1,
                reason: RejectReason::Busy
            }
        );
        let candidates = br#"{"callId":1,"removed":false,"candidates":[{"candidate":"candidate:1 1 udp 1 1.2.3.4 5 typ host","sdpMid":"0","sdpMLineIndex":0,"ufrag":"ab"}]}"#;
        let Signal::Candidates { candidates, .. } =
            Signal::decode(CallSignalKind::IceCandidates, candidates).unwrap()
        else {
            panic!();
        };
        assert_eq!(candidates[0].sdp_m_line_index, Some(0));
        let (_, encoded) = Signal::Candidates {
            call_id: 1,
            candidates: candidates.clone(),
        }
        .encode();
        assert!(
            String::from_utf8(encoded)
                .unwrap()
                .contains("\"sdpMLineIndex\":0")
        );
        assert_eq!(
            Signal::decode(CallSignalKind::Hangup, b"").unwrap(),
            Signal::Hangup { call_id: 0 }
        );
        assert!(Signal::decode(CallSignalKind::Answer, br#"{"action":1}"#).is_err());
    }

    #[test]
    fn outgoing_call_flow() {
        let mut calls = Calls::default();
        let effects = calls.start("ECHOECHO", 5, "offer".into(), false, 1_000);
        assert!(matches!(
            &effects[0],
            Effect::Send {
                signal: Signal::Offer { call_id: 5, .. },
                ..
            }
        ));
        assert_eq!(
            calls.on_signal("ECHOECHO", Signal::Ringing { call_id: 5 }, 2_000),
            vec![Effect::Ringing]
        );
        // Another call id is ignored.
        assert!(
            calls
                .on_signal("ECHOECHO", Signal::Ringing { call_id: 6 }, 2_000)
                .is_empty()
        );
        let effects = calls.on_signal(
            "ECHOECHO",
            Signal::Accept {
                call_id: 5,
                sdp: "answer".into(),
                video: false,
            },
            3_000,
        );
        assert_eq!(
            effects,
            vec![Effect::RemoteAnswer {
                sdp: "answer".into(),
                video: false,
            }]
        );
        let effects = calls.hang_up(63_000);
        assert!(matches!(
            &effects[0],
            Effect::Send {
                signal: Signal::Hangup { call_id: 5 },
                ..
            }
        ));
        let Effect::Ended { record } = &effects[1] else {
            panic!()
        };
        assert_eq!(
            (record.result.as_str(), record.duration, record.outgoing),
            ("finished", Some(60), true)
        );
        assert!(calls.current.is_none());
    }

    #[test]
    fn incoming_call_flow() {
        let mut calls = Calls::default();
        let effects = calls.on_signal(
            "ECHOECHO",
            Signal::Offer {
                call_id: 9,
                sdp: "offer".into(),
                video: true,
            },
            0,
        );
        assert!(matches!(
            &effects[0],
            Effect::Send {
                signal: Signal::Ringing { call_id: 9 },
                ..
            }
        ));
        assert!(matches!(&effects[1], Effect::Incoming { video: true, .. }));
        // A second caller is busy-rejected, and the chat shows that they tried.
        let busy = calls.on_signal(
            "OTHER123",
            Signal::Offer {
                call_id: 1,
                sdp: "o".into(),
                video: false,
            },
            0,
        );
        assert!(matches!(
            &busy[0],
            Effect::Send {
                signal: Signal::Reject {
                    reason: RejectReason::Busy,
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            &busy[1],
            Effect::RecordOnly { record }
                if record.peer == "OTHER123" && record.call_id == 1 && record.result == "missed"
        ));
        assert!(
            calls.is_current("ECHOECHO", 9),
            "the call in progress goes on"
        );
        // Hung up before we answered: missed.
        let effects = calls.on_signal("ECHOECHO", Signal::Hangup { call_id: 9 }, 5_000);
        let Effect::Ended { record } = &effects[0] else {
            panic!()
        };
        assert_eq!((record.result.as_str(), record.outgoing), ("missed", false));

        calls.on_signal(
            "ECHOECHO",
            Signal::Offer {
                call_id: 10,
                sdp: "o".into(),
                video: false,
            },
            0,
        );
        let effects = calls.accept("answer".into(), false, 1_000);
        assert!(matches!(
            &effects[0],
            Effect::Send {
                signal: Signal::Accept { call_id: 10, .. },
                ..
            }
        ));
        let effects = calls.on_signal("ECHOECHO", Signal::Hangup { call_id: 10 }, 11_000);
        let Effect::Ended { record } = &effects[0] else {
            panic!()
        };
        assert_eq!(
            (record.result.as_str(), record.duration, record.outgoing),
            ("finished", Some(10), false)
        );
    }

    #[test]
    fn hangup_for_an_unknown_call_is_missed_and_ends_nothing() {
        let mut calls = Calls::default();
        let effects = calls.on_signal("ECHOECHO", Signal::Hangup { call_id: 3 }, 0);
        assert!(matches!(
            &effects[..],
            [Effect::RecordOnly { record }] if record.result == "missed" && record.call_id == 3
        ));

        // Not even the call in progress, also with the same peer.
        calls.start("ECHOECHO", 5, "sdp".into(), false, 0);
        let effects = calls.on_signal("ECHOECHO", Signal::Hangup { call_id: 3 }, 0);
        assert!(matches!(&effects[..], [Effect::RecordOnly { .. }]));
        assert!(calls.is_current("ECHOECHO", 5));
    }

    #[test]
    fn an_answer_without_video_makes_it_an_audio_call() {
        let mut calls = Calls::default();
        calls.start("ECHOECHO", 5, "sdp".into(), true, 0);
        let effects = calls.on_signal(
            "ECHOECHO",
            Signal::Accept {
                call_id: 5,
                sdp: "answer".into(),
                video: false,
            },
            0,
        );
        assert_eq!(
            effects,
            vec![Effect::RemoteAnswer {
                sdp: "answer".into(),
                video: false,
            }]
        );
        assert!(!calls.current.unwrap().video);
    }

    #[test]
    fn unanswered_calls_time_out() {
        let mut calls = Calls::default();
        calls.start("ECHOECHO", 5, "sdp".into(), false, 0);
        let effects = calls.time_out(60_000);
        assert!(matches!(&effects[..], [
            Effect::Send { signal: Signal::Hangup { call_id: 5 }, .. },
            Effect::Ended { record }
        ] if record.result == "unanswered" && record.outgoing));

        calls.on_signal(
            "ECHOECHO",
            Signal::Offer {
                call_id: 6,
                sdp: "sdp".into(),
                video: false,
            },
            0,
        );
        let effects = calls.time_out(60_000);
        assert!(matches!(&effects[..], [
            Effect::Send { signal: Signal::Reject { call_id: 6, reason: RejectReason::Timeout }, .. },
            Effect::Ended { record }
        ] if record.result == "missed" && !record.outgoing));
        assert!(calls.current.is_none());
    }
}
