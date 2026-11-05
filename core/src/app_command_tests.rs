// SPDX-License-Identifier: AGPL-3.0-only
//! Group management, Threema Safe restore and sending media, driven the way the UI drives them:
//! requests through the command layer, replies as the core loop hands them out (ADR 0028), with a
//! stand-in for Threema's servers on [`crate::https::HttpsClient`]. No connection runs here, so
//! nothing leaves the machine; [`Ui::settle`] does what the loop does after a command.
//!
//! Nothing here waits for time to pass. A request held by a [`Gate`] can't be answered before it is
//! released, so what must come first comes first on any machine; the time limit in [`scenario`] only
//! turns a hang into a failure.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use data_encoding::{BASE64, HEXLOWER};
use libthreema::{
    common::keys::{ClientKey, PublicKey},
    https::{HttpsError, HttpsMethod, HttpsRequest, HttpsResponse, HttpsResult},
    model::message::GroupMessageBody,
};
use serde_json::{Value, json};
use tokio::sync::{Semaphore, mpsc::UnboundedReceiver};
use zeroize::Zeroizing;

use crate::{
    app::{App, StartConfig},
    app_media::{
        FinishedFileWork, STATE_FAILED, STATE_LOADING, STATE_LOCAL, outgoing_file_message,
    },
    connection::ToConnection,
    detached::{Finished, Lane},
    groups::{
        OUTBOX_GROUP_DELETE_PICTURE, OUTBOX_GROUP_LEAVE, OUTBOX_GROUP_NAME, OUTBOX_GROUP_SETUP,
    },
    https::TestServersInPlace,
    media::MediaPart,
    messages::OutgoingState,
    providers::CoreEvent,
    runtime::Emit,
    safe::{MasterKey, parse_backup},
    store::{StoredIdentity, keys},
};

const ME: &str = "AAAAAAAA";
const ME_KEY: [u8; 32] = [5; 32];
const PASSWORD: &str = "a test password of some length";

/// Runs a test on one thread, as the core does. Only a hang gets near this limit.
fn scenario(test: impl Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    runtime.block_on(local.run_until(async {
        tokio::time::timeout(Duration::from_secs(300), test)
            .await
            .expect("the scenario hangs");
    }));
}

/// A server of Threema's, by what is asked of it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Route {
    SafeConfig,
    SafeBackup,
    /// `identity/fetch_priv` before and after the challenge.
    FetchPrivate,
    FetchPrivateAnswer,
    FetchBulk,
    FeatureMask,
    BlobUpload,
    Unknown,
}

fn method_of(request: &HttpsRequest) -> &'static str {
    match request.method {
        HttpsMethod::Delete => "DELETE",
        HttpsMethod::Get => "GET",
        HttpsMethod::Post => "POST",
        HttpsMethod::Put => "PUT",
    }
}

fn route_of(request: &HttpsRequest) -> Route {
    let url = request.url.as_str();
    let answered = serde_json::from_slice::<Value>(&request.body)
        .is_ok_and(|body| body.get("response").is_some());
    if url.contains("://safe-") && url.ends_with("/config") {
        Route::SafeConfig
    } else if url.contains("://safe-") && url.contains("/backups/") {
        Route::SafeBackup
    } else if url.ends_with("/identity/fetch_priv") {
        if answered {
            Route::FetchPrivateAnswer
        } else {
            Route::FetchPrivate
        }
    } else if url.ends_with("/identity/fetch_bulk") {
        Route::FetchBulk
    } else if url.ends_with("/identity/set_featuremask") {
        Route::FeatureMask
    } else if url.contains("upload") && url.contains("persist=") {
        Route::BlobUpload
    } else {
        Route::Unknown
    }
}

/// How a route answers, until told otherwise.
#[derive(Clone, Copy)]
enum Answer {
    Unreachable,
    Status(u16),
}

/// Holds the next request of a route until released.
struct Gate {
    arrived: Semaphore,
    release: Semaphore,
}

impl Gate {
    async fn reached(&self) {
        self.arrived.acquire().await.unwrap().forget();
    }

    fn open(&self) {
        self.release.add_permits(1);
    }
}

/// A request as the stand-in got it: route, method, URL, body.
type Seen = (Route, &'static str, String, Vec<u8>);

/// The stand-in for Threema's servers.
#[derive(Default)]
struct Servers {
    seen: RefCell<Vec<Seen>>,
    backups: RefCell<HashMap<String, Vec<u8>>>,
    /// IDs the directory knows; it leaves out the rest, as it does revoked ones.
    directory: RefCell<HashSet<String>>,
    answers: RefCell<HashMap<Route, Answer>>,
    gates: RefCell<HashMap<Route, Rc<Gate>>>,
    /// Client key the directory's challenge is expected to be answered with.
    client_key: Cell<[u8; 32]>,
    uploads: Cell<u8>,
}

impl Servers {
    fn start() -> (Rc<Self>, TestServersInPlace) {
        let servers = Rc::new(Self::default());
        servers.client_key.set(ME_KEY);
        for id in [
            "BBBBBBBB", "CCCCCCCC", "DDDDDDDD", "EEEEEEEE", "FFFFFFFF", "GGGGGGGG",
        ] {
            servers.directory.borrow_mut().insert(id.to_owned());
        }
        let serving = Rc::clone(&servers);
        let guard = crate::https::answer_requests_with(move |request| {
            let serving = Rc::clone(&serving);
            let route = route_of(request);
            serving.seen.borrow_mut().push((
                route,
                method_of(request),
                request.url.clone(),
                request.body.clone(),
            ));
            let (url, method, body) = (
                request.url.clone(),
                method_of(request),
                request.body.clone(),
            );
            Box::pin(async move {
                let gate = serving.gates.borrow_mut().remove(&route);
                if let Some(gate) = gate {
                    gate.arrived.add_permits(1);
                    gate.release.acquire().await.unwrap().forget();
                }
                serving.answer(route, &url, method, &body)
            })
        });
        (servers, guard)
    }

    fn answer(&self, route: Route, url: &str, method: &str, body: &[u8]) -> HttpsResult {
        let answer = self.answers.borrow().get(&route).copied();
        match answer {
            Some(Answer::Unreachable) => {
                return Err(HttpsError::Unreachable("stand-in is down".into()));
            }
            Some(Answer::Status(status)) => return response(status, Vec::new()),
            None => {}
        }
        match route {
            Route::SafeConfig => {
                json_response(&json!({"maxBackupBytes": 524_288, "retentionDays": 180}))
            }
            Route::SafeBackup => self.backup(url, method, body),
            Route::FetchPrivate | Route::FeatureMask if !is_answered(body) => challenge(),
            Route::FetchPrivateAnswer | Route::FeatureMask => {
                let right = self.answered_rightly(body);
                json_response(&json!({
                    "success": right,
                    "error": if right { "" } else { "wrong response" },
                    "serverGroup": "a1",
                }))
            }
            Route::FetchPrivate | Route::Unknown => Err(HttpsError::Unreachable("nobody".into())),
            Route::FetchBulk => self.bulk(body),
            Route::BlobUpload => {
                self.uploads.set(self.uploads.get() + 1);
                let id = [self.uploads.get(); 16];
                response(200, HEXLOWER.encode(&id).into_bytes())
            }
        }
    }

    fn backup(&self, url: &str, method: &str, body: &[u8]) -> HttpsResult {
        match method {
            "PUT" => {
                self.backups
                    .borrow_mut()
                    .insert(url.to_owned(), body.to_vec());
                response(200, Vec::new())
            }
            "GET" => match self.backups.borrow().get(url) {
                Some(blob) => response(200, blob.clone()),
                None => response(404, Vec::new()),
            },
            _ => response(405, Vec::new()),
        }
    }

    fn bulk(&self, body: &[u8]) -> HttpsResult {
        let asked: Value = serde_json::from_slice(body).unwrap();
        let directory = self.directory.borrow();
        let identities: Vec<Value> = asked["identities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .filter(|id| directory.contains(*id))
            .map(|id| {
                json!({"identity": id, "type": 0, "publicKey": BASE64.encode(&[9; 32]),
                       "state": 0, "featureMask": 255})
            })
            .collect();
        json_response(&json!({ "identities": identities }))
    }

    /// Whoever answers the challenge holds the ID's key.
    fn answered_rightly(&self, body: &[u8]) -> bool {
        let body: Value = serde_json::from_slice(body).unwrap();
        let token = BASE64
            .decode(body["token"].as_str().unwrap().as_bytes())
            .unwrap();
        let expected = ClientKey::from(self.client_key.get())
            .solve_directory_challenge(&PublicKey::from(CHALLENGE_KEY), &token)
            .unwrap();
        body["response"].as_str() == Some(BASE64.encode(&expected).as_str())
    }

    fn answer_with(&self, route: Route, answer: Answer) {
        self.answers.borrow_mut().insert(route, answer);
    }

    fn back_up(&self, route: Route) {
        self.answers.borrow_mut().remove(&route);
    }

    fn hold(&self, route: Route) -> Rc<Gate> {
        let gate = Rc::new(Gate {
            arrived: Semaphore::new(0),
            release: Semaphore::new(0),
        });
        self.gates.borrow_mut().insert(route, Rc::clone(&gate));
        gate
    }

    fn asked(&self, route: Route) -> usize {
        self.seen
            .borrow()
            .iter()
            .filter(|(seen, ..)| *seen == route)
            .count()
    }

    fn asked_at_all(&self) -> usize {
        self.seen.borrow().len()
    }

    /// The stored backup, with `change` made to its JSON: a backup of another kind than ours makes.
    fn change_backup(&self, change: impl FnOnce(&mut Value)) {
        let key = MasterKey::derive(ME, PASSWORD).unwrap();
        let mut backups = self.backups.borrow_mut();
        let blob = backups.values_mut().next().unwrap();
        let mut json: Value = serde_json::from_slice(&key.decrypt(blob).unwrap()).unwrap();
        change(&mut json);
        *blob = key.encrypt(&serde_json::to_vec(&json).unwrap()).unwrap();
    }

    /// What is on the server now.
    fn stored_backup(&self) -> crate::safe::backup::Backup {
        let key = MasterKey::derive(ME, PASSWORD).unwrap();
        let backups = self.backups.borrow();
        let json = key.decrypt(backups.values().next().unwrap()).unwrap();
        parse_backup(&json).unwrap()
    }

    /// The last upload's plain form: decrypted with the file's key, and whether it stays.
    fn uploaded(&self, keys: &crate::media::FileKeys) -> (Vec<u8>, bool) {
        let seen = self.seen.borrow();
        let (_, _, url, body) = seen
            .iter()
            .rev()
            .find(|(route, ..)| *route == Route::BlobUpload)
            .unwrap();
        let blob = multipart_content(body);
        let plain = crate::media::decrypt_blob(&keys.key, MediaPart::File, blob).unwrap();
        (plain.to_vec(), url.contains("persist=1"))
    }

    fn uploads_seen(&self) -> usize {
        self.asked(Route::BlobUpload)
    }
}

const CHALLENGE_KEY: [u8; 32] = [11; 32];

fn is_answered(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body).is_ok_and(|body| body.get("response").is_some())
}

fn challenge() -> HttpsResult {
    json_response(&json!({
        "token": BASE64.encode(b"a token"),
        "tokenRespKeyPub": BASE64.encode(&CHALLENGE_KEY),
    }))
}

#[expect(clippy::unnecessary_wraps, reason = "what a request gives")]
fn response(status: u16, body: Vec<u8>) -> HttpsResult {
    Ok(HttpsResponse { status, body })
}

fn json_response(body: &Value) -> HttpsResult {
    response(200, serde_json::to_vec(body).unwrap())
}

/// The blob in an upload's form.
fn multipart_content(body: &[u8]) -> &[u8] {
    let start = body.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let end = body.windows(4).rposition(|w| w == b"\r\n--").unwrap();
    &body[start..end]
}

/// A core and what the UI gets from it, without the connection.
struct Ui {
    app: App,
    dir: tempfile::TempDir,
    detached: UnboundedReceiver<Finished>,
    file_work: UnboundedReceiver<FinishedFileWork>,
    media_events: Option<UnboundedReceiver<CoreEvent>>,
    connection: UnboundedReceiver<ToConnection>,
    said: Arc<Mutex<Vec<Value>>>,
    next_id: u64,
}

impl Ui {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Self::on(dir)
    }

    fn on(dir: tempfile::TempDir) -> Self {
        let mut app = App::new(
            &StartConfig {
                data_dir: dir.path().to_owned(),
                app_version: "0.1.0".into(),
                language: "en".into(),
                country: "CH".into(),
                model: "test".into(),
                os_version: "5.1".into(),
                pictures_dir: None,
                calls: false,
                log_level: None,
                update_check: false,
            },
            &[3; 32],
        )
        .unwrap();
        let (tx, connection) = tokio::sync::mpsc::unbounded_channel();
        app.connection.replace(Some(tx));
        Self {
            detached: app.take_detached_work().unwrap(),
            file_work: app.take_file_work().unwrap(),
            media_events: app.take_media_events(),
            connection,
            said: Arc::default(),
            next_id: 1000,
            app,
            dir,
        }
    }

    fn licensed() -> Self {
        let ui = Self::new();
        ui.app
            .store
            .set_setting(keys::LICENSE_KEY, "ABCDE-12345")
            .unwrap();
        ui
    }

    /// With our ID, and contacts who can be made members.
    fn with_identity() -> Self {
        let ui = Self::licensed();
        ui.app
            .store
            .set_identity(&StoredIdentity {
                threema_id: ME.into(),
                client_key: Zeroizing::new(ME_KEY),
                server_group: "a1".into(),
            })
            .unwrap();
        for id in ["BBBBBBBB", "CCCCCCCC", "DDDDDDDD", "EEEEEEEE"] {
            ui.app
                .store
                .upsert_contact(&crate::contacts::test_contact(id))
                .unwrap();
        }
        ui
    }

    /// The core going away and starting again, on what it stored.
    fn restarted(self) -> Self {
        Self::on(self.dir)
    }

    /// Asks like the UI; `None` if the reply comes later.
    async fn ask(&mut self, id: u64, request: Value, attachments: Vec<Vec<u8>>) -> Option<Value> {
        let mut request = request;
        request["id"] = json!(id);
        let attachments = attachments.into_iter().map(Zeroizing::new).collect();
        let reply = self
            .app
            .handle_from_ui(&request.to_string(), attachments)
            .await;
        reply.map(|reply| serde_json::from_str(&reply).unwrap())
    }

    /// The next reply that comes from work beside the loop, finished as the loop does.
    async fn later_reply(&mut self) -> Value {
        loop {
            tokio::select! {
                Some(finished) = self.detached.recv() => {
                    if let Some(reply) = self.app.finish_detached(finished) {
                        self.app.in_background.set(true);
                        let reply = self
                            .app
                            .reply(reply.request, reply.change, reply.outcome)
                            .await;
                        self.app.in_background.set(false);
                        return serde_json::from_str(&reply).unwrap();
                    }
                }
                Some(finished) = self.file_work.recv() => {
                    let reply = self.app.finish_file_work(finished).await;
                    return serde_json::from_str(&reply).unwrap();
                }
            }
        }
    }

    /// Asks, and waits for the reply however it comes.
    async fn command(&mut self, request: Value) -> Value {
        self.command_with(request, Vec::new()).await
    }

    async fn command_with(&mut self, request: Value, attachments: Vec<Vec<u8>>) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let reply = match self.ask(id, request, attachments).await {
            Some(reply) => reply,
            None => self.later_reply().await,
        };
        assert_eq!(reply["reply"], id, "{reply}");
        reply
    }

    async fn ok(&mut self, request: Value) -> Value {
        let reply = self.command(request).await;
        assert_eq!(reply["ok"], true, "{reply}");
        reply["result"].clone()
    }

    /// The running transfers have ended, and the loop has heard of how they went.
    async fn settle(&mut self) {
        self.app.transfers.ended().await;
        let said = Arc::clone(&self.said);
        let emit: Emit = Box::new(move |message| {
            said.lock()
                .unwrap()
                .push(serde_json::from_str(&message).unwrap());
        });
        crate::runtime::settle_command(&mut self.app, &mut self.media_events, &emit);
    }

    /// How often the connection was told to send what is pending, since the last look.
    fn sends_asked(&mut self) -> usize {
        let mut count = 0;
        while let Ok(message) = self.connection.try_recv() {
            if matches!(message, ToConnection::SendPending) {
                count += 1;
            }
        }
        count
    }

    /// The events the UI hears now.
    fn events(&mut self) -> Vec<&'static str> {
        self.app.take_ui_events()
    }

    fn outbox(&self, key: &str) -> Vec<(String, Value, Vec<String>)> {
        let store = &self.app.store;
        store
            .outbox()
            .unwrap()
            .into_iter()
            .filter(|entry| entry.conversation == key)
            .map(|entry| {
                let receivers = store.pending_receivers(key, entry.message_id).unwrap();
                (entry.kind, entry.body, receivers)
            })
            .collect()
    }

    fn statuses(&self, key: &str) -> Vec<String> {
        self.app
            .store
            .conversation_messages(key, 100)
            .unwrap()
            .into_iter()
            .filter(|message| message.kind == "status")
            .map(|message| message.body)
            .collect()
    }

    async fn group(&mut self, members: &[&str]) -> String {
        let group = self
            .ok(json!({"cmd": "createGroup", "name": "Crew", "members": members}))
            .await;
        let key = group["key"].as_str().unwrap().to_owned();
        self.clear_outbox(&key);
        key
    }

    fn clear_outbox(&mut self, key: &str) {
        self.app.store.outbox_clear(key).unwrap();
        let _ = self.sends_asked();
    }

    /// A group someone else made, with us in it.
    fn group_of_someone_else(&self, creator: &str, members: &[&str]) -> String {
        let key = format!("g:{creator}:0000000000000007");
        self.app
            .store
            .upsert_group(&crate::groups::Group {
                identity: crate::groups::parse_key(&key).unwrap(),
                key: key.clone(),
                name: "Theirs".into(),
                members: members.iter().map(|m| (*m).to_owned()).collect(),
                left: false,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        key
    }

    async fn lock(&mut self) {
        self.ok(json!({"cmd": "setLock", "enabled": true, "pin": "2468"}))
            .await;
        self.ok(json!({"cmd": "lock"})).await;
    }

    fn body_of(&self, message: i64) -> Value {
        let message = self.app.store.message(message).unwrap().unwrap();
        serde_json::from_str(&message.body).unwrap()
    }

    fn upload_state(&self, message: i64) -> Option<i64> {
        self.body_of(message)["uploadState"].as_i64()
    }

    /// What the connection would send for this file now.
    fn file_going_out(&self, message: i64) -> Option<libthreema::model::message::FileMessage> {
        let keys = self.app.store.file_keys(message).unwrap()?;
        outgoing_file_message(&self.body_of(message), &keys)
    }

    fn message_state(&self, message: i64) -> i64 {
        self.app.store.message(message).unwrap().unwrap().state
    }

    async fn send_a_file(&mut self, chat: &str, content: &[u8]) -> i64 {
        let reply = self
            .command_with(
                json!({"cmd": "sendFile", "identity": chat, "rendering": "file",
                       "mediaType": "text/plain", "name": "a.txt", "caption": ""}),
                vec![content.to_vec()],
            )
            .await;
        assert_eq!(reply["ok"], true, "{reply}");
        reply["result"]["id"].as_i64().unwrap()
    }
}

fn names(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

/// What the connection makes of a group's outbox entry.
fn setup_members(ui: &Ui, key: &str, index: usize) -> Vec<String> {
    let entry = ui
        .app
        .store
        .outbox()
        .unwrap()
        .into_iter()
        .filter(|entry| entry.conversation == key)
        .nth(index)
        .unwrap();
    let Some(GroupMessageBody::Setup(setup)) = crate::connection::outbox_group_body(&entry) else {
        panic!("not a group setup: {}", entry.kind);
    };
    setup.members.iter().map(ToString::to_string).collect()
}

// Group management

#[test]
fn a_new_group_tells_its_members_who_is_in_it_and_how_it_is_called() {
    scenario(async {
        let (_servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        // Ourselves and a duplicate don't count as members.
        let group = ui
            .ok(json!({"cmd": "createGroup", "name": " Crew ",
                       "members": ["CCCCCCCC", "BBBBBBBB", ME, "BBBBBBBB"]}))
            .await;
        let key = group["key"].as_str().unwrap().to_owned();
        assert!(key.starts_with(&format!("g:{ME}:")), "{key}");
        assert_eq!(group["title"], "Crew");

        let stored = ui.app.store.group(&key).unwrap().unwrap();
        assert_eq!(stored.members, names(&["CCCCCCCC", "BBBBBBBB"]));
        assert!(!stored.left);
        let everyone = names(&["BBBBBBBB", "CCCCCCCC"]);
        let outbox = ui.outbox(&key);
        assert_eq!(outbox.len(), 3, "{outbox:?}");
        assert_eq!(outbox[0].0, OUTBOX_GROUP_SETUP);
        assert_eq!(outbox[0].2, everyone);
        assert_eq!(
            setup_members(&ui, &key, 0),
            names(&["CCCCCCCC", "BBBBBBBB"])
        );
        assert_eq!(outbox[1].0, OUTBOX_GROUP_NAME);
        assert_eq!(outbox[1].1["name"], "Crew");
        assert_eq!(outbox[1].2, everyone);
        assert_eq!(
            outbox[2].0, OUTBOX_GROUP_DELETE_PICTURE,
            "no picture is news too"
        );
        assert_eq!(outbox[2].2, everyone);
        assert_eq!(ui.statuses(&key), ["groupCreated"]);
        assert_eq!(ui.sends_asked(), 1);
    });
}

#[test]
fn a_group_with_someone_unknown_is_not_made() {
    scenario(async {
        let (_servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        for (members, kind) in [
            (json!(["BBBBBBBB", "ZZZZZZZZ"]), "contactNotFound"),
            (json!(["BBBBBBBB", "short"]), "identityFormat"),
        ] {
            let reply = ui
                .command(json!({"cmd": "createGroup", "name": "Crew", "members": members}))
                .await;
            assert_eq!(reply["error"]["kind"], kind, "{reply}");
        }
        assert!(ui.app.store.groups().unwrap().is_empty());
        assert!(ui.app.store.outbox().unwrap().is_empty());
        assert_eq!(ui.sends_asked(), 0);
    });
}

/// The heart of it: everyone left over hears the new list, whoever is new also hears the name, and
/// whoever goes hears an empty list and nothing of who stays.
#[test]
fn changing_a_groups_members_tells_each_of_them_what_they_are_to_know() {
    scenario(async {
        let (_servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        let key = ui.group(&["BBBBBBBB", "CCCCCCCC"]).await;

        let reply = ui
            .ok(json!({"cmd": "setGroupMembers", "key": key,
                       "members": ["CCCCCCCC", "DDDDDDDD", "EEEEEEEE"]}))
            .await;
        assert_eq!(ui.sends_asked(), 1);
        let shown: Vec<&str> = reply["members"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|member| member["identity"].as_str())
            .filter(|member| *member != ME)
            .collect();
        assert_eq!(
            sorted(names(&shown)),
            names(&["CCCCCCCC", "DDDDDDDD", "EEEEEEEE"])
        );

        let new_list = names(&["CCCCCCCC", "DDDDDDDD", "EEEEEEEE"]);
        let stored = ui.app.store.group(&key).unwrap().unwrap();
        assert_eq!(stored.members, new_list);

        let outbox = ui.outbox(&key);
        assert_eq!(outbox.len(), 4, "{outbox:?}");

        assert_eq!(outbox[0].0, OUTBOX_GROUP_SETUP);
        assert_eq!(outbox[0].2, names(&["BBBBBBBB"]), "the one who goes");
        assert!(
            setup_members(&ui, &key, 0).is_empty(),
            "hears nobody's name"
        );

        assert_eq!(outbox[1].0, OUTBOX_GROUP_SETUP);
        assert_eq!(outbox[1].2, new_list, "not the one who went");
        assert_eq!(setup_members(&ui, &key, 1), new_list);

        assert_eq!(outbox[2].0, OUTBOX_GROUP_NAME);
        assert_eq!(outbox[2].1["name"], "Crew");
        assert_eq!(
            outbox[2].2,
            names(&["DDDDDDDD", "EEEEEEEE"]),
            "the new ones"
        );

        let statuses = ui.statuses(&key);
        assert_eq!(statuses.len(), 4, "{statuses:?}");
        assert_eq!(statuses[0], "groupCreated");
        assert!(statuses[1].starts_with("groupMemberAdded|"), "{statuses:?}");
        assert!(statuses[2].starts_with("groupMemberAdded|"), "{statuses:?}");
        assert!(
            statuses[3].starts_with("groupMemberRemoved|"),
            "{statuses:?}"
        );
    });
}

#[test]
fn asking_for_the_members_a_group_has_tells_nobody_anything() {
    scenario(async {
        let (_servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        let key = ui.group(&["BBBBBBBB", "CCCCCCCC"]).await;
        ui.ok(json!({"cmd": "setGroupMembers", "key": key,
                     "members": ["CCCCCCCC", "BBBBBBBB", ME]}))
            .await;
        assert!(ui.outbox(&key).is_empty());
        assert_eq!(ui.sends_asked(), 0);
        assert_eq!(ui.statuses(&key), ["groupCreated"]);
    });
}

/// Nothing is half done: the list stays, and nobody hears of a list that isn't one.
#[test]
fn a_member_list_that_is_refused_changes_nothing() {
    scenario(async {
        let (_servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        let key = ui.group(&["BBBBBBBB"]).await;
        let reply = ui
            .command(json!({"cmd": "setGroupMembers", "key": key,
                            "members": ["CCCCCCCC", "ZZZZZZZZ"]}))
            .await;
        assert_eq!(reply["error"]["kind"], "contactNotFound", "{reply}");
        assert_eq!(
            ui.app.store.group(&key).unwrap().unwrap().members,
            names(&["BBBBBBBB"])
        );
        assert!(ui.outbox(&key).is_empty());
        assert_eq!(ui.sends_asked(), 0);
    });
}

#[test]
fn only_the_creator_of_a_group_still_in_it_manages_it() {
    scenario(async {
        let (_servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        let theirs = ui.group_of_someone_else("BBBBBBBB", &["BBBBBBBB", "CCCCCCCC"]);
        let ours = ui.group(&["BBBBBBBB"]).await;
        ui.ok(json!({"cmd": "leaveGroup", "key": ours})).await;
        ui.clear_outbox(&ours);

        for (key, kind) in [
            (theirs.as_str(), "notGroupCreator"),
            (ours.as_str(), "groupLeft"),
            ("g:AAAAAAAA:00000000000000ff", "groupNotFound"),
        ] {
            let requests = [
                json!({"cmd": "setGroupMembers", "key": key, "members": ["DDDDDDDD"]}),
                json!({"cmd": "renameGroup", "key": key, "name": "Mine now"}),
            ];
            for request in requests {
                let reply = ui.command(request).await;
                assert_eq!(reply["error"]["kind"], kind, "{reply}");
            }
            assert!(ui.outbox(key).is_empty());
        }
        let stored = ui.app.store.group(&theirs).unwrap().unwrap();
        assert_eq!(stored.members, names(&["BBBBBBBB", "CCCCCCCC"]));
        assert_eq!(stored.name, "Theirs");
        assert_eq!(ui.sends_asked(), 0);
    });
}

#[test]
fn renaming_a_group_tells_everyone_once() {
    scenario(async {
        let (_servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        let key = ui.group(&["BBBBBBBB", "CCCCCCCC"]).await;

        ui.ok(json!({"cmd": "renameGroup", "key": key, "name": " Ship "}))
            .await;
        ui.ok(json!({"cmd": "renameGroup", "key": key, "name": "Ship"}))
            .await;
        assert_eq!(ui.app.store.group(&key).unwrap().unwrap().name, "Ship");
        let outbox = ui.outbox(&key);
        assert_eq!(outbox.len(), 1, "{outbox:?}");
        assert_eq!(outbox[0].0, OUTBOX_GROUP_NAME);
        assert_eq!(outbox[0].1["name"], "Ship");
        assert_eq!(outbox[0].2, names(&["BBBBBBBB", "CCCCCCCC"]));
        assert_eq!(ui.statuses(&key), ["groupCreated", "groupRenamed|Ship"]);
        assert_eq!(ui.sends_asked(), 1);

        let reply = ui
            .command(json!({"cmd": "renameGroup", "key": key, "name": "x".repeat(257)}))
            .await;
        assert_eq!(reply["error"]["kind"], "messageTooLong", "{reply}");
        assert_eq!(ui.app.store.group(&key).unwrap().unwrap().name, "Ship");
    });
}

#[test]
fn leaving_a_group_tells_its_members_once_and_ends_writing_to_it() {
    scenario(async {
        let (_servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        let key = ui.group_of_someone_else("BBBBBBBB", &["BBBBBBBB", "CCCCCCCC"]);

        let left = ui.ok(json!({"cmd": "leaveGroup", "key": key})).await;
        assert_eq!(left["key"], key.as_str());
        let stored = ui.app.store.group(&key).unwrap().unwrap();
        assert!(stored.left);
        assert_eq!(stored.members, names(&["BBBBBBBB", "CCCCCCCC"]));
        let outbox = ui.outbox(&key);
        assert_eq!(outbox.len(), 1, "{outbox:?}");
        assert_eq!(outbox[0].0, OUTBOX_GROUP_LEAVE);
        assert_eq!(outbox[0].2, names(&["BBBBBBBB", "CCCCCCCC"]));
        assert_eq!(ui.statuses(&key), ["groupLeft"]);
        assert_eq!(ui.sends_asked(), 1);
        let farewell = ui.app.store.outbox().unwrap().remove(0);
        assert!(matches!(
            crate::connection::outbox_group_body(&farewell),
            Some(GroupMessageBody::Leave(_))
        ));

        // Tapped twice: nobody hears it twice.
        ui.ok(json!({"cmd": "leaveGroup", "key": key})).await;
        assert_eq!(ui.outbox(&key).len(), 1);
        assert_eq!(ui.statuses(&key), ["groupLeft"]);
        assert_eq!(ui.sends_asked(), 0);

        let reply = ui
            .command(json!({"cmd": "sendText", "identity": key, "text": "still here"}))
            .await;
        assert_eq!(reply["error"]["kind"], "groupLeft", "{reply}");
    });
}

#[test]
fn the_creator_leaving_disbands_the_group_with_an_empty_list_for_everyone() {
    scenario(async {
        let (_servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        let key = ui.group(&["BBBBBBBB", "CCCCCCCC"]).await;

        ui.ok(json!({"cmd": "leaveGroup", "key": key})).await;
        assert!(ui.app.store.group(&key).unwrap().unwrap().left);
        let outbox = ui.outbox(&key);
        assert_eq!(outbox.len(), 1, "{outbox:?}");
        assert_eq!(outbox[0].0, OUTBOX_GROUP_SETUP);
        assert_eq!(outbox[0].2, names(&["BBBBBBBB", "CCCCCCCC"]));
        assert!(setup_members(&ui, &key, 0).is_empty());
        assert_eq!(ui.statuses(&key), ["groupCreated", "groupDisbanded"]);
        assert_eq!(ui.sends_asked(), 1);
    });
}

// Threema Safe restore

/// A device's life up to a backup: `(its Ui, the servers holding the backup)`. Contacts BBBBBBBB,
/// CCCCCCCC, a private DDDDDDDD, one that has left the directory (GGGGGGGG), our group "Crew",
/// someone else's, and a list.
async fn backed_up(servers: &Servers) -> String {
    let mut ui = Ui::with_identity();
    ui.app
        .store
        .upsert_contact(&crate::contacts::test_contact("GGGGGGGG"))
        .unwrap();
    ui.app.store.set_setting(keys::NICKNAME, "julia").unwrap();
    ui.app.store.set_private("DDDDDDDD", true).unwrap();
    let crew = ui.group(&["BBBBBBBB", "EEEEEEEE"]).await;
    ui.group_of_someone_else("CCCCCCCC", &["CCCCCCCC", ME]);
    ui.ok(json!({"cmd": "createDistributionList", "name": "Friends",
                 "members": ["BBBBBBBB", "CCCCCCCC"]}))
        .await;
    ui.ok(json!({"cmd": "safeEnable", "password": PASSWORD}))
        .await;
    assert_eq!(servers.backups.borrow().len(), 1);
    servers.directory.borrow_mut().remove("GGGGGGGG");
    crew
}

fn restore_request() -> Value {
    json!({"cmd": "safeRestore", "identity": ME, "password": PASSWORD})
}

async fn safe_status(ui: &mut Ui) -> Value {
    ui.ok(json!({"cmd": "safeStatus"})).await
}

fn contact_ids(ui: &Ui) -> Vec<String> {
    sorted(
        ui.app
            .store
            .contacts()
            .unwrap()
            .iter()
            .map(|contact| contact.identity.to_string())
            .collect(),
    )
}

/// Everything that a restore brings back has come; the pending part is empty.
fn assert_all_restored(ui: &Ui, crew: &str) {
    assert_eq!(
        contact_ids(ui),
        names(&["BBBBBBBB", "CCCCCCCC", "DDDDDDDD", "EEEEEEEE"]),
        "the revoked one isn't among them"
    );
    let store = &ui.app.store;
    assert!(
        store.is_private("DDDDDDDD").unwrap(),
        "private stays private"
    );
    assert!(!store.is_private("BBBBBBBB").unwrap());
    let group = store.group(crew).unwrap().unwrap();
    assert_eq!(group.name, "Crew");
    assert_eq!(group.members, names(&["BBBBBBBB", "EEEEEEEE"]));
    assert!(!group.left);
    let theirs = store.group("g:CCCCCCCC:0000000000000007").unwrap().unwrap();
    assert_eq!(theirs.members, names(&["CCCCCCCC"]));
    assert!(!theirs.left);
    let lists = store.distribution_lists().unwrap();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].name, "Friends");
    assert_eq!(lists[0].members, names(&["BBBBBBBB", "CCCCCCCC"]));
    let pending: Value =
        serde_json::from_str(&store.setting(keys::SAFE_PENDING_IMPORT).unwrap().unwrap()).unwrap();
    assert_eq!(
        pending,
        json!({}),
        "nothing left to import or to carry into backups"
    );
}

#[test]
fn a_restore_brings_back_what_the_backup_holds() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let crew = backed_up(&servers).await;

        let mut ui = Ui::licensed();
        let reply = ui.ok(restore_request()).await;
        assert_eq!(reply["identity"], ME);
        assert_eq!(reply["contacts"], 5);
        assert_eq!(reply["groups"], 2);

        let identity = ui.app.store.identity().unwrap().unwrap();
        assert_eq!(identity.threema_id, ME);
        assert_eq!(*identity.client_key, ME_KEY);
        assert_eq!(identity.server_group, "a1");
        assert_eq!(
            ui.app.store.setting(keys::NICKNAME).unwrap().as_deref(),
            Some("julia")
        );
        assert_all_restored(&ui, &crew);

        let events = ui.events();
        assert!(events.contains(&"contactsChanged"), "{events:?}");
        assert!(events.contains(&"groupsChanged"), "{events:?}");

        let status = safe_status(&mut ui).await;
        assert_eq!(status["enabled"], true);
        assert_eq!(status["restore"]["pendingContacts"], 0);
        assert_eq!(status["restore"]["pendingGroups"], 0);
        assert_eq!(status["restore"]["pendingLists"], 0);
        assert_eq!(status["restore"]["importError"], Value::Null);
        assert_eq!(status["restore"]["summary"]["contacts"], 4);
        assert_eq!(status["restore"]["summary"]["revokedContacts"], 1);
        assert_eq!(status["restore"]["summary"]["groups"], 2);
        assert_eq!(status["restore"]["summary"]["lists"], 1);
        assert_eq!(
            status["maxBytes"], 524_288,
            "the server's limits are known from here on"
        );
        assert_eq!(servers.asked(Route::Unknown), 0);
    });
}

/// Before the ID is stored nothing has happened: however it fails, it can be tried again.
#[test]
fn a_restore_cut_off_before_the_id_is_stored_leaves_nothing_and_can_be_tried_again() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let crew = backed_up(&servers).await;
        let mut ui = Ui::licensed();

        for (route, answer, kind) in [
            (Route::SafeBackup, Answer::Unreachable, "network"),
            (Route::SafeBackup, Answer::Status(503), "serverTrouble"),
            (Route::FetchPrivate, Answer::Unreachable, "network"),
            (Route::FetchPrivateAnswer, Answer::Unreachable, "network"),
        ] {
            servers.answer_with(route, answer);
            let reply = ui.command(restore_request()).await;
            assert_eq!(reply["error"]["kind"], kind, "{route:?}: {reply}");
            servers.back_up(route);
            let store = &ui.app.store;
            assert!(store.identity().unwrap().is_none(), "{route:?}");
            assert!(store.contacts().unwrap().is_empty());
            for setting in [
                keys::SAFE_MASTER_KEY,
                keys::SAFE_PENDING_IMPORT,
                keys::SAFE_RESTORE_SETUP,
                keys::NICKNAME,
            ] {
                assert_eq!(store.setting(setting).unwrap(), None, "{route:?}");
            }
        }

        ui.ok(restore_request()).await;
        assert_all_restored(&ui, &crew);
    });
}

#[test]
fn a_restore_that_cannot_work_says_why_and_leaves_no_id() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        backed_up(&servers).await;

        // A wrong password points at a backup that isn't there.
        let mut ui = Ui::licensed();
        let reply = ui
            .command(json!({"cmd": "safeRestore", "identity": ME, "password": "another password"}))
            .await;
        assert_eq!(reply["error"]["kind"], "safeNotFound", "{reply}");
        assert!(ui.app.store.identity().unwrap().is_none());

        // The directory doesn't take the key the backup holds.
        servers.client_key.set([6; 32]);
        let reply = ui.command(restore_request()).await;
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(ui.app.store.identity().unwrap().is_none());
        servers.client_key.set(ME_KEY);

        // Damaged, however it got that way.
        let good = servers.backups.borrow().values().next().unwrap().clone();
        for (url, blob) in servers.backups.borrow_mut().iter_mut() {
            assert!(url.contains("/backups/"));
            let last = blob.len() - 1;
            blob[last] ^= 1;
        }
        let reply = ui.command(restore_request()).await;
        assert_eq!(reply["error"]["kind"], "safeDamaged", "{reply}");
        assert!(ui.app.store.identity().unwrap().is_none());
        servers
            .backups
            .borrow_mut()
            .values_mut()
            .for_each(|blob| blob.clone_from(&good));

        // A key that isn't one.
        servers.change_backup(|backup| {
            backup["user"]["privatekey"] = json!(BASE64.encode(&[1; 16]));
        });
        let reply = ui.command(restore_request()).await;
        assert_eq!(reply["error"]["kind"], "safeDamaged", "{reply}");
        assert!(ui.app.store.identity().unwrap().is_none());
    });
}

/// These are refused before anything goes to any server.
#[test]
fn a_restore_needs_a_license_and_no_id() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let mut ui = Ui::new();
        let reply = ui.command(restore_request()).await;
        assert_eq!(reply["error"]["kind"], "noLicense", "{reply}");

        let mut ui = Ui::with_identity();
        let reply = ui.command(restore_request()).await;
        assert_eq!(reply["error"]["kind"], "identityExists", "{reply}");
        let reply = ui
            .command(json!({"cmd": "safeRestore", "identity": "nope", "password": PASSWORD}))
            .await;
        assert_eq!(reply["error"]["kind"], "identityExists", "{reply}");
        assert_eq!(servers.asked_at_all(), 0);
    });
}

/// The day it is needed the network drops: the ID is there, nothing of the backup is lost, the next
/// backup still holds all of it, a restart doesn't change that, and the import finishes later.
#[test]
fn a_restore_whose_import_fails_loses_nothing_and_finishes_later() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let crew = backed_up(&servers).await;
        let backed_up_contacts = servers.stored_backup().contacts.unwrap().len();
        let mut ui = Ui::licensed();

        servers.answer_with(Route::FetchBulk, Answer::Unreachable);
        let reply = ui.ok(restore_request()).await;
        assert_eq!(reply["identity"], ME);
        assert!(ui.app.store.identity().unwrap().is_some());
        assert!(ui.app.store.contacts().unwrap().is_empty());
        assert!(ui.events().contains(&"safeChanged"));

        let status = safe_status(&mut ui).await;
        assert_eq!(status["enabled"], true);
        assert_eq!(status["restore"]["pendingContacts"], backed_up_contacts);
        assert_eq!(status["restore"]["pendingGroups"], 2);
        assert_eq!(status["restore"]["pendingLists"], 1);
        assert_eq!(status["restore"]["importError"], "network");

        // Turned on and failing: a backup now must not lose what waits to be imported.
        let (carried, carried_groups) = {
            ui.ok(json!({"cmd": "safeBackupNow"})).await;
            let backup = servers.stored_backup();
            (backup.contacts.unwrap().len(), backup.groups.unwrap().len())
        };
        assert_eq!(carried, backed_up_contacts);
        assert_eq!(carried_groups, 2);
        assert_eq!(ui.events(), ["safeChanged"], "the upload");

        // Trying again while it still fails says so, once.
        let status = ui.ok(json!({"cmd": "safeRetryImport"})).await;
        assert_eq!(status["restore"]["importError"], "network");
        assert_eq!(status["restore"]["pendingContacts"], backed_up_contacts);
        let events = ui.events();
        assert!(events.is_empty(), "nothing new to hear: {events:?}");

        let mut ui = ui.restarted();
        let status = safe_status(&mut ui).await;
        assert_eq!(status["restore"]["pendingContacts"], backed_up_contacts);
        assert_eq!(status["restore"]["importError"], "network");

        servers.back_up(Route::FetchBulk);
        let status = ui.ok(json!({"cmd": "safeRetryImport"})).await;
        assert_eq!(status["restore"]["pendingContacts"], 0);
        assert_eq!(status["restore"]["pendingGroups"], 0);
        assert_eq!(status["restore"]["importError"], Value::Null);
        assert_eq!(status["restore"]["summary"]["contacts"], 4);
        assert_all_restored(&ui, &crew);
        let events = ui.events();
        assert!(events.contains(&"safeChanged"), "{events:?}");
        assert!(events.contains(&"contactsChanged"), "{events:?}");

        // Once.
        let lookups = servers.asked(Route::FetchBulk);
        ui.ok(json!({"cmd": "safeRetryImport"})).await;
        assert_eq!(servers.asked(Route::FetchBulk), lookups);
        assert_eq!(ui.statuses(&crew).len(), 0, "no chat noise from the import");
    });
}

/// Contacts in, the group's stranger not yet: the contacts stay, the groups wait, and a connection
/// coming up finishes it; the setup steps only begin after that.
#[test]
fn a_connection_coming_up_finishes_an_import_cut_off_between_contacts_and_groups() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let crew = backed_up(&servers).await;
        // In the group, not among the contacts: looked up with the group.
        servers.change_backup(|backup| {
            let contacts = backup["contacts"].as_array_mut().unwrap();
            contacts.retain(|contact| contact["identity"] != "EEEEEEEE");
        });

        let mut ui = Ui::licensed();
        // The contacts' lookup answers; the one after it, for the groups, is held.
        let gate = servers.hold(Route::FetchBulk);
        assert!(ui.ask(1, restore_request(), Vec::new()).await.is_none());
        gate.reached().await;
        gate.open();
        let second = servers.hold(Route::FetchBulk);
        second.reached().await;
        servers.answer_with(Route::FetchBulk, Answer::Unreachable);
        second.open();
        let reply = ui.later_reply().await;
        assert_eq!(reply["reply"], 1);
        assert_eq!(reply["ok"], true, "{reply}");

        assert_eq!(
            contact_ids(&ui),
            names(&["BBBBBBBB", "CCCCCCCC", "DDDDDDDD"])
        );
        let status = safe_status(&mut ui).await;
        assert_eq!(status["restore"]["pendingContacts"], 0);
        assert_eq!(status["restore"]["pendingGroups"], 2);
        assert_eq!(status["restore"]["pendingLists"], 1);
        assert_eq!(status["restore"]["importError"], "network");

        ui.app.connection_state.set("connected");
        ui.app.on_connected().await;
        assert_eq!(servers.asked(Route::FeatureMask), 0, "the setup steps wait");
        assert!(ui.app.store.group(&crew).unwrap().is_none());

        servers.back_up(Route::FetchBulk);
        // A minute later: sooner, a connection coming and going asks nothing.
        ui.app.last_import_attempt.set(None);
        let events = ui.app.on_connected().await;
        assert!(events.contains(&"groupsChanged"), "{events:?}");
        assert_all_restored(&ui, &crew);
        assert_eq!(ui.app.store.setting(keys::SAFE_IMPORT_ERROR).unwrap(), None);
        assert_eq!(servers.asked(Route::FeatureMask), 2, "the steps begin");
        assert_eq!(
            ui.app
                .store
                .contact("EEEEEEEE")
                .unwrap()
                .unwrap()
                .acquaintance_level,
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::GroupOrDeleted,
            "known through the group alone"
        );
    });
}

/// The directory says "too many": automatic tries rest, and the user's own may still ask.
#[test]
fn a_directory_that_says_too_many_is_left_alone_until_the_user_asks() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let crew = backed_up(&servers).await;
        let mut ui = Ui::licensed();

        servers.answer_with(Route::FetchBulk, Answer::Status(429));
        ui.ok(restore_request()).await;
        let status = safe_status(&mut ui).await;
        assert_eq!(status["restore"]["importError"], "rateLimited");
        assert!(ui.app.store.directory_resting());

        servers.back_up(Route::FetchBulk);
        let lookups = servers.asked(Route::FetchBulk);
        ui.app.connection_state.set("connected");
        ui.app.on_connected().await;
        assert_eq!(servers.asked(Route::FetchBulk), lookups, "automatic: rests");

        ui.ok(json!({"cmd": "safeRetryImport"})).await;
        assert_all_restored(&ui, &crew);
    });
}

/// ADR 0028: a restore waits beside the loop, others are answered meanwhile, its reply comes with
/// its own id, and a second one asked meanwhile takes its turn and finds the ID there.
#[test]
fn a_restore_waits_beside_the_loop_and_a_second_one_after_it() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let crew = backed_up(&servers).await;
        let mut ui = Ui::licensed();
        let downloads = servers.asked(Route::SafeBackup);

        let gate = servers.hold(Route::SafeBackup);
        assert!(ui.ask(1, restore_request(), Vec::new()).await.is_none());
        assert!(ui.ask(2, restore_request(), Vec::new()).await.is_none());
        gate.reached().await;

        let status = ui.ask(3, json!({"cmd": "status"}), Vec::new()).await;
        assert_eq!(status.unwrap()["reply"], 3, "answered while it waits");
        assert!(ui.app.store.identity().unwrap().is_none());

        gate.open();
        let first = ui.later_reply().await;
        assert_eq!(first["reply"], 1);
        assert_eq!(first["ok"], true, "{first}");
        let second = ui.later_reply().await;
        assert_eq!(second["reply"], 2);
        assert_eq!(second["error"]["kind"], "identityExists", "{second}");
        assert_eq!(servers.asked(Route::SafeBackup), downloads + 1);
        assert_all_restored(&ui, &crew);
        assert_eq!(ui.app.detached.replies_due, 0);
    });
}

/// Retrying the import depends on nothing else but its turn: what's asked for after it goes after it.
#[test]
fn what_is_in_turn_behind_an_import_sees_how_it_ended() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let crew = backed_up(&servers).await;
        let mut ui = Ui::licensed();
        servers.answer_with(Route::FetchBulk, Answer::Unreachable);
        ui.ok(restore_request()).await;
        servers.back_up(Route::FetchBulk);

        let gate = servers.hold(Route::FetchBulk);
        let retry = json!({"cmd": "safeRetryImport"});
        assert!(ui.ask(1, retry.clone(), Vec::new()).await.is_none());
        assert!(ui.ask(2, retry, Vec::new()).await.is_none());
        gate.reached().await;
        let lookups = servers.asked(Route::FetchBulk);

        gate.open();
        let first = ui.later_reply().await;
        assert_eq!(first["reply"], 1);
        assert_eq!(first["result"]["restore"]["pendingContacts"], 0);
        let second = ui.later_reply().await;
        assert_eq!(second["reply"], 2);
        assert_eq!(second["result"]["restore"]["pendingContacts"], 0);
        assert_eq!(second["result"]["restore"]["importError"], Value::Null);
        assert_eq!(
            servers.asked(Route::FetchBulk),
            lookups,
            "nothing left to ask"
        );
        assert_all_restored(&ui, &crew);
    });
}

/// ADR 0028: nothing may be written for an ID that went.
#[test]
fn an_import_under_way_writes_nothing_once_the_id_is_gone() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        backed_up(&servers).await;
        let mut ui = Ui::licensed();
        servers.answer_with(Route::FetchBulk, Answer::Unreachable);
        ui.ok(restore_request()).await;
        servers.back_up(Route::FetchBulk);

        let gate = servers.hold(Route::FetchBulk);
        assert!(
            ui.ask(1, json!({"cmd": "safeRetryImport"}), Vec::new())
                .await
                .is_none()
        );
        gate.reached().await;
        // No connection to wait for.
        ui.app.connection.replace(None);
        ui.ok(json!({"cmd": "deleteAllData"})).await;
        assert!(ui.app.store.identity().unwrap().is_none());
        assert_eq!(ui.app.detached.replies_due, 0);

        gate.open();
        // Work in turn starts once the import has ended, whichever way it ended.
        ui.app
            .detached_quietly(Lane::InTurn, |_| async {}, |_, ()| {});
        let finished = ui.detached.recv().await.unwrap();
        assert!(ui.app.finish_detached(finished).is_none());
        assert!(ui.detached.try_recv().is_err(), "no reply is due");
        assert!(ui.app.store.contacts().unwrap().is_empty());
        assert_eq!(
            ui.app.store.setting(keys::SAFE_PENDING_IMPORT).unwrap(),
            None
        );
        assert_eq!(ui.app.store.setting(keys::SAFE_IMPORT_ERROR).unwrap(), None);
    });
}

// Sending media

#[test]
fn a_file_goes_up_and_then_out_with_the_blob_that_came_back() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        let id = ui.send_a_file("BBBBBBBB", b"the file").await;
        ui.settle().await;

        assert_eq!(ui.upload_state(id), Some(STATE_LOCAL));
        assert_eq!(servers.uploads_seen(), 1);
        let keys = ui.app.store.file_keys(id).unwrap().unwrap();
        let (plain, stays) = servers.uploaded(&keys);
        assert_eq!(plain, b"the file");
        assert!(!stays, "a contact's copy goes when it was fetched");

        let file = ui.file_going_out(id).expect("ready to go out");
        assert_eq!(file.blob_id.0, [1; 16]);
        assert_eq!(*keys.key, file.key);
        assert_eq!(file.size, 8);
        assert_eq!(file.name.as_deref(), Some("a.txt"));

        assert_eq!(
            ui.message_state(id),
            OutgoingState::Pending as i64,
            "up isn't out: that takes the chat server"
        );
        assert!(ui.sends_asked() > 0, "and the connection is told");
        let pending = ui.app.store.pending_outgoing().unwrap();
        assert!(pending.iter().any(|message| message.id == id));
    });
}

#[test]
fn a_file_for_a_group_stays_on_the_server_for_all_its_members() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        let key = ui.group(&["BBBBBBBB", "CCCCCCCC"]).await;
        let id = ui.send_a_file(&key, b"for all").await;
        ui.settle().await;
        let keys = ui.app.store.file_keys(id).unwrap().unwrap();
        let (plain, stays) = servers.uploaded(&keys);
        assert_eq!(plain, b"for all");
        assert!(stays);
        assert!(ui.file_going_out(id).is_some());
    });
}

/// What failed is not sent, and doesn't read as sent; a tap sends it up, and then it can go.
#[test]
fn a_failed_upload_goes_nowhere_until_a_try_works() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        for (answer, due_again) in [(Answer::Unreachable, true), (Answer::Status(503), true)] {
            servers.answer_with(Route::BlobUpload, answer);
            let id = ui.send_a_file("BBBBBBBB", b"the file").await;
            ui.settle().await;

            assert_eq!(ui.upload_state(id), Some(STATE_FAILED));
            assert!(ui.file_going_out(id).is_none());
            assert_eq!(ui.app.store.file_keys(id).unwrap().unwrap().blob_id, None);
            assert_eq!(ui.message_state(id), OutgoingState::Pending as i64);
            assert_eq!(
                ui.body_of(id)["uploadRetry"]["at"].is_i64(),
                due_again,
                "{}",
                ui.body_of(id)
            );
            let _ = ui.sends_asked();

            // Failing once more is no different from the first time.
            let tries = servers.uploads_seen();
            ui.ok(json!({"cmd": "retryUpload", "message": id})).await;
            ui.settle().await;
            assert_eq!(servers.uploads_seen(), tries + 1);
            assert_eq!(ui.upload_state(id), Some(STATE_FAILED));
            assert!(ui.file_going_out(id).is_none());

            servers.back_up(Route::BlobUpload);
            ui.ok(json!({"cmd": "retryUpload", "message": id})).await;
            assert_eq!(ui.upload_state(id), Some(STATE_LOADING));
            assert!(ui.file_going_out(id).is_none(), "not while it goes up");
            ui.settle().await;

            assert_eq!(ui.upload_state(id), Some(STATE_LOCAL));
            assert!(ui.file_going_out(id).is_some());
            assert_eq!(
                ui.body_of(id)["uploadRetry"],
                Value::Null,
                "no try left over"
            );
            assert_eq!(ui.message_state(id), OutgoingState::Pending as i64);
            assert!(ui.sends_asked() > 0);
        }
    });
}

/// The blob server turned it down: trying again by itself won't change that, a tap may.
#[test]
fn a_refused_upload_waits_for_a_tap() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        servers.answer_with(Route::BlobUpload, Answer::Status(413));
        ui.app.connection_state.set("connected");
        let id = ui.send_a_file("BBBBBBBB", b"the file").await;
        ui.settle().await;
        assert_eq!(ui.upload_state(id), Some(STATE_FAILED));
        assert_eq!(
            ui.body_of(id)["uploadRetry"]["at"],
            Value::Null,
            "no try planned"
        );

        let tries = servers.uploads_seen();
        ui.app.retry_failed_transfers();
        ui.settle().await;
        assert_eq!(servers.uploads_seen(), tries, "nothing by itself");

        servers.back_up(Route::BlobUpload);
        ui.ok(json!({"cmd": "retryUpload", "message": id})).await;
        ui.settle().await;
        assert_eq!(ui.upload_state(id), Some(STATE_LOCAL));
    });
}

/// A file that is up, or on its way, isn't sent up again by a second tap.
#[test]
fn a_retry_of_a_file_not_failed_does_nothing() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        let gate = servers.hold(Route::BlobUpload);
        let id = ui.send_a_file("BBBBBBBB", b"the file").await;
        gate.reached().await;
        ui.ok(json!({"cmd": "retryUpload", "message": id})).await;
        assert_eq!(servers.uploads_seen(), 1, "under way");
        gate.open();
        ui.settle().await;
        assert_eq!(ui.upload_state(id), Some(STATE_LOCAL));

        let blob = ui.app.store.file_keys(id).unwrap().unwrap().blob_id;
        ui.ok(json!({"cmd": "retryUpload", "message": id})).await;
        ui.settle().await;
        assert_eq!(servers.uploads_seen(), 1, "it's up");
        assert_eq!(ui.app.store.file_keys(id).unwrap().unwrap().blob_id, blob);
    });
}

/// A file failed for good goes up anew with its message: the old blob may be gone, and the state
/// says so until the new one is up.
#[test]
fn a_failed_file_sent_again_goes_up_anew_before_it_goes_out() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        let id = ui.send_a_file("BBBBBBBB", b"the file").await;
        ui.settle().await;
        let old = ui
            .app
            .store
            .file_keys(id)
            .unwrap()
            .unwrap()
            .blob_id
            .unwrap();
        ui.app.store.set_outgoing_failed(id).unwrap();

        let gate = servers.hold(Route::BlobUpload);
        ui.ok(json!({"cmd": "resendMessage", "message": id})).await;
        assert_eq!(ui.upload_state(id), Some(STATE_LOADING));
        tokio::select! {
            () = gate.reached() => {}
            () = ui.app.transfers.ended() => panic!("nothing went up"),
        }
        assert_eq!(ui.message_state(id), OutgoingState::Pending as i64);
        assert!(ui.file_going_out(id).is_none());
        gate.open();
        ui.settle().await;
        let file = ui.file_going_out(id).unwrap();
        assert_ne!(file.blob_id.0, old.0);
    });
}

/// The row id of a file deleted during its upload may be the next file's: nothing of the upload
/// may land on it.
#[test]
fn a_file_deleted_while_it_goes_up_leaves_no_trace() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        let gate = servers.hold(Route::BlobUpload);
        let id = ui.send_a_file("BBBBBBBB", b"the file").await;
        gate.reached().await;
        let message_id = ui.app.store.message(id).unwrap().unwrap().message_id;
        ui.ok(json!({"cmd": "deleteMessage", "identity": "BBBBBBBB",
                     "messageId": format!("{message_id:016x}"),
                     "outgoing": true, "everyone": false}))
            .await;
        assert!(ui.app.store.message(id).unwrap().is_none());

        let next = ui.send_a_file("BBBBBBBB", b"the next").await;
        assert_eq!(next, id, "the row id came back: what this is about");
        // The next file is up before the deleted one's upload ends, which then must not touch it.
        while ui.upload_state(next) != Some(STATE_LOCAL) {
            let event = ui.media_events.as_mut().unwrap().recv().await.unwrap();
            ui.app.on_media_event(&event);
        }
        let up = ui.app.store.file_keys(next).unwrap().unwrap();
        gate.open();
        ui.settle().await;
        assert_eq!(ui.upload_state(next), Some(STATE_LOCAL));
        let keys = ui.app.store.file_keys(next).unwrap().unwrap();
        assert_eq!(*keys.key, *up.key);
        assert_eq!(keys.blob_id, up.blob_id);
        let file = ui.file_going_out(next).unwrap();
        assert_eq!(file.size, 8, "the next file's own");
        assert!(
            ui.app
                .store
                .pending_outgoing()
                .unwrap()
                .iter()
                .all(|m| m.id == next)
        );
    });
}

/// §7.8: a hidden chat's file doesn't start up by a guessed id, and says nothing of itself.
#[test]
fn a_locked_app_retries_no_upload_of_a_private_chat() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        servers.answer_with(Route::BlobUpload, Answer::Status(413));
        let id = ui.send_a_file("BBBBBBBB", b"the file").await;
        ui.settle().await;
        ui.app.store.set_private("BBBBBBBB", true).unwrap();
        ui.lock().await;
        servers.back_up(Route::BlobUpload);

        let tries = servers.uploads_seen();
        let reply = ui
            .command(json!({"cmd": "retryUpload", "message": id}))
            .await;
        assert_eq!(reply["error"]["kind"], "locked", "{reply}");
        ui.settle().await;
        assert_eq!(servers.uploads_seen(), tries);
        assert_eq!(ui.upload_state(id), Some(STATE_FAILED));
    });
}

#[test]
fn a_retry_needs_a_file_of_ours_in_a_chat_that_can_be_written_to() {
    scenario(async {
        let (servers, _in_place) = Servers::start();
        let mut ui = Ui::with_identity();
        servers.answer_with(Route::BlobUpload, Answer::Status(413));
        let id = ui.send_a_file("BBBBBBBB", b"the file").await;
        ui.settle().await;
        servers.back_up(Route::BlobUpload);
        let tries = servers.uploads_seen();

        let reply = ui
            .command(json!({"cmd": "retryUpload", "message": 9999}))
            .await;
        assert_eq!(reply["ok"], false, "{reply}");

        let incoming = ui
            .app
            .store
            .insert_incoming("BBBBBBBB", 77, "file", r#"{"uploadState": 3}"#, 1)
            .unwrap()
            .unwrap();
        let reply = ui
            .command(json!({"cmd": "retryUpload", "message": incoming.id}))
            .await;
        assert_eq!(reply["ok"], false, "{reply}");

        ui.ok(json!({"cmd": "setBlocked", "identity": "BBBBBBBB", "blocked": true}))
            .await;
        let reply = ui
            .command(json!({"cmd": "retryUpload", "message": id}))
            .await;
        assert_eq!(reply["error"]["kind"], "contactBlocked", "{reply}");

        ui.settle().await;
        assert_eq!(servers.uploads_seen(), tries);
        assert_eq!(ui.upload_state(id), Some(STATE_FAILED));
    });
}
