// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Web in the app (ADR 0013): pairing from a scanned code, one task per running session
//! (WebSocket, SaltyRTC responder, ARP connection layer), and the app's answers to the browser.
//!
//! Commands: `webSessions`, `webSetEnabled {enabled}`, `webPair {qr, allowHost?}`, `webStart {session}`, `webStop
//! {session}`, `webDelete {session}`, `webAcceptCertificate {session}`, `webDeclineCertificate
//! {session}`, `webRemoveServer {host}`. Events: `webSession {id, state, error?}`
//! (`connecting`, `waiting` for the browser, `connected`, `stopped`; `error`: a [`WebFailure`]'s
//! code), `webCertificateChanged {id, host}`.

use std::collections::{HashMap, HashSet};

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
        error: Option<WebFailure>,
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

/// Why a session's task ended against its will. Only fixed codes reach the log and the UI: what
/// a server gives as its reason for closing is its own text, and a self-hosted server isn't
/// Threema's (CLAUDE.md §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebFailure {
    Timeout,
    Tls,
    CertificateChanged,
    /// The server closed the connection, with this close code.
    Closed(Option<u16>),
    /// One of our own fixed texts.
    Protocol(&'static str),
    Socket,
}

impl WebFailure {
    fn code(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Tls => "tls",
            Self::CertificateChanged => "certificateChanged",
            Self::Closed(_) => "closed",
            Self::Protocol(_) => "protocol",
            Self::Socket => "socket",
        }
    }
}

impl From<WebSocketError> for WebFailure {
    fn from(error: WebSocketError) -> Self {
        use tokio_tungstenite::tungstenite::Error as Socket;
        match error {
            WebSocketError::Closed { code, .. } => Self::Closed(code),
            WebSocketError::NotSecure | WebSocketError::NotThreema | WebSocketError::Tls(_) => {
                Self::Tls
            }
            WebSocketError::Text => Self::Protocol("text message"),
            WebSocketError::Socket(Socket::Io(error))
                if error
                    .get_ref()
                    .is_some_and(|inner| inner.is::<rustls::Error>()) =>
            {
                Self::Tls
            }
            WebSocketError::Socket(_) => Self::Socket,
        }
    }
}

impl From<saltyrtc::Error> for WebFailure {
    fn from(error: saltyrtc::Error) -> Self {
        match error {
            saltyrtc::Error::Protocol(text) => Self::Protocol(text),
            saltyrtc::Error::NotConnected => Self::Protocol("not connected"),
        }
    }
}

/// From the app to a session task.
pub(crate) enum ToWeb {
    Send(Value),
    Stop(StopReason),
}

/// What the browser is told when the app ends a session (`connectionDisconnect`'s `reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopReason {
    /// Disconnected for now: the pairing stays and the browser may come back.
    Stop,
    /// Removed here: the browser should forget the session.
    Delete,
    /// Threema Web was switched off: the pairing stays for when it is on again.
    Disable,
}

impl StopReason {
    fn wire(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Delete => "delete",
            Self::Disable => "disable",
        }
    }
}

/// A self-hosted server's new certificate, waiting for the user's word. It has to name the host
/// itself: a session the browser never stored is gone by the time the user answers.
struct ChangedCertificate {
    host: String,
    spki: String,
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
    /// A changed certificate per session, until the user answers or the session starts again.
    changed: HashMap<i64, ChangedCertificate>,
    /// What each connected browser subscribed to (app_web_data).
    pub(crate) subscriptions: HashMap<i64, crate::app_web_data::Subscriptions>,
    /// `request/blob`s waiting for a download.
    pub(crate) pending_blobs: Vec<crate::app_web_media::PendingBlob>,
    /// What the browser is to be told once it has the reply to the request in hand.
    pub(crate) alert_after_reply: Option<Value>,
    /// When the browsers were last brought up to date (app_web_data::web_refresh).
    pub(crate) refresh: crate::app_web_data::RefreshState,
    pub(crate) thumbnails: std::cell::RefCell<crate::app_web_media::WebThumbnails>,
    /// Since when the connection to the chat server has been up.
    pub(crate) connected_since: Option<std::time::Instant>,
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
            alert_after_reply: None,
            refresh: crate::app_web_data::RefreshState::default(),
            thumbnails: std::cell::RefCell::default(),
            connected_since: None,
            events,
            events_rx: Some(events_rx),
        }
    }
}

impl App {
    /// The receiver of session events (taken once by the core loop).
    pub(crate) fn take_web_events(&mut self) -> Option<tokio::sync::mpsc::Receiver<WebEvent>> {
        // Sessions the browser never stored can't come back: leftovers of a killed app.
        let mut sessions = self.store.web_sessions().unwrap_or_default();
        sessions.retain(|session| {
            if !session.persistent {
                let _ = self.store.remove_web_session(session.id);
            }
            session.persistent
        });
        // One browser, one session: older versions let the same code pair twice. A wake-up only
        // reaches one of them, so the one it used last stays.
        sessions.sort_by_key(|session| {
            std::cmp::Reverse(session.last_connection.unwrap_or(session.created_at))
        });
        let mut browsers = HashSet::new();
        for session in &sessions {
            if !browsers.insert(session.initiator_key) {
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

    /// The user's switch. Off, nothing connects: not from a scanned code, a tap or a wake-up.
    fn web_enabled(&self) -> Result<bool, ApiError> {
        Ok(self.store.setting(keys::WEB_ENABLED)?.as_deref() != Some("0"))
    }

    /// `webSetEnabled {enabled}`: like `webSessions`. Off also disconnects what is running; the
    /// pairings stay.
    pub(crate) fn web_set_enabled(&mut self, enabled: bool) -> Result<serde_json::Value, ApiError> {
        self.store
            .set_setting(keys::WEB_ENABLED, if enabled { "1" } else { "0" })?;
        if !enabled {
            self.web_stop_all(StopReason::Disable);
        }
        self.web_sessions_json()
    }

    /// `webSessions`: `{enabled, sessions: [{id, host, selfHosted, persistent, client, createdAt,
    /// lastConnection, state}], servers: [host], certificates: [{id, host}]}` (`servers`: the
    /// self-hosted ones allowed; `certificates`: new ones waiting for the user's word).
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
        let mut certificates: Vec<_> = self.web.changed.iter().collect();
        certificates.sort_unstable_by_key(|(id, _)| **id);
        let certificates: Vec<serde_json::Value> = certificates
            .into_iter()
            .map(|(id, changed)| json!({ "id": id, "host": changed.host }))
            .collect();
        Ok(json!({
            "enabled": self.web_enabled()?,
            "sessions": sessions,
            "servers": self.store.web_hosts()?,
            "certificates": certificates,
        }))
    }

    /// `webPair {qr, allowHost}`: a new session from Threema Web's code. A server outside
    /// threema.ch needs `allowHost` once (`{confirmHost}` asks for it).
    pub(crate) fn web_pair(
        &mut self,
        code: &str,
        allow_host: bool,
    ) -> Result<serde_json::Value, ApiError> {
        if !self.web_enabled()? {
            return Err(ApiError::WebDisabled);
        }
        let qr = qr::parse(code).map_err(|error| match error {
            qr::QrError::Invalid => ApiError::WebCodeInvalid,
            qr::QrError::Version(_) => ApiError::WebVersion,
        })?;
        let same_browser = self.web_sessions_of_browser(&qr.initiator_key)?;
        if self.web_session_in_use(&same_browser) {
            return Err(ApiError::WebAlreadyPaired);
        }
        if self.web_host_needs_asking(&qr.host)? {
            if !allow_host {
                return Ok(json!({ "confirmHost": qr.host }));
            }
            self.store.set_web_host(&qr.host, None)?;
        }
        // Left by a kill during pairing: it never worked, so nobody has to remove it first.
        for stale in same_browser {
            self.store.remove_web_session(stale.id)?;
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

    fn web_sessions_of_browser(
        &self,
        initiator_key: &[u8; 32],
    ) -> Result<Vec<WebSession>, ApiError> {
        let mut sessions = self.store.web_sessions()?;
        sessions.retain(|session| session.initiator_key == *initiator_key);
        Ok(sessions)
    }

    /// One of these sessions works, or can come back: a second one would only compete with it
    /// on the same SaltyRTC path. Android refuses this too.
    fn web_session_in_use(&self, sessions: &[WebSession]) -> bool {
        sessions
            .iter()
            .any(|session| session.persistent || self.web.running.contains_key(&session.id))
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
        // Say so before asking anything about pairing it.
        if code["kind"] == "web"
            && let Ok(qr) = qr::parse(text)
        {
            if !self.web_enabled()? {
                code["error"] = json!(ApiError::WebDisabled.kind());
            } else if self.web_session_in_use(&self.web_sessions_of_browser(&qr.initiator_key)?) {
                code["error"] = json!(ApiError::WebAlreadyPaired.kind());
            }
        }
        Ok(code)
    }

    /// `webStart {session}`: connect a stored session (the browser must be waiting for us).
    pub(crate) fn web_start(&mut self, id: i64) -> Result<serde_json::Value, ApiError> {
        self.start_web_session(id, None)?;
        Ok(json!({}))
    }

    /// The push relay woke us for a stored session (its browser key's hex SHA-256): connect it,
    /// if the browser stored it too and Threema Web is on.
    pub(crate) fn web_resume(&mut self, hash: &str) {
        use sha2::{Digest as _, Sha256};
        if !self.web_enabled().unwrap_or(false) {
            tracing::info!("Threema Web wake-up while it is off");
            return;
        }
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
            let _ = running.tx.send(ToWeb::Stop(StopReason::Stop));
        }
        json!({})
    }

    /// Stop every session at once and forget what they were waiting for: the identity they were
    /// paired with is going away (security audit 2026-10-27, S01) with `Delete`; the license was
    /// refused with `Stop` or Threema Web switched off with `Disable`, which leave the pairings.
    /// Their tasks have no say any more, so this is also where their end is reported.
    pub(crate) fn web_stop_all(&mut self, reason: StopReason) {
        let running: Vec<(i64, Running)> = self.web.running.drain().collect();
        for (id, running) in running {
            // As when its task ends by itself: one the browser never stored can't come back, so
            // it goes, and the browser hears that (Android does this when switched off too).
            let never_stored = matches!(
                self.store.web_session(id),
                Ok(Some(session)) if !session.persistent
            );
            let reason = if never_stored {
                StopReason::Delete
            } else {
                reason
            };
            let _ = running.tx.send(ToWeb::Stop(reason));
            if never_stored {
                let _ = self.store.remove_web_session(id);
            }
            self.emit_web_state(id, "stopped", None);
        }
        self.web.changed.clear();
        self.web.subscriptions.clear();
        self.web.pending_blobs.clear();
    }

    /// `webDelete {session}`: stop and forget the session.
    pub(crate) fn web_delete(&mut self, id: i64) -> Result<serde_json::Value, ApiError> {
        let running = self.web.running.remove(&id);
        if let Some(running) = &running {
            let _ = running.tx.send(ToWeb::Stop(StopReason::Delete));
        }
        // Nothing waiting for it may reach a later session with the same ID.
        self.web.changed.remove(&id);
        self.web.subscriptions.remove(&id);
        self.web.pending_blobs.retain(|blob| blob.session != id);
        self.store.remove_web_session(id)?;
        // Its task has no say any more, so its end is reported here.
        if running.is_some() {
            self.emit_web_state(id, "stopped", None);
        }
        Ok(json!({}))
    }

    /// `webRemoveServer {host}`: the user takes back a self-hosted server's permission. Its
    /// sessions go with it, as nothing may connect there any more, and with the permission the
    /// pin: the next code naming the server asks again (ADR 0013).
    pub(crate) fn web_remove_server(&mut self, host: &str) -> Result<serde_json::Value, ApiError> {
        for session in self.store.web_sessions()? {
            if session.host == host {
                self.web_delete(session.id)?;
            }
        }
        // Also those of sessions that are gone already.
        self.web.changed.retain(|_, changed| changed.host != host);
        self.store.remove_web_host(host)?;
        self.web_sessions_json()
    }

    /// `webAcceptCertificate {session}`: the self-hosted server's new certificate is fine; pin it
    /// and connect again. `{scanAgain: true}`: the session was one the browser hadn't stored, so
    /// its auth token is gone with it and the next connection needs the browser's code again.
    pub(crate) fn web_accept_certificate(
        &mut self,
        id: i64,
    ) -> Result<serde_json::Value, ApiError> {
        let changed = self
            .web
            .changed
            .remove(&id)
            .ok_or(ApiError::WebSessionNotFound)?;
        self.store
            .set_web_host(&changed.host, Some(&changed.spki))?;
        // The pin is the server's, so this answers for its other sessions too.
        self.web
            .changed
            .retain(|_, other| other.host != changed.host);
        if self
            .store
            .web_session(id)?
            .is_some_and(|session| session.persistent && session.host == changed.host)
        {
            self.start_web_session(id, None)?;
            Ok(json!({}))
        } else {
            Ok(json!({ "scanAgain": true }))
        }
    }

    /// `webDeclineCertificate {session}`: not this certificate; the pin stays, and the session
    /// asks again the next time it connects.
    pub(crate) fn web_decline_certificate(&mut self, id: i64) -> serde_json::Value {
        self.web.changed.remove(&id);
        json!({})
    }

    fn start_web_session(
        &mut self,
        id: i64,
        token: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<(), ApiError> {
        if !self.web_enabled()? {
            return Err(ApiError::WebDisabled);
        }
        if self.web.running.contains_key(&id) {
            return Ok(());
        }
        let session = self
            .store
            .web_session(id)?
            .ok_or(ApiError::WebSessionNotFound)?;
        // Any question about an earlier attempt, perhaps of a session that had this ID before,
        // is asked anew if it still applies.
        self.web.changed.remove(&id);
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
                if let Some(error) = error {
                    tracing::warn!(failure = ?error, "Threema Web session ended");
                }
                self.emit_web_state(session, "stopped", error.map(WebFailure::code));
            }
            FromWeb::Pinned { host, spki, .. } => {
                let _ = self.store.set_web_host(&host, Some(&spki));
            }
            FromWeb::CertificateChanged { session, spki } => {
                let Some(host) = self
                    .store
                    .web_session(session)
                    .ok()
                    .flatten()
                    .map(|stored| stored.host)
                else {
                    return;
                };
                self.web.changed.insert(
                    session,
                    ChangedCertificate {
                        host: host.clone(),
                        spki,
                    },
                );
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
            let alert = self.web.alert_after_reply.take();
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
            if let Some(alert) = alert {
                self.send_web(session, alert);
            }
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
            ("request", "profile") => {
                let reply = self
                    .web_profile()
                    .map(|data| arp::message("response", "profile", None, Some(data)));
                if reply.is_ok() {
                    self.web_subscribe_profile(session);
                }
                reply
            }
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

    /// `response/profile`: our ID, public key, nickname and picture.
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
        if let Some(picture) = self.web_own_picture() {
            fields.push(("avatar", Value::Binary(picture)));
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
) -> Result<bool, WebFailure> {
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
    }
    .map_err(|_| WebFailure::Tls)?;
    let connected = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        Datagrams::connect_with(&url, Some(saltyrtc::SUBPROTOCOL), tls),
    )
    .await
    .map_err(|_| WebFailure::Timeout)
    .and_then(|result| result.map_err(WebFailure::from));
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
                return Err(WebFailure::CertificateChanged);
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
                let frame = frame?;
                let saltyrtc_events = match responder.receive(&frame) {
                    Ok(saltyrtc_events) => saltyrtc_events,
                    Err(error) => {
                        socket.close().await;
                        return Err(error.into());
                    }
                };
                for event in saltyrtc_events {
                    match event {
                        Event::Send(frame) => out.push(frame),
                        Event::PeerConnected => {
                            let id = responder
                                .connection_id()
                                .ok_or(WebFailure::Protocol("no connection ID"))?;
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
                command @ (Some(ToWeb::Stop(_)) | None) => {
                    let reason = match command {
                        Some(ToWeb::Stop(reason)) => reason,
                        _ => StopReason::Stop,
                    };
                    if let Some(connection) = connection.as_mut() {
                        for action in connection.send(goodbye(reason)) {
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
                Action::Chunk(chunk) => match responder.send_data(Value::Binary(chunk)) {
                    Ok(frame) => out.push(frame),
                    // Between two browsers: the old connection's last words go nowhere.
                    Err(saltyrtc::Error::NotConnected) => {}
                    Err(error) => return Err(error.into()),
                },
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
            socket.send(frame).await?;
        }
    }
}

/// The last message on a connection the app ends.
fn goodbye(reason: StopReason) -> Value {
    arp::message(
        "update",
        "connectionDisconnect",
        None,
        Some(arp::map(vec![("reason", reason.wire().into())])),
    )
}

/// After the server handshake without a browser on the path: we wait for it.
fn responder_waiting(sent: &[Vec<u8>]) -> bool {
    sent.is_empty()
}

#[cfg(test)]
#[path = "app_web_security_tests.rs"]
mod security_tests;
