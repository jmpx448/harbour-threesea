// SPDX-License-Identifier: AGPL-3.0-only
//! Call commands and events (ADR 0012): the bridge's media engine (GStreamer webrtcbin) makes
//! and takes SDP and ICE candidates; the core patches the SDP (`sdp.rs`), runs the call state
//! (`calls.rs`), sends the signalling messages and records each call in the chat.
//!
//! Events to the UI: `callIncoming {identity, callId, sdp, video}`, `callRinging`,
//! `callAnswer {sdp, video}`, `callCandidates {candidates}`, `callEnded {record}`.

use libthreema::{common::keys::ClientKey, protobuf::d2d_sync::contact as pc};
use serde_json::{Value, json};

use data_encoding::BASE64;

use crate::{
    app::{ApiError, App, Shared},
    call_video::{self, CaptureDevice, ChannelMessage, VideoParams},
    calls::{CallRecord, Candidate, Effect, RejectReason, Signal},
    connection::ToConnection,
    providers::{CallSignalKind, CoreEvent},
    sdp::{HeaderExtensions, SdpKind, SdpPatcher},
};

/// Whether this build announces call support (feature mask). On since a call with Threema for
/// Android connected and carried audio both ways (2026-10-23); off would let callers' apps hide
/// the call button for us.
pub const CALLS_ANNOUNCED: bool = true;

/// Whether this build announces video call support. On since a video call with Threema for
/// Android worked both ways (2026-10-23); Android refuses video from callers without it.
pub const VIDEO_CALLS_ANNOUNCED: bool = true;

/// The media side of the current call as far as the core keeps it.
#[derive(Debug, Default)]
pub(crate) struct CallMedia {
    /// Through TURN (no MAX video profile then).
    pub relayed: bool,
    /// The peer's video quality profile, once it sent one.
    pub peer_video: Option<VideoParams>,
}

/// Android: RTP header extensions only with video.
fn patcher(video: bool) -> SdpPatcher {
    SdpPatcher::new(if video {
        HeaderExtensions::OneAndTwoByte
    } else {
        HeaderExtensions::Disable
    })
}

/// GStreamer names the codec `OPUS` in its SDP; Android's SdpPatcher (ours too) looks for
/// `opus` and would reject the call.
fn local_sdp(sdp: &str) -> String {
    sdp.replace(" OPUS/48000", " opus/48000")
}

impl App {
    /// Video in calls: calls work here and Settings → Calls → Video calls is on.
    pub(crate) fn video_calls_enabled(&self) -> bool {
        self.calls_enabled() && self.video_calls_setting().unwrap_or(true)
    }

    /// Calls can be made and taken here (Full build, ADR 0012) and the user has them on
    /// (Settings → Calls, Android's "Threema calls"; in Threema Safe and synced).
    pub(crate) fn calls_enabled(&self) -> bool {
        self.calls_supported
            && self
                .app_settings()
                .is_ok_and(|settings| settings.threema_calls)
    }

    fn emit_call(&self, event: Value) {
        let _ = self.media_events.send(CoreEvent::Call(event));
    }

    fn apply_call_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Send { peer, signal } => {
                    self.tell_connection(ToConnection::CallSignal { peer, signal });
                }
                Effect::Incoming {
                    peer,
                    call_id,
                    sdp,
                    video,
                } => {
                    // Video only if we take video calls (else we answer as an audio call).
                    let video = video && self.video_calls_enabled();
                    self.call_media = CallMedia::default();
                    let mut patcher = patcher(video);
                    match patcher.patch(SdpKind::RemoteOffer, &sdp) {
                        Ok(sdp) => {
                            self.call_patcher = Some(patcher);
                            let private = self.store.is_private(&peer).unwrap_or(true);
                            self.emit_call(json!({
                                "event": "callIncoming",
                                "identity": peer,
                                "callId": call_id,
                                "sdp": sdp,
                                "video": video,
                                // Its notification names nobody then (ADR 0009).
                                "private": private,
                            }));
                        }
                        Err(error) => {
                            tracing::warn!(%error, "incoming call offer unusable");
                            let effects = self.calls.fail(crate::messages::now_ms());
                            self.apply_call_effects(effects);
                        }
                    }
                }
                Effect::Ringing => self.emit_call(json!({ "event": "callRinging" })),
                Effect::RemoteAnswer { sdp, video } => {
                    let patched = self
                        .call_patcher
                        .as_mut()
                        .map(|patcher| patcher.patch(SdpKind::RemoteAnswer, &sdp));
                    match patched {
                        Some(Ok(sdp)) => self.emit_call(json!({
                            "event": "callAnswer",
                            "sdp": sdp,
                            "video": video,
                        })),
                        _ => {
                            tracing::warn!("call answer unusable");
                            let effects = self.calls.fail(crate::messages::now_ms());
                            self.apply_call_effects(effects);
                        }
                    }
                }
                Effect::RemoteCandidates(candidates) => {
                    self.emit_call(json!({ "event": "callCandidates", "candidates": candidates }));
                }
                Effect::Ended { record } => {
                    tracing::info!(result = %record.result, outgoing = record.outgoing, "call ended");
                    self.call_patcher = None;
                    self.record_call(&record);
                    self.emit_call(json!({ "event": "callEnded", "record": record }));
                }
                Effect::RecordOnly { record } => self.record_call(&record),
            }
        }
    }

    /// In the chat, like Android: "call|result|outgoing|seconds|callId". The call id is how a
    /// late hangup or offer of this call is recognised, also after a restart (`on_call_signal`).
    fn record_call(&self, record: &CallRecord) {
        let code = format!(
            "call|{}|{}|{}|{}",
            record.result,
            u8::from(record.outgoing),
            record.duration.unwrap_or(0),
            record.call_id
        );
        if let Ok(Some(message)) = self.store.insert_status(&record.peer, &code) {
            let _ = self.media_events.send(CoreEvent::MessageAdded { message });
        }
    }

    /// A call message from the chat server (or reflected from another device).
    pub(crate) fn on_call_signal(&mut self, peer: &str, kind: CallSignalKind, json: &[u8]) {
        let signal = match Signal::decode(kind, json) {
            Ok(signal) => signal,
            Err(error) => {
                tracing::warn!(%error, "call message dropped");
                return;
            }
        };
        if !self.calls_enabled() {
            if let Signal::Offer { call_id, .. } = signal {
                self.note_call_not_taken(peer, call_id);
                self.tell_connection(ToConnection::CallSignal {
                    peer: peer.to_owned(),
                    signal: Signal::Reject {
                        call_id,
                        reason: RejectReason::Disabled,
                    },
                });
            }
            return;
        }
        // A call that is over already: the hangup crossing ours, or an offer delivered again after
        // a lost acknowledgement. Neither is a missed call, and neither touches the current call.
        // No age check on offers: `created_at` is the sender's clock, and the server's short-lived
        // queue (about 30 s) and our ring time bound them anyway.
        let call_id = signal.call_id();
        if matches!(signal, Signal::Offer { .. } | Signal::Hangup { .. })
            && call_id != 0
            && !self.calls.is_current(peer, call_id)
            && self.store.call_recorded(peer, call_id).unwrap_or(false)
        {
            tracing::debug!("signal of a call that is over ignored");
            return;
        }
        let effects = self
            .calls
            .on_signal(peer, signal, crate::messages::now_ms());
        self.apply_call_effects(effects);
    }

    /// A build without calls got an offer, from someone whose app still has it down as taking
    /// them: the chat says that they called, the user never chose not to hear of it. Someone who
    /// turned calls off did. An offer can come twice, after a lost acknowledgement.
    fn note_call_not_taken(&self, peer: &str, call_id: u32) {
        if self.calls_supported || self.store.call_recorded(peer, call_id).unwrap_or(false) {
            return;
        }
        self.record_call(&CallRecord {
            peer: peer.to_owned(),
            call_id,
            outgoing: false,
            result: "missed".to_owned(),
            duration: None,
            at: crate::messages::now_ms(),
        });
    }
}

impl Shared {
    /// A call with `peer` goes only through TURN, which hides our IP address from them: always
    /// for someone unverified or not a contact, so that a stranger can't learn it just by calling
    /// (Android's `VoipCallService`), else if the user asked for it (Settings → Calls "Always
    /// relay calls").
    fn call_relay_only(&self, peer: &str) -> Result<bool, ApiError> {
        if self.app_settings()?.relay_threema_calls {
            return Ok(true);
        }
        Ok(self
            .store
            .contact(peer)?
            .is_none_or(|contact| contact.verification_level == pc::VerificationLevel::Unverified))
    }

    /// `callTurnServers {identity}`: for the media engine's call with `identity` (`{urls,
    /// username, password, relayOnly}`). `relayOnly` needs the dual-stack servers in case the
    /// device has IPv6 only (Android's `PeerConnectionClient.getIceServers`).
    pub(crate) async fn call_turn_servers(&self, peer: &str) -> Result<Value, ApiError> {
        let relay_only = self.call_relay_only(peer)?;
        let now = crate::messages::now_ms();
        let cached = self.turn_servers.borrow().clone();
        if let Some((servers, until)) = cached
            && until > now + 60_000
        {
            return Ok(turn_json(&servers, relay_only).await);
        }
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let servers = crate::directory::turn_servers(
            &self.https,
            &identity.threema_id,
            &ClientKey::from(*identity.client_key),
            "voip",
        )
        .await?;
        let until = now + i64::try_from(servers.expiration).unwrap_or(0) * 1000;
        let result = turn_json(&servers, relay_only).await;
        self.turn_servers.replace(Some((servers, until)));
        Ok(result)
    }
}

impl App {
    /// `callStart`: the engine's offer for `identity`; returns `{callId}`.
    pub(crate) fn call_start(
        &mut self,
        identity: &str,
        sdp: &str,
        video: bool,
    ) -> Result<Value, ApiError> {
        if !self.calls_enabled() || self.calls.current.is_some() {
            return Err(ApiError::Internal("no call possible now".into()));
        }
        self.check_callable(identity)?;
        let video = video && self.video_calls_enabled();
        self.call_media = CallMedia::default();
        let mut patcher = patcher(video);
        let sdp = patcher
            .patch(SdpKind::LocalOffer, &local_sdp(sdp))
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let call_id = loop {
            let id: u32 = rand::random();
            if id != 0 {
                break id;
            }
        };
        self.call_patcher = Some(patcher);
        tracing::info!(video, "call started");
        let effects = self
            .calls
            .start(identity, call_id, sdp, video, crate::messages::now_ms());
        self.apply_call_effects(effects);
        Ok(json!({ "callId": call_id }))
    }

    /// Refused up front, like Android's `VoipUtil`: the signalling wouldn't get through
    /// (`providers::may_receive`), and the call page would wait for the timeout.
    fn check_callable(&self, identity: &str) -> Result<(), ApiError> {
        let (_, conversation) = self.conversation(identity)?;
        conversation.check_writable(&self.app_settings()?.blocked_contacts)?;
        if crate::providers::identity_blocked(&self.store, &self.config, identity)? {
            return Err(ApiError::ContactUnknown);
        }
        Ok(())
    }

    /// Whether a command of the bridge concerns the call in progress. These commands name their
    /// call: the bridge hears of a call's end through an event, and until that arrives another
    /// call may have taken its place here. A hangup, a failure or candidates (with addresses a
    /// relay-only call must not show) meant for the old call must not reach the new one.
    fn is_current_call(&self, call_id: u32) -> bool {
        self.calls
            .current
            .as_ref()
            .is_some_and(|call| call.call_id == call_id)
    }

    /// `callAccept`: the engine's answer to the incoming offer.
    pub(crate) fn call_accept(
        &mut self,
        call_id: u32,
        sdp: &str,
        video: bool,
    ) -> Result<Value, ApiError> {
        if !self.is_current_call(call_id) {
            return Ok(json!({})); // that call's `callEnded` is on its way
        }
        let sdp = self
            .call_patcher
            .as_mut()
            .ok_or_else(|| ApiError::Internal("no incoming call".into()))?
            .patch(SdpKind::LocalAnswer, &local_sdp(sdp))
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let effects = self.calls.accept(sdp, video, crate::messages::now_ms());
        self.apply_call_effects(effects);
        Ok(json!({}))
    }

    /// `callReject` (incoming), `callHangUp` (any).
    pub(crate) fn call_reject(&mut self, call_id: u32) -> Value {
        if self.is_current_call(call_id) {
            let effects = self.calls.reject(false, crate::messages::now_ms());
            self.apply_call_effects(effects);
        }
        json!({})
    }

    /// `callTimeout`: the ring time is over (the bridge's timer).
    pub(crate) fn call_timeout(&mut self, call_id: u32) -> Value {
        if self.is_current_call(call_id) {
            let effects = self.calls.time_out(crate::messages::now_ms());
            self.apply_call_effects(effects);
        }
        json!({})
    }

    pub(crate) fn call_hang_up(&mut self, call_id: u32) -> Value {
        if self.is_current_call(call_id) {
            let effects = self.calls.hang_up(crate::messages::now_ms());
            self.apply_call_effects(effects);
        }
        json!({})
    }

    /// The ID is being removed: whatever call there is ends, the other side is told, and
    /// nothing of it stays for the next identity's first call.
    pub(crate) fn call_end_for_removal(&mut self) {
        let effects = self.calls.hang_up(crate::messages::now_ms());
        self.apply_call_effects(effects);
        self.call_patcher = None;
        self.call_media = CallMedia::default();
    }

    /// `callFailed`: the media side gave up (`reason`: `certificate`, else `connection`).
    pub(crate) fn call_failed(&mut self, call_id: u32, reason: &str) -> Value {
        if self.is_current_call(call_id) {
            let certificate = reason == "certificate";
            tracing::warn!(certificate, "call failed");
            let effects = self.calls.fail(crate::messages::now_ms());
            self.apply_call_effects(effects);
        }
        json!({})
    }

    /// `callCandidates`: our ICE candidates.
    pub(crate) fn call_candidates(&mut self, call_id: u32, candidates: Vec<Candidate>) -> Value {
        if self.is_current_call(call_id) {
            let effects = self.calls.local_candidates(candidates);
            self.apply_call_effects(effects);
        }
        json!({})
    }

    /// `callConnected`: the media is up (`relayed`: through TURN). Sends our video quality
    /// profile to the peer like Android does when its call connects.
    pub(crate) fn call_connected(&mut self, relayed: bool) -> Value {
        self.calls.connected(crate::messages::now_ms());
        self.call_media.relayed = relayed;
        let own = VideoParams::from_setting(&self.video_quality_setting().unwrap_or_default());
        self.send_on_channel(&call_video::encode_quality(&own));
        self.emit_video_params();
        json!({})
    }

    /// `callChannel`: a message from the peer's data channel (`data`: base64).
    pub(crate) fn call_channel(&mut self, data: &str) -> Result<Value, ApiError> {
        let bytes = BASE64
            .decode(data.as_bytes())
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        match call_video::decode(&bytes) {
            Some(ChannelMessage::Capture { device, on }) => self.emit_call(json!({
                "event": "callRemoteCapture",
                "device": match device {
                    CaptureDevice::Camera => "camera",
                    CaptureDevice::Microphone => "microphone",
                },
                "on": on,
            })),
            Some(ChannelMessage::VideoQuality(params)) => {
                self.call_media.peer_video = Some(params);
                self.emit_video_params();
            }
            None => tracing::debug!("unknown call channel message"),
        }
        Ok(json!({}))
    }

    /// `callCapture`: our camera or microphone went on/off; the peer is told.
    pub(crate) fn call_capture(&mut self, device: &str, on: bool) -> Result<Value, ApiError> {
        let device = match device {
            "camera" => CaptureDevice::Camera,
            "microphone" => CaptureDevice::Microphone,
            _ => {
                return Err(ApiError::Internal(format!(
                    "unknown capture device {device}"
                )));
            }
        };
        self.send_on_channel(&call_video::encode_capture(device, on));
        Ok(json!({}))
    }

    fn send_on_channel(&self, data: &[u8]) {
        self.emit_call(json!({ "event": "callChannelSend", "data": BASE64.encode(data) }));
    }

    /// What our video sender may use now (both profiles and the relay considered).
    fn emit_video_params(&self) {
        let own = VideoParams::from_setting(&self.video_quality_setting().unwrap_or_default());
        let params = own.common(self.call_media.peer_video.as_ref(), self.call_media.relayed);
        self.emit_call(json!({
            "event": "callVideoParams",
            "bitrateKbps": params.bitrate_kbps,
            "fps": params.fps,
            "width": params.width,
            "height": params.height,
        }));
    }

    /// `callState`: `{call: {identity, callId, phase, video, startedAt, acceptedAt} | null}`.
    pub(crate) fn call_state(&self) -> Value {
        json!({ "call": self.calls.current.as_ref().map(|call| json!({
            "identity": call.peer,
            "callId": call.call_id,
            "phase": match call.phase {
                crate::calls::Phase::Outgoing { ringing: false } => "calling",
                crate::calls::Phase::Outgoing { ringing: true } => "ringing",
                crate::calls::Phase::Incoming => "incoming",
                crate::calls::Phase::Active => "active",
            },
            "outgoing": call.outgoing,
            "video": call.video,
            "startedAt": call.started_at,
            "acceptedAt": call.accepted_at,
        })) })
    }
}

async fn turn_json(servers: &crate::directory::TurnServers, relay_only: bool) -> Value {
    let urls = if relay_only {
        &servers.turn_urls_dual_stack
    } else {
        &servers.turn_urls
    };
    json!({
        "urls": resolve_turn_urls(urls).await,
        "username": servers.turn_username,
        "password": servers.turn_password,
        "relayOnly": relay_only,
    })
}

/// `turn:host:port?transport=udp` → (`turn:`, host, port, `?transport=udp`).
fn split_turn_url(url: &str) -> Option<(&str, &str, u16, &str)> {
    let (scheme, rest) = url.split_once(':')?;
    let (address, query) = rest.find('?').map_or((rest, ""), |i| rest.split_at(i));
    let (host, port) = address.rsplit_once(':')?;
    Some((scheme, host, port.parse().ok()?, query))
}

/// The TURN servers with their addresses instead of host names: inside Sailjail libnice can't
/// resolve names (GLib's resolver fails there, our own lookups work), so the engine gets IPs.
/// IPv6 addresses only if IPv6 is on (Settings → Advanced). A name that doesn't resolve stays.
async fn resolve_turn_urls(urls: &[String]) -> Vec<String> {
    let ipv4_only = crate::https::IPV4_ONLY.load(std::sync::atomic::Ordering::Relaxed);
    let mut resolved = Vec::new();
    for url in urls {
        let Some((scheme, host, port, query)) = split_turn_url(url) else {
            continue;
        };
        let addresses = match tokio::net::lookup_host((host, port)).await {
            Ok(addresses) => addresses.collect::<Vec<_>>(),
            Err(error) => {
                tracing::warn!(%error, "TURN server not resolved");
                resolved.push(url.clone());
                continue;
            }
        };
        for address in addresses {
            let url = match address {
                std::net::SocketAddr::V4(v4) => format!("{scheme}:{v4}{query}"),
                std::net::SocketAddr::V6(v6) if !ipv4_only => {
                    format!("{scheme}:[{}]:{}{query}", v6.ip(), v6.port())
                }
                std::net::SocketAddr::V6(_) => continue,
            };
            if !resolved.contains(&url) {
                resolved.push(url);
            }
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::StartConfig;

    const OFFER: &str = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n\
                         a=fmtp:111 minptime=10;useinbandfec=1\r\n";

    /// What webrtcbin (GStreamer 1.26, SFOS 5.1) offers (tests/callengine).
    const GST_OFFER: &str = "v=0\r\no=- 3353720450527525677 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n\
        a=ice-options:trickle\r\na=group:BUNDLE audio0 application1\r\n\
        m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=setup:actpass\r\n\
        a=ice-ufrag:dCpF8G5nz8O4z9dHBY1Ag08SI6mX24Fi\r\na=ice-pwd:BvgDBlWCZamluaSkSoDzsn82rAOzirmV\r\n\
        a=rtcp-mux\r\na=rtcp-rsize\r\na=sendrecv\r\na=rtpmap:111 OPUS/48000/2\r\n\
        a=rtcp-fb:111 transport-cc\r\na=fmtp:111 minptime=10;useinbandfec=1\r\na=mid:audio0\r\n\
        a=fingerprint:sha-256 12:62:53:A2:5F:F7:86:AF:45:65:1C:36:F2:BB:AF:85:20:7F:65:84:60:B3:DD:90:F2:47:6E:1F:AA:12:2B:06\r\n\
        a=rtcp-mux-only\r\nm=application 0 UDP/DTLS/SCTP webrtc-datachannel\r\nc=IN IP4 0.0.0.0\r\n\
        a=setup:actpass\r\na=ice-ufrag:dCpF8G5nz8O4z9dHBY1Ag08SI6mX24Fi\r\n\
        a=ice-pwd:BvgDBlWCZamluaSkSoDzsn82rAOzirmV\r\na=bundle-only\r\na=mid:application1\r\n\
        a=sctp-port:5000\r\n\
        a=fingerprint:sha-256 12:62:53:A2:5F:F7:86:AF:45:65:1C:36:F2:BB:AF:85:20:7F:65:84:60:B3:DD:90:F2:47:6E:1F:AA:12:2B:06\r\n";

    #[test]
    fn gstreamer_sdp_passes_the_patcher() {
        for kind in [
            SdpKind::LocalOffer,
            SdpKind::RemoteOffer,
            SdpKind::LocalAnswer,
            SdpKind::RemoteAnswer,
        ] {
            let sdp = patcher(false).patch(kind, &local_sdp(GST_OFFER)).unwrap();
            assert!(sdp.contains("a=rtpmap:111 opus/48000/2\r\n"), "{sdp}");
            assert!(
                sdp.contains(
                    "a=fmtp:111 minptime=10;useinbandfec=1;stereo=0;sprop-stereo=0;cbr=1\r\n"
                ),
                "{sdp}"
            );
            assert!(
                sdp.contains("m=application 0 UDP/DTLS/SCTP webrtc-datachannel\r\n"),
                "{sdp}"
            );
        }
    }

    fn app(
        calls: bool,
    ) -> (
        tempfile::TempDir,
        App,
        tokio::sync::mpsc::UnboundedReceiver<ToConnection>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(
            &StartConfig {
                data_dir: dir.path().to_owned(),
                app_version: "0.1.0".into(),
                language: "en".into(),
                country: "US".into(),
                model: "test".into(),
                os_version: "5.1".into(),
                pictures_dir: None,
                calls,
                log_level: None,
                update_check: false,
            },
            &[3; 32],
        )
        .unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        app.connection.replace(Some(tx));
        (dir, app, rx)
    }

    #[test]
    fn a_contact_that_dropped_forward_secrecy_is_told_and_the_chat_says_so() {
        let (_dir, app, mut rx) = app(false);
        let own: libthreema::common::ThreemaId = "ABCD1234".parse().unwrap();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([3; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        let mut contact = crate::contacts::test_contact("ECHOECHO");
        contact.acquaintance_level =
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::Direct;
        app.store.upsert_contact(&contact).unwrap();
        let client_key = libthreema::common::keys::ClientKey::from([3; 32]);
        let mut fs = crate::fs_store::StoreFs {
            store: std::rc::Rc::clone(&app.store),
            my_identity: own,
            events: crate::providers::EventSink::default(),
        };
        libthreema::csp_e2e::fs::encapsulate(
            &mut fs,
            own,
            &client_key,
            contact.identity,
            &contact.public_key,
            0x01,
            b"hello",
            None,
            1,
        )
        .unwrap();

        app.fs_support_lost(&contact).unwrap();
        assert!(app.store.fs_peers().unwrap().is_empty());
        let Ok(ToConnection::FsControl { peer, peer_key, .. }) = rx.try_recv() else {
            panic!("the Terminate");
        };
        assert_eq!(peer, "ECHOECHO");
        assert!(peer_key == Some(contact.public_key));
        assert!(
            app.store
                .newest_is_status("ECHOECHO", "fsUnavailable")
                .unwrap()
        );

        // Nothing left to end: no line for it.
        app.store.insert_status("ECHOECHO", "fsReset").unwrap();
        app.fs_support_lost(&contact).unwrap();
        assert!(rx.try_recv().is_err());
        assert!(app.store.newest_is_status("ECHOECHO", "fsReset").unwrap());
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn sent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<ToConnection>) -> Vec<Signal> {
        let mut signals = Vec::new();
        while let Ok(message) = rx.try_recv() {
            if let ToConnection::CallSignal { peer, signal } = message {
                assert_eq!(peer, "ECHOECHO");
                signals.push(signal);
            }
        }
        signals
    }

    fn offer(call_id: u32) -> (CallSignalKind, Vec<u8>) {
        Signal::Offer {
            call_id,
            sdp: OFFER.into(),
            video: false,
        }
        .encode()
    }

    #[test]
    fn strangers_and_unverified_contacts_only_get_the_relay() {
        let (_dir, app, _rx) = app(true);
        assert!(app.call_relay_only("ABCD1234").unwrap(), "not a contact");

        let mut contact = crate::contacts::test_contact("ABCD1234");
        contact.verification_level = pc::VerificationLevel::Unverified;
        app.store.upsert_contact(&contact).unwrap();
        assert!(app.call_relay_only("ABCD1234").unwrap(), "unverified");

        for level in [
            pc::VerificationLevel::ServerVerified,
            pc::VerificationLevel::FullyVerified,
        ] {
            contact.verification_level = level;
            app.store.upsert_contact(&contact).unwrap();
            assert!(!app.call_relay_only("ABCD1234").unwrap(), "{level:?}");

            // The setting asks for the relay with everyone.
            let mut settings = app.app_settings().unwrap();
            settings.relay_threema_calls = true;
            app.save_app_settings(&settings).unwrap();
            assert!(app.call_relay_only("ABCD1234").unwrap(), "{level:?}");
            settings.relay_threema_calls = false;
            app.save_app_settings(&settings).unwrap();
        }
    }

    #[test]
    fn calls_nobody_can_answer_are_refused_up_front() {
        use libthreema::protobuf::d2d_sync::contact::{AcquaintanceLevel, ActivityState};
        let (_dir, mut app, mut rx) = app(true);
        let kind = |app: &mut App| {
            let error = app.call_start("BBBBBBBB", OFFER, false).unwrap_err();
            assert!(app.calls.current.is_none());
            error.kind()
        };
        assert_eq!(kind(&mut app), "contactNotFound");

        let mut contact = crate::contacts::test_contact("BBBBBBBB");
        contact.activity_state = ActivityState::Invalid;
        app.store.upsert_contact(&contact).unwrap();
        assert_eq!(kind(&mut app), "contactRevoked");

        contact.activity_state = ActivityState::Active;
        contact.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
        app.store.upsert_contact(&contact).unwrap();
        let mut settings = app.app_settings().unwrap();
        settings.block_unknown = true;
        app.save_app_settings(&settings).unwrap();
        assert_eq!(kind(&mut app), "contactUnknown");

        settings.blocked_contacts = vec!["BBBBBBBB".into()];
        app.save_app_settings(&settings).unwrap();
        assert_eq!(kind(&mut app), "contactBlocked");
        assert!(sent(&mut rx).is_empty());

        // A direct contact who isn't blocked can be called.
        settings.blocked_contacts.clear();
        app.save_app_settings(&settings).unwrap();
        contact.acquaintance_level = AcquaintanceLevel::Direct;
        app.store.upsert_contact(&contact).unwrap();
        assert!(app.call_start("BBBBBBBB", OFFER, false).is_ok());
    }

    #[test]
    fn without_calls_offers_are_rejected() {
        let (_dir, mut app, mut rx) = app(false);
        let (kind, json) = offer(7);
        app.on_call_signal("ECHOECHO", kind, &json);
        assert_eq!(
            sent(&mut rx),
            vec![Signal::Reject {
                call_id: 7,
                reason: RejectReason::Disabled,
            }]
        );
        assert!(app.call_start("ECHOECHO", OFFER, false).is_err());
        assert!(app.calls.current.is_none());
    }

    /// The other side was told "calls are off"; here the chat says they called, and a notification
    /// comes from that line as for any missed call.
    #[test]
    fn without_calls_an_offer_leaves_one_missed_call_line() {
        let (_dir, mut app, mut rx) = app(false);
        let mut events = app.take_media_events().unwrap();
        let (kind, json) = offer(7);
        app.on_call_signal("ECHOECHO", kind, &json);
        let Ok(event) = events.try_recv() else {
            panic!("the line");
        };
        let json_event = event.to_json(&app.store, true);
        assert_eq!(json_event["message"]["body"], "call|missed|0|0|7");
        assert_eq!(json_event["notify"], true);

        // Delivered again: rejected again, no second line.
        let (kind, json) = offer(7);
        app.on_call_signal("ECHOECHO", kind, &json);
        assert_eq!(sent(&mut rx).len(), 2);
        assert!(events.try_recv().is_err());
    }

    /// A hand-made offer in libwebrtc's format for an Android audio call with video on (audio,
    /// video with RTX and encrypted header extensions, data channel), not a captured one.
    const ANDROID_OFFER: &str = include_str!("../../tests/callengine/android-offer.sdp");

    #[test]
    fn android_style_offer_passes_the_patcher() {
        let sdp = patcher(true)
            .patch(SdpKind::RemoteOffer, ANDROID_OFFER)
            .unwrap();
        assert!(sdp.contains("m=video 9 UDP/TLS/RTP/SAVPF"), "{sdp}");
        assert!(
            sdp.contains("a=fmtp:111 minptime=10;useinbandfec=1;stereo=0;sprop-stereo=0;cbr=1\r\n"),
            "{sdp}"
        );
        // Audio-only calls: Android's rules drop the header extensions.
        let sdp = patcher(false)
            .patch(SdpKind::RemoteOffer, ANDROID_OFFER)
            .unwrap();
        assert!(!sdp.contains("a=extmap:"), "{sdp}");
    }

    #[test]
    fn turn_urls_split() {
        assert_eq!(
            split_turn_url("turn:ds-turn-dyn-23.threema.ch:443?transport=udp"),
            Some(("turn", "ds-turn-dyn-23.threema.ch", 443, "?transport=udp"))
        );
        assert_eq!(
            split_turn_url("turns:turn.threema.ch:443"),
            Some(("turns", "turn.threema.ch", 443, ""))
        );
        assert_eq!(split_turn_url("turn:nohost"), None);
    }

    #[test]
    fn turn_urls_resolve_to_addresses() {
        let urls = vec!["turn:127.0.0.1:3478?transport=udp".to_owned()];
        let resolved = rt().block_on(resolve_turn_urls(&urls));
        assert_eq!(resolved, vec!["turn:127.0.0.1:3478?transport=udp"]);
    }

    #[test]
    fn calls_turned_off_reject_offers() {
        let (_dir, mut app, mut rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        app.set_app_setting("threemaCalls", false).unwrap();
        let (kind, json) = offer(8);
        app.on_call_signal("ECHOECHO", kind, &json);
        assert_eq!(
            sent(&mut rx),
            vec![Signal::Reject {
                call_id: 8,
                reason: RejectReason::Disabled,
            }]
        );
        assert!(drain(&mut events).0.is_empty(), "the user's own choice");
    }

    /// What reached the chat, and what reached the bridge.
    fn drain(
        events: &mut tokio::sync::mpsc::UnboundedReceiver<CoreEvent>,
    ) -> (Vec<String>, Vec<Value>) {
        let (mut lines, mut calls) = (Vec::new(), Vec::new());
        while let Ok(event) = events.try_recv() {
            match event {
                CoreEvent::MessageAdded { message, .. } => lines.push(message.body),
                CoreEvent::Call(event) => calls.push(event),
                _ => {}
            }
        }
        (lines, calls)
    }

    #[test]
    fn a_private_chats_call_says_so_for_its_notification() {
        let (_dir, mut app, _rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        app.store.set_private("ECHOECHO", true).unwrap();
        let (kind, json) = offer(7);
        app.on_call_signal("ECHOECHO", kind, &json);
        let Ok(CoreEvent::Call(event)) = events.try_recv() else {
            panic!("no incoming call");
        };
        assert_eq!(event["private"], true);
    }

    #[test]
    fn incoming_call_rings() {
        let (_dir, mut app, mut rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        let (kind, json) = offer(7);
        app.on_call_signal("ECHOECHO", kind, &json);
        let Ok(CoreEvent::Call(event)) = events.try_recv() else {
            panic!("no incoming call");
        };
        assert_eq!(event["event"], "callIncoming");
        assert_eq!(event["callId"], 7);
        assert_eq!(event["private"], false);
        assert!(event["sdp"].as_str().unwrap().contains("cbr=1"));
        assert_eq!(app.call_state()["call"]["phase"], "incoming");
        // The core tells the caller it rings.
        assert_eq!(sent(&mut rx), vec![Signal::Ringing { call_id: 7 }]);

        app.call_reject(7);
        assert!(matches!(
            sent(&mut rx)[..],
            [Signal::Reject { call_id: 7, .. }]
        ));
        assert!(app.call_state()["call"].is_null());
    }

    /// Removing the ID ends the call and tells the UI's engine. The caller is told while the
    /// contact is still there, or the signal can't be encoded; the next ID's first call isn't
    /// refused because of the old one.
    #[tokio::test(flavor = "current_thread")]
    async fn removing_the_id_ends_the_call() {
        let (_dir, mut app, mut rx) = app(true);
        app.store
            .upsert_contact(&crate::contacts::test_contact("ECHOECHO"))
            .unwrap();
        let mut events = app.take_media_events().unwrap();
        let (kind, json) = offer(7);
        app.on_call_signal("ECHOECHO", kind, &json);
        assert!(app.call_patcher.is_some());
        sent(&mut rx);
        while events.try_recv().is_ok() {}

        let store = std::rc::Rc::clone(&app.store);
        tokio::task::LocalSet::new()
            .run_until(async {
                let connection = tokio::task::spawn_local(async move {
                    let Some(ToConnection::CallSignal { signal, .. }) = rx.recv().await else {
                        panic!("the caller is not told");
                    };
                    assert!(matches!(signal, Signal::Reject { call_id: 7, .. }));
                    let Some(ToConnection::Flush(reply)) = rx.recv().await else {
                        panic!("the removal doesn't wait for it");
                    };
                    assert!(
                        store.contact("ECHOECHO").unwrap().is_some(),
                        "not wiped yet"
                    );
                    reply.send(()).unwrap();
                });
                app.delete_all_data().await.unwrap();
                connection.await.unwrap();
            })
            .await;

        assert!(app.calls.current.is_none());
        assert!(app.call_patcher.is_none());
        assert!(
            call_events(&mut events)
                .iter()
                .any(|event| event["event"] == "callEnded"),
            "the UI is told"
        );
        assert!(app.connection.borrow().is_none());
    }

    #[test]
    fn outgoing_call_sends_a_patched_offer_and_records_the_hangup() {
        let (_dir, mut app, mut rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        app.store
            .upsert_contact(&crate::contacts::test_contact("ECHOECHO"))
            .unwrap();
        let call_id = app
            .call_start("ECHOECHO", &OFFER.replace("opus", "OPUS"), false)
            .unwrap()["callId"]
            .as_u64()
            .unwrap();
        assert_ne!(call_id, 0);
        let [Signal::Offer { sdp, .. }] = &sent(&mut rx)[..] else {
            panic!("no offer");
        };
        assert!(sdp.contains("opus/48000") && sdp.contains("cbr=1"));
        assert_eq!(app.call_state()["call"]["phase"], "calling");

        app.call_hang_up(u32::try_from(call_id).unwrap());
        assert!(matches!(sent(&mut rx)[..], [Signal::Hangup { .. }]));
        let Ok(CoreEvent::MessageAdded { message, .. }) = events.try_recv() else {
            panic!("no call record");
        };
        assert_eq!(message.body, format!("call|aborted|1|0|{call_id}"));
        let Ok(CoreEvent::Call(event)) = events.try_recv() else {
            panic!("no end event");
        };
        assert_eq!(event["event"], "callEnded");
        assert_eq!(event["record"]["callId"], call_id);
    }

    /// A call that is taken and hung up, then its peer's hangup comes after ours.
    fn finished_call(
        app: &mut App,
        events: &mut tokio::sync::mpsc::UnboundedReceiver<CoreEvent>,
        call_id: u32,
    ) {
        let (kind, json) = offer(call_id);
        app.on_call_signal("ECHOECHO", kind, &json);
        app.call_accept(call_id, OFFER, false).unwrap();
        app.call_connected(false);
        app.call_hang_up(call_id);
        assert_eq!(
            drain(events).0,
            vec![format!("call|finished|0|0|{call_id}")]
        );
    }

    #[test]
    fn a_hangup_crossing_ours_is_no_missed_call() {
        let (_dir, mut app, _rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        finished_call(&mut app, &mut events, 7);
        let (kind, json) = Signal::Hangup { call_id: 7 }.encode();
        app.on_call_signal("ECHOECHO", kind, &json);
        assert!(events.try_recv().is_err());

        // The memory is the chat's, so this holds after a restart too.
        app.calls = crate::calls::Calls::default();
        app.on_call_signal("ECHOECHO", kind, &json);
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn only_the_chats_last_call_lines_count() {
        let (_dir, app, _rx) = app(true);
        // An imported line has no call id; its seconds are not one.
        app.store
            .insert_status("ECHOECHO", "call|finished|1|7")
            .unwrap();
        assert!(!app.store.call_recorded("ECHOECHO", 7).unwrap());

        app.store
            .insert_status("ECHOECHO", "call|missed|0|0|7")
            .unwrap();
        assert!(app.store.call_recorded("ECHOECHO", 7).unwrap());
        assert!(!app.store.call_recorded("ABCD1234", 7).unwrap());

        // Other status lines in between don't push it out, four later calls do.
        app.store.insert_status("ECHOECHO", "fsReset").unwrap();
        for id in 8..11 {
            app.store
                .insert_status("ECHOECHO", &format!("call|missed|0|0|{id}"))
                .unwrap();
        }
        assert!(app.store.call_recorded("ECHOECHO", 7).unwrap());
        app.store
            .insert_status("ECHOECHO", "call|missed|0|0|11")
            .unwrap();
        assert!(!app.store.call_recorded("ECHOECHO", 7).unwrap());
    }

    #[test]
    fn an_offer_delivered_again_does_not_ring() {
        let (_dir, mut app, mut rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        finished_call(&mut app, &mut events, 7);
        sent(&mut rx);
        let (kind, json) = offer(7);
        app.on_call_signal("ECHOECHO", kind, &json);
        assert!(app.calls.current.is_none());
        assert!(sent(&mut rx).is_empty());
        assert!(call_events(&mut events).is_empty());
    }

    #[test]
    fn a_hangup_for_an_unseen_call_records_a_missed_call_and_ends_nothing() {
        let (_dir, mut app, mut rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        let (kind, json) = Signal::Hangup { call_id: 3 }.encode();
        app.on_call_signal("ECHOECHO", kind, &json);
        // No event for the bridge: it may be setting up the next call right now.
        let (lines, calls) = drain(&mut events);
        assert_eq!(lines, vec!["call|missed|0|0|3"]);
        assert!(calls.is_empty());
        // Once only.
        app.on_call_signal("ECHOECHO", kind, &json);
        assert!(events.try_recv().is_err());
        assert!(sent(&mut rx).is_empty());
    }

    #[test]
    fn a_call_during_a_call_leaves_a_line_and_the_call_goes_on() {
        let (_dir, mut app, mut rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        let (kind, json) = offer(7);
        app.on_call_signal("ECHOECHO", kind, &json);
        assert_eq!(call_events(&mut events)[0]["event"], "callIncoming");
        sent(&mut rx);

        let (kind, json) = offer(8);
        app.on_call_signal("ABCD1234", kind, &json);
        let mut busy = Vec::new();
        while let Ok(message) = rx.try_recv() {
            if let ToConnection::CallSignal { peer, signal } = message {
                busy.push((peer, signal));
            }
        }
        assert_eq!(
            busy,
            vec![(
                "ABCD1234".to_owned(),
                Signal::Reject {
                    call_id: 8,
                    reason: RejectReason::Busy,
                },
            )]
        );
        let (lines, calls) = drain(&mut events);
        assert_eq!(lines, vec!["call|missed|0|0|8"]);
        assert!(calls.is_empty(), "no callEnded");
        assert!(app.call_patcher.is_some());
        assert_eq!(app.call_state()["call"]["phase"], "incoming");
        assert_eq!(app.call_state()["call"]["identity"], "ECHOECHO");
    }

    #[test]
    fn a_request_for_another_call_leaves_the_current_one_alone() {
        let (_dir, mut app, mut rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        let (kind, json) = offer(7);
        app.on_call_signal("ECHOECHO", kind, &json);
        sent(&mut rx);
        call_events(&mut events);

        // The bridge is still on an earlier call: it hasn't heard of this one yet.
        let candidates = vec![Candidate {
            candidate: "candidate:1 1 udp 1 192.0.2.1 5 typ host".into(),
            sdp_mid: Some("0".into()),
            sdp_m_line_index: Some(0),
            ufrag: None,
        }];
        app.call_hang_up(6);
        app.call_failed(6, "other");
        app.call_reject(6);
        app.call_timeout(6);
        app.call_candidates(6, candidates.clone());
        app.call_accept(6, OFFER, false).unwrap();
        assert!(sent(&mut rx).is_empty());
        assert_eq!(drain(&mut events), (Vec::new(), Vec::new()));
        assert_eq!(app.call_state()["call"]["phase"], "incoming");
        assert_eq!(app.call_state()["call"]["callId"], 7);

        // An offer of ours is refused now, and that doesn't touch the call either.
        assert!(app.call_start("ABCD1234", OFFER, false).is_err());
        assert!(sent(&mut rx).is_empty());
        assert_eq!(app.call_state()["call"]["callId"], 7);

        // Its own id reaches it.
        app.call_candidates(7, candidates);
        assert!(matches!(
            sent(&mut rx)[..],
            [Signal::Candidates { call_id: 7, .. }]
        ));
        app.call_timeout(7);
        assert!(matches!(
            sent(&mut rx)[..],
            [Signal::Reject {
                call_id: 7,
                reason: RejectReason::Timeout,
            }]
        ));
        assert!(app.call_state()["call"].is_null());
    }

    #[test]
    fn call_requests_name_their_call() {
        use crate::app::Command;
        let hang_up: Command =
            serde_json::from_str(r#"{"cmd":"callHangUp","callId":4294967295}"#).unwrap();
        assert!(matches!(
            hang_up,
            Command::CallHangUp {
                call_id: 4_294_967_295
            }
        ));
        assert!(serde_json::from_str::<Command>(r#"{"cmd":"callHangUp"}"#).is_err());
    }

    #[test]
    fn a_failed_incoming_call_is_no_declined_call() {
        let (_dir, mut app, mut rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        let (kind, json) = offer(7);
        app.on_call_signal("ECHOECHO", kind, &json);
        sent(&mut rx);
        call_events(&mut events);

        app.call_failed(7, "connection");
        assert_eq!(
            sent(&mut rx),
            vec![Signal::Reject {
                call_id: 7,
                reason: RejectReason::Unknown,
            }]
        );
        assert!(app.calls.current.is_none());
        let (lines, calls) = drain(&mut events);
        assert_eq!(lines, vec!["call|failed|0|0|7"]);
        assert_eq!(calls[0]["event"], "callEnded");
        assert_eq!(calls[0]["record"]["result"], "failed");
    }

    #[test]
    fn an_answer_without_video_reaches_the_ui() {
        let (_dir, mut app, _rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        app.store
            .upsert_contact(&crate::contacts::test_contact("ECHOECHO"))
            .unwrap();
        let call_id = app.call_start("ECHOECHO", OFFER, true).unwrap()["callId"]
            .as_u64()
            .unwrap();
        let (kind, json) = Signal::Accept {
            call_id: u32::try_from(call_id).unwrap(),
            sdp: OFFER.into(),
            video: false,
        }
        .encode();
        app.on_call_signal("ECHOECHO", kind, &json);
        let event = call_events(&mut events).pop().unwrap();
        assert_eq!(event["event"], "callAnswer");
        assert_eq!(event["video"], false);
        assert_eq!(app.call_state()["call"]["video"], false);
    }

    #[test]
    fn an_unusable_offer_is_a_failure_not_a_decline() {
        let (_dir, mut app, mut rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        // No Opus: the patcher refuses it.
        let (kind, json) = Signal::Offer {
            call_id: 7,
            sdp: "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 0\r\n".into(),
            video: false,
        }
        .encode();
        app.on_call_signal("ECHOECHO", kind, &json);
        assert!(sent(&mut rx).contains(&Signal::Reject {
            call_id: 7,
            reason: RejectReason::Unknown,
        }));
        assert_eq!(drain(&mut events).0, vec!["call|failed|0|0|7"]);
    }

    fn call_events(events: &mut tokio::sync::mpsc::UnboundedReceiver<CoreEvent>) -> Vec<Value> {
        let mut found = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let CoreEvent::Call(event) = event {
                found.push(event);
            }
        }
        found
    }

    #[test]
    fn data_channel_messages() {
        let (_dir, mut app, _rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        // Connected: our profile (default "auto" = high) goes to the peer, the sender gets it.
        app.call_connected(false);
        let sent = call_events(&mut events);
        assert_eq!(sent[0]["event"], "callChannelSend");
        let data = BASE64
            .decode(sent[0]["data"].as_str().unwrap().as_bytes())
            .unwrap();
        assert_eq!(
            call_video::decode(&data),
            Some(ChannelMessage::VideoQuality(VideoParams::high()))
        );
        assert_eq!(sent[1]["event"], "callVideoParams");
        assert_eq!(sent[1]["bitrateKbps"], 2000);
        // The peer wants low: the sender follows.
        let low = BASE64.encode(&call_video::encode_quality(&VideoParams::low()));
        app.call_channel(&low).unwrap();
        assert_eq!(call_events(&mut events)[0]["width"], 960);
        // The peer's camera.
        let camera = BASE64.encode(&call_video::encode_capture(CaptureDevice::Camera, true));
        app.call_channel(&camera).unwrap();
        let event = &call_events(&mut events)[0];
        assert_eq!(event["event"], "callRemoteCapture");
        assert_eq!(event["device"], "camera");
        assert_eq!(event["on"], true);
        // Our microphone.
        app.call_capture("microphone", false).unwrap();
        let data = call_events(&mut events)[0]["data"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            call_video::decode(&BASE64.decode(data.as_bytes()).unwrap()),
            Some(ChannelMessage::Capture {
                device: CaptureDevice::Microphone,
                on: false
            })
        );
    }

    #[test]
    fn video_off_answers_as_audio_call() {
        let (_dir, mut app, _rx) = app(true);
        let mut events = app.take_media_events().unwrap();
        app.set_app_setting("videoCalls", false).unwrap();
        assert_eq!(app.settings_json().unwrap()["videoCalls"], false);
        let (kind, json) = Signal::Offer {
            call_id: 9,
            sdp: OFFER.into(),
            video: true,
        }
        .encode();
        app.on_call_signal("ECHOECHO", kind, &json);
        let incoming = call_events(&mut events);
        assert_eq!(incoming[0]["event"], "callIncoming");
        assert_eq!(incoming[0]["video"], false);
    }
}
