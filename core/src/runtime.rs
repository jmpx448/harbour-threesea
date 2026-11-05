// SPDX-License-Identifier: AGPL-3.0-only
//! The core thread: a single-threaded tokio runtime owning the [`App`]. libthreema's protocol
//! state is not `Send`, so everything stays on this one thread; the bridge talks to it only via
//! a channel of JSON strings and a callback.

use std::{sync::Arc, thread::JoinHandle};

use serde_json::json;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::{
    app::{ApiError, App, StartConfig},
    connection::FromConnection,
    media::MediaStore,
    providers::CoreEvent,
};

/// A request for the core thread: JSON plus binary attachments.
pub struct CoreRequest {
    json: Zeroizing<String>,
    attachments: Vec<Zeroizing<Vec<u8>>>,
}

/// Receives every JSON message (replies and events) from the core thread.
pub type Emit = Box<dyn Fn(String) + Send>;

/// Handle to a running core thread. Dropping it stops the core and joins the thread.
pub struct CoreHandle {
    tx: Option<mpsc::UnboundedSender<CoreRequest>>,
    thread: Option<JoinHandle<()>>,
    media: Arc<MediaStore>,
}

impl CoreHandle {
    /// Start the core. It emits `{"event":"started"}` or `{"event":"fatal","kind":...,
    /// "message":...}`, once the store is open; `{"event":"upgrading"}` before that if the store's
    /// schema is brought up to date first. On an error here `fatal` has told why as well.
    pub fn start(
        config: StartConfig,
        db_key: Zeroizing<[u8; 32]>,
        emit: Emit,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::unbounded_channel::<CoreRequest>();
        let media = std::fs::create_dir_all(&config.data_dir)
            .and_then(|()| MediaStore::open(&config.data_dir, &db_key));
        let media = match media {
            Ok(media) => Arc::new(media),
            Err(error) => {
                let kind = if error.kind() == std::io::ErrorKind::StorageFull {
                    "diskFull"
                } else {
                    "storeUnreadable"
                };
                let code = error
                    .raw_os_error()
                    .map_or(String::new(), |code| format!("os {code}"));
                tracing::error!(kind, code, "could not set up the data directory");
                emit(json!({"event": "fatal", "kind": kind, "message": code}).to_string());
                return Err(error);
            }
        };
        let core_media = Arc::clone(&media);
        let thread = std::thread::Builder::new()
            .name("threesea-core".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        emit(json!({"event": "fatal", "kind": "internal", "message": error.to_string()}).to_string());
                        return;
                    },
                };
                let upgrading = || {
                    tracing::info!("bringing the store up to date");
                    emit(json!({"event": "upgrading"}).to_string());
                };
                let app = match App::with_media(&config, &db_key, core_media, &upgrading) {
                    Ok(app) => app,
                    Err(error) => {
                        let kind = error.kind();
                        // Only the store's are codes; anything else could say anything.
                        let code = if matches!(error, ApiError::StoreFailed { .. }) {
                            error.to_string()
                        } else {
                            String::new()
                        };
                        tracing::error!(kind, code, "could not open the stored data");
                        emit(json!({"event": "fatal", "kind": kind, "message": code}).to_string());
                        return;
                    },
                };
                drop(db_key);
                emit(json!({"event": "started", "version": crate::VERSION}).to_string());
                // A LocalSet: the chat server connection runs as a local task next to the
                // command loop (libthreema state is not `Send`).
                let local = tokio::task::LocalSet::new();
                local.block_on(&runtime, run_loop(app, rx, emit));
            })?;
        Ok(Self {
            tx: Some(tx),
            thread: Some(thread),
            media,
        })
    }

    /// The media store, readable from any thread.
    pub fn media(&self) -> &MediaStore {
        &self.media
    }

    /// Queue a JSON request for the core.
    pub fn send(&self, request: String) {
        self.send_with(request, Vec::new());
    }

    /// Queue a JSON request with binary attachments (see `sendFile`).
    pub fn send_with(&self, request: String, attachments: Vec<Zeroizing<Vec<u8>>>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(CoreRequest {
                json: Zeroizing::new(request),
                attachments,
            });
        }
    }
}

impl Drop for CoreHandle {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The log level the user chose (Settings → Advanced), changeable while running.
static LOG_LEVEL: std::sync::OnceLock<
    tracing_subscriber::reload::Handle<
        tracing_subscriber::filter::LevelFilter,
        tracing_subscriber::Registry,
    >,
> = std::sync::OnceLock::new();

/// The in-app log (Settings → Advanced → Log): the same lines that go to the journal, kept
/// in the app's own directory so the troubleshooting page can show and export them without
/// terminal access. The content rules of CLAUDE.md §7.3 hold at every level, so the file
/// never carries message text, keys, numbers or addresses. Rotated once, bounded size.
const LOG_FILE: &str = "threesea.log";
const LOG_FILE_OLD: &str = "threesea.log.0";
const LOG_ROTATE_BYTES: u64 = 512 * 1024;

struct LogSink {
    path: std::path::PathBuf,
    file: Option<std::fs::File>,
    written: u64,
}

impl LogSink {
    fn open(dir: &std::path::Path) -> Self {
        let path = dir.join(LOG_FILE);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
        let written = file
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map_or(0, |data| data.len());
        Self {
            path,
            file,
            written,
        }
    }

    fn write(&mut self, buf: &[u8]) {
        use std::io::Write as _;
        if self.written >= LOG_ROTATE_BYTES {
            self.file = None;
            let _ = std::fs::rename(&self.path, self.path.with_file_name(LOG_FILE_OLD));
            self.file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .ok();
            self.written = 0;
        }
        if let Some(file) = self.file.as_mut()
            && file.write_all(buf).is_ok()
        {
            self.written += buf.len() as u64;
        }
    }
}

static LOG_SINK: std::sync::Mutex<Option<LogSink>> = std::sync::Mutex::new(None);

/// A writer the fmt layer can hand out per event: appends to the sink, if one is attached.
struct SinkWriter;

impl std::io::Write for SinkWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut guard) = LOG_SINK.lock()
            && let Some(sink) = guard.as_mut()
        {
            sink.write(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A line from the bridge (Qt/QML warnings), into the same outputs as the core's own.
pub fn log_bridge(level: &str, message: &str) {
    match level {
        "debug" => tracing::debug!(target: "bridge", "{message}"),
        "info" => tracing::info!(target: "bridge", "{message}"),
        _ => tracing::warn!(target: "bridge", "{message}"),
    }
}

/// The log as text: the rotated file first, at most the newest `most` bytes (whole lines).
/// The viewer shows a small tail; the support package takes everything rotation kept.
pub fn log_text(dir: &std::path::Path, most: usize) -> String {
    let mut text = String::new();
    for name in [LOG_FILE_OLD, LOG_FILE] {
        if let Ok(part) = std::fs::read_to_string(dir.join(name)) {
            text.push_str(&part);
        }
    }
    if text.len() > most {
        let bytes = text.as_bytes();
        let mut cut = text.len() - most;
        // Start at a whole line (drop the first, cut-off one).
        if cut > 0 && bytes[cut - 1] != b'\n' {
            cut = bytes[cut..]
                .iter()
                .position(|b| *b == b'\n')
                .map_or(text.len(), |n| cut + n + 1);
        }
        text = String::from_utf8_lossy(&bytes[cut..]).into_owned();
    }
    text
}

/// Remove the log files (the troubleshooting page's "Clear").
pub fn log_clear(dir: &std::path::Path) {
    if let Ok(mut guard) = LOG_SINK.lock()
        && let Some(sink) = guard.as_mut()
    {
        sink.file = None;
        sink.written = 0;
    }
    for name in [LOG_FILE_OLD, LOG_FILE] {
        let _ = std::fs::remove_file(dir.join(name));
    }
    if let Ok(mut guard) = LOG_SINK.lock()
        && let Some(sink) = guard.as_mut()
    {
        *sink = LogSink::open(sink.path.parent().unwrap_or(dir));
    }
}

/// In the data directory from just before the store empties until the directory is emptied.
const REMOVAL_MARK: &str = "removal-pending";

/// The ID is going: a start that finds this finishes the removal (ADR 0018), and still knows
/// whether linked devices remain.
///
/// # Errors
/// The mark could not be written.
pub fn mark_removal(dir: &std::path::Path, linked_devices_remain: bool) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::File::create(dir.join(REMOVAL_MARK))?;
    file.write_all(if linked_devices_remain { b"1" } else { b"0" })?;
    file.sync_all()
}

/// A removal was under way when the app last ended: whether linked devices remain. A mark that
/// doesn't say counts as yes: better a notice too many than one missing.
pub fn removal_mark(dir: &std::path::Path) -> Option<bool> {
    match std::fs::read(dir.join(REMOVAL_MARK)) {
        Ok(content) => Some(content != b"0"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(true),
    }
}

/// Remove everything in the data directory: the database with its `-wal` and `-shm`, media and
/// logs, whatever may have been added there since. After the core has stopped; the next start
/// makes a new database in the emptied directory (ADR 0018). The log attaches to a new file.
///
/// # Errors
/// The first entry that could not be removed; the others are still tried.
pub fn remove_data_dir(dir: &std::path::Path) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        entries => entries?,
    };
    let mut first_error = None;
    for entry in entries {
        let removed = entry.and_then(|entry| {
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(entry.path())
            } else {
                std::fs::remove_file(entry.path())
            }
        });
        if let Err(error) = removed {
            first_error.get_or_insert(error);
        }
    }
    log_clear(dir);
    first_error.map_or(Ok(()), Err)
}

/// Install stderr logging (ends up in the journal on device) and, with a directory, the
/// in-app log file: warnings and errors until [`set_log_level`] says otherwise;
/// `THREESEA_LOG` (an env filter) overrides it for development. Idempotent; the file sink
/// attaches on whichever call first brings a directory.
pub fn init_logging(dir: Option<&std::path::Path>) {
    use tracing_subscriber::prelude::*;
    if let Some(dir) = dir
        && let Ok(mut guard) = LOG_SINK.lock()
        && guard.is_none()
    {
        *guard = Some(LogSink::open(dir));
    }
    macro_rules! output {
        () => {
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(false)
        };
    }
    macro_rules! file_output {
        () => {
            tracing_subscriber::fmt::layer()
                .with_writer(|| SinkWriter)
                .with_ansi(false)
        };
    }
    if let Ok(filter) = tracing_subscriber::EnvFilter::try_from_env("THREESEA_LOG") {
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(output!())
            .with(file_output!())
            .try_init();
        return;
    }
    let (level, handle) =
        tracing_subscriber::reload::Layer::new(tracing_subscriber::filter::LevelFilter::WARN);
    if tracing_subscriber::registry()
        .with(level)
        .with(output!())
        .with(file_output!())
        .try_init()
        .is_ok()
    {
        let _ = LOG_LEVEL.set(handle);
    }
}

/// `warn` (default), `info` or `debug`. No level logs content (CLAUDE.md §7.3).
pub fn set_log_level(level: &str) {
    use tracing_subscriber::filter::LevelFilter;
    let filter = match level {
        "debug" => LevelFilter::DEBUG,
        "info" => LevelFilter::INFO,
        _ => LevelFilter::WARN,
    };
    if let Some(handle) = LOG_LEVEL.get() {
        let _ = handle.modify(|current| *current = filter);
    }
}

/// An event from the core's own queue (media, marks): the UI and the browsers learn of it.
fn on_media_event(app: &mut App, event: &CoreEvent, emit: &Emit) {
    app.gate_media(event);
    emit(event.to_json(&app.store, app.unlocked).to_string());
    app.on_media_event(event);
    app.web_on_core_event(event);
    app.web_refresh();
}

/// After a command: what it queued goes out before the browsers' refresh. A group taken out of
/// the private chats reaches them as a receiver first, then as a conversation of that receiver.
pub(crate) fn settle_command(
    app: &mut App,
    media_events: &mut Option<mpsc::UnboundedReceiver<CoreEvent>>,
    emit: &Emit,
) {
    if let Some(events) = media_events.as_mut() {
        while let Ok(event) = events.try_recv() {
            on_media_event(app, &event, emit);
        }
    }
    app.web_refresh();
}

/// A stopped connection has no more to say. What it said before that lies unread in its channel,
/// and must not take effect now: a "connected" from a login that was stopped would start what runs
/// on connecting. It can't say "disconnected" itself any more either.
fn forget_stopped_connection(
    app: &App,
    connection: &mut Option<mpsc::UnboundedReceiver<FromConnection>>,
    emit: &Emit,
) {
    if app.connection.is_none() && connection.take().is_some() {
        emit(json!({"event": "connectionState", "state": app.connection_state}).to_string());
    }
}

/// The connection is through with a wake-up: if transfers are running too, the device stays up for
/// those (the window's limit holds either way).
fn wake_settled(app: &mut App, emit: &Emit) {
    app.wake_settled_held = !app.transfers.none_running();
    if !app.wake_settled_held {
        emit(json!({"event": "wakeSettled"}).to_string());
    }
}

pub(crate) async fn run_maintenance(app: &mut App, emit: &Emit) {
    app.maintenance_ran_at = std::time::SystemTime::now();
    for event in app.maintenance().await {
        emit(json!({ "event": event }).to_string());
    }
    // Keep messages may have taken some out of what the browsers show.
    app.web_refresh();
    app.start_update_check(false);
}

/// The core loop: commands (in order), connection events, hourly maintenance.
async fn run_loop(mut app: App, mut rx: mpsc::UnboundedReceiver<CoreRequest>, emit: Emit) {
    let mut tick = tokio::time::interval(crate::app::MAINTENANCE_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    if let Err(error) = crate::sync::reflect_group_contacts_once(&app.store) {
        tracing::warn!(%error, "group contacts not queued for the other devices");
    }
    let mut connection: Option<mpsc::UnboundedReceiver<FromConnection>> = app.ensure_connection();
    let mut media_events = app.take_media_events();
    let mut web_events = app.take_web_events();
    app.resume_media();
    // Downloads of contact and group pictures that never finished (kept descriptors).
    app.resume_picture_downloads();
    // Locked at start: the private chats' media stay hidden from the UI's readers.
    app.update_media_gate();
    loop {
        forget_stopped_connection(&app, &mut connection, &emit);
        let transfer_retry_at = app.transfer_retry_at;
        let wake_settled_held = app.wake_settled_held;
        tokio::select! {
            request = rx.recv() => {
                let Some(request) = request else { break };
                emit(app.handle_with(&request.json, request.attachments).await);
                for event in app.take_ui_events() {
                    emit(json!({ "event": event }).to_string());
                }
                settle_command(&mut app, &mut media_events, &emit);
                if app.refresh_imported_contacts().await {
                    emit(json!({ "event": "contactsChanged" }).to_string());
                    app.web_refresh();
                }
                // An ID may just have been created/restored (or removed).
                if let Some(started) = app.ensure_connection() {
                    connection = Some(started);
                    if let Err(error) = app.publish_feature_mask().await {
                        tracing::warn!(kind = error.kind(), "publishing the feature mask failed");
                    }
                }
                if core::mem::take(&mut app.maintenance_overdue) {
                    run_maintenance(&mut app, &emit).await;
                    tick.reset();
                }
            }
            event = async {
                match connection.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match event {
                    None => connection = None,
                    Some(FromConnection::State(state)) => {
                        app.connection_state = state.as_str();
                        app.web_connection_changed(matches!(
                            state,
                            crate::connection::ConnectionState::Connected
                        ));
                        emit(json!({"event": "connectionState", "state": state.as_str()}).to_string());
                        // Online again: finish a restored backup's pending import right away
                        // (maintenance alone would retry only hourly).
                        if matches!(state, crate::connection::ConnectionState::Connected) {
                            for event in app.on_connected().await {
                                emit(json!({ "event": event }).to_string());
                            }
                            app.web_refresh();
                        }
                    }
                    Some(FromConnection::Event(event)) => {
                        app.gate_media(&event);
                        app.on_core_event(&event);
                        app.web_on_core_event(&event);
                        app.web_refresh();
                        emit(event.to_json(&app.store, app.unlocked).to_string());
                        if matches!(*event, CoreEvent::MultiDeviceChanged) {
                            app.apply_multi_device();
                            if let Err(error) = app.publish_feature_mask().await {
                                tracing::warn!(kind = error.kind(), "publishing the feature mask failed");
                            }
                        }
                    }
                    Some(FromConnection::OfflineReason(reason)) => {
                        app.web_server_said(&reason);
                        emit(
                            json!({"event": "offlineReason", "reason": reason.to_json()})
                                .to_string(),
                        );
                    }
                    Some(FromConnection::Backlog(active)) => {
                        emit(json!({"event": "backlog", "active": active}).to_string());
                    }
                    Some(FromConnection::WakeSettled) => wake_settled(&mut app, &emit),
                }
            }
            event = async {
                match media_events.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(event) = event {
                    on_media_event(&mut app, &event, &emit);
                }
            }
            event = async {
                match web_events.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(event) = event {
                    app.on_web_event(event).await;
                    app.web_refresh();
                }
            }
            () = async {
                match transfer_retry_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => app.retry_failed_transfers(),
            () = app.transfers.ended(), if wake_settled_held => wake_settled(&mut app, &emit),
            _ = tick.tick() => run_maintenance(&mut app, &emit).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn a_wake_up_stays_for_a_transfer_that_is_running() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let mut app = App::new(&start_config(dir.path()), &[1; 32]).unwrap();
                let said = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                let emit: Emit = {
                    let said = std::sync::Arc::clone(&said);
                    Box::new(move |message| said.lock().unwrap().push(message))
                };

                let (finish, transfer) = tokio::sync::oneshot::channel::<()>();
                tokio::task::spawn_local(app.transfers.track(async move {
                    let _ = transfer.await;
                }));
                wake_settled(&mut app, &emit);
                assert!(app.wake_settled_held);
                assert!(said.lock().unwrap().is_empty());

                finish.send(()).unwrap();
                app.transfers.ended().await;
                wake_settled(&mut app, &emit);
                assert!(!app.wake_settled_held);
                assert_eq!(said.lock().unwrap().len(), 1);
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_wake_up_runs_maintenance_when_an_hour_has_passed_by_the_clock() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(&start_config(dir.path()), &[1; 32]).unwrap();
        app.wake_connection();
        assert!(!app.maintenance_overdue, "it has just run");

        let now = std::time::SystemTime::now();
        app.maintenance_ran_at = now - crate::app::MAINTENANCE_INTERVAL;
        app.wake_connection();
        assert!(app.maintenance_overdue);

        // The clock set back says nothing: the tick is still there.
        app.maintenance_ran_at = now + crate::app::MAINTENANCE_INTERVAL;
        app.wake_connection();
        assert!(!app.maintenance_overdue);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_new_wake_up_does_not_end_with_the_word_held_back_for_the_last() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(&start_config(dir.path()), &[1; 32]).unwrap();
        app.wake_settled_held = true;
        app.wake_connection();
        assert!(!app.wake_settled_held);
    }

    #[test]
    fn what_a_stopped_connection_said_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(&start_config(dir.path()), &[1; 32]).unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(FromConnection::State(
            crate::connection::ConnectionState::Connected,
        ))
        .unwrap();
        let mut connection = Some(rx);
        let said = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let emit: Emit = {
            let said = std::sync::Arc::clone(&said);
            Box::new(move |message| said.lock().unwrap().push(message))
        };

        // Still running: it keeps its channel.
        let (to_connection, _to_rx) = mpsc::unbounded_channel();
        app.connection = Some(to_connection);
        forget_stopped_connection(&app, &mut connection, &emit);
        assert!(connection.is_some());
        assert!(said.lock().unwrap().is_empty());

        app.stop_connection();
        forget_stopped_connection(&app, &mut connection, &emit);
        assert!(connection.is_none());
        assert!(tx.is_closed(), "nobody reads the \"connected\"");
        let said = said.lock().unwrap();
        assert_eq!(said.len(), 1);
        let state: serde_json::Value = serde_json::from_str(&said[0]).unwrap();
        assert_eq!(state["state"], "disconnected");
    }

    fn start_config(dir: &std::path::Path) -> StartConfig {
        serde_json::from_value(json!({
            "dataDir": dir,
            "appVersion": "0.0.0",
            "language": "en",
            "country": "CH",
            "model": "test",
            "osVersion": "5.1",
        }))
        .unwrap()
    }

    /// Starts the core and waits for what it says first.
    fn first_answer(dir: &std::path::Path, key: [u8; 32]) -> serde_json::Value {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let emit: Emit = Box::new(move |message| {
            let _ = tx.lock().unwrap().send(message);
        });
        let _handle = CoreHandle::start(start_config(dir), Zeroizing::new(key), emit);
        let message = rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        serde_json::from_str(&message).unwrap()
    }

    #[test]
    fn a_failed_start_gives_the_reason_as_a_kind_and_a_code() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(first_answer(dir.path(), [1; 32])["event"], "started");

        let wrong_key = first_answer(dir.path(), [2; 32]);
        assert_eq!(wrong_key["event"], "fatal");
        assert_eq!(wrong_key["kind"], "storeKey", "{wrong_key}");
        assert_eq!(wrong_key["message"], "", "{wrong_key}");
    }

    #[test]
    fn a_data_directory_that_cannot_be_made_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, b"in the way").unwrap();
        let answer = first_answer(&file.join("data"), [1; 32]);
        assert_eq!(answer["event"], "fatal");
        assert_eq!(answer["kind"], "storeUnreadable", "{answer}");
        assert!(
            answer["message"].as_str().unwrap().starts_with("os "),
            "{answer}"
        );
    }

    #[test]
    fn remove_data_dir_leaves_it_empty() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "threesea.db",
            "threesea.db-wal",
            "threesea.db-shm",
            LOG_FILE,
            LOG_FILE_OLD,
        ] {
            std::fs::write(dir.path().join(name), b"old").unwrap();
        }
        std::fs::create_dir_all(dir.path().join("media/inner")).unwrap();
        std::fs::write(dir.path().join("media/inner/1.file"), b"old").unwrap();

        assert_eq!(removal_mark(dir.path()), None);
        mark_removal(dir.path(), false).unwrap();
        assert_eq!(removal_mark(dir.path()), Some(false));
        mark_removal(dir.path(), true).unwrap();
        assert_eq!(removal_mark(dir.path()), Some(true));
        // Killed between creating it and writing to it.
        std::fs::write(dir.path().join(REMOVAL_MARK), b"").unwrap();
        assert_eq!(removal_mark(dir.path()), Some(true));

        remove_data_dir(dir.path()).unwrap();

        // The process-wide log sink belongs to no test here, so it may or may not reopen a log.
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name != LOG_FILE)
            .collect();
        assert!(left.is_empty(), "{left:?}");
        assert_eq!(removal_mark(dir.path()), None);
        // Nothing there is nothing to do: a repeat after a crash, say.
        remove_data_dir(&dir.path().join("not there")).unwrap();
    }

    #[test]
    fn log_file_rotates_reads_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let mut sink = LogSink::open(dir.path());
        sink.write(b"first line\n");
        // Pretend the file grew past the bound: the next write rotates.
        sink.written = LOG_ROTATE_BYTES;
        sink.write(b"second line\n");
        let old = std::fs::read_to_string(dir.path().join(LOG_FILE_OLD)).unwrap();
        assert!(old.contains("first line"));
        let text = log_text(dir.path(), usize::MAX);
        let (first, second) = (
            text.find("first line").unwrap(),
            text.find("second line").unwrap(),
        );
        assert!(first < second, "old file first");
        assert_eq!(
            log_text(dir.path(), 12).trim_end(),
            "second line",
            "the tail keeps whole lines"
        );
        log_clear(dir.path());
        assert_eq!(log_text(dir.path(), usize::MAX), "");
    }
}
