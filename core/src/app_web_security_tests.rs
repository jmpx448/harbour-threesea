// SPDX-License-Identifier: AGPL-3.0-only
//! Regression tests for the security audit of 2026-10-27 (docs/security-audit-2026-10-27.md):
//! its reproductions, turned around: each unauthorized operation must be rejected.
use super::*;
use crate::{app::StartConfig, media::MediaPart, store::StoredIdentity};

/// A core with an identity, two contacts and an app lock (private chats hidden).
fn fixture() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(
        &StartConfig {
            data_dir: dir.path().to_owned(),
            app_version: "test".into(),
            language: "en".into(),
            country: "CH".into(),
            model: "test".into(),
            os_version: "test".into(),
            pictures_dir: None,
            calls: false,
            log_level: None,
            update_check: false,
        },
        &[3; 32],
    )
    .unwrap();
    app.store
        .set_identity(&StoredIdentity {
            threema_id: "AAAAAAAA".into(),
            client_key: Zeroizing::new([7; 32]),
            server_group: "a".into(),
        })
        .unwrap();
    for id in ["BBBBBBBB", "CCCCCCCC"] {
        app.store
            .upsert_contact(&crate::contacts::test_contact(id))
            .unwrap();
    }
    app.store
        .set_lock(Some(&crate::applock::Lock::with_pin(
            "2468", false, true, 0,
        )))
        .unwrap();
    (dir, app)
}

/// An authenticated browser session (id 1) without a network behind it.
fn browser(app: &mut App) -> UnboundedReceiver<ToWeb> {
    app.web_test_session(1).0
}

/// S03: a browser subscribed to a chat gets nothing more of it once it is private.
#[test]
fn web_subscription_ends_when_chat_becomes_private() {
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    let original = app
        .store
        .insert_incoming("BBBBBBBB", 1, "text", "old text", 1)
        .unwrap()
        .unwrap();
    app.web_messages(1, "BBBBBBBB", None).unwrap();
    app.store.set_private("BBBBBBBB", true).unwrap();
    app.web_refresh();
    while rx.try_recv().is_ok() {}
    let changed = app
        .store
        .edit_message(original.id, "NEW PRIVATE CONTENT", Some(2))
        .unwrap()
        .unwrap();
    app.web_on_core_event(&CoreEvent::MessagesChanged(vec![changed]));
    while let Ok(sent) = rx.try_recv() {
        if let ToWeb::Send(wire) = sent {
            assert!(!wire.to_string().contains("NEW PRIVATE CONTENT"));
        }
    }
    assert!(
        !app.web.subscriptions[&1].shows("BBBBBBBB"),
        "subscription revoked"
    );
}

/// S03: a download asked for before the chat became private is not delivered.
#[test]
fn web_pending_download_not_delivered_after_chat_becomes_private() {
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    let message = app
        .store
        .insert_incoming(
            "BBBBBBBB",
            2,
            "file",
            &json!({"fileState": 1, "mediaType": "application/octet-stream", "name": "p.bin"})
                .to_string(),
            1,
        )
        .unwrap()
        .unwrap();
    app.web
        .pending_blobs
        .push(crate::app_web_media::PendingBlob {
            session: 1,
            message: message.id,
            id: Some("request-1".into()),
        });
    app.store.set_private("BBBBBBBB", true).unwrap();
    app.media
        .write(message.id, MediaPart::File, b"NEW PRIVATE FILE")
        .unwrap();
    app.store
        .set_media_state(message.id, MediaPart::File, crate::app_media::STATE_LOCAL)
        .unwrap();
    app.web_blobs_changed();
    let ToWeb::Send(wire) = rx.try_recv().unwrap() else {
        panic!("no answer to the request")
    };
    let blob = saltyrtc::field(&wire, "data").and_then(|data| saltyrtc::field(data, "blob"));
    assert!(blob.is_none(), "no file data for a private chat");
}

/// S02: a browser can't manage a private group (nor rename, leave or sync it).
#[tokio::test(flavor = "current_thread")]
async fn web_cannot_manage_private_group() {
    let (_dir, mut app) = fixture();
    let group = app
        .create_group("private group", &["BBBBBBBB".into()])
        .unwrap();
    let key = group["key"].as_str().unwrap().to_owned();
    app.store.set_private(&key, true).unwrap();
    for (kind, sub_type, data) in [
        (
            "update",
            "group",
            Some(arp::map(vec![(
                "members",
                Value::Array(vec!["BBBBBBBB".into(), "CCCCCCCC".into()]),
            )])),
        ),
        (
            "update",
            "group",
            Some(arp::map(vec![("name", "renamed".into())])),
        ),
        ("delete", "group", None),
        ("request", "groupSync", None),
    ] {
        let wire = arp::message(
            kind,
            sub_type,
            Some(arp::map(vec![("id", key.as_str().into())])),
            data,
        );
        let incoming = Incoming::parse(&wire).unwrap();
        assert!(
            app.web_manage(&incoming).await.unwrap().is_err(),
            "{kind}/{sub_type} on a private group"
        );
    }
    let group = app.store.group(&key).unwrap().unwrap();
    assert!(!group.members.contains(&"CCCCCCCC".to_owned()));
    assert_eq!(group.name, "private group");
    assert!(!group.left);
}

/// S04: while private chats are hidden, nothing returns or forwards their content.
#[tokio::test(flavor = "current_thread")]
async fn core_commands_respect_hidden_private_chats() {
    let (_dir, mut app) = fixture();
    let message = app
        .store
        .insert_incoming("BBBBBBBB", 1, "text", "PRIVATE BODY", 1)
        .unwrap()
        .unwrap();
    app.store.set_private("BBBBBBBB", true).unwrap();
    app.store
        .set_setting("draft:BBBBBBBB", "PRIVATE DRAFT")
        .unwrap();
    for request in [
        json!({"id": 1, "cmd": "message", "message": message.id}),
        json!({"id": 2, "cmd": "setStarred", "message": message.id, "starred": true}),
        json!({"id": 3, "cmd": "draft", "identity": "BBBBBBBB"}),
        json!({"id": 4, "cmd": "forwardMessage", "message": message.id, "identity": "CCCCCCCC"}),
    ] {
        let reply = app.handle(&request.to_string()).await;
        assert!(!reply.contains("PRIVATE"), "{request}: {reply}");
        let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["error"]["kind"], "locked", "{request}");
    }
    assert!(
        app.store
            .conversation_messages("CCCCCCCC", 10)
            .unwrap()
            .is_empty()
    );
    assert!(!app.store.message(message.id).unwrap().unwrap().starred);
}

/// S01: deleting the identity ends its browser sessions; nothing of the next identity reaches
/// them.
#[tokio::test(flavor = "current_thread")]
async fn web_sessions_end_with_identity_deletion() {
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    app.delete_all_data().unwrap();
    assert!(app.web.running.is_empty());
    assert!(matches!(rx.try_recv(), Ok(ToWeb::Stop)));
    app.store
        .set_identity(&StoredIdentity {
            threema_id: "DDDDDDDD".into(),
            client_key: Zeroizing::new([8; 32]),
            server_group: "a".into(),
        })
        .unwrap();
    app.store
        .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
        .unwrap();
    app.store
        .insert_incoming("BBBBBBBB", 10, "text", "NEW ID DATA", 10)
        .unwrap();
    let message = arp::message(
        "request",
        "messages",
        Some(arp::map(vec![("id", "BBBBBBBB".into())])),
        None,
    );
    // The deleted session's task was the first one (1).
    app.on_web_event((
        1,
        FromWeb::Message {
            session: 1,
            message,
        },
    ))
    .await;
    assert!(rx.try_recv().is_err(), "the old session got no answer");
}

/// S01: a request the session task had queued is not run after the session was deleted.
#[tokio::test(flavor = "current_thread")]
async fn web_request_after_deletion_is_ignored() {
    let (_dir, mut app) = fixture();
    let _rx = browser(&mut app);
    app.web_delete(1).unwrap();
    let message = arp::message(
        "create",
        "textMessage",
        Some(arp::map(vec![("id", "BBBBBBBB".into())])),
        Some(arp::map(vec![("text", "AFTER REVOCATION".into())])),
    );
    // The deleted session's task was the first one (1).
    app.on_web_event((
        1,
        FromWeb::Message {
            session: 1,
            message,
        },
    ))
    .await;
    assert!(
        app.store
            .conversation_messages("BBBBBBBB", 10)
            .unwrap()
            .is_empty()
    );
}

/// Locked private chats: a group rename's event carries no name.
#[test]
fn private_rename_event_has_no_name() {
    let (_dir, app) = fixture();
    let group = app
        .store
        .insert_status("BBBBBBBB", "groupRenamed|Secret plans")
        .unwrap()
        .unwrap();
    app.store.set_private("BBBBBBBB", true).unwrap();
    let event = CoreEvent::MessageAdded {
        message: group,
        send_delivery_receipt: false,
    }
    .to_json(&app.store, false);
    assert!(!event.to_string().contains("Secret plans"), "{event}");
}

/// Wrong PINs pause PIN entry like Android's: after three, 30 s, then longer; not even the
/// right PIN is checked meanwhile; the right one afterwards starts over.
#[tokio::test(flavor = "current_thread")]
async fn wrong_pins_pause_pin_entry() {
    let (_dir, mut app) = fixture();
    let unlock =
        |pin: &str| json!({"id": 1, "cmd": "unlock", "pin": pin, "privateChats": true}).to_string();
    let kind = |reply: String| -> String {
        let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
        reply["error"]["kind"].as_str().unwrap_or("ok").to_owned()
    };
    let pause_over = |app: &App| {
        app.store
            .set_setting(crate::store::keys::PIN_LOCKOUT_UNTIL, "0")
            .unwrap();
    };
    for _ in 0..3 {
        assert_eq!(kind(app.handle(&unlock("0000")).await), "wrongPin");
    }
    let reply: serde_json::Value =
        serde_json::from_str(&app.handle(&unlock("0000")).await).unwrap();
    assert_eq!(reply["error"]["kind"], "pinLockout");
    assert_eq!(reply["error"]["message"], "30");
    assert_eq!(kind(app.handle(&unlock("2468")).await), "pinLockout");
    assert!(!app.unlocked);
    pause_over(&app);
    for _ in 0..3 {
        assert_eq!(kind(app.handle(&unlock("0000")).await), "wrongPin");
    }
    let reply: serde_json::Value =
        serde_json::from_str(&app.handle(&unlock("0000")).await).unwrap();
    assert_eq!(reply["error"]["message"], "60", "the next pause is longer");
    pause_over(&app);
    assert_eq!(kind(app.handle(&unlock("2468")).await), "ok");
    assert!(app.unlocked);
    // Started over: three wrong ones are allowed again, then 30 s.
    for _ in 0..3 {
        assert_eq!(kind(app.handle(&unlock("0000")).await), "wrongPin");
    }
    let reply: serde_json::Value =
        serde_json::from_str(&app.handle(&unlock("0000")).await).unwrap();
    assert_eq!(reply["error"]["message"], "30");
}

fn stored_session(app: &App) -> i64 {
    app.store
        .add_web_session(&crate::web::WebSession {
            id: 0,
            initiator_key: [1; 32],
            private_key: Zeroizing::new([2; 32]),
            server_key: None,
            host: "saltyrtc.threema.ch".into(),
            port: 443,
            self_hosted: false,
            persistent: false,
            client: "test".into(),
            created_at: 1,
            last_connection: None,
        })
        .unwrap()
}

/// Deep audit D01: a deleted session's ID can come back for a new session; the old task's
/// late requests and its end must not touch the new one.
#[tokio::test(flavor = "current_thread")]
async fn replaced_session_ignores_old_task() {
    let (_dir, mut app) = fixture();
    let old = stored_session(&app);
    let (_old_rx, old_task) = app.web_test_session(old);
    app.web_delete(old).unwrap();
    let next = stored_session(&app);
    assert_eq!(old, next, "SQLite reused the ID (the case this guards)");
    let (_new_rx, new_task) = app.web_test_session(next);
    assert_ne!(old_task, new_task);
    app.store.set_web_session_persistent(next).unwrap();

    let message = arp::message(
        "create",
        "textMessage",
        Some(arp::map(vec![("id", "BBBBBBBB".into())])),
        Some(arp::map(vec![("text", "REVOKED SESSION COMMAND".into())])),
    );
    app.on_web_event((
        old_task,
        FromWeb::Message {
            session: old,
            message,
        },
    ))
    .await;
    assert!(
        app.store
            .conversation_messages("BBBBBBBB", 10)
            .unwrap()
            .is_empty(),
        "the old task's request did not run"
    );
    app.on_web_event((
        old_task,
        FromWeb::Ended {
            session: old,
            delete: true,
            error: None,
        },
    ))
    .await;
    assert!(
        app.store.web_session(next).unwrap().is_some(),
        "the new session stays"
    );
    app.on_web_event((
        old_task,
        FromWeb::Pinned {
            session: old,
            host: "evil.example".into(),
            spki: "x".into(),
        },
    ))
    .await;
    assert!(app.store.web_host("evil.example").unwrap().is_none());
    // The new task's own end is taken.
    app.on_web_event((
        new_task,
        FromWeb::Ended {
            session: next,
            delete: true,
            error: None,
        },
    ))
    .await;
    assert!(app.store.web_session(next).unwrap().is_none());
}

/// Deep audit D02: votes, reactions, edits and copies don't reach a hidden private chat's
/// content; its media are not handed to the UI's readers while hidden.
#[tokio::test(flavor = "current_thread")]
async fn private_content_stays_hidden_from_mutations_and_media() {
    let (_dir, mut app) = fixture();
    let text = app
        .store
        .insert_incoming("BBBBBBBB", 123, "text", "PRIVATE REACTION BODY", 1)
        .unwrap()
        .unwrap();
    let poll = crate::polls::new_poll(
        42,
        "BBBBBBBB",
        "PRIVATE POLL QUESTION",
        &["Yes".into(), "No".into()],
        false,
        false,
    )
    .unwrap();
    let poll = app
        .store
        .insert_incoming("BBBBBBBB", 124, "poll", &poll.to_string(), 1)
        .unwrap()
        .unwrap();
    let mine = app
        .store
        .insert_outgoing("BBBBBBBB", 125, "text", "PRIVATE OWN TEXT")
        .unwrap();
    let voice = app
        .store
        .insert_incoming(
            "BBBBBBBB",
            128,
            "file",
            &json!({"mediaType": "audio/aac", "name": "PRIVATE VOICE"}).to_string(),
            1,
        )
        .unwrap()
        .unwrap();
    let other = app
        .store
        .insert_incoming("CCCCCCCC", 126, "text", "public", 1)
        .unwrap()
        .unwrap();
    app.store.set_private("BBBBBBBB", true).unwrap();
    app.update_media_gate();
    for request in [
        json!({"id": 1, "cmd": "vote", "message": poll.id, "choices": [0]}),
        json!({"id": 2, "cmd": "react", "identity": "BBBBBBBB", "messageId": "000000000000007b", "emoji": "👍", "apply": true}),
        json!({"id": 3, "cmd": "editMessage", "identity": "BBBBBBBB", "messageId": format!("{:016x}", mine.message_id), "text": "changed"}),
        json!({"id": 4, "cmd": "sendText", "identity": "BBBBBBBB", "text": "into a hidden chat"}),
        json!({"id": 5, "cmd": "markRead", "identity": "BBBBBBBB"}),
        json!({"id": 6, "cmd": "addGifFavourite", "message": text.id}),
        json!({"id": 7, "cmd": "setVoiceListened", "message": voice.id}),
    ] {
        let reply = app.handle(&request.to_string()).await;
        assert!(!reply.contains("PRIVATE"), "{request}: {reply}");
        let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["error"]["kind"], "locked", "{request}");
    }
    assert_eq!(
        app.store.message(mine.id).unwrap().unwrap().body,
        "PRIVATE OWN TEXT"
    );
    assert!(
        !app.store
            .message(voice.id)
            .unwrap()
            .unwrap()
            .body
            .contains("listened"),
        "the hidden message stays unchanged"
    );
    assert!(!app.media.shown(text.id) && !app.media.shown(poll.id));
    assert!(
        app.media.shown(other.id),
        "other chats' media stay readable"
    );
    // A message arriving in the hidden chat is hidden at once.
    let late = app
        .store
        .insert_incoming("BBBBBBBB", 127, "text", "late", 1)
        .unwrap()
        .unwrap();
    app.gate_media(&CoreEvent::MessageAdded {
        message: late.clone(),
        send_delivery_receipt: false,
    });
    assert!(!app.media.shown(late.id));
    // Unlocked: everything is readable again.
    let reply = app
        .handle(&json!({"id": 7, "cmd": "unlock", "pin": "2468", "privateChats": true}).to_string())
        .await;
    assert!(reply.contains("\"ok\":true"), "{reply}");
    assert!(app.media.shown(text.id) && app.media.shown(late.id));
}

/// A chat marked private on a linked device: its older media leave the UI's readers at once and
/// the event tells the UI (which closes the chat's notification); the same mark again changes
/// nothing.
#[test]
fn private_mark_from_a_linked_device_reaches_the_gate_and_the_ui() {
    use libthreema::protobuf::{
        d2d::{ContactSync, contact_sync},
        d2d_sync::{self, ConversationCategory},
    };
    let (_dir, app) = fixture();
    let old = app
        .store
        .insert_incoming("BBBBBBBB", 1, "text", "older message", 1)
        .unwrap()
        .unwrap();
    app.update_media_gate();
    assert!(app.media.shown(old.id));
    let sync = ContactSync {
        action: Some(contact_sync::Action::Update(contact_sync::Update {
            contact: Some(d2d_sync::Contact {
                identity: "BBBBBBBB".into(),
                conversation_category: Some(ConversationCategory::Protected as i32),
                ..Default::default()
            }),
        })),
    };
    let marked = |events: &[CoreEvent]| -> Vec<serde_json::Value> {
        events
            .iter()
            .filter(|event| matches!(event, CoreEvent::PrivateChanged { .. }))
            .map(|event| event.to_json(&app.store, app.unlocked))
            .collect()
    };

    let mut events = Vec::new();
    crate::sync::contact_sync(&app.store, sync.clone(), &mut events).unwrap();
    assert_eq!(
        marked(&events),
        [json!({"event": "privateChanged", "identity": "BBBBBBBB", "private": true})]
    );
    for event in &events {
        app.gate_media(event);
    }
    assert!(!app.media.shown(old.id));

    let mut events = Vec::new();
    crate::sync::contact_sync(&app.store, sync, &mut events).unwrap();
    assert!(marked(&events).is_empty());
}

/// Marking a chat in the UI goes the same way, so its notification closes there too.
#[tokio::test(flavor = "current_thread")]
async fn marking_a_chat_private_tells_the_ui() {
    let (_dir, mut app) = fixture();
    let mut events = app.take_media_events().unwrap();
    let reply = app
        .handle(
            &json!({"id": 1, "cmd": "setPrivate", "identity": "BBBBBBBB", "private": true})
                .to_string(),
        )
        .await;
    assert!(reply.contains("\"ok\":true"), "{reply}");
    let event = events.try_recv().unwrap();
    assert_eq!(
        event.to_json(&app.store, app.unlocked),
        json!({"event": "privateChanged", "identity": "BBBBBBBB", "private": true})
    );
}

/// Deep audit D05: a browser's own text doesn't end up in the log, also for requests the app
/// doesn't know.
#[tokio::test(flavor = "current_thread")]
async fn browser_text_is_not_logged() {
    use std::sync::{Arc, Mutex};
    struct Writer(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let output = bytes.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || Writer(output.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (_dir, mut app) = fixture();
    let (_rx, task) = app.web_test_session(1);
    let message = arp::message("PEER_SECRET_SENTINEL", "ARBITRARY_PEER_TEXT", None, None);
    app.on_web_event((
        task,
        FromWeb::Message {
            session: 1,
            message,
        },
    ))
    .await;
    let logged = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("not handled"), "{logged}");
    assert!(!logged.contains("PEER_SECRET_SENTINEL") && !logged.contains("ARBITRARY_PEER_TEXT"));
}

/// Timing (deep audit P02): an unchanged refresh with 1000 chats of 100 messages each, one
/// browser subscribed to the chat list (`cargo test -p threesea-core --release -- --ignored
/// --nocapture unchanged_refresh`).
#[test]
#[ignore = "timing, not a check"]
fn unchanged_refresh_timing() {
    let (_dir, mut app) = fixture();
    app.store.set_lock(None).unwrap();
    app.store.conn.execute_batch("BEGIN").unwrap();
    for chat in 0..1000_u64 {
        let id = format!("C{chat:07}");
        app.store
            .upsert_contact(&crate::contacts::test_contact(&id))
            .unwrap();
        for n in 0..100_u64 {
            app.store
                .insert_incoming(&id, chat * 1000 + n, "text", "x", 1)
                .unwrap();
        }
    }
    app.store.conn.execute_batch("COMMIT").unwrap();
    let _rx = browser(&mut app);
    app.web_conversations(1).unwrap();
    app.web_refresh();
    let start = std::time::Instant::now();
    for _ in 0..10 {
        app.web_refresh();
    }
    eprintln!("unchanged web_refresh: {:?} each", start.elapsed() / 10);
    app.store
        .insert_incoming("C0000001", 999_999, "text", "y", 2)
        .unwrap();
    let start = std::time::Instant::now();
    app.web_refresh();
    eprintln!("web_refresh after a new message: {:?}", start.elapsed());
}

/// The browser's chat list follows the state of the newest message (pending → sent →
/// delivered), not only new messages: its status icon stayed on "pending".
#[test]
fn web_chat_list_follows_message_state() {
    let (_dir, mut app) = fixture();
    app.store.set_lock(None).unwrap();
    let mut rx = browser(&mut app);
    let sent = app
        .store
        .insert_outgoing("BBBBBBBB", 77, "text", "hello")
        .unwrap();
    app.web_conversations(1).unwrap();
    app.web_refresh();
    while rx.try_recv().is_ok() {}
    app.store
        .raise_outgoing_state(
            "BBBBBBBB",
            &[sent.message_id],
            crate::messages::OutgoingState::Delivered,
        )
        .unwrap();
    app.web_refresh();
    let mut updated = false;
    while let Ok(message) = rx.try_recv() {
        if let ToWeb::Send(wire) = message {
            let text = wire.to_string();
            updated |= text.contains("conversation") && text.contains("modified");
        }
    }
    assert!(updated, "the chat list entry was sent again");
}

/// Reading a chat anywhere but in the app (Threema Web's `request/read`) tells the UI, which
/// takes the chat's notification away: once, and only when something was unread.
#[tokio::test(flavor = "current_thread")]
async fn reading_tells_the_ui() {
    let (_dir, mut app) = fixture();
    app.store.set_lock(None).unwrap();
    let mut events = app.take_media_events().unwrap();
    app.store
        .insert_incoming("BBBBBBBB", 5, "text", "hi", 1)
        .unwrap();
    let (_rx, task) = app.web_test_session(1);
    let read = arp::message(
        "request",
        "read",
        Some(arp::map(vec![("id", "BBBBBBBB".into())])),
        None,
    );
    app.on_web_event((
        task,
        FromWeb::Message {
            session: 1,
            message: read.clone(),
        },
    ))
    .await;
    let mut seen = 0;
    while let Ok(event) = events.try_recv() {
        let json = event.to_json(&app.store, true);
        if json["event"] == "conversationRead" {
            assert_eq!(json["identity"], "BBBBBBBB");
            seen += 1;
        }
    }
    assert_eq!(seen, 1);
    // Nothing unread any more: no event.
    app.on_web_event((
        task,
        FromWeb::Message {
            session: 1,
            message: read,
        },
    ))
    .await;
    while let Ok(event) = events.try_recv() {
        assert_ne!(event.to_json(&app.store, true)["event"], "conversationRead");
    }
}
