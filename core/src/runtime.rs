// SPDX-License-Identifier: AGPL-3.0-only
//! The core thread: a single-threaded tokio runtime owning the [`App`]. libthreema's protocol
//! state is not `Send`, so everything stays on this one thread; the bridge talks to it only via
//! a channel of JSON strings and a callback.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use serde_json::json;
use tokio::sync::{Notify, mpsc};
use zeroize::Zeroizing;

use crate::{
    app::{ApiError, App, StartConfig},
    connection::FromConnection,
    detached::Lane,
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

/// How long [`CoreHandle::quit`] lets the core finish what it is doing and what is queued.
const QUIT_GRACE: Duration = Duration::from_secs(3);
/// How long it then gives the core to end after being told to drop what it is waiting for.
const QUIT_CUT_OFF: Duration = Duration::from_millis(2500);
/// Of that, how long the requests behind what was dropped have to run.
const QUIT_QUEUE: Duration = Duration::from_secs(1);
/// And how long the end waits for blocking tasks; those left run on until the process ends.
const QUIT_BLOCKING_TASKS: Duration = Duration::from_secs(1);

/// What [`CoreHandle::quit`] tells the core thread.
#[derive(Default)]
struct QuitSignal {
    /// The process is ending: nothing holds the thread's end back for long.
    quitting: AtomicBool,
    /// Drop what the core is doing.
    cut_off: Notify,
}

/// Handle to a running core thread. Dropping it stops the core and joins the thread, however long
/// that takes.
pub struct CoreHandle {
    tx: Option<mpsc::UnboundedSender<CoreRequest>>,
    thread: Option<JoinHandle<()>>,
    media: Arc<MediaStore>,
    quit: Arc<QuitSignal>,
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
        let quit = Arc::new(QuitSignal::default());
        let core_quit = Arc::clone(&quit);
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
                drive(runtime, run_until_quit(app, rx, emit, &core_quit), &core_quit);
            })?;
        Ok(Self {
            tx: Some(tx),
            thread: Some(thread),
            media,
            quit,
        })
    }

    /// Stop the core for the end of the process, without waiting on it for long: what is queued
    /// and running gets [`QUIT_GRACE`], then a wait that goes on is dropped and what was queued
    /// behind it gets [`QUIT_QUEUE`]: a draft or a message from just before the window closed. All
    /// of that is what a killed process leaves too, which every command has to bear: the store's
    /// writes stand or fall together, an export is a `.part` file until it is done, and an import
    /// that was cut off fills in the rest when run again. Returns whether the thread has ended; if
    /// not, it still runs and can still emit, so the caller leaves the process at once.
    pub fn quit(self) -> bool {
        self.quit_within(QUIT_GRACE, QUIT_CUT_OFF)
    }

    fn quit_within(mut self, grace: Duration, cut_off: Duration) -> bool {
        self.quit.quitting.store(true, Ordering::SeqCst);
        drop(self.tx.take());
        let Some(thread) = self.thread.take() else {
            return true;
        };
        let ended = finished_within(&thread, grace) || {
            self.quit.cut_off.notify_one();
            finished_within(&thread, cut_off)
        };
        if ended {
            let _ = thread.join();
        } else {
            tracing::warn!("the core did not stop in time");
        }
        ended
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

fn finished_within(thread: &JoinHandle<()>, wait: Duration) -> bool {
    let until = Instant::now() + wait;
    while !thread.is_finished() {
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

/// Runs the core's work and ends the runtime.
///
/// A `LocalSet`: the chat server connection runs as a local task next to the command loop
/// (libthreema state is not `Send`).
fn drive(runtime: tokio::runtime::Runtime, work: impl Future<Output = ()>, quit: &QuitSignal) {
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, work);
    drop(local);
    if quit.quitting.load(Ordering::SeqCst) {
        runtime.shutdown_timeout(QUIT_BLOCKING_TASKS);
    }
}

/// Runs the work until it ends or a quit cuts it off. Says whether it was cut off.
async fn until_cut_off(work: impl Future<Output = ()>, quit: &QuitSignal) -> bool {
    tokio::select! {
        () = work => false,
        () = quit.cut_off.notified() => true,
    }
}

/// The command loop, and after a cut-off what it had not got to. The `App` goes with this, which
/// closes the store as on any other end.
async fn run_until_quit(
    mut app: App,
    mut rx: mpsc::UnboundedReceiver<CoreRequest>,
    emit: Emit,
    quit: &QuitSignal,
) {
    // Out here, as file work on a blocking thread still reports after a cut-off.
    let mut media_events = app.take_media_events();
    let mut file_work = app.take_file_work();
    let mut detached = app.take_detached_work();
    let work = run_loop(
        &mut app,
        &mut rx,
        &mut media_events,
        &mut file_work,
        &mut detached,
        &emit,
    );
    if until_cut_off(work, quit).await {
        tracing::warn!("quitting: cut off what the core was doing");
        app.drop_detached_work();
        let rest = async {
            run_queued_requests(&mut app, &mut rx, &emit).await;
            finish_running_file_work(&mut app, &mut file_work, &mut media_events, &emit).await;
        };
        // What doesn't make it stays without an answer, which is how the bridge knows.
        if tokio::time::timeout(QUIT_QUEUE, rest).await.is_err() {
            tracing::warn!("quitting: requests or file work left");
        }
    }
}

/// What the user did just before the end, behind the command that was cut off: its words would
/// be gone. Nothing new can arrive, the sender having gone.
async fn run_queued_requests(
    app: &mut App,
    rx: &mut mpsc::UnboundedReceiver<CoreRequest>,
    emit: &Emit,
) {
    // The command that was cut off may have been halfway: from here on all waits in place.
    app.in_background.set(false);
    while let Ok(request) = rx.try_recv() {
        emit(app.handle_with(&request.json, request.attachments).await);
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
    emit(event.to_json(&app.store, app.unlocked.get()).to_string());
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
    if app.connection.borrow().is_none() && connection.take().is_some() {
        emit(json!({"event": "connectionState", "state": app.connection_state.get()}).to_string());
    }
}

/// The connection is through with a wake-up: if transfers or a round of maintenance are running
/// too, the device stays up for those (the window's limit holds either way). A device that sleeps
/// halfway through a round would keep it going over many wake-ups, and what is in turn behind it.
fn wake_settled(app: &mut App, emit: &Emit) {
    app.wake_settled_held = !app.transfers.none_running() || app.detached.quiet_running > 0;
    if !app.wake_settled_held {
        emit(json!({"event": "wakeSettled"}).to_string());
    }
}

#[cfg(test)]
pub(crate) async fn run_maintenance(app: &mut App, emit: &Emit) {
    app.maintenance_ran_at = std::time::SystemTime::now();
    for event in app.maintenance().await {
        emit(json!({ "event": event }).to_string());
    }
    // Keep messages may have taken some out of what the browsers show.
    app.web_refresh();
    app.start_update_check(false);
}

/// How long after the start the first round of maintenance comes. Not at once, as all it asks for
/// competes with what the UI wants first; not much later, as the Lite flavour may not run for long.
const FIRST_MAINTENANCE_AFTER: Duration = Duration::from_secs(10);
/// How long the start's clean-up waits for the UI's first question.
const FIRST_REQUEST_WAIT: Duration = Duration::from_secs(1);

/// The UI's first `status` is what ends the start page, and it needs nothing of what the start
/// cleans up in the store and the media, which reads through all file messages. That first, then.
/// Anything else may depend on the clean-up: it comes back to be handled after it. Nothing else
/// runs here, so nothing can start a download for the clean-up to undo.
async fn answer_first_status(
    app: &mut App,
    rx: &mut mpsc::UnboundedReceiver<CoreRequest>,
    emit: &Emit,
) -> Option<CoreRequest> {
    #[derive(serde::Deserialize)]
    struct Named<'a> {
        cmd: &'a str,
    }
    let first = tokio::time::timeout(FIRST_REQUEST_WAIT, rx.recv())
        .await
        .ok()??;
    let is_status =
        serde_json::from_str::<Named<'_>>(&first.json).is_ok_and(|request| request.cmd == "status");
    if !is_status {
        return Some(first);
    }
    if let Some(reply) = app.handle_from_ui(&first.json, first.attachments).await {
        emit(reply);
    }
    None
}

/// A round of maintenance beside the loop, unless the last one is still going.
fn start_maintenance(app: &mut App) {
    app.maintenance_ran_at = std::time::SystemTime::now();
    if std::mem::replace(&mut app.detached.maintenance_running, true) {
        return;
    }
    app.detached_quietly(
        Lane::InTurn,
        |app| async move { app.maintenance_away().await },
        |app, round| {
            app.detached.maintenance_running = false;
            let events = app.finish_maintenance(round);
            app.note_ui_events(&events);
            app.start_update_check(false);
        },
    );
}

fn publish_feature_mask(app: &mut App) {
    app.detached_quietly(
        Lane::InTurn,
        |app| async move {
            if let Err(error) = app.publish_feature_mask().await {
                tracing::warn!(kind = error.kind(), "publishing the feature mask failed");
            }
        },
        |_, ()| {},
    );
}

/// What follows a command, whichever way its reply went out, and what it asked for.
fn after_command(
    app: &mut App,
    media_events: &mut Option<mpsc::UnboundedReceiver<CoreEvent>>,
    connection: &mut Option<mpsc::UnboundedReceiver<FromConnection>>,
    tick: &mut tokio::time::Interval,
    emit: &Emit,
) {
    for event in app.take_ui_events() {
        emit(json!({ "event": event }).to_string());
    }
    settle_command(app, media_events, emit);
    if app.imported_contacts_to_check() {
        app.detached_quietly(
            Lane::Free,
            |app| async move { app.refresh_contacts(false).await.unwrap_or(false) },
            |app, changed| {
                if changed {
                    app.note_ui_events(&["contactsChanged"]);
                }
            },
        );
    }
    // An ID may just have been created/restored (or removed).
    if let Some(started) = app.ensure_connection() {
        *connection = Some(started);
        publish_feature_mask(app);
    }
    if core::mem::take(&mut app.maintenance_overdue) {
        start_maintenance(app);
        tick.reset();
    }
}

/// A command's or a round's work beside the loop is done: finish it like any command.
async fn finish_detached_work(app: &mut App, finished: crate::detached::Finished, emit: &Emit) {
    if let Some(reply) = app.finish_detached(finished) {
        app.in_background.set(true);
        let reply = app.reply(reply.request, reply.change, reply.outcome).await;
        app.in_background.set(false);
        emit(reply);
    }
}

/// The core loop: commands (in order), connection events, hourly maintenance.
async fn run_loop(
    app: &mut App,
    rx: &mut mpsc::UnboundedReceiver<CoreRequest>,
    media_events: &mut Option<mpsc::UnboundedReceiver<CoreEvent>>,
    file_work: &mut Option<mpsc::UnboundedReceiver<crate::app_media::FinishedFileWork>>,
    detached: &mut Option<mpsc::UnboundedReceiver<crate::detached::Finished>>,
    emit: &Emit,
) {
    let mut tick = tokio::time::interval_at(
        tokio::time::Instant::now() + FIRST_MAINTENANCE_AFTER,
        crate::app::MAINTENANCE_INTERVAL,
    );
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    if let Err(error) = crate::sync::reflect_group_contacts_once(&app.store) {
        tracing::warn!(%error, "group contacts not queued for the other devices");
    }
    let mut connection: Option<mpsc::UnboundedReceiver<FromConnection>> = app.ensure_connection();
    let mut web_events = app.take_web_events();
    // Locked at start: the private chats' media stay hidden from the UI's readers.
    app.update_media_gate();
    let mut held_back = answer_first_status(app, rx, emit).await;
    app.resume_media();
    // Downloads of contact and group pictures that never finished (kept descriptors).
    app.resume_picture_downloads();
    // Group calls kept from the last run may have ended meanwhile.
    app.refresh_stored_group_calls();
    loop {
        forget_stopped_connection(app, &mut connection, emit);
        if let Some(request) = held_back.take() {
            if let Some(reply) = app.handle_from_ui(&request.json, request.attachments).await {
                emit(reply);
            }
            after_command(app, media_events, &mut connection, &mut tick, emit);
            continue;
        }
        let transfer_retry_at = app.transfer_retry_at;
        // Quiet work ending comes by as an event of its own, and this is looked at again after it.
        let wake_settled_held = app.wake_settled_held && app.detached.quiet_running == 0;
        tokio::select! {
            request = rx.recv() => {
                let Some(request) = request else { break };
                if let Some(reply) = app.handle_from_ui(&request.json, request.attachments).await {
                    emit(reply);
                }
                after_command(app, media_events, &mut connection, &mut tick, emit);
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
                        app.connection_state.set(state.as_str());
                        app.web_connection_changed(matches!(
                            state,
                            crate::connection::ConnectionState::Connected
                        ));
                        emit(json!({"event": "connectionState", "state": state.as_str()}).to_string());
                        // Online again: finish a restored backup's pending import right away
                        // (maintenance alone would retry only hourly).
                        if matches!(state, crate::connection::ConnectionState::Connected) {
                            if app.on_connected_here() {
                                app.detached_quietly(
                                    Lane::InTurn,
                                    |app| async move { app.on_connected_away().await },
                                    |app, events| app.note_ui_events(&events),
                                );
                            }
                            app.web_refresh();
                        }
                    }
                    Some(FromConnection::Event(event)) => {
                        app.gate_media(&event);
                        app.on_core_event(&event);
                        app.web_on_core_event(&event);
                        app.web_refresh();
                        emit(event.to_json(&app.store, app.unlocked.get()).to_string());
                        if matches!(*event, CoreEvent::MultiDeviceChanged) {
                            app.apply_multi_device();
                            publish_feature_mask(app);
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
                    Some(FromConnection::WakeSettled) => wake_settled(app, emit),
                }
            }
            finished = async {
                match file_work.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(finished) = finished {
                    finish_file_work(app, finished, media_events, emit).await;
                }
            }
            finished = async {
                match detached.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(finished) = finished {
                    finish_detached_work(app, finished, emit).await;
                    after_command(app, media_events, &mut connection, &mut tick, emit);
                }
            }
            event = async {
                match media_events.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(event) = event {
                    on_media_event(app, &event, emit);
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
            () = app.transfers.ended(), if wake_settled_held => wake_settled(app, emit),
            _ = tick.tick() => start_maintenance(app),
        }
    }
    // Asked to stop: what was asked for before that still counts, as commands waiting in the queue
    // do. A file sent just before closing the window must not be lost.
    finish_running_file_work(app, file_work, media_events, emit).await;
    app.drop_quiet_work();
    while app.detached.replies_due > 0 {
        let Some(finished) = (match detached.as_mut() {
            Some(rx) => rx.recv().await,
            None => None,
        }) else {
            break;
        };
        finish_detached_work(app, finished, emit).await;
        for event in app.take_ui_events() {
            emit(json!({ "event": event }).to_string());
        }
        settle_command(app, media_events, emit);
    }
}

async fn finish_running_file_work(
    app: &mut App,
    file_work: &mut Option<mpsc::UnboundedReceiver<crate::app_media::FinishedFileWork>>,
    media_events: &mut Option<mpsc::UnboundedReceiver<CoreEvent>>,
    emit: &Emit,
) {
    while app.file_work_running > 0 {
        let Some(finished) = (match file_work.as_mut() {
            Some(rx) => rx.recv().await,
            None => None,
        }) else {
            break;
        };
        finish_file_work(app, finished, media_events, emit).await;
    }
}

/// A command's work on a blocking thread is done: finish it like any command.
async fn finish_file_work(
    app: &mut App,
    finished: crate::app_media::FinishedFileWork,
    media_events: &mut Option<mpsc::UnboundedReceiver<CoreEvent>>,
    emit: &Emit,
) {
    emit(app.finish_file_work(finished).await);
    for event in app.take_ui_events() {
        emit(json!({ "event": event }).to_string());
    }
    settle_command(app, media_events, emit);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the loop on `requests` until `until` says it has heard enough, at most a few seconds
    /// (the requests' network waits don't end by themselves in these tests). What was said.
    async fn loop_said(
        app: &mut App,
        requests: Vec<serde_json::Value>,
        until: impl Fn(&[String]) -> bool,
    ) -> Vec<String> {
        let said = Arc::new(std::sync::Mutex::new(Vec::new()));
        let emit: Emit = {
            let said = Arc::clone(&said);
            Box::new(move |message| said.lock().unwrap().push(message))
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        for request in requests {
            tx.send(CoreRequest {
                json: Zeroizing::new(request.to_string()),
                attachments: Vec::new(),
            })
            .unwrap();
        }
        let mut media_events = app.take_media_events();
        let mut file_work = app.take_file_work();
        let mut detached = app.take_detached_work();
        let heard = async {
            while !until(&said.lock().unwrap()) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        tokio::select! {
            () = run_loop(app, &mut rx, &mut media_events, &mut file_work, &mut detached, &emit) => {}
            result = tokio::time::timeout(Duration::from_secs(5), heard) => {
                assert!(result.is_ok(), "not heard in time: {:?}", said.lock().unwrap());
            }
        }
        said.lock().unwrap().clone()
    }

    fn replied(said: &[String], request: u64) -> bool {
        said.iter().any(|message| {
            serde_json::from_str::<serde_json::Value>(message).is_ok_and(|m| m["reply"] == request)
        })
    }

    /// What a stalled mobile link used to do: every reply waited for the request's time-out.
    #[tokio::test(flavor = "current_thread")]
    async fn the_core_answers_while_a_request_of_another_command_stalls() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let mut app = App::new(&start_config(dir.path()), &[1; 32]).unwrap();
                crate::https::stall_requests();
                let asked = Instant::now();
                let said = loop_said(
                    &mut app,
                    vec![
                        json!({"id": 1, "cmd": "checkLicense", "licenseKey": "ABCDE-12345"}),
                        json!({"id": 2, "cmd": "status"}),
                        json!({"id": 3, "cmd": "connectionState"}),
                        json!({"id": 4, "cmd": "revocationStatus", "cached": true}),
                    ],
                    |said| replied(said, 2) && replied(said, 3) && replied(said, 4),
                )
                .await;
                assert!(asked.elapsed() < Duration::from_secs(2));
                assert!(!replied(&said, 1), "that one is still waiting: {said:?}");
            })
            .await;
    }

    /// An overdue round starts with the wake-up, and the device stays up until it has ended.
    #[tokio::test(flavor = "current_thread")]
    async fn a_wake_up_stays_for_a_round_of_maintenance_that_is_running() {
        const ROUND: Duration = Duration::from_millis(100);
        tokio::task::LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let mut app = App::new(&start_config(dir.path()), &[1; 32]).unwrap();
                let said = Arc::new(std::sync::Mutex::new(Vec::new()));
                let emit: Emit = {
                    let said = Arc::clone(&said);
                    Box::new(move |message| said.lock().unwrap().push(message))
                };
                app.detached_quietly(Lane::InTurn, |_| tokio::time::sleep(ROUND), |_, ()| {});
                wake_settled(&mut app, &emit);
                assert!(app.wake_settled_held);
                assert!(said.lock().unwrap().is_empty());

                let asked = Instant::now();
                let settled =
                    loop_said(&mut app, vec![json!({"id": 1, "cmd": "status"})], |said| {
                        said.iter().any(|message| message.contains("wakeSettled"))
                    })
                    .await;
                assert!(
                    asked.elapsed() >= ROUND - Duration::from_millis(5),
                    "{settled:?}"
                );
                assert!(!app.wake_settled_held);
            })
            .await;
    }

    fn request(json: serde_json::Value) -> CoreRequest {
        CoreRequest {
            json: Zeroizing::new(json.to_string()),
            attachments: Vec::new(),
        }
    }

    /// A download marked as under way when the process ended is what the start's clean-up resets.
    #[tokio::test(flavor = "current_thread")]
    async fn the_first_status_is_answered_before_the_clean_up_and_others_after_it() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let mut app = App::new(&start_config(dir.path()), &[1; 32]).unwrap();
                let said = Arc::new(std::sync::Mutex::new(Vec::new()));
                let emit: Emit = {
                    let said = Arc::clone(&said);
                    Box::new(move |message| said.lock().unwrap().push(message))
                };
                let (tx, mut rx) = mpsc::unbounded_channel();
                tx.send(request(json!({"id": 1, "cmd": "status"}))).unwrap();
                let held = answer_first_status(&mut app, &mut rx, &emit).await;
                assert!(held.is_none());
                assert!(replied(&said.lock().unwrap(), 1));

                tx.send(request(json!({"id": 2, "cmd": "connectionState"})))
                    .unwrap();
                let held = answer_first_status(&mut app, &mut rx, &emit).await;
                assert!(held.is_some(), "it comes back for after the clean-up");
                assert!(!replied(&said.lock().unwrap(), 2));

                // A UI that doesn't ask holds up nothing for long.
                let asked = Instant::now();
                assert!(
                    answer_first_status(&mut app, &mut rx, &emit)
                        .await
                        .is_none()
                );
                assert!(asked.elapsed() < FIRST_REQUEST_WAIT * 2);
            })
            .await;
    }

    /// The daily checks being due on a stalled link was what kept the start page spinning.
    #[tokio::test(flavor = "current_thread")]
    async fn a_round_of_maintenance_waits_beside_the_loop_and_only_one_at_a_time() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let mut app = App::new(&start_config(dir.path()), &[1; 32]).unwrap();
                app.store
                    .set_identity(&crate::store::StoredIdentity {
                        threema_id: "ABCD1234".into(),
                        client_key: Zeroizing::new([9; 32]),
                        server_group: "01".into(),
                    })
                    .unwrap();
                app.store
                    .set_setting(crate::store::keys::LICENSE_KEY, "ABCDE-12345")
                    .unwrap();
                crate::https::stall_requests();
                let mut finished = app.take_detached_work().unwrap();
                start_maintenance(&mut app);
                assert!(app.detached.maintenance_running);
                start_maintenance(&mut app);
                let nothing =
                    tokio::time::timeout(Duration::from_millis(50), finished.recv()).await;
                assert!(nothing.is_err(), "it is waiting for the license check");
                assert_eq!(app.connection_state.get(), "disconnected");
                app.drop_quiet_work();
                assert!(!app.detached.maintenance_running);
            })
            .await;
    }

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
        app.connection.replace(Some(to_connection));
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

    /// As a command waiting in the queue would be: a file sent just before closing the window
    /// isn't lost while it is on its blocking thread.
    #[test]
    fn stopping_the_core_finishes_the_file_on_its_way() {
        let dir = tempfile::tempdir().unwrap();
        let key = [1; 32];
        {
            let app = App::new(&start_config(dir.path()), &key).unwrap();
            app.store
                .upsert_contact(&crate::contacts::test_contact("ABCD1234"))
                .unwrap();
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let emit: Emit = Box::new(move |message| {
            let _ = tx.lock().unwrap().send(message);
        });
        let handle =
            CoreHandle::start(start_config(dir.path()), Zeroizing::new(key), emit).unwrap();
        let request = json!({"id": 3, "cmd": "sendFile", "identity": "ABCD1234",
                             "rendering": "file", "mediaType": "text/plain", "name": "a.txt"});
        handle.send_with(request.to_string(), vec![Zeroizing::new(vec![1, 2, 3])]);
        drop(handle);

        let reply = loop {
            let message = rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
            let message: serde_json::Value = serde_json::from_str(&message).unwrap();
            if message["reply"] == 3 {
                break message;
            }
        };
        assert_eq!(reply["result"]["kind"], "file", "{reply}");
    }

    fn started_core(
        dir: &std::path::Path,
    ) -> (CoreHandle, std::sync::mpsc::Receiver<serde_json::Value>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let emit: Emit = Box::new(move |message| {
            let message = serde_json::from_str(&message).unwrap();
            let _ = tx.lock().unwrap().send(message);
        });
        let core = CoreHandle::start(start_config(dir), Zeroizing::new([1; 32]), emit).unwrap();
        let first: serde_json::Value = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert_eq!(first["event"], "started", "{first}");
        (core, rx)
    }

    #[test]
    fn a_quit_still_answers_what_was_asked_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let (core, said) = started_core(dir.path());
        core.send(json!({"id": 7, "cmd": "status"}).to_string());
        assert!(core.quit());
        assert!(
            said.try_iter().any(|message| message["reply"] == 7),
            "the request in the queue was answered"
        );
    }

    /// A closing chat page hands these over last thing, and the core stops right after.
    #[test]
    fn what_a_closing_page_hands_over_is_there_after_a_quit() {
        let dir = tempfile::tempdir().unwrap();
        {
            let app = App::new(&start_config(dir.path()), &[1; 32]).unwrap();
            app.store
                .upsert_contact(&crate::contacts::test_contact("ABCD1234"))
                .unwrap();
        }
        let (core, said) = started_core(dir.path());
        core.send_with(
            json!({"id": 1, "cmd": "sendFile", "identity": "ABCD1234", "rendering": "file",
                   "mediaType": "text/plain", "name": "a.txt", "caption": ""})
            .to_string(),
            vec![vec![1_u8, 2, 3].into()],
        );
        core.send(
            json!({"id": 2, "cmd": "setDraft", "identity": "ABCD1234", "text": "typed"})
                .to_string(),
        );
        assert!(core.quit());
        let oks: Vec<_> = said
            .try_iter()
            .filter(|message| message["ok"] == true)
            .map(|message| message["reply"].clone())
            .collect();
        assert!(
            oks.contains(&json!(1)) && oks.contains(&json!(2)),
            "{oks:?}"
        );

        let app = App::new(&start_config(dir.path()), &[1; 32]).unwrap();
        assert_eq!(app.draft("ABCD1234").unwrap()["text"], "typed");
        let messages = app.store.conversation_messages("ABCD1234", 10).unwrap();
        assert_eq!(messages.len(), 1, "the file");
    }

    #[test]
    fn a_quit_gives_up_on_a_core_that_does_not_end() {
        let dir = tempfile::tempdir().unwrap();
        let (release, stuck) = std::sync::mpsc::channel::<()>();
        let stuck = std::sync::Mutex::new(stuck);
        // Held up in saying that it started, where no signal reaches it.
        let emit: Emit = Box::new(move |_| {
            let _ = stuck.lock().unwrap().recv();
        });
        let core =
            CoreHandle::start(start_config(dir.path()), Zeroizing::new([1; 32]), emit).unwrap();
        let began = Instant::now();
        assert!(!core.quit_within(Duration::from_millis(50), Duration::from_millis(50)));
        assert!(began.elapsed() < Duration::from_secs(10));
        drop(release);
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn a_quit_drops_a_wait_that_goes_on() {
        struct Gone(Arc<AtomicBool>);
        impl Drop for Gone {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let gone = Arc::new(AtomicBool::new(false));
        let waiting = Gone(Arc::clone(&gone));
        let quit = QuitSignal::default();
        quit.cut_off.notify_one();
        let cut_off = runtime().block_on(until_cut_off(
            async move {
                let _waiting = waiting;
                std::future::pending::<()>().await;
            },
            &quit,
        ));
        assert!(cut_off);
        assert!(gone.load(Ordering::SeqCst), "and with it what it holds");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn what_is_queued_behind_a_cut_off_command_still_runs() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let mut app = App::new(&start_config(dir.path()), &[1; 32]).unwrap();
                app.store
                    .upsert_contact(&crate::contacts::test_contact("ABCD1234"))
                    .unwrap();
                let said = Arc::new(std::sync::Mutex::new(Vec::new()));
                let emit: Emit = {
                    let said = Arc::clone(&said);
                    Box::new(move |message| said.lock().unwrap().push(message))
                };
                let (tx, mut rx) = mpsc::unbounded_channel();
                for (id, cmd) in [
                    (
                        1,
                        json!({"cmd": "setDraft", "identity": "ABCD1234", "text": "typed"}),
                    ),
                    (2, json!({"cmd": "draft", "identity": "ABCD1234"})),
                ] {
                    let mut request = cmd;
                    request["id"] = json!(id);
                    tx.send(CoreRequest {
                        json: Zeroizing::new(request.to_string()),
                        attachments: Vec::new(),
                    })
                    .unwrap();
                }
                drop(tx);
                run_queued_requests(&mut app, &mut rx, &emit).await;
                let said = said.lock().unwrap();
                assert_eq!(said.len(), 2);
                let draft: serde_json::Value = serde_json::from_str(&said[1]).unwrap();
                assert_eq!(draft["result"]["text"], "typed", "{draft}");
            })
            .await;
    }

    /// Being on a blocking thread when the command loop is cut off doesn't lose a file.
    #[test]
    fn a_cut_off_still_finishes_the_file_on_its_way() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(&start_config(dir.path()), &[1; 32]).unwrap();
        app.store
            .upsert_contact(&crate::contacts::test_contact("ABCD1234"))
            .unwrap();
        let (said_tx, said) = std::sync::mpsc::channel();
        let said_tx = std::sync::Mutex::new(said_tx);
        let emit: Emit = Box::new(move |message| {
            let _ = said_tx.lock().unwrap().send(message);
        });
        // Still open: the loop has no end of the queue to stop at, only the cut-off.
        let (_tx, rx) = mpsc::unbounded_channel();
        let quit = QuitSignal::default();
        quit.quitting.store(true, Ordering::SeqCst);
        quit.cut_off.notify_one();
        let request = json!({"id": 3, "cmd": "sendFile", "identity": "ABCD1234",
                             "rendering": "file", "mediaType": "text/plain", "name": "a.txt"});
        let work = async {
            let reply = app
                .handle_from_ui(&request.to_string(), vec![vec![1_u8, 2, 3].into()])
                .await;
            assert!(reply.is_none(), "it went to a blocking thread");
            run_until_quit(app, rx, emit, &quit).await;
        };
        drive(runtime(), work, &quit);
        let answered = said.try_iter().any(|message: String| {
            let message: serde_json::Value = serde_json::from_str(&message).unwrap();
            message["reply"] == 3 && message["result"]["kind"] == "file"
        });
        assert!(answered, "the file was stored and answered");
    }

    #[test]
    fn a_quit_does_not_wait_for_a_blocking_task_to_the_end() {
        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let quit = QuitSignal::default();
        quit.quitting.store(true, Ordering::SeqCst);
        quit.cut_off.notify_one();
        let began = Instant::now();
        let work = async {
            let task = tokio::task::spawn_blocking(move || blocked.recv());
            until_cut_off(
                async {
                    let _ = task.await;
                },
                &quit,
            )
            .await;
        };
        drive(runtime(), work, &quit);
        // The task only ends after the drop below, so the end got there without it.
        assert!(began.elapsed() < Duration::from_secs(10));
        drop(release);
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
