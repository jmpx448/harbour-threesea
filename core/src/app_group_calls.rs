// SPDX-License-Identifier: AGPL-3.0-only
//! Group calls in the app (ADR 0012, docs/research/group-calls.md): announced calls are checked
//! and kept per group, and the _Group Call Refresh Steps_ peek them to find the running one.
//!
//! Events to the UI: `groupCall {conversation, call: {callId, startedAt, maxParticipants,
//! participants: [{identity, name}]} | null}` (participants from the call state, if any),
//! `groupCallOffer {sdp}` (a new remote offer: participants came or went),
//! `groupCallParticipants {participants}`, `groupCallAlone` (alone for 3 minutes: leave).

use std::{cell::RefCell, future::Future, rc::Rc};

use libthreema::common::keys::ClientKey;
use libthreema::model::message::GroupCallStartMessage;
use serde_json::{Value, json};

use std::collections::{HashMap, HashSet};

use data_encoding::{BASE64, HEXLOWER};
use libthreema::protobuf::group_call::{
    participant_to_participant::{self, OuterEnvelope},
    participant_to_sfu::{self, participant_microphone},
    sfu_to_participant,
};
use prost::Message as _;
use tokio::task::AbortHandle;

use crate::{
    app::{ApiError, App, Shared},
    chat_notifications::GROUP_CALL_STARTED,
    group_call_handshake::{Event, Known, Local, MediaKey, Peer, rekey},
    group_call_sdp::{Description, Extensions, Session},
    group_calls::{self, CallKeys, Join, Peek, RunningCall, SfuCredentials, StateParticipant},
    https::HttpsClient,
    messages::Message,
    providers::CoreEvent,
    store::{Store, keys},
};

/// Status line: we started the group's call.
const GROUP_CALL_YOU_STARTED: &str = "groupCallYouStarted";
/// Status line: the group has no running call any more.
const GROUP_CALL_ENDED: &str = "groupCallEnded";

/// Whether this build announces group call support (feature mask; Full only, like 1:1 calls).
/// On since joining, starting, audio, video and rekeying worked with Threema for Android
/// (2026-10-26). Without it, other apps don't send us their `GroupCallStart`.
pub const GROUP_CALLS_ANNOUNCED: bool = true;

/// A new key goes out this long before we use it (`group-call.proto`, Join/Leave).
const REKEY_DELAY_MS: i64 = 2000;
/// The designated participant refreshes the call state this often.
const STATE_UPDATE_MS: i64 = 10_000;
/// A new call is announced to the group no sooner than this after the user started it.
const ANNOUNCE_AFTER_MS: i64 = 2000;
/// Alone in a call this long: leave (the SFU ends it after 5 minutes).
const ALONE_MS: i64 = 3 * 60 * 1000;

/// A group's calls are peeked again this often while one runs (Android's refresh steps, which
/// notice a call's end without anyone looking at the chat)...
const REFRESH_MS: u64 = 10_000;
/// ... and this often while none answers, as without a network.
const REFRESH_UNREACHABLE_MS: u64 = 60_000;

/// The pauses before the next try at the SFU credentials for an announced call.
const CREDENTIALS_RETRY: [std::time::Duration; 4] = [
    std::time::Duration::from_secs(5),
    std::time::Duration::from_secs(30),
    std::time::Duration::from_secs(120),
    std::time::Duration::from_secs(600),
];

/// Calls received more than this long ago are dropped after 3 failed peeks in a row (spec).
const STALE_AFTER_MS: i64 = 10 * 3600 * 1000;

/// The SFU credentials, kept until they expire (shared with the refresh tasks).
#[derive(Default)]
pub(crate) struct SfuCache(Option<(SfuCredentials, i64)>);

/// The tasks of group calls, which run beside the core loop for minutes or as long as a call does:
/// retrying the SFU credentials, peeking calls. They hold on to the store, so they go when the ID
/// does ([`App::drop_detached_work`]).
#[derive(Clone, Default)]
pub(crate) struct GroupCallTasks(Rc<RefCell<Vec<AbortHandle>>>);

impl GroupCallTasks {
    fn spawn(&self, work: impl Future<Output = ()> + 'static) {
        let task = tokio::task::spawn_local(work);
        let mut running = self.0.borrow_mut();
        running.retain(|task| !task.is_finished());
        running.push(task.abort_handle());
    }

    pub(crate) fn abort_all(&self) {
        for task in self.0.borrow_mut().drain(..) {
            task.abort();
        }
    }
}

/// What the background tasks need from the app.
#[derive(Clone)]
struct Context {
    store: Rc<Store>,
    https: HttpsClient,
    sfu: Rc<RefCell<SfuCache>>,
    events: tokio::sync::mpsc::UnboundedSender<CoreEvent>,
    /// The groups with a task peeking their calls until they are over.
    watched: Rc<RefCell<HashSet<String>>>,
    tasks: GroupCallTasks,
}

impl Context {
    /// How the UI names a participant: like a contact, else `~nickname`, else the ID.
    fn participant_name(&self, identity: &str, nickname: &str) -> String {
        participant_name(&self.store, identity, nickname)
    }

    /// The SFU credentials; `refresh`: fetch new ones even if cached (after a 401).
    async fn credentials(&self, refresh: bool) -> Result<SfuCredentials, ApiError> {
        let now = crate::messages::now_ms();
        if !refresh
            && let Some((credentials, until)) = &self.sfu.borrow().0
            && *until > now + 10_000
        {
            return Ok(credentials.clone());
        }
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let credentials = crate::directory::sfu_credentials(
            &self.https,
            &identity.threema_id,
            &ClientKey::from(*identity.client_key),
        )
        .await?;
        let until = now + i64::try_from(credentials.expiration).unwrap_or(0) * 1000;
        self.sfu.borrow_mut().0 = Some((credentials.clone(), until));
        Ok(credentials)
    }

    /// Check and store an announced call with its line in the chat, then refresh the group; the
    /// notification waits for that, an announcement read late is of a call long over.
    async fn on_start(&self, conversation: String, caller: String, start: GroupCallStartMessage) {
        // The message is acknowledged and won't come again, so it waits for a working network here.
        let Some(credentials) = with_retries(&CREDENTIALS_RETRY, || self.credentials(false)).await
        else {
            tracing::warn!("group call announcement dropped: no SFU credentials");
            return;
        };
        // The user may have turned group calls off during that time.
        if self
            .store
            .setting(keys::GROUP_CALLS)
            .ok()
            .flatten()
            .as_deref()
            == Some("0")
        {
            return;
        }
        if !group_calls::sfu_url_allowed(
            &start.sfu_base_url,
            &credentials.allowed_sfu_hostname_suffixes,
        ) {
            tracing::warn!("group call announcement with a disallowed SFU URL dropped");
            return;
        }
        let Some(identity) = crate::groups::parse_key(&conversation) else {
            return;
        };
        let call = RunningCall {
            call_id: group_calls::call_id(
                &identity.creator_identity.to_string(),
                &identity.group_id.to_le_bytes(),
                start.protocol_version,
                &start.gck,
                &start.sfu_base_url,
            ),
            conversation: conversation.clone(),
            gck: start.gck,
            sfu_base_url: start.sfu_base_url,
            protocol_version: start.protocol_version,
            received_at: crate::messages::now_ms(),
            failed: 0,
        };
        let line = match self.store.add_group_call(&call) {
            Ok(true) => {
                tracing::info!("group call announced");
                self.started_line(&conversation, &caller)
            }
            Ok(false) => {
                tracing::warn!("group call announced twice (same key)");
                None
            }
            Err(error) => {
                tracing::warn!(%error, "group call not stored");
                None
            }
        };
        let chosen = self.refresh(&conversation).await;
        if let (Some(message), Some((running, _, _))) = (line, chosen) {
            let _ = self.events.send(CoreEvent::GroupCallRunning {
                message,
                call_id: running.call_id,
            });
        }
    }

    /// "Someone started a group call" in the chat. Returns the line if it is one to notify of:
    /// someone else's.
    fn started_line(&self, conversation: &str, caller: &str) -> Option<Message> {
        let own = self.store.identity().ok().flatten()?.threema_id;
        let mine = own == caller;
        let code = if mine {
            GROUP_CALL_YOU_STARTED.to_owned()
        } else {
            format!("{GROUP_CALL_STARTED}{}", self.participant_name(caller, ""))
        };
        let message = add_line(&self.store, &self.events, conversation, &code)?;
        (!mine).then_some(message)
    }

    /// Peek the group again while it has calls, so that their end shows without the chat open.
    /// One task per group.
    fn keep_watching(&self, conversation: &str) {
        if !self.watched.borrow_mut().insert(conversation.to_owned()) {
            return;
        }
        let context = self.clone();
        let conversation = conversation.to_owned();
        let mut pause = REFRESH_MS;
        let tasks = self.tasks.clone();
        tasks.spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(pause)).await;
                let off = context.store.setting(keys::GROUP_CALLS).ok().flatten();
                if off.as_deref() == Some("0")
                    || context
                        .store
                        .group_calls(&conversation)
                        .map_or(true, |calls| calls.is_empty())
                {
                    break;
                }
                pause = match context.refresh_only(&conversation).await {
                    Some(_) => REFRESH_MS,
                    None => REFRESH_UNREACHABLE_MS,
                };
            }
            context.watched.borrow_mut().remove(&conversation);
        });
    }

    /// The _Group Call Refresh Steps_: peek every running call of the group, drop the ended ones,
    /// and tell the UI which one runs (the most recently started).
    async fn refresh(&self, conversation: &str) -> Option<(RunningCall, u64, u32)> {
        let chosen = self.refresh_only(conversation).await;
        if !self
            .store
            .group_calls(conversation)
            .unwrap_or_default()
            .is_empty()
        {
            self.keep_watching(conversation);
        }
        chosen
    }

    /// [`Self::refresh`] without starting the task that repeats it, which is what this is for.
    async fn refresh_only(&self, conversation: &str) -> Option<(RunningCall, u64, u32)> {
        let (chosen, state) = self.chosen(conversation).await;
        let _ = self.events.send(CoreEvent::Call(json!({
            "event": "groupCall",
            "conversation": conversation,
            "call": chosen.as_ref().map(|(call, started_at, max)| json!({
                "callId": data_encoding::HEXLOWER.encode(&call.call_id),
                "startedAt": started_at,
                "maxParticipants": max,
                "participants": state.iter().flatten().map(|(identity, nickname)| json!({
                    "identity": identity,
                    "name": self.participant_name(identity, nickname),
                })).collect::<Vec<_>>(),
            })),
        })));
        chosen
    }

    /// Those people just got the group's state from us: if a call runs, it goes to them as well
    /// (_Active Group Update_ and _State Resync Steps_). Goes by a fresh look at the calls rather
    /// than the last one's result, with nothing for the UI to hear of: this can come from outside
    /// while the app is locked, for a private group too.
    async fn announce_to(&self, conversation: &str, receivers: &[String]) {
        let (Some((call, started_at, _)), _) = self.chosen(conversation).await else {
            return;
        };
        match queue_call_start(&self.store, &call, started_at, receivers) {
            Ok(true) => {
                let _ = self.events.send(CoreEvent::OutboxChanged);
            }
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, "repeating a group call's start failed"),
        }
    }

    /// The _Group Call Refresh Steps_ without telling anyone: the running call, the most recently
    /// started, with who is in it.
    async fn chosen(
        &self,
        conversation: &str,
    ) -> (
        Option<(RunningCall, u64, u32)>,
        Option<Vec<(String, String)>>,
    ) {
        let calls = self.store.group_calls(conversation).unwrap_or_default();
        let mut chosen: Option<(RunningCall, u64, u32)> = None;
        let mut state: Option<Vec<(String, String)>> = None;
        let mut ended = false;
        for call in calls {
            let result = self.peek(&call).await;
            match result {
                Ok(Peek::Running {
                    started_at,
                    max_participants,
                    encrypted_call_state,
                }) => {
                    let _ = self.store.set_group_call_reachable(&call.call_id, true);
                    if call.protocol_version != group_calls::PROTOCOL_VERSION {
                        tracing::warn!(
                            version = call.protocol_version,
                            "group call with an unsupported version running"
                        );
                        continue;
                    }
                    if chosen.as_ref().is_none_or(|(_, at, _)| started_at > *at) {
                        let keys = CallKeys::derive(&call.gck);
                        state = encrypted_call_state
                            .and_then(|sealed| group_calls::opened_call_state(&keys.gcsk, &sealed));
                        chosen = Some((call, started_at, max_participants));
                    }
                }
                Ok(Peek::NotRunning) => {
                    ended |= self.store.remove_group_call(&call.call_id).unwrap_or(false);
                }
                Ok(Peek::Unauthorized) | Err(_) => {
                    if let Err(error) = &result {
                        tracing::warn!(%error, "group call peek failed");
                    }
                    let _ = self.store.set_group_call_reachable(&call.call_id, false);
                    let stale = crate::messages::now_ms() - call.received_at > STALE_AFTER_MS;
                    if call.failed + 1 >= 3 && stale {
                        ended |= self.store.remove_group_call(&call.call_id).unwrap_or(false);
                    }
                }
            }
        }
        if ended {
            ended_line(&self.store, &self.events, conversation);
        }
        tracing::info!(
            running = chosen.is_some(),
            participants_max = chosen.as_ref().map(|(_, _, max)| *max),
            "group call refresh"
        );
        (chosen, state)
    }

    /// Peek, with one token refresh after a 401.
    async fn peek(&self, call: &RunningCall) -> Result<Peek, group_calls::SfuError> {
        for refresh in [false, true] {
            let credentials = self
                .credentials(refresh)
                .await
                .map_err(|error| group_calls::SfuError::Network(error.to_string()))?;
            let peek = group_calls::peek(
                &self.https,
                &call.sfu_base_url,
                &credentials.sfu_token,
                &call.call_id,
            )
            .await?;
            if peek != Peek::Unauthorized {
                return Ok(peek);
            }
        }
        Ok(Peek::Unauthorized)
    }
}

/// How the UI names a participant: like a contact, else `~nickname`, else the ID.
/// Of `members`, those who can take part in a group call.
fn call_receivers(store: &Store, members: impl IntoIterator<Item = String>) -> Vec<String> {
    members
        .into_iter()
        .filter(|member| {
            store.contact(member).ok().flatten().is_some_and(|contact| {
                contact.feature_mask.0 & libthreema::common::FeatureMask::GROUP_CALL_SUPPORT != 0
            })
        })
        .collect()
}

/// Queue a running call's start for `receivers`, those of them still in the group and able to take
/// part. Returns whether anything was queued.
fn queue_call_start(
    store: &Store,
    call: &RunningCall,
    started_at: u64,
    receivers: &[String],
) -> Result<bool, crate::store::StoreError> {
    let Some(group) = store.group(&call.conversation)?.filter(|group| !group.left) else {
        return Ok(false);
    };
    let receivers = call_receivers(
        store,
        receivers
            .iter()
            .filter(|receiver| group.members.contains(receiver))
            .cloned(),
    );
    store.group_outbox_add(
        &call.conversation,
        &receivers,
        crate::groups::OUTBOX_GROUP_CALL_START,
        &json!({
            "gck": HEXLOWER.encode(&call.gck),
            "url": call.sfu_base_url,
            "version": call.protocol_version,
            "startedAt": i64::try_from(started_at).unwrap_or(i64::MAX),
        }),
    )?;
    Ok(!receivers.is_empty())
}

/// `attempt`, then again after each of `pauses`, until it works.
async fn with_retries<T, F, Fut>(pauses: &[std::time::Duration], mut attempt: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, ApiError>>,
{
    let mut pauses = pauses.iter();
    loop {
        match attempt().await {
            Ok(value) => return Some(value),
            Err(error) => {
                tracing::warn!(kind = error.kind(), "group call: no SFU credentials");
                tokio::time::sleep(*pauses.next()?).await;
            }
        }
    }
}

/// A status line in a group's chat, for the UI too; none in a group that is gone or left.
fn add_line(
    store: &Store,
    events: &tokio::sync::mpsc::UnboundedSender<CoreEvent>,
    conversation: &str,
    code: &str,
) -> Option<Message> {
    if store
        .group(conversation)
        .ok()
        .flatten()
        .is_none_or(|group| group.left)
    {
        return None;
    }
    let message = match store.insert_status(conversation, code) {
        Ok(message) => message?,
        Err(error) => {
            tracing::warn!(%error, "group call line not stored");
            return None;
        }
    };
    let _ = events.send(CoreEvent::MessageAdded {
        message: message.clone(),
    });
    Some(message)
}

/// The group's last known call is over: one line for that, however many ended together.
fn ended_line(
    store: &Store,
    events: &tokio::sync::mpsc::UnboundedSender<CoreEvent>,
    conversation: &str,
) {
    if store
        .group_calls(conversation)
        .is_ok_and(|calls| calls.is_empty())
    {
        add_line(store, events, conversation, GROUP_CALL_ENDED);
    }
}

fn participant_name(store: &Store, identity: &str, nickname: &str) -> String {
    match store.contact(identity).ok().flatten() {
        Some(contact) => crate::contacts::display_name(&contact),
        None if !nickname.is_empty() => format!("~{nickname}"),
        None => identity.to_owned(),
    }
}

/// The group call we are in.
pub(crate) struct ActiveGroupCall {
    pub conversation: String,
    pub call_id: [u8; 32],
    pub keys: CallKeys,
    pub local: u32,
    pub description: Description,
    /// The other participants, as the SFU announced them.
    pub participants: Vec<u32>,
    /// `Hello` received.
    pub hello: bool,
    /// The handshake and messages with each other participant.
    pub peers: HashMap<u32, Peer>,
    /// Our media key (applied), then a pending one (sent before it applies).
    pub media_keys: Vec<MediaKey>,
    /// The others' media keys (applied first), from their Auth and rekeys.
    pub their_keys: HashMap<u32, Vec<MediaKey>>,
    /// Our microphone is on (others are told after the handshake and on changes).
    pub microphone: bool,
    /// The others' microphones as they told us (after the handshake they only announce what is
    /// on, so off until then).
    pub their_microphones: HashMap<u32, bool>,
    /// Our camera is on (the others are told, like the microphone).
    pub camera: bool,
    /// The others' cameras that are on (we are subscribed to these).
    pub their_cameras: HashMap<u32, bool>,
    /// When the pending media key (`media_keys[1]`) replaces ours.
    pub rekey_at: Option<i64>,
    /// Someone else left while a key was pending: rekey again once it applies.
    pub rekey_stale: bool,
    /// When we next send the call state (we are the designated participant).
    pub state_update_at: Option<i64>,
    /// A call we started, announced to the group once the SFU said `Hello`.
    pub announce: Option<RunningCall>,
    /// When the user joined or started the call.
    pub started: i64,
    /// Since when nobody else is in the call.
    pub alone_since: Option<i64>,
}

impl ActiveGroupCall {
    /// Someone left: a new random key, one epoch up, to everyone authenticated, used in 2 s
    /// (`group-call.proto` Join/Leave). Returns the data channel messages.
    fn rekey(&mut self, now: i64) -> Vec<Vec<u8>> {
        if self.media_keys.len() > 1 {
            self.rekey_stale = true;
            return Vec::new();
        }
        let next = MediaKey::random(self.media_keys[0].epoch.wrapping_add(1));
        let envelope = rekey(&next);
        let local = self.local;
        let outgoing = self
            .peers
            .values_mut()
            .filter_map(|peer| {
                let id = peer.id;
                peer.envelope(envelope.clone())
                    .map(|data| relay(local, id, data))
            })
            .collect();
        self.media_keys.push(next);
        self.rekey_at = Some(now + REKEY_DELAY_MS);
        outgoing
    }

    /// The participant list changed: are we the one keeping the call state current?
    fn redesignate(&mut self, now: i64) {
        let authenticated = self
            .peers
            .values()
            .filter(|peer| peer.identity().is_some())
            .map(|peer| peer.id);
        if group_calls::designated(self.local, authenticated) {
            self.state_update_at = Some(now);
        } else {
            self.state_update_at = None;
        }
    }

    /// What is due: the pending key, the call state. Returns the data channel messages.
    fn due(&mut self, now: i64, identity: &str, nickname: &str) -> Vec<Vec<u8>> {
        let mut outgoing = Vec::new();
        if self.rekey_at.is_some_and(|at| now >= at) {
            self.rekey_at = None;
            let pending = self.media_keys.remove(1);
            crate::group_call_frames::set_local(&pending);
            self.media_keys[0] = pending;
            if std::mem::take(&mut self.rekey_stale) {
                outgoing.extend(self.rekey(now));
            }
        }
        if self.state_update_at.is_some_and(|at| now >= at) {
            self.state_update_at = Some(now + STATE_UPDATE_MS);
            let mut participants = vec![StateParticipant {
                id: self.local,
                identity,
                nickname,
            }];
            participants.extend(self.peers.values().filter_map(|peer| {
                let (identity, nickname) = peer.identity()?;
                Some(StateParticipant {
                    id: peer.id,
                    identity,
                    nickname,
                })
            }));
            let sealed = group_calls::sealed_call_state(
                &self.keys.gcsk,
                self.local,
                now.cast_unsigned(),
                &participants,
            );
            outgoing.push(to_sfu(
                participant_to_sfu::envelope::Content::UpdateCallState(
                    participant_to_sfu::UpdateCallState {
                        encrypted_call_state: sealed,
                    },
                ),
            ));
        }
        outgoing
    }
}

/// Our camera's capture state for another participant.
fn camera_state(on: bool) -> participant_to_participant::Envelope {
    use participant_to_participant::capture_state::{self, camera};
    let unit = libthreema::protobuf::common::Unit {};
    participant_to_participant::Envelope {
        content: Some(participant_to_participant::envelope::Content::CaptureState(
            participant_to_participant::CaptureState {
                state: Some(capture_state::State::Camera(capture_state::Camera {
                    state: Some(if on {
                        camera::State::On(unit)
                    } else {
                        camera::State::Off(unit)
                    }),
                })),
            },
        )),
        ..Default::default()
    }
}

/// What we ask the SFU for of someone's camera (Android asks for its tile's size at 30 fps).
const CAMERA_WIDTH: u32 = 640;
const CAMERA_HEIGHT: u32 = 360;
const CAMERA_FPS: u32 = 30;

/// Subscribe to (or unsubscribe from) a participant's camera.
fn camera_subscription(participant_id: u32, on: bool) -> Vec<u8> {
    use participant_to_sfu::participant_camera;
    to_sfu(
        participant_to_sfu::envelope::Content::RequestParticipantCamera(
            participant_to_sfu::ParticipantCamera {
                participant_id,
                action: Some(if on {
                    participant_camera::Action::Subscribe(participant_camera::Subscribe {
                        desired_resolution: Some(libthreema::protobuf::common::Resolution {
                            width: CAMERA_WIDTH,
                            height: CAMERA_HEIGHT,
                        }),
                        desired_fps: CAMERA_FPS,
                    })
                } else {
                    participant_camera::Action::Unsubscribe(participant_camera::Unsubscribe {})
                }),
            },
        ),
    )
}

/// Joining a call with this many others starts with the microphone off, so a big room doesn't
/// hear us before we meant to speak (Android's `MUTE_MICROPHONE_PARTICIPANT_THRESHOLD`).
fn starts_muted(others: usize) -> bool {
    others >= 4
}

/// Our microphone's capture state for another participant.
fn microphone_state(on: bool) -> participant_to_participant::Envelope {
    use participant_to_participant::capture_state::{self, microphone};
    let unit = libthreema::protobuf::common::Unit {};
    participant_to_participant::Envelope {
        content: Some(participant_to_participant::envelope::Content::CaptureState(
            participant_to_participant::CaptureState {
                state: Some(capture_state::State::Microphone(
                    capture_state::Microphone {
                        state: Some(if on {
                            microphone::State::On(unit)
                        } else {
                            microphone::State::Off(unit)
                        }),
                    },
                )),
            },
        )),
        ..Default::default()
    }
}

/// A message for another participant, relayed by the SFU.
fn relay(sender: u32, receiver: u32, encrypted_data: Vec<u8>) -> Vec<u8> {
    to_sfu(participant_to_sfu::envelope::Content::Relay(
        OuterEnvelope {
            sender,
            receiver,
            encrypted_data,
        },
    ))
}

/// A `ParticipantToSfu.Envelope` with random padding.
fn to_sfu(content: participant_to_sfu::envelope::Content) -> Vec<u8> {
    #[expect(
        deprecated,
        reason = "padding as the protocol describes it, random length"
    )]
    let envelope = participant_to_sfu::Envelope {
        padding: vec![0; usize::from(rand::random::<u8>() % 32)],
        content: Some(content),
    };
    envelope.encode_to_vec()
}

impl Shared {
    fn group_call_context(&self) -> Context {
        Context {
            store: Rc::clone(&self.store),
            https: self.https.clone(),
            sfu: Rc::clone(&self.sfu),
            events: self.media_events.clone(),
            watched: Rc::clone(&self.group_calls_watched),
            tasks: self.group_call_tasks.clone(),
        }
    }

    /// This device takes part in group calls: a build with calls (ADR 0012), and the user didn't
    /// turn them off (Android's group call policy). Otherwise it stays out of them: no SFU
    /// credentials, no peek at the SFU, no stored call key, no banner, no notification.
    fn group_calls_wanted(&self) -> bool {
        self.calls_supported && self.group_calls_setting().unwrap_or(true)
    }
}

impl App {
    /// A `GroupCallStart` arrived in a group.
    pub(crate) fn on_group_call_start(
        &self,
        conversation: &str,
        caller: &str,
        start: &GroupCallStartMessage,
    ) {
        // Read from the queue after the wipe: there is nobody left to store it for.
        if self.identity_removed || !self.group_calls_wanted() {
            return;
        }
        let context = self.group_call_context();
        let (conversation, caller, start) =
            (conversation.to_owned(), caller.to_owned(), start.clone());
        self.group_call_tasks
            .spawn(async move { context.on_start(conversation, caller, start).await });
    }

    /// The call this line announces was found running: a notification, unless we are in it.
    pub(crate) fn on_group_call_running(&self, message: &Message, call_id: &[u8; 32]) {
        // Joining counts: the answer may take a while beside the core loop, and whichever of the
        // group's calls this line announces, the join ends up in the one that runs.
        if self
            .group_call
            .as_ref()
            .is_some_and(|joined| joined.call_id == *call_id)
            || self.group_calls_joining.contains(&message.conversation)
        {
            return;
        }
        let _ = self.media_events.send(CoreEvent::Notify(message.clone()));
    }

    /// Calls kept from the last run: peeked at the start, like Android, which also notices the
    /// ones that ended meanwhile.
    pub(crate) fn refresh_stored_group_calls(&self) {
        if !self.group_calls_wanted() {
            return;
        }
        for conversation in self.store.groups_with_calls().unwrap_or_default() {
            self.group_call_refresh(&conversation);
        }
    }

    /// Those people just got the group's state from us; see [`Context::announce_to`]. Nothing
    /// to do, and nobody to ask, if we know of no call.
    pub(crate) fn announce_running_group_call(&self, conversation: &str, receivers: Vec<String>) {
        if !self.group_calls_wanted()
            || self
                .store
                .group_calls(conversation)
                .unwrap_or_default()
                .is_empty()
        {
            return;
        }
        let context = self.group_call_context();
        let conversation = conversation.to_owned();
        self.group_call_tasks.spawn(async move {
            context.announce_to(&conversation, &receivers).await;
        });
    }
}

impl Shared {
    /// `groupCallRefresh {conversation}`: peek the group's calls now (the chat shows a running
    /// call); answers with the `groupCall` event.
    pub(crate) fn group_call_refresh(&self, conversation: &str) -> Value {
        if !self.group_calls_wanted() {
            return json!({});
        }
        let context = self.group_call_context();
        let conversation = conversation.to_owned();
        self.group_call_tasks.spawn(async move {
            context.refresh(&conversation).await;
        });
        json!({})
    }
}

impl App {
    /// `groupCallJoin {conversation, fingerprint, create}`: join the group's running call, or
    /// with `create` start one if none runs (_Create or Join_, _Group Call Join Steps_):
    /// `{offer, candidates, participantId}` for the media engine, which answers the offer itself.
    pub(crate) async fn group_call_join(
        &mut self,
        conversation: String,
        fingerprint: String,
        create: bool,
    ) -> Result<Value, ApiError> {
        if !self.group_calls_wanted() {
            return Err(ApiError::GroupCallNotRunning);
        }
        let started = crate::messages::now_ms();
        let fingerprint = HEXLOWER
            .decode(fingerprint.to_ascii_lowercase().as_bytes())
            .map_err(|_| ApiError::Internal("invalid fingerprint".into()))?;
        let leaves = self.group_call_leaves;
        self.group_calls_joining.push(conversation.clone());
        self.detached_then(
            crate::detached::Lane::Free,
            {
                let conversation = conversation.clone();
                move |app| async move { app.join_at_sfu(&conversation, &fingerprint, create).await }
            },
            move |app, joined| {
                if let Some(at) = app
                    .group_calls_joining
                    .iter()
                    .position(|joining| *joining == conversation)
                {
                    app.group_calls_joining.remove(at);
                }
                let (call, new, response) = joined?;
                // Left while the SFU took its time: the page has gone, and the SFU drops someone
                // who never shows up.
                if app.group_call_leaves != leaves {
                    return Err(ApiError::GroupCallNotRunning);
                }
                Ok(app.joined_group_call(&conversation, call, new, &response, started))
            },
        )
        .await
    }

    /// What this device takes from the SFU for a join.
    fn joined_group_call(
        &mut self,
        conversation: &str,
        call: RunningCall,
        new: bool,
        response: &libthreema::protobuf::group_call::sfu_http_response::Join,
        started: i64,
    ) -> Value {
        tracing::info!(
            participant = response.participant_id,
            features = response.supported_features,
            "group call joined"
        );
        let ipv4_only = crate::https::IPV4_ONLY.load(std::sync::atomic::Ordering::Relaxed);
        let candidates: Vec<Value> = response
            .addresses
            .iter()
            .filter(|address| !(ipv4_only && address.ip.contains(':')))
            .map(|address| {
                let priority = if address.ip.contains(':') { 2 } else { 1 };
                json!({
                    "candidate": format!("candidate:0 1 udp {priority} {} {} typ host", address.ip, address.port),
                    "sdpMid": "",
                    "sdpMLineIndex": 0,
                })
            })
            .collect();
        let session = Session {
            local: response.participant_id,
            ice_ufrag: response.ice_username_fragment.clone(),
            ice_pwd: response.ice_password.clone(),
            dtls_fingerprint: response.dtls_fingerprint.clone(),
            extensions: Extensions::from_join(response.rtp_header_extension_ids.as_ref()),
        };
        let mut description = Description::new(session);
        let offer = description.remote_offer(&[]);
        let keys = CallKeys::derive(&call.gck);
        let media_key = MediaKey::random(0);
        crate::group_call_frames::reset(keys.gckh);
        crate::group_call_frames::set_local(&media_key);
        self.group_call = Some(ActiveGroupCall {
            conversation: conversation.to_owned(),
            call_id: call.call_id,
            keys,
            local: response.participant_id,
            description,
            participants: Vec::new(),
            hello: false,
            peers: HashMap::new(),
            media_keys: vec![media_key],
            their_keys: HashMap::new(),
            // On unless the call turns out to be big (see `starts_muted`); the UI can mute.
            microphone: true,
            their_microphones: HashMap::new(),
            camera: false,
            their_cameras: HashMap::new(),
            rekey_at: None,
            rekey_stale: false,
            state_update_at: None,
            announce: new.then_some(call),
            started,
            alone_since: None,
        });
        json!({
            "offer": offer,
            "candidates": candidates,
            "participantId": response.participant_id,
        })
    }
}

impl Shared {
    /// The network's part of [`App::group_call_join`]: the call, whether it is a new one, and what
    /// the SFU said.
    async fn join_at_sfu(
        &self,
        conversation: &str,
        fingerprint: &[u8],
        create: bool,
    ) -> Result<
        (
            RunningCall,
            bool,
            libthreema::protobuf::group_call::sfu_http_response::Join,
        ),
        ApiError,
    > {
        let context = self.group_call_context();
        let (call, new) = match context.refresh(conversation).await {
            Some((call, _, _)) => (call, false),
            None if create => (self.new_group_call(&context, conversation).await?, true),
            None => return Err(ApiError::GroupCallNotRunning),
        };
        let mut joined = None;
        for refresh in [false, true] {
            let credentials = context.credentials(refresh).await?;
            match group_calls::join(
                &self.https,
                &call.sfu_base_url,
                &credentials.sfu_token,
                &call.call_id,
                fingerprint,
            )
            .await
            .map_err(|error| ApiError::Network(error.to_string()))?
            {
                Join::Joined(response) => {
                    joined = Some(response);
                    break;
                }
                Join::Unauthorized => continue,
                Join::Full => return Err(ApiError::GroupCallFull),
                Join::UnsupportedVersion => return Err(ApiError::GroupCallVersion),
            }
        }
        let response = joined.ok_or_else(|| ApiError::Network("SFU token refused".into()))?;
        Ok((call, new, response))
    }

    /// A new call for the group (not sent yet): a random GCK on our SFU.
    async fn new_group_call(
        &self,
        context: &Context,
        conversation: &str,
    ) -> Result<RunningCall, ApiError> {
        let group = self
            .store
            .group(conversation)?
            .filter(|group| !group.left)
            .ok_or(ApiError::GroupCallNotRunning)?;
        let credentials = context.credentials(false).await?;
        if !group_calls::sfu_url_allowed(
            &credentials.sfu_base_url,
            &credentials.allowed_sfu_hostname_suffixes,
        ) {
            return Err(ApiError::Internal("SFU URL not allowed".into()));
        }
        let gck: [u8; 32] = rand::random();
        Ok(RunningCall {
            call_id: group_calls::call_id(
                &group.identity.creator_identity.to_string(),
                &group.identity.group_id.to_le_bytes(),
                group_calls::PROTOCOL_VERSION,
                &gck,
                &credentials.sfu_base_url,
            ),
            conversation: conversation.to_owned(),
            gck,
            sfu_base_url: credentials.sfu_base_url,
            protocol_version: group_calls::PROTOCOL_VERSION,
            received_at: crate::messages::now_ms(),
            failed: 0,
        })
    }
}

impl App {
    /// Our new call runs: keep it and send `GroupCallStart` to the members that support group
    /// calls (through the outbox, like the other group messages).
    fn announce_group_call(&self, call: &RunningCall) -> Result<(), ApiError> {
        let user = self
            .store
            .identity()?
            .ok_or(ApiError::NoIdentity)?
            .threema_id;
        if self.store.add_group_call(call)? {
            add_line(
                &self.store,
                &self.media_events,
                &call.conversation,
                GROUP_CALL_YOU_STARTED,
            );
        }
        let members = self
            .store
            .group(&call.conversation)?
            .map(|group| group.members)
            .unwrap_or_default();
        let receivers = call_receivers(
            &self.store,
            members.into_iter().filter(|member| *member != user),
        );
        tracing::info!(receivers = receivers.len(), "group call started");
        self.store.group_outbox_add(
            &call.conversation,
            &receivers,
            crate::groups::OUTBOX_GROUP_CALL_START,
            &json!({
                "gck": HEXLOWER.encode(&call.gck),
                "url": call.sfu_base_url,
                "version": call.protocol_version,
            }),
        )?;
        self.tell_connection(crate::connection::ToConnection::SendPending);
        self.group_call_refresh(&call.conversation);
        Ok(())
    }

    /// `groupCallTick`: every second while in a call.
    pub(crate) fn group_call_tick(&mut self) -> Result<Value, ApiError> {
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let nickname = self.store.setting(keys::NICKNAME)?.unwrap_or_default();
        let now = crate::messages::now_ms();
        let Some(call) = self.group_call.as_mut() else {
            return Ok(json!({}));
        };
        let outgoing = call.due(now, &identity.threema_id, &nickname);
        let announce = if call.hello && now - call.started >= ANNOUNCE_AFTER_MS {
            call.announce.take()
        } else {
            None
        };
        let alone = call
            .alone_since
            .is_some_and(|since| now - since >= ALONE_MS);
        if alone {
            call.alone_since = None;
        }
        for data in outgoing {
            self.emit_group_call(
                json!({ "event": "groupCallChannelSend", "data": BASE64.encode(&data) }),
            );
        }
        if let Some(call) = announce {
            self.announce_group_call(&call)?;
        }
        if alone {
            tracing::info!("group call: alone for 3 minutes, leaving");
            self.emit_group_call(json!({ "event": "groupCallAlone" }));
        }
        Ok(json!({}))
    }

    /// `groupCallChannel {data}`: a message from the SFU's data channel (base64).
    pub(crate) fn group_call_channel(&mut self, data: &str) -> Result<Value, ApiError> {
        use sfu_to_participant::envelope::Content;
        let bytes = BASE64
            .decode(data.as_bytes())
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let envelope = sfu_to_participant::Envelope::decode(bytes.as_slice())
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let nickname = self.store.setting(keys::NICKNAME)?.unwrap_or_default();
        let client_key = ClientKey::from(*identity.client_key);
        let members = self
            .group_call
            .as_ref()
            .and_then(|call| self.store.group(&call.conversation).ok().flatten())
            .map(|group| group.members)
            .unwrap_or_default();
        let shared = Rc::clone(&self.shared);
        let Some(call) = self.group_call.as_mut() else {
            return Ok(json!({}));
        };
        let mut outgoing = Vec::new();
        let mut changed = false;
        let mut participants_changed = false;
        let mut microphone_off = false;
        match envelope.content {
            Some(Content::Hello(hello)) => {
                if call.hello {
                    tracing::warn!("second Hello from the SFU ignored");
                } else {
                    tracing::info!(
                        participants = hello.participant_ids.len(),
                        "group call: Hello"
                    );
                    call.hello = true;
                    call.participants = hello.participant_ids;
                    changed = true;
                    // Before the handshakes: the others can't decrypt our audio until then, and
                    // the state they are told afterwards reads this.
                    if starts_muted(call.participants.len()) {
                        call.microphone = false;
                        microphone_off = true;
                    }
                    // We are the newcomer: our Hello to everyone.
                    let local = Local {
                        identity: &identity.threema_id,
                        nickname: &nickname,
                        client_key: &client_key,
                        gckh: &call.keys.gckh,
                        gchk: &call.keys.gchk,
                        media_keys: &call.media_keys,
                    };
                    for id in call.participants.clone() {
                        let (peer, hello) = Peer::joined(id, &local);
                        call.peers.insert(id, peer);
                        outgoing.push(relay(call.local, id, hello));
                    }
                }
            }
            Some(Content::ParticipantJoined(joined)) => {
                tracing::info!("group call: participant joined");
                let id = joined.participant_id;
                if !call.participants.contains(&id) {
                    call.participants.push(id);
                    changed = true;
                    // Someone new: our key moves one ratchet step on, then its handshake.
                    match call.media_keys[0].ratcheted() {
                        Some(next) => {
                            crate::group_call_frames::set_local(&next);
                            call.media_keys[0] = next;
                        }
                        None => tracing::warn!("group call: media key ratchet exhausted"),
                    }
                    call.peers.insert(id, Peer::newcomer(id));
                }
            }
            Some(Content::ParticipantLeft(left)) => {
                tracing::info!("group call: participant left");
                let before = call.participants.len();
                call.participants.retain(|id| *id != left.participant_id);
                call.peers.remove(&left.participant_id);
                call.their_keys.remove(&left.participant_id);
                call.their_microphones.remove(&left.participant_id);
                call.their_cameras.remove(&left.participant_id);
                crate::group_call_frames::remove_peer(left.participant_id);
                changed = before != call.participants.len();
                participants_changed = changed;
                if changed {
                    outgoing.extend(call.rekey(crate::messages::now_ms()));
                }
            }
            Some(Content::Relay(relayed)) => {
                if relayed.receiver != call.local {
                    tracing::warn!("group call: relayed message for someone else dropped");
                } else if let Some(peer) = call.peers.get_mut(&relayed.sender) {
                    let local = Local {
                        identity: &identity.threema_id,
                        nickname: &nickname,
                        client_key: &client_key,
                        gckh: &call.keys.gckh,
                        gchk: &call.keys.gchk,
                        media_keys: &call.media_keys,
                    };
                    let user = identity.threema_id.clone();
                    let store = &shared.store;
                    let lookup = |claimed: &str| {
                        if claimed == user {
                            return Some(Known {
                                public_key: client_key.public_key(),
                                member: true,
                            });
                        }
                        let contact = store.contact(claimed).ok().flatten()?;
                        Some(Known {
                            public_key: contact.public_key,
                            member: members.iter().any(|member| member == claimed),
                        })
                    };
                    match peer.receive(&relayed.encrypted_data, &local, &lookup) {
                        Ok(events) => {
                            for event in events {
                                match event {
                                    Event::Send(data) => outgoing.push(relay(call.local, relayed.sender, data)),
                                    Event::Authenticated { media_keys, .. } => {
                                        tracing::info!("group call: participant authenticated");
                                        crate::group_call_frames::add_peer_keys(relayed.sender, &media_keys);
                                        call.their_keys.insert(relayed.sender, media_keys);
                                        participants_changed = true;
                                        // Post-handshake: our microphone's and camera's state, and
                                        // hear them.
                                        let (on, camera) = (call.microphone, call.camera);
                                        if let Some(peer) = call.peers.get_mut(&relayed.sender) {
                                            if let Some(data) = peer.envelope(microphone_state(on)) {
                                                outgoing.push(relay(call.local, relayed.sender, data));
                                            }
                                            if camera && let Some(data) = peer.envelope(camera_state(true)) {
                                                outgoing.push(relay(call.local, relayed.sender, data));
                                            }
                                        }
                                        outgoing.push(to_sfu(
                                            participant_to_sfu::envelope::Content::RequestParticipantMicrophone(
                                                participant_to_sfu::ParticipantMicrophone {
                                                    participant_id: relayed.sender,
                                                    action: Some(participant_microphone::Action::Subscribe(
                                                        participant_microphone::Subscribe {},
                                                    )),
                                                },
                                            ),
                                        ));
                                    }
                                    Event::Message(envelope) => match envelope.content {
                                        Some(participant_to_participant::envelope::Content::Rekey(key)) => {
                                            if let Some(key) = crate::group_call_handshake::media_key(&key) {
                                                crate::group_call_frames::add_peer_keys(
                                                    relayed.sender,
                                                    std::slice::from_ref(&key),
                                                );
                                                if let Some(keys) = call.their_keys.get_mut(&relayed.sender) {
                                                    keys.push(key);
                                                }
                                            }
                                        }
                                        Some(participant_to_participant::envelope::Content::CaptureState(state)) => {
                                            use participant_to_participant::capture_state::{State, camera, microphone};
                                            match state.state {
                                                Some(State::Microphone(mic)) => {
                                                    let on = matches!(mic.state, Some(microphone::State::On(_)));
                                                    call.their_microphones.insert(relayed.sender, on);
                                                    participants_changed = true;
                                                }
                                                // Their video only while their camera is on.
                                                Some(State::Camera(cam)) => {
                                                    let on = matches!(cam.state, Some(camera::State::On(_)));
                                                    let was = call.their_cameras.insert(relayed.sender, on).unwrap_or(false);
                                                    if on != was {
                                                        outgoing.push(camera_subscription(relayed.sender, on));
                                                    }
                                                    participants_changed = true;
                                                }
                                                _ => {}
                                            }
                                        }
                                        _ => {}
                                    },
                                }
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "group call: participant message refused")
                        }
                    }
                }
            }
            Some(Content::TimestampResponse(timestamp)) => {
                tracing::debug!(ms = timestamp.ms, "group call: SFU timestamp (alive)");
            }
            None => {}
        }
        let now = crate::messages::now_ms();
        if changed || participants_changed {
            call.redesignate(now);
            call.alone_since = if call.participants.is_empty() {
                call.alone_since.or(Some(now))
            } else {
                None
            };
        }
        outgoing.extend(call.due(now, &identity.threema_id, &nickname));
        if microphone_off {
            self.emit_group_call(json!({ "event": "groupCallMicrophoneOff" }));
        }
        for data in outgoing {
            self.emit_group_call(
                json!({ "event": "groupCallChannelSend", "data": BASE64.encode(&data) }),
            );
        }
        let Some(call) = self.group_call.as_mut() else {
            return Ok(json!({}));
        };
        if changed {
            let participants = call.participants.clone();
            let sdp = call.description.remote_offer(&participants);
            self.emit_group_call(json!({ "event": "groupCallOffer", "sdp": sdp }));
        }
        if changed || participants_changed {
            self.emit_group_call_participants();
        }
        Ok(json!({}))
    }

    /// `groupCallParticipants {participants: [{id, microphoneMid, cameraMid, identity, nickname,
    /// name, pictureVersion, microphone, camera, authenticated}]}`.
    pub(crate) fn emit_group_call_participants(&self) {
        let Some(call) = self.group_call.as_ref() else {
            return;
        };
        let participants: Vec<Value> = call
            .participants
            .iter()
            .map(|id| {
                let identity = call.peers.get(id).and_then(Peer::identity);
                let mids = crate::group_call_sdp::Mids::of(*id);
                json!({
                    "id": id,
                    // The media engine finds a stream's participant by its MID.
                    "microphoneMid": mids.as_ref().map(|mids| mids.microphone.clone()),
                    "cameraMid": mids.as_ref().map(|mids| mids.camera.clone()),
                    "identity": identity.map(|(identity, _)| identity),
                    "nickname": identity.map(|(_, nickname)| nickname),
                    "name": identity.map(|(identity, nickname)| participant_name(&self.store, identity, nickname)),
                    "pictureVersion": identity
                        .and_then(|(identity, _)| self.store.shown_contact_picture_version(identity).ok())
                        .unwrap_or(0),
                    "microphone": call.their_microphones.get(id).copied().unwrap_or(false),
                    "camera": call.their_cameras.get(id).copied().unwrap_or(false),
                    "authenticated": identity.is_some(),
                })
            })
            .collect();
        self.emit_group_call(json!({
            "event": "groupCallParticipants",
            "conversation": call.conversation,
            "participants": participants,
        }));
    }

    /// The app locked (§7.8): the UI forgets the running calls of private groups, and of the call
    /// we are in, which goes on, keeps no names of a private group's people.
    pub(crate) fn group_calls_locked(&self) {
        if !self.calls_supported {
            return;
        }
        let keep: Vec<String> = self
            .store
            .groups_with_calls()
            .unwrap_or_default()
            .into_iter()
            .filter(|conversation| !self.store.is_private(conversation).unwrap_or(true))
            .collect();
        self.emit_group_call(json!({ "event": "groupCallsLocked", "keep": keep }));
        self.emit_group_call_participants();
    }

    /// `groupCallMute {muted}`: tell the others (the media engine mutes the microphone itself).
    pub(crate) fn group_call_mute(&mut self, muted: bool) -> Value {
        let Some(call) = self.group_call.as_mut() else {
            return json!({});
        };
        call.microphone = !muted;
        let local = call.local;
        let mut outgoing = Vec::new();
        for peer in call.peers.values_mut() {
            if let Some(data) = peer.envelope(microphone_state(!muted)) {
                outgoing.push(relay(local, peer.id, data));
            }
        }
        for data in outgoing {
            self.emit_group_call(
                json!({ "event": "groupCallChannelSend", "data": BASE64.encode(&data) }),
            );
        }
        json!({})
    }

    /// `groupCallCamera {on}`: tell the others (the media engine starts or stops the camera).
    pub(crate) fn group_call_camera(&mut self, on: bool) -> Value {
        let Some(call) = self.group_call.as_mut() else {
            return json!({});
        };
        call.camera = on;
        let local = call.local;
        let mut outgoing = Vec::new();
        for peer in call.peers.values_mut() {
            if let Some(data) = peer.envelope(camera_state(on)) {
                outgoing.push(relay(local, peer.id, data));
            }
        }
        for data in outgoing {
            self.emit_group_call(
                json!({ "event": "groupCallChannelSend", "data": BASE64.encode(&data) }),
            );
        }
        json!({})
    }

    /// `groupCallLeave`: we left (the media engine closed the connection).
    pub(crate) fn group_call_leave(&mut self) -> Value {
        self.group_call_leaves += 1;
        self.group_calls_joining.clear();
        crate::group_call_frames::reset([0; 32]);
        if let Some(call) = self.group_call.take() {
            self.group_call_refresh(&call.conversation);
        }
        json!({})
    }

    /// The ID is being removed: the call's state and frame keys go, without the refresh a
    /// leave asks for (nothing is left to show it).
    pub(crate) fn group_call_forget(&mut self) {
        self.group_call_leaves += 1;
        self.group_calls_joining.clear();
        crate::group_call_frames::reset([0; 32]);
        self.group_call = None;
    }

    fn emit_group_call(&self, event: Value) {
        let _ = self.media_events.send(CoreEvent::Call(event));
    }
}

#[cfg(test)]
mod app_tests {
    use libthreema::model::message::GroupCallStartMessage;

    use super::*;
    use crate::{app::StartConfig, store::StoredIdentity};

    fn app(calls: bool) -> (tempfile::TempDir, App) {
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
        (dir, app)
    }

    /// No `LocalSet` here: anything spawned for the SFU would panic, so these only pass if the
    /// build without calls stays out of the announcement and the refresh altogether.
    #[test]
    fn a_build_without_calls_stays_out_of_group_calls() {
        let (_dir, app) = app(false);
        assert!(!app.group_calls_wanted());
        app.on_group_call_start(
            "group",
            "BBBBBBBB",
            &GroupCallStartMessage {
                protocol_version: 1,
                gck: [1; 32],
                sfu_base_url: "https://sfu.example.org".into(),
            },
        );
        assert_eq!(app.group_call_refresh("group"), json!({}));
        app.store.add_group_call(&stored_call("group", 1)).unwrap();
        app.refresh_stored_group_calls();
    }

    fn stored_call(conversation: &str, seed: u8) -> RunningCall {
        RunningCall {
            call_id: [seed; 32],
            conversation: conversation.to_owned(),
            gck: [seed; 32],
            sfu_base_url: "https://sfu.example.org".into(),
            protocol_version: group_calls::PROTOCOL_VERSION,
            received_at: 1,
            failed: 0,
        }
    }

    /// An app with an ID and a group that "BBBBBBBB" is in, and the group's key.
    fn app_with_group(left: bool) -> (tempfile::TempDir, App, String) {
        let (dir, app) = app(true);
        app.store
            .set_identity(&StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([3; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        let identity = libthreema::common::GroupIdentity {
            creator_identity: "BBBBBBBB".parse().unwrap(),
            group_id: 7,
        };
        let key = crate::groups::key(&identity);
        app.store
            .upsert_group(&crate::groups::Group {
                key: key.clone(),
                identity,
                name: "Crew".into(),
                members: vec!["ABCD1234".into(), "BBBBBBBB".into()],
                left,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        (dir, app, key)
    }

    /// A flaky directory request leaves the announcement waiting, bounded.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn an_announcement_outlasts_failing_credentials_for_a_while() {
        let tries = std::cell::Cell::new(0);
        let third = with_retries(&CREDENTIALS_RETRY, || {
            tries.set(tries.get() + 1);
            let result = if tries.get() < 3 {
                Err(ApiError::NoIdentity)
            } else {
                Ok(7)
            };
            async move { result }
        })
        .await;
        assert_eq!((third, tries.get()), (Some(7), 3));

        tries.set(0);
        let never: Option<u8> = with_retries(&CREDENTIALS_RETRY, || {
            tries.set(tries.get() + 1);
            async { Err(ApiError::NoIdentity) }
        })
        .await;
        assert_eq!(never, None);
        assert_eq!(tries.get(), CREDENTIALS_RETRY.len() + 1);
    }

    /// A late announcement, or a peek, must not write to the store the removal just emptied.
    #[tokio::test(flavor = "current_thread")]
    async fn group_call_tasks_go_when_the_id_does() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_dir, mut app, key) = app_with_group(false);
                let context = app.group_call_context();
                // Sleeps before it looks at anything: no network here.
                context.keep_watching(&key);
                tokio::task::yield_now().await;
                assert!(app.group_calls_watched.borrow().contains(&key));

                let held = Rc::new(());
                let in_task = Rc::clone(&held);
                app.group_call_tasks.spawn(async move {
                    let _in_task = in_task;
                    std::future::pending::<()>().await;
                });
                // Not polled before the end of this test, so this one reaches no server either.
                app.on_group_call_start(
                    &key,
                    "BBBBBBBB",
                    &GroupCallStartMessage {
                        protocol_version: 1,
                        gck: [1; 32],
                        sfu_base_url: "https://sfu.example.org".into(),
                    },
                );
                assert_eq!(app.group_call_tasks.0.borrow().len(), 3);

                app.drop_detached_work();
                tokio::task::yield_now().await;
                assert_eq!(Rc::strong_count(&held), 1, "the task has gone");
                assert!(app.group_call_tasks.0.borrow().is_empty());

                // A next ID's group can be watched again: no name stays behind as "has a task".
                assert!(app.group_calls_watched.borrow().is_empty());
                context.keep_watching(&key);
                assert_eq!(app.group_call_tasks.0.borrow().len(), 1);
                app.drop_detached_work();
            })
            .await;
    }

    #[test]
    fn someone_elses_call_gets_a_line_with_their_name_and_ours_no_notification() {
        let (_dir, mut app, key) = app_with_group(false);
        let mut events = app.take_media_events().unwrap();
        let context = app.group_call_context();

        let message = context.started_line(&key, "BBBBBBBB").unwrap();
        assert_eq!(message.body, "groupCallStarted|BBBBBBBB");
        assert!(matches!(
            events.try_recv(),
            Ok(CoreEvent::MessageAdded { message: added }) if added.id == message.id
        ));

        assert!(context.started_line(&key, "ABCD1234").is_none());
        assert!(
            app.store
                .newest_is_status(&key, GROUP_CALL_YOU_STARTED)
                .unwrap()
        );
    }

    #[test]
    fn a_group_we_are_out_of_gets_no_line() {
        let (_dir, app, key) = app_with_group(true);
        let context = app.group_call_context();
        assert!(context.started_line(&key, "BBBBBBBB").is_none());
        ended_line(&app.store, &context.events, &key);
        assert!(!app.store.newest_is_status(&key, GROUP_CALL_ENDED).unwrap());
    }

    /// Announced is not running: the notification only follows from `GroupCallRunning`.
    #[test]
    fn the_notification_waits_for_the_peek_and_skips_the_call_we_are_in() {
        let (_dir, mut app, key) = app_with_group(false);
        let mut events = app.take_media_events().unwrap();
        let message = app
            .group_call_context()
            .started_line(&key, "BBBBBBBB")
            .unwrap();
        let added = events.try_recv().unwrap();
        assert_eq!(added.to_json(&app.store, true)["notify"], false);

        app.group_call = Some(super::tests::call());
        app.on_group_call_running(&message, &[1; 32]);
        assert!(events.try_recv().is_err(), "we are in this call");
        app.group_call = None;
        app.group_calls_joining.push(key.clone());
        app.on_group_call_running(&message, &[2; 32]);
        assert!(
            events.try_recv().is_err(),
            "we are about to be in this call"
        );
        app.group_call_leave();
        let _left = events.try_recv();
        app.group_call = Some(super::tests::call());

        app.on_group_call_running(&message, &[2; 32]);
        let notify = events.try_recv().unwrap();
        let json = notify.to_json(&app.store, true);
        assert_eq!(json["event"], "messageNotify");
        assert_eq!(json["notify"], true);

        // Mentions only is about messages: a call mentions nobody and still notifies.
        app.store
            .set_chat_notifications(
                &key,
                Some((crate::chat_notifications::Policy::Mentioned, None)),
            )
            .unwrap();
        assert_eq!(notify.to_json(&app.store, true)["notify"], true);

        // A muted group stays silent, the same as for its messages.
        app.store
            .set_chat_notifications(&key, Some((crate::chat_notifications::Policy::Never, None)))
            .unwrap();
        assert_eq!(notify.to_json(&app.store, true)["notify"], false);
    }

    /// The words of the status line in a message mean nothing: its notification isn't held back,
    /// and it gets none where only a call or a mention would.
    #[test]
    fn a_message_saying_what_the_line_says_is_a_message() {
        let (_dir, app, key) = app_with_group(false);
        let mut message = app
            .group_call_context()
            .started_line(&key, "BBBBBBBB")
            .unwrap();
        message.kind = "text".into();
        let added = CoreEvent::MessageAdded {
            message: message.clone(),
        };
        assert_eq!(added.to_json(&app.store, true)["notify"], true);
        app.store
            .set_chat_notifications(
                &key,
                Some((crate::chat_notifications::Policy::Mentioned, None)),
            )
            .unwrap();
        assert!(!app.store.should_notify(&message));
    }

    /// §7.8: a locked private group's call shows neither who started it nor who is in it, and it
    /// can still give the generic notification.
    #[test]
    fn a_locked_private_group_gives_away_no_names() {
        let (_dir, mut app, key) = app_with_group(false);
        let mut events = app.take_media_events().unwrap();
        app.store.set_private(&key, true).unwrap();
        let message = app
            .group_call_context()
            .started_line(&key, "CCCCCCCC")
            .unwrap();
        for event in [events.try_recv().unwrap(), CoreEvent::Notify(message)] {
            let locked = event.to_json(&app.store, false);
            assert_eq!(locked["message"]["body"], "groupCallStarted", "{locked}");
            assert_eq!(locked["private"], true);
            assert!(!locked.to_string().contains("CCCCCCCC"), "{locked}");
        }

        let running = CoreEvent::Call(json!({
            "event": "groupCall",
            "conversation": key,
            "call": { "participants": [{ "identity": "CCCCCCCC", "name": "CCCCCCCC" }] },
        }));
        assert_eq!(running.to_json(&app.store, false)["event"], "internal");
        assert_eq!(running.to_json(&app.store, true)["event"], "groupCall");
    }

    #[test]
    fn locking_takes_a_private_groups_people_from_the_ui_but_not_the_call() {
        let (_dir, mut app, key) = app_with_group(false);
        let mut events = app.take_media_events().unwrap();
        app.store.set_private(&key, true).unwrap();
        app.store.add_group_call(&stored_call(&key, 1)).unwrap();
        let open = crate::groups::key(&libthreema::common::GroupIdentity {
            creator_identity: "BBBBBBBB".parse().unwrap(),
            group_id: 8,
        });
        app.store.add_group_call(&stored_call(&open, 2)).unwrap();
        let mut call = super::tests::call();
        call.conversation = key.clone();
        call.participants = vec![2];
        call.peers
            .insert(2, Peer::authenticated_for_test(2, "CCCCCCCC"));
        app.group_call = Some(call);

        app.group_calls_locked();
        let forget = events.try_recv().unwrap().to_json(&app.store, false);
        assert_eq!(forget["event"], "groupCallsLocked");
        assert_eq!(forget["keep"], json!([open]));
        let people = events.try_recv().unwrap();
        let locked = people.to_json(&app.store, false);
        assert_eq!(locked["event"], "groupCallParticipants");
        assert_eq!(locked["participants"][0]["id"], 2);
        assert_eq!(locked["participants"][0]["authenticated"], true);
        assert!(!locked.to_string().contains("CCCCCCCC"), "{locked}");
        assert!(
            app.group_call.is_some(),
            "the call we are in goes on behind the lock"
        );
        let unlocked = people.to_json(&app.store, true);
        assert_eq!(unlocked["participants"][0]["identity"], "CCCCCCCC");
    }

    #[test]
    fn the_end_line_waits_for_the_last_call_of_the_group() {
        let (_dir, mut app, key) = app_with_group(false);
        let mut events = app.take_media_events().unwrap();
        let context = app.group_call_context();
        let (first, second) = (stored_call(&key, 1), stored_call(&key, 2));
        app.store.add_group_call(&first).unwrap();
        app.store.add_group_call(&second).unwrap();

        app.store.remove_group_call(&first.call_id).unwrap();
        ended_line(&app.store, &context.events, &key);
        assert!(events.try_recv().is_err(), "one still runs");

        app.store.remove_group_call(&second.call_id).unwrap();
        ended_line(&app.store, &context.events, &key);
        assert!(matches!(
            events.try_recv(),
            Ok(CoreEvent::MessageAdded { message }) if message.body == GROUP_CALL_ENDED
        ));
    }

    /// The old ID's token would go out with the next ID's requests while it lasts, and its call
    /// would go on in memory.
    #[tokio::test(flavor = "current_thread")]
    async fn removing_the_id_forgets_the_group_call_and_the_sfu_token() {
        let (_dir, mut app) = app(true);
        app.group_call = Some(super::tests::call());
        app.sfu.borrow_mut().0 = Some((
            SfuCredentials {
                sfu_base_url: "https://sfu.example.org".into(),
                allowed_sfu_hostname_suffixes: Vec::new(),
                sfu_token: "token".into(),
                expiration: 3600,
            },
            i64::MAX,
        ));

        app.delete_all_data().await.unwrap();

        assert!(app.group_call.is_none());
        assert!(app.sfu.borrow().0.is_none());
    }

    /// The tiles of a running call follow "Show profile pictures" at once.
    #[test]
    fn a_running_call_follows_the_picture_setting() {
        let (_dir, mut app) = app(true);
        let mut events = app.take_media_events().unwrap();
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        app.store.set_contact_picture("BBBBBBBB", Some(5)).unwrap();
        let mut call = super::tests::call();
        call.participants = vec![2];
        call.peers
            .insert(2, Peer::authenticated_for_test(2, "BBBBBBBB"));
        app.group_call = Some(call);
        let mut shown = |app: &mut App, value: bool| -> i64 {
            app.set_app_setting("showProfilePictures", value).unwrap();
            let mut version = None;
            while let Ok(event) = events.try_recv() {
                let event = event.to_json(&app.store, true);
                if event["event"] == "groupCallParticipants" {
                    version = event["participants"][0]["pictureVersion"].as_i64();
                }
            }
            version.expect("the call heard of it")
        };
        assert_eq!(shown(&mut app, false), 0);
        assert!(shown(&mut app, true) > 0);
    }

    /// In a call, at the point where the SFU's Hello arrives with `others` participants: what
    /// the app thinks of our microphone and what it told the bridge.
    fn after_hello(others: u32) -> (bool, Vec<Value>) {
        let (_dir, mut app) = app(true);
        let mut events = app.take_media_events().unwrap();
        app.store
            .set_identity(&StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([7; 32]),
                server_group: "30".into(),
            })
            .unwrap();
        let mut call = super::tests::call();
        call.hello = false;
        app.group_call = Some(call);
        #[expect(
            deprecated,
            reason = "the field is part of the message a test has to build"
        )]
        let envelope = sfu_to_participant::Envelope {
            padding: Vec::new(),
            content: Some(sfu_to_participant::envelope::Content::Hello(
                sfu_to_participant::Hello {
                    participant_ids: (2..2 + others).collect(),
                },
            )),
        };
        app.group_call_channel(&BASE64.encode(&envelope.encode_to_vec()))
            .unwrap();
        let mut seen = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let CoreEvent::Call(event) = event {
                seen.push(event);
            }
        }
        (app.group_call.unwrap().microphone, seen)
    }

    fn says_microphone_off(events: &[Value]) -> bool {
        events
            .iter()
            .any(|event| event["event"] == "groupCallMicrophoneOff")
    }

    #[test]
    fn a_big_call_starts_with_the_microphone_off() {
        let (microphone, events) = after_hello(3);
        assert!(microphone, "three others: on");
        assert!(!says_microphone_off(&events));

        let (microphone, events) = after_hello(4);
        assert!(!microphone, "four others: off");
        assert!(says_microphone_off(&events), "the bridge must mute too");
        // Before anything goes out to the others.
        assert_eq!(events[0]["event"], "groupCallMicrophoneOff");
    }

    /// The cover names the group of a running call: a private one has to be told apart, locked
    /// or not.
    #[test]
    fn the_group_lookup_says_when_a_group_is_private() {
        let (_dir, app) = app(true);
        app.store
            .set_identity(&StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([7; 32]),
                server_group: "30".into(),
            })
            .unwrap();
        let identity = libthreema::common::GroupIdentity {
            creator_identity: "ABCD1234".parse().unwrap(),
            group_id: 1,
        };
        let key = crate::groups::key(&identity);
        app.store
            .upsert_group(&crate::groups::Group {
                key: key.clone(),
                identity,
                name: "Secret plans".into(),
                members: Vec::new(),
                left: false,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        assert_eq!(app.group_info(&key).unwrap()["private"], false);
        app.store.set_private(&key, true).unwrap();
        assert_eq!(app.group_info(&key).unwrap()["private"], true);
    }

    #[test]
    fn a_running_call_is_repeated_to_the_new_people_who_can_join_it() {
        let (_dir, app) = app(true);
        let mut able = crate::contacts::test_contact("AAAAAAAA");
        able.feature_mask =
            libthreema::common::FeatureMask(libthreema::common::FeatureMask::GROUP_CALL_SUPPORT);
        app.store.upsert_contact(&able).unwrap();
        let mut unable = crate::contacts::test_contact("BBBBBBBB");
        unable.feature_mask = libthreema::common::FeatureMask(0);
        app.store.upsert_contact(&unable).unwrap();
        let identity = libthreema::common::GroupIdentity {
            creator_identity: "CCCCCCCC".parse().unwrap(),
            group_id: 7,
        };
        let mut group = crate::groups::Group {
            key: crate::groups::key(&identity),
            identity,
            name: String::new(),
            members: vec!["AAAAAAAA".into(), "BBBBBBBB".into()],
            left: false,
            created_at: 1,
            picture: None,
            picture_version: 0,
        };
        app.store.upsert_group(&group).unwrap();
        let call = RunningCall {
            call_id: [1; 32],
            conversation: group.key.clone(),
            gck: [2; 32],
            sfu_base_url: "https://sfu.example".into(),
            protocol_version: group_calls::PROTOCOL_VERSION,
            received_at: 1,
            failed: 0,
        };
        // "AAAAAAAA" can join; "BBBBBBBB" can't; "EEEEEEEE" isn't in the group.
        let asked = ["AAAAAAAA".into(), "BBBBBBBB".into(), "EEEEEEEE".into()];
        assert!(queue_call_start(&app.store, &call, 1234, &asked).unwrap());
        let entries = app.store.outbox().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, crate::groups::OUTBOX_GROUP_CALL_START);
        assert_eq!(entries[0].body["startedAt"], json!(1234));
        assert_eq!(
            app.store
                .pending_receivers(&group.key, entries[0].message_id)
                .unwrap(),
            vec!["AAAAAAAA"]
        );

        // Nobody to tell, and not for a group we left.
        assert!(!queue_call_start(&app.store, &call, 1234, &["BBBBBBBB".into()]).unwrap());
        group.left = true;
        app.store.upsert_group(&group).unwrap();
        assert!(!queue_call_start(&app.store, &call, 1234, &["AAAAAAAA".into()]).unwrap());
        assert_eq!(app.store.outbox().unwrap().len(), 1);
    }

    #[test]
    fn group_calls_follow_the_build_and_the_setting() {
        let (_dir, app) = app(true);
        assert!(app.group_calls_wanted());
        app.store.set_setting(keys::GROUP_CALLS, "0").unwrap();
        assert!(!app.group_calls_wanted());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn call() -> ActiveGroupCall {
        let session = Session {
            local: 1,
            ice_ufrag: String::new(),
            ice_pwd: String::new(),
            dtls_fingerprint: Vec::new(),
            extensions: Extensions::from_join(None),
        };
        ActiveGroupCall {
            conversation: String::new(),
            call_id: [1; 32],
            keys: CallKeys::derive(&[1; 32]),
            local: 1,
            description: Description::new(session),
            participants: Vec::new(),
            hello: true,
            peers: HashMap::new(),
            media_keys: vec![MediaKey::random(255)],
            their_keys: HashMap::new(),
            microphone: true,
            their_microphones: HashMap::new(),
            camera: false,
            their_cameras: HashMap::new(),
            rekey_at: None,
            rekey_stale: false,
            state_update_at: None,
            announce: None,
            started: 0,
            alone_since: None,
        }
    }

    #[test]
    fn rekey_applies_after_two_seconds_and_reruns_when_stale() {
        let mut call = call();
        call.rekey(1000);
        assert_eq!(call.media_keys.len(), 2);
        assert_eq!(call.media_keys[1].epoch, 0, "epoch wraps");
        // A second leave while pending: marked stale, nothing new yet.
        call.rekey(1500);
        assert_eq!(call.media_keys.len(), 2);
        assert!(call.due(2999, "ABCD1234", "").is_empty());
        assert_eq!(call.media_keys[0].epoch, 255);
        call.due(3000, "ABCD1234", "");
        // Applied, and the stale rerun made the next one pending.
        assert_eq!(call.media_keys[0].epoch, 0);
        assert_eq!(call.media_keys[1].epoch, 1);
        assert_eq!(call.rekey_at, Some(5000));
        call.due(5000, "ABCD1234", "");
        assert_eq!(call.media_keys.len(), 1);
        assert_eq!(call.media_keys[0].epoch, 1);
    }

    #[test]
    fn designated_participant_sends_the_call_state_every_ten_seconds() {
        let mut call = call();
        call.redesignate(100);
        assert_eq!(call.due(100, "ABCD1234", "Me").len(), 1);
        assert!(call.due(10_099, "ABCD1234", "Me").is_empty());
        assert_eq!(call.due(10_100, "ABCD1234", "Me").len(), 1);
    }
}
