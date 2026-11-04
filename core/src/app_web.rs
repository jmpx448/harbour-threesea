// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Web in the app (ADR 0013): pairing from a scanned code, one task per running session
//! (WebSocket, SaltyRTC responder, ARP connection layer), and the app's answers to the browser.
//!
//! Commands: `webSessions`, `webPair {qr, allowHost?}`, `webStart {session}`, `webStop
//! {session}`, `webDelete {session}`, `webAcceptCertificate {session}`. Events: `webSession {id, state, error?}`
//! (`connecting`, `waiting` for the browser, `connected`, `stopped`), `webCertificateChanged
//! {id, host}`.

use std::collections::HashMap;

use data_encoding::HEXLOWER;
use libthreema::common::keys::{ClientKey, PublicKey};
use rmpv::Value;
use serde_json::json;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use zeroize::Zeroizing;

use crate::{
    app::{ApiError, App},
    group_call_handshake::public_bytes,
    providers::CoreEvent,
    store::keys,
    web::{
        WebSession,
        arp::{self, Action, Incoming},
        qr,
        saltyrtc::{self, Event, Responder},
    },
    websocket::{Datagrams, WebSocketError},
};

/// From a session task to the core loop.
pub(crate) enum FromWeb {
    State {
        session: i64,
        state: &'static str,
    },
    /// A message from the browser for the app.
    Message {
        session: i64,
        message: Value,
    },
    /// The task ended (`delete`: the browser asked to forget the session).
    Ended {
        session: i64,
        delete: bool,
        error: Option<String>,
    },
    /// A self-hosted server's first certificate, to pin.
    Pinned {
        session: i64,
        host: String,
        spki: String,
    },
    /// A self-hosted server shows another certificate than the pinned one.
    CertificateChanged {
        session: i64,
        spki: String,
    },
}

/// From the app to a session task.
pub(crate) enum ToWeb {
    Send(Value),
    Stop,
}

/// A session's task as the core knows it.
struct Running {
    tx: UnboundedSender<ToWeb>,
    state: &'static str,
    /// Unique per task in this process: events are only taken from the session's current task
    /// (a session ID can come back after deletion; deep audit 2026-10-27, D01).
    task: u64,
}

/// A session task's events, marked with the task.
pub(crate) type WebEvent = (u64, FromWeb);

/// How a session task reports to the core.
struct TaskEvents {
    task: u64,
    tx: tokio::sync::mpsc::Sender<WebEvent>,
}

impl TaskEvents {
    /// Waits while the core is behind: a browser sending faster than the core handles its
    /// requests is slowed down instead of queuing without limit (deep audit D03).
    async fn send(&self, event: FromWeb) {
        let _ = self.tx.send((self.task, event)).await;
    }
}

/// Session events (all sessions together) waiting for the core at most.
const EVENT_QUEUE: usize = 16;

/// The running sessions and their events.
pub(crate) struct WebSessions {
    running: HashMap<i64, Running>,
    next_task: u64,
    /// A changed certificate per session, until the user accepts it.
    changed: HashMap<i64, String>,
    /// What each connected browser subscribed to (app_web_data).
    pub(crate) subscriptions: HashMap<i64, crate::app_web_data::Subscriptions>,
    /// `request/blob`s waiting for a download.
    pub(crate) pending_blobs: Vec<crate::app_web_media::PendingBlob>,
    /// When the browsers were last brought up to date (app_web_data::web_refresh).
    pub(crate) refresh: crate::app_web_data::RefreshState,
    events: tokio::sync::mpsc::Sender<WebEvent>,
    events_rx: Option<tokio::sync::mpsc::Receiver<WebEvent>>,
}

impl Default for WebSessions {
    fn default() -> Self {
        let (events, events_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE);
        Self {
            running: HashMap::new(),
            next_task: 0,
            changed: HashMap::new(),
            subscriptions: HashMap::new(),
            pending_blobs: Vec::new(),
            refresh: crate::app_web_data::RefreshState::default(),
            events,
            events_rx: Some(events_rx),
        }
    }
}

impl App {
    /// The receiver of session events (taken once by the core loop).
    pub(crate) fn take_web_events(&mut self) -> Option<tokio::sync::mpsc::Receiver<WebEvent>> {
        // Sessions the browser never stored can't come back: leftovers of a killed app.
        for session in self.store.web_sessions().unwrap_or_default() {
            if !session.persistent {
                let _ = self.store.remove_web_session(session.id);
            }
        }
        self.web.events_rx.take()
    }

    /// Tests: an authenticated session `id` without a network behind it; returns what it would
    /// send and its task (for its events).
    #[cfg(test)]
    pub(crate) fn web_test_session(&mut self, id: i64) -> (UnboundedReceiver<ToWeb>, u64) {
        let (tx, rx) = unbounded_channel();
        self.web.next_task += 1;
        let task = self.web.next_task;
        self.web.running.insert(
            id,
            Running {
                tx,
                state: "connected",
                task,
            },
        );
        (rx, task)
    }

    /// `webSessions`: `{sessions: [{id, host, selfHosted, persistent, client, createdAt,
    /// lastConnection, state}]}`.
    pub(crate) fn web_sessions_json(&self) -> Result<serde_json::Value, ApiError> {
        let sessions: Vec<serde_json::Value> = self
            .store
            .web_sessions()?
            .iter()
            .map(|session| {
                let mut value = session.to_json();
                value["state"] = json!(
                    self.web
                        .running
                        .get(&session.id)
                        .map_or("stopped", |running| running.state)
                );
                value
            })
            .collect();
        Ok(json!({ "sessions": sessions }))
    }

    /// `webPair {qr, allowHost}`: a new session from Threema Web's code. A server outside
    /// threema.ch needs `allowHost` once (`{confirmHost}` asks for it).
    pub(crate) fn web_pair(
        &mut self,
        code: &str,
        allow_host: bool,
    ) -> Result<serde_json::Value, ApiError> {
        let qr = qr::parse(code).map_err(|error| match error {
            qr::QrError::Invalid => ApiError::WebCodeInvalid,
            qr::QrError::Version(_) => ApiError::WebVersion,
        })?;
        if self.web_host_needs_asking(&qr.host)? {
            if !allow_host {
                return Ok(json!({ "confirmHost": qr.host }));
            }
            self.store.set_web_host(&qr.host, None)?;
        }
        let session = WebSession {
            id: 0,
            initiator_key: qr.initiator_key,
            private_key: Zeroizing::new(*ClientKey::random().as_bytes()),
            server_key: qr.server_key,
            host: qr.host.clone(),
            port: qr.port,
            self_hosted: qr.self_hosted,
            persistent: false,
            client: String::new(),
            created_at: crate::messages::now_ms(),
            last_connection: None,
        };
        let id = self.store.add_web_session(&session)?;
        tracing::info!("Threema Web session paired");
        self.start_web_session(id, Some(qr.auth_token))?;
        Ok(json!({ "id": id }))
    }

    /// Not Threema's and not allowed by the user before (ADR 0013).
    fn web_host_needs_asking(&self, host: &str) -> Result<bool, ApiError> {
        Ok(!qr::is_threema_host(host) && self.store.web_host(host)?.is_none())
    }

    /// `qrKind`: what a scanned code is, and for Threema Web whether its server must be allowed.
    pub(crate) fn classify_qr(&self, text: &str) -> Result<serde_json::Value, ApiError> {
        let mut code = crate::app::qr_kind(text, crate::messages::now_ms() / 1000);
        if let Some(host) = code["host"].as_str() {
            code["confirmHost"] = json!(self.web_host_needs_asking(host)?);
        }
        Ok(code)
    }

    /// `webStart {session}`: connect a stored session (the browser must be waiting for us).
    pub(crate) fn web_start(&mut self, id: i64) -> Result<serde_json::Value, ApiError> {
        self.start_web_session(id, None)?;
        Ok(json!({}))
    }

    /// The push relay woke us for a stored session (its browser key's hex SHA-256): connect it,
    /// if the browser stored it too.
    pub(crate) fn web_resume(&mut self, hash: &str) {
        use sha2::{Digest as _, Sha256};
        let Ok(sessions) = self.store.web_sessions() else {
            return;
        };
        let Some(session) = sessions
            .iter()
            .find(|session| HEXLOWER.encode(&Sha256::digest(session.initiator_key)) == hash)
        else {
            tracing::info!("Threema Web wake-up for an unknown session");
            return;
        };
        if session.persistent
            && let Err(error) = self.start_web_session(session.id, None)
        {
            tracing::warn!(kind = error.kind(), "Threema Web wake-up failed");
        }
    }

    /// `webStop {session}`: tell the browser and disconnect.
    pub(crate) fn web_stop(&mut self, id: i64) -> serde_json::Value {
        if let Some(running) = self.web.running.get(&id) {
            let _ = running.tx.send(ToWeb::Stop);
        }
        json!({})
    }

    /// Stop every session and forget what they were waiting for: the identity they were paired
    /// with is going away (security audit 2026-10-27, S01).
    pub(crate) fn web_stop_all(&mut self) {
        for running in self.web.running.values() {
            let _ = running.tx.send(ToWeb::Stop);
        }
        self.web.running.clear();
        self.web.changed.clear();
        self.web.subscriptions.clear();
        self.web.pending_blobs.clear();
    }

    /// `webDelete {session}`: stop and forget the session.
    pub(crate) fn web_delete(&mut self, id: i64) -> Result<serde_json::Value, ApiError> {
        self.web_stop(id);
        self.web.running.remove(&id);
        // Nothing waiting for it may reach a later session with the same ID.
        self.web.changed.remove(&id);
        self.web.subscriptions.remove(&id);
        self.web.pending_blobs.retain(|blob| blob.session != id);
        self.store.remove_web_session(id)?;
        Ok(json!({}))
    }

    /// `webAcceptCertificate {session}`: the self-hosted server's new certificate is fine; pin it and
    /// connect again.
    pub(crate) fn web_accept_certificate(
        &mut self,
        id: i64,
    ) -> Result<serde_json::Value, ApiError> {
        let spki = self
            .web
            .changed
            .remove(&id)
            .ok_or(ApiError::WebSessionNotFound)?;
        let session = self
            .store
            .web_session(id)?
            .ok_or(ApiError::WebSessionNotFound)?;
        self.store.set_web_host(&session.host, Some(&spki))?;
        self.start_web_session(id, None)?;
        Ok(json!({}))
    }

    fn start_web_session(
        &mut self,
        id: i64,
        token: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<(), ApiError> {
        if self.web.running.contains_key(&id) {
            return Ok(());
        }
        let session = self
            .store
            .web_session(id)?
            .ok_or(ApiError::WebSessionNotFound)?;
        // Threema's hosts: their built-in pins only. Others: allowed by the user, pinned on
        // first use.
        let pin = if qr::is_threema_host(&session.host) {
            None
        } else {
            Some(
                self.store
                    .web_host(&session.host)?
                    .ok_or(ApiError::WebSessionNotFound)?,
            )
        };
        let (tx, rx) = unbounded_channel();
        self.web.next_task += 1;
        let task = self.web.next_task;
        self.web.running.insert(
            id,
            Running {
                tx,
                state: "connecting",
                task,
            },
        );
        let events = TaskEvents {
            task,
            tx: self.web.events.clone(),
        };
        tokio::task::spawn_local(async move {
            let result = run_session(&session, token, pin, rx, &events).await;
            let (delete, error) = match result {
                Ok(delete) => (delete, None),
                Err(error) => (false, Some(error)),
            };
            events
                .send(FromWeb::Ended {
                    session: session.id,
                    delete,
                    error,
                })
                .await;
        });
        self.emit_web_state(id, "connecting", None);
        Ok(())
    }

    fn emit_web_state(&self, id: i64, state: &str, error: Option<&str>) {
        let _ = self.media_events.send(CoreEvent::Call(
            json!({ "event": "webSession", "id": id, "state": state, "error": error }),
        ));
    }

    /// An event of a session task (core loop).
    pub(crate) async fn on_web_event(&mut self, (task, event): WebEvent) {
        // Only the session's current task has a say: a deleted, stopped or replaced one's late
        // events (requests, its end, certificates) are dropped (S01, deep audit D01).
        let session = match &event {
            FromWeb::State { session, .. }
            | FromWeb::Message { session, .. }
            | FromWeb::Ended { session, .. }
            | FromWeb::Pinned { session, .. }
            | FromWeb::CertificateChanged { session, .. } => *session,
        };
        if self
            .web
            .running
            .get(&session)
            .is_none_or(|running| running.task != task)
        {
            return;
        }
        match event {
            FromWeb::State { session, state } => {
                if let Some(running) = self.web.running.get_mut(&session) {
                    running.state = state;
                }
                // A new connection starts over (no resumption yet).
                self.web.subscriptions.remove(&session);
                if state == "connected" {
                    let _ = self.store.set_web_session_connected(
                        session,
                        "",
                        crate::messages::now_ms(),
                    );
                }
                self.emit_web_state(session, state, None);
            }
            FromWeb::Message { session, message } => self.on_web_message(session, &message).await,
            FromWeb::Ended {
                session,
                delete,
                error,
            } => {
                self.web.running.remove(&session);
                self.web.subscriptions.remove(&session);
                let persistent = self
                    .store
                    .web_session(session)
                    .ok()
                    .flatten()
                    .is_some_and(|stored| stored.persistent);
                // A session the browser never stored can't come back (Android deletes it too).
                if delete || !persistent {
                    let _ = self.store.remove_web_session(session);
                }
                if let Some(error) = &error {
                    tracing::warn!(%error, "Threema Web session ended");
                }
                self.emit_web_state(session, "stopped", error.as_deref());
            }
            FromWeb::Pinned { host, spki, .. } => {
                let _ = self.store.set_web_host(&host, Some(&spki));
            }
            FromWeb::CertificateChanged { session, spki } => {
                let host = self
                    .store
                    .web_session(session)
                    .ok()
                    .flatten()
                    .map(|stored| stored.host)
                    .unwrap_or_default();
                self.web.changed.insert(session, spki);
                let _ = self.media_events.send(CoreEvent::Call(
                    json!({ "event": "webCertificateChanged", "id": session, "host": host }),
                ));
            }
        }
    }

    pub(crate) fn send_web(&self, session: i64, message: Value) {
        if let Some(running) = self.web.running.get(&session) {
            let _ = running.tx.send(ToWeb::Send(message));
        }
    }

    /// The browser's requests (the first ones it needs to show its UI).
    async fn on_web_message(&mut self, session: i64, message: &Value) {
        let Some(incoming) = Incoming::parse(message) else {
            return;
        };
        if let Some(result) = self.web_media_action(session, &incoming).await {
            let reply = match (result, incoming.id) {
                (Ok(None), _) => return,
                (Ok(Some(reply)), Some(id)) => arp::with_ack(reply, id, None),
                (Ok(Some(reply)), None) => reply,
                (Err(code), Some(id)) => {
                    tracing::info!(
                        sub_type = incoming.sub_type,
                        code,
                        "Threema Web request refused"
                    );
                    arp::with_ack(
                        arp::message("update", "confirm", None, None),
                        id,
                        Some(code),
                    )
                }
                (Err(_), None) => return,
            };
            self.send_web(session, reply);
            return;
        }
        let result = match self.web_action(&incoming).await {
            Some(result) => Some(result),
            None => self.web_manage(&incoming).await,
        };
        if let Some(result) = result {
            let reply = match (result, incoming.id) {
                (Ok(reply), Some(id)) => arp::with_ack(reply, id, None),
                (Err(code), Some(id)) => {
                    tracing::info!(
                        kind = incoming.kind,
                        sub_type = incoming.sub_type,
                        code,
                        "Threema Web action refused"
                    );
                    arp::with_ack(
                        arp::message("update", "confirm", None, None),
                        id,
                        Some(code),
                    )
                }
                (Ok(reply), None) if reply_has_content(&reply) => reply,
                _ => return,
            };
            self.send_web(session, reply);
            return;
        }
        let reply = match (incoming.kind, incoming.sub_type) {
            ("request", "clientInfo") => {
                let client = incoming
                    .data
                    .and_then(|data| crate::web::saltyrtc::field(data, "browserName"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let _ = self.store.set_web_session_connected(
                    session,
                    &client,
                    crate::messages::now_ms(),
                );
                // The session list shows the browser's name now.
                self.emit_web_state(session, "connected", None);
                self.web_client_info()
                    .map(|data| arp::message("response", "clientInfo", None, Some(data)))
            }
            ("request", "profile") => self
                .web_profile()
                .map(|data| arp::message("response", "profile", None, Some(data))),
            ("request", "receivers") => {
                let reply = self
                    .web_receivers()
                    .map(|data| arp::message("response", "receivers", None, Some(data)));
                if reply.is_ok() {
                    self.web_subscribe_receivers(session);
                }
                reply
            }
            ("request", "conversations") => self
                .web_conversations(session)
                .map(|data| arp::message("response", "conversations", None, Some(data))),
            ("request", "messages") => {
                let ref_id = incoming.arg("refMsgId");
                self.web_receiver_key(incoming.arg("id"))
                    .and_then(|key| self.web_messages(session, &key, ref_id))
                    .map(|(args, data)| {
                        arp::message("response", "messages", Some(args), Some(data))
                    })
            }
            ("request", "read") => self
                .web_receiver_key(incoming.arg("id"))
                .and_then(|key| self.mark_read(&key))
                .map(|_| arp::message("update", "confirm", None, None)),
            ("request", "keyPersisted") => {
                let _ = self.store.set_web_session_persistent(session);
                Ok(arp::message("update", "confirm", None, None))
            }
            // Deprecated; we send nothing for it.
            ("request", "batteryStatus") => Ok(arp::message("update", "confirm", None, None)),
            _ => {
                // The browser's own text is not logged (CLAUDE.md §7.3; deep audit D05).
                tracing::info!("Threema Web: message not handled");
                Err(ApiError::Internal("not handled".into()))
            }
        };
        let reply = match (reply, incoming.id) {
            (Ok(reply), Some(id)) => arp::with_ack(reply, id, None),
            (Err(_), Some(id)) => arp::with_ack(
                arp::message("update", "confirm", None, None),
                id,
                Some("badRequest"),
            ),
            // Only replies that mean something without an acknowledgement go out.
            (Ok(reply), None) if incoming.kind == "request" && reply_has_content(&reply) => reply,
            _ => return,
        };
        self.send_web(session, reply);
    }

    /// `response/clientInfo`, to the user's own browser. `os` selects the browser's push token
    /// parser, which reads a `threema-gateway` token only for "android", so this is a protocol
    /// setting (docs/research/threema-web.md §2.1); `appVersion` names Threesea. The token wakes
    /// us through the chat server, as for Android without Google services.
    fn web_client_info(&self) -> Result<Value, ApiError> {
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let public = ClientKey::from(*identity.client_key).public_key();
        let token = format!(
            "threema-gateway;{};{}",
            identity.threema_id,
            HEXLOWER.encode(&public_bytes(&public))
        );
        Ok(arp::map(vec![
            ("device", self.version.model.clone().into()),
            ("os", "android".into()),
            ("osVersion", self.version.os_version.clone().into()),
            (
                "appVersion",
                format!("{} (Threesea)", self.version.app_version).into(),
            ),
            ("isWork", false.into()),
            ("pushToken", token.into()),
            (
                "configuration",
                arp::map(vec![
                    ("voipEnabled", false.into()),
                    ("voipForceTurn", false.into()),
                    ("largeSingleEmoji", true.into()),
                    ("showInactiveIDs", true.into()),
                ]),
            ),
            (
                "capabilities",
                arp::map(vec![
                    ("maxGroupSize", 256.into()),
                    ("maxFileSize", (100 * 1024 * 1024).into()),
                    ("maxMessageBodySize", 6000.into()),
                    ("distributionLists", true.into()),
                    ("quotesV2", true.into()),
                    ("emojiReactions", true.into()),
                    (
                        "imageFormat",
                        arp::map(vec![
                            ("avatar", "image/jpeg".into()),
                            ("thumbnail", "image/jpeg".into()),
                        ]),
                    ),
                ]),
            ),
        ]))
    }

    /// `response/profile`: our ID, public key and nickname.
    fn web_profile(&self) -> Result<Value, ApiError> {
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let public = ClientKey::from(*identity.client_key).public_key();
        let mut fields = vec![
            ("identity", Value::from(identity.threema_id)),
            ("publicKey", Value::Binary(public_bytes(&public).to_vec())),
        ];
        if let Some(nickname) = self
            .store
            .setting(keys::NICKNAME)?
            .filter(|n| !n.is_empty())
        {
            fields.push(("publicNickname", nickname.into()));
        }
        Ok(arp::map(fields))
    }
}

/// A reply other than a bare `update/confirm`.
fn reply_has_content(reply: &Value) -> bool {
    crate::web::saltyrtc::field(reply, "subType").and_then(Value::as_str) != Some("confirm")
}

/// One session's connection: the WebSocket to the signalling server, SaltyRTC, the ARP
/// connection layer. Ends when the browser or the user stops it; `Ok(true)`: forget the session.
async fn run_session(
    session: &WebSession,
    token: Option<Zeroizing<[u8; 32]>>,
    pin: Option<Option<String>>,
    mut rx: UnboundedReceiver<ToWeb>,
    events: &TaskEvents,
) -> Result<bool, String> {
    let mut responder = Responder::new(
        ClientKey::from(*session.private_key),
        PublicKey::from(session.initiator_key),
        token.map(|token| *token),
        session.server_key.map(PublicKey::from),
    );
    let url = format!(
        "wss://{}:{}/{}",
        session.host,
        session.port,
        responder.path()
    );
    let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
    let tls = match &pin {
        None => crate::https::tls_config(),
        Some(expected) => {
            crate::https::tofu_tls_config(expected.clone(), std::sync::Arc::clone(&seen))
        }
    }?;
    let connected = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        Datagrams::connect_with(&url, Some(saltyrtc::SUBPROTOCOL), tls),
    )
    .await
    .map_err(|_| "timeout".to_owned())
    .and_then(|result| result.map_err(|error: WebSocketError| error.to_string()));
    let presented = seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let (Some(expected), Some(presented)) = (&pin, presented) {
        match expected {
            None => {
                events
                    .send(FromWeb::Pinned {
                        session: session.id,
                        host: session.host.clone(),
                        spki: presented,
                    })
                    .await;
            }
            Some(expected) if *expected != presented => {
                events
                    .send(FromWeb::CertificateChanged {
                        session: session.id,
                        spki: presented,
                    })
                    .await;
                return Err("certificate changed".into());
            }
            Some(_) => {}
        }
    }
    let mut socket = connected?;
    let mut connection: Option<arp::Connection> = None;
    let mut ack_timer = tokio::time::interval(std::time::Duration::from_secs(10));
    ack_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The battery, every 30 s while connected (sent when it changed).
    let mut battery_timer = tokio::time::interval(std::time::Duration::from_secs(30));
    battery_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut battery_sent: Option<(Option<u8>, bool)> = None;
    loop {
        let mut out: Vec<Vec<u8>> = Vec::new();
        let mut actions: Vec<Action> = Vec::new();
        tokio::select! {
            frame = socket.receive() => {
                let frame = frame.map_err(|error| error.to_string())?;
                let saltyrtc_events = match responder.receive(&frame) {
                    Ok(saltyrtc_events) => saltyrtc_events,
                    Err(error) => {
                        socket.close().await;
                        return Err(error.to_string());
                    }
                };
                for event in saltyrtc_events {
                    match event {
                        Event::Send(frame) => out.push(frame),
                        Event::PeerConnected => {
                            let id = responder.connection_id().ok_or("no connection ID")?;
                            let (started, first) = arp::Connection::start(id);
                            connection = Some(started);
                            actions.extend(first);
                            battery_sent = None;
                            events.send(FromWeb::State { session: session.id, state: "connected" }).await;
                        }
                        Event::Data(payload) => {
                            if let Some(connection) = connection.as_mut() {
                                actions.extend(connection.receive(&payload));
                            }
                        }
                        Event::PeerClosed(_) => {
                            socket.close().await;
                            return Ok(false);
                        }
                        Event::PeerGone => {
                            connection = None;
                            events.send(FromWeb::State { session: session.id, state: "waiting" }).await;
                        }
                    }
                }
                if connection.is_none() && !responder.connected() && responder_waiting(&out) {
                    events.send(FromWeb::State { session: session.id, state: "waiting" }).await;
                }
            }
            command = rx.recv() => match command {
                Some(ToWeb::Send(message)) => {
                    if let Some(connection) = connection.as_mut() {
                        actions.extend(connection.send(message));
                    }
                }
                Some(ToWeb::Stop) | None => {
                    if let Some(connection) = connection.as_mut() {
                        let goodbye = arp::message(
                            "update",
                            "connectionDisconnect",
                            None,
                            Some(arp::map(vec![("reason", "stop".into())])),
                        );
                        for action in connection.send(goodbye) {
                            if let Action::Chunk(chunk) = action
                                && let Ok(frame) = responder.send_data(Value::Binary(chunk))
                            {
                                let _ = socket.send(frame).await;
                            }
                        }
                    }
                    if let Some(frame) = responder.close(saltyrtc::CLOSE_GOING_AWAY) {
                        let _ = socket.send(frame).await;
                    }
                    socket.close().await;
                    return Ok(false);
                }
            },
            _ = battery_timer.tick() => {
                if let Some(connection) = connection.as_mut() {
                    let status = crate::web::battery::status(std::path::Path::new(crate::web::battery::POWER_SUPPLY));
                    if let Some((percent, charging)) = status
                        && battery_sent != status
                    {
                        battery_sent = status;
                        actions.extend(connection.send(arp::message(
                            "update",
                            "batteryStatus",
                            None,
                            Some(arp::map(vec![
                                ("percent", percent.map_or(Value::Nil, Value::from)),
                                ("isCharging", charging.into()),
                            ])),
                        )));
                    }
                }
            }
            _ = ack_timer.tick() => {
                if let Some(connection) = connection.as_mut()
                    && connection.needs_acknowledgement()
                {
                    actions.extend(connection.acknowledge());
                }
            }
        }
        for action in actions {
            match action {
                Action::Chunk(chunk) => {
                    out.push(
                        responder
                            .send_data(Value::Binary(chunk))
                            .map_err(|error| error.to_string())?,
                    );
                }
                Action::Message(message) => {
                    events
                        .send(FromWeb::Message {
                            session: session.id,
                            message,
                        })
                        .await;
                }
                Action::Disconnect { delete } => {
                    if let Some(frame) = responder.close(saltyrtc::CLOSE_GOING_AWAY) {
                        let _ = socket.send(frame).await;
                    }
                    socket.close().await;
                    return Ok(delete);
                }
            }
        }
        for frame in out {
            socket
                .send(frame)
                .await
                .map_err(|error| error.to_string())?;
        }
    }
}

/// After the server handshake without a browser on the path: we wait for it.
fn responder_waiting(sent: &[Vec<u8>]) -> bool {
    sent.is_empty()
}

#[cfg(test)]
#[path = "app_web_security_tests.rs"]
mod security_tests;
