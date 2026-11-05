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

/// A private group's or list's name and members stay on the device: never in the receiver lists,
/// taken out of a connected browser when marked and back in when unmarked. A private contact
/// stays, as locked and not visible, since groups name their members by ID.
#[test]
fn private_groups_and_lists_stay_out_of_the_browser() {
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    app.web_subscribe_receivers(1);
    let group = app
        .create_group("TELLING GROUP NAME", &["BBBBBBBB".into()])
        .unwrap();
    let key = group["key"].as_str().unwrap().to_owned();
    let list = crate::distribution_lists::DistributionList {
        key: "d:00000000000000ff".into(),
        name: "TELLING LIST NAME".into(),
        members: vec!["CCCCCCCC".into()],
        created_at: 1,
    };
    app.store.upsert_distribution_list(&list).unwrap();
    let names = |app: &App| {
        (
            app.web_receivers_of("group").unwrap().len(),
            app.web_receivers_of("distributionList").unwrap().len(),
        )
    };
    assert_eq!(names(&app), (1, 1));
    app.web_refresh();
    wires(&mut rx);

    for (conversation, name) in [
        (key.as_str(), "TELLING GROUP NAME"),
        (list.key.as_str(), "TELLING LIST NAME"),
    ] {
        app.store.set_private(conversation, true).unwrap();
        app.web_refresh();
        let update = wires(&mut rx);
        assert_eq!(update.len(), 1, "one update for {conversation}");
        assert!(update[0].contains("removed") && !update[0].contains(name));
    }
    assert_eq!(names(&app), (0, 0));
    let everything = app.web_receivers().unwrap().to_string();
    assert!(!everything.contains("TELLING"));

    app.store.set_private(&key, false).unwrap();
    app.web_refresh();
    assert!(wires(&mut rx)[0].contains("TELLING GROUP NAME"));

    app.store.set_private("BBBBBBBB", true).unwrap();
    let contact = app
        .web_receivers_of("contact")
        .unwrap()
        .into_iter()
        .find(|contact| saltyrtc::field(contact, "id") == Some(&Value::from("BBBBBBBB")))
        .unwrap();
    assert_eq!(
        saltyrtc::field(&contact, "locked"),
        Some(&Value::from(true))
    );
    assert_eq!(
        saltyrtc::field(&contact, "visible"),
        Some(&Value::from(false))
    );
}

/// What a browser was sent since the last call, as text.
fn wires(rx: &mut UnboundedReceiver<ToWeb>) -> Vec<String> {
    let mut wires = Vec::new();
    while let Ok(message) = rx.try_recv() {
        if let ToWeb::Send(wire) = message {
            wires.push(wire.to_string());
        }
    }
    wires
}

/// Taking the mark off a group in the UI: the browser hears of the receiver before it gets the
/// conversation of that receiver.
#[tokio::test(flavor = "current_thread")]
async fn unmarked_group_reaches_the_browser_as_receiver_first() {
    let (_dir, mut app) = fixture();
    let mut media_events = app.take_media_events();
    let emit: crate::runtime::Emit = Box::new(|_| {});
    let mut rx = browser(&mut app);
    let group = app
        .create_group("TELLING GROUP NAME", &["BBBBBBBB".into()])
        .unwrap();
    let key = group["key"].as_str().unwrap().to_owned();
    app.store.set_private(&key, true).unwrap();
    app.unlocked = true;
    app.web_subscribe_receivers(1);
    app.web_conversations(1).unwrap();
    crate::runtime::settle_command(&mut app, &mut media_events, &emit);
    wires(&mut rx);

    let reply = app
        .handle(
            &json!({"id": 1, "cmd": "setPrivate", "identity": key, "private": false}).to_string(),
        )
        .await;
    assert!(reply.contains("\"ok\":true"), "{reply}");
    crate::runtime::settle_command(&mut app, &mut media_events, &emit);

    let sent = wires(&mut rx);
    let position = |subtype: &str| {
        sent.iter()
            .position(|wire| wire.contains(subtype))
            .unwrap_or_else(|| panic!("{subtype} not sent: {sent:?}"))
    };
    assert!(sent[position("\"receiver\"")].contains("TELLING GROUP NAME"));
    assert!(position("\"receiver\"") < position("\"conversation\""));
}

/// The private marks can't be read: the browser's lists are refused and nothing goes out, rather
/// than everything counting as public.
#[test]
fn browser_gets_nothing_when_the_private_marks_cannot_be_read() {
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    let group = app
        .create_group("TELLING GROUP NAME", &["BBBBBBBB".into()])
        .unwrap();
    let key = group["key"].as_str().unwrap().to_owned();
    app.store.set_private(&key, true).unwrap();
    app.store
        .insert_incoming("BBBBBBBB", 1, "text", "older message", 1)
        .unwrap();
    app.web_messages(1, "BBBBBBBB", None).unwrap();
    app.web_subscribe_receivers(1);
    app.web_conversations(1).unwrap();
    app.web_refresh();
    wires(&mut rx);

    app.store
        .conn
        .execute_batch("DROP TABLE private_chats")
        .unwrap();
    for kind in ["contact", "group", "distributionList"] {
        assert!(app.web_receivers_of(kind).is_err(), "{kind}");
    }
    assert!(app.web_receivers().is_err());
    // Something new for the browser, so the refresh does run.
    app.store
        .insert_incoming("CCCCCCCC", 2, "text", "newer message", 2)
        .unwrap();
    app.web_refresh();
    assert_eq!(wires(&mut rx), Vec::<String>::new());
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

/// Removing the ID takes its logs and media along, and the old private chats' gate doesn't block
/// the media of the next ID, whose message ids start over.
#[tokio::test(flavor = "current_thread")]
async fn removing_the_id_leaves_no_media_log_or_gate() {
    let (dir, mut app) = fixture();
    app.store.set_private("BBBBBBBB", true).unwrap();
    let message = app
        .store
        .insert_incoming("BBBBBBBB", 1, "text", "private words", 1)
        .unwrap()
        .unwrap();
    app.media
        .write(message.id, MediaPart::File, b"private picture")
        .unwrap();
    app.media
        .write_picture("group:x", b"group picture")
        .unwrap();
    std::fs::write(dir.path().join("threesea.log"), b"old lines").unwrap();
    std::fs::write(dir.path().join("threesea.log.0"), b"older lines").unwrap();
    app.unlocked = true;
    app.update_media_gate();
    app.unlocked = false;
    app.update_media_gate();
    assert!(!app.media.shown(message.id));

    app.delete_all_data().await.unwrap();

    assert_eq!(app.media.size(), 0);
    assert!(app.media.read_picture("group:x").unwrap().is_none());
    assert!(app.media.shown(message.id), "the gate is gone");
    assert!(!app.unlocked);
    assert_eq!(crate::runtime::log_text(dir.path(), usize::MAX), "");
}

/// The store refuses (a full disk has no room for its write-ahead log): the removal goes on, as
/// replacing the file needs no space. No connection comes back for the ID still in it, and a kill
/// now would be finished by the next start.
#[tokio::test(flavor = "current_thread")]
async fn a_store_that_refuses_does_not_stop_the_removal() {
    let (dir, mut app) = fixture();
    app.store.set_lock(None).unwrap();
    app.store.set_setting("anything", "x").unwrap();
    app.store
        .conn
        .execute_batch(
            "CREATE TRIGGER refuse BEFORE DELETE ON settings
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
    app.media.write(1, MediaPart::File, b"picture").unwrap();

    let reply = app
        .handle(&json!({"id": 1, "cmd": "deleteAllData"}).to_string())
        .await;

    assert!(reply.contains("\"ok\":true"), "{reply}");
    assert!(app.store.identity().unwrap().is_some(), "the trigger held");
    assert_eq!(app.media.size(), 0);
    assert_eq!(crate::runtime::removal_mark(dir.path()), Some(false));
    assert!(app.ensure_connection().is_none());
}

/// A refused PIN is the one way out, and nothing has happened by then.
#[tokio::test(flavor = "current_thread")]
async fn a_refused_pin_removes_nothing() {
    let (dir, mut app) = fixture();
    app.media.write(1, MediaPart::File, b"picture").unwrap();

    let reply = app
        .handle(&json!({"id": 1, "cmd": "deleteAllData", "pin": "1111"}).to_string())
        .await;

    assert!(reply.contains("\"ok\":false"), "{reply}");
    assert!(app.store.identity().unwrap().is_some());
    assert!(app.media.size() > 0);
    assert_eq!(crate::runtime::removal_mark(dir.path()), None);
    assert!(!app.identity_removed);
}

/// S01: deleting the identity ends its browser sessions; nothing of the next identity reaches
/// them.
#[tokio::test(flavor = "current_thread")]
async fn web_sessions_end_with_identity_deletion() {
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    app.delete_all_data().await.unwrap();
    assert!(app.web.running.is_empty());
    assert!(matches!(rx.try_recv(), Ok(ToWeb::Stop(StopReason::Delete))));
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
    let event = CoreEvent::MessageAdded { message: group }.to_json(&app.store, false);
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
    });
    assert!(!app.media.shown(late.id));
    // Unlocked: everything is readable again.
    let reply = app
        .handle(&json!({"id": 7, "cmd": "unlock", "pin": "2468", "privateChats": true}).to_string())
        .await;
    assert!(reply.contains("\"ok\":true"), "{reply}");
    assert!(app.media.shown(text.id) && app.media.shown(late.id));
}

/// What takes the ID or the chats off the device, or gives someone a hold on the ID, needs the
/// PIN whether or not the private chats are open, in the core; without a lock there is no PIN to
/// ask for.
#[tokio::test(flavor = "current_thread")]
async fn pin_guards_backups_linking_and_removal() {
    async fn kind(app: &mut App, mut command: serde_json::Value) -> String {
        command["id"] = json!(1);
        let reply: serde_json::Value =
            serde_json::from_str(&app.handle(&command.to_string()).await).unwrap();
        reply["error"]["kind"].as_str().unwrap_or("").to_owned()
    }
    let (dir, mut app) = fixture();
    let open = json!({"cmd": "unlock", "pin": "2468", "privateChats": true});
    assert_eq!(kind(&mut app, open.clone()).await, "");
    let folder = dir.path().to_str().unwrap();
    let guarded = [
        json!({"cmd": "exportIdentityBackup", "password": "long enough"}),
        json!({"cmd": "safeEnable", "password": "long enough"}),
        json!({"cmd": "safeDisable"}),
        json!({"cmd": "androidBackupExport", "directory": folder, "password": "long enough"}),
        json!({"cmd": "androidBackupSchedule", "directory": folder, "password": "long enough"}),
        json!({"cmd": "setRevocationPassword", "password": "long enough"}),
        json!({"cmd": "startLinking", "code": "not a code"}),
        json!({"cmd": "deleteAllData"}),
    ];
    for command in &guarded {
        assert_eq!(
            kind(&mut app, command.clone()).await,
            "wrongPin",
            "{command}"
        );
        let mut wrong = command.clone();
        wrong["pin"] = json!("1111");
        assert_eq!(kind(&mut app, wrong).await, "wrongPin", "{command}");
        // The right PIN clears the count of wrong ones.
        assert_eq!(kind(&mut app, open.clone()).await, "");
    }
    assert!(app.store.identity().unwrap().is_some());
    assert_eq!(
        app.android_export_status().unwrap()["directory"],
        serde_json::Value::Null
    );

    // With the PIN they go through; these need no network.
    let right = |mut command: serde_json::Value| {
        command["pin"] = json!("2468");
        command
    };
    for command in [&guarded[0], &guarded[2], &guarded[4]] {
        assert_eq!(
            kind(&mut app, right(command.clone())).await,
            "",
            "{command}"
        );
    }
    assert_ne!(kind(&mut app, right(guarded[6].clone())).await, "wrongPin");
    assert_eq!(kind(&mut app, right(guarded[7].clone())).await, "");
    assert!(app.store.identity().unwrap().is_none());
    assert_eq!(kind(&mut app, json!({"cmd": "lockStatus"})).await, "");
}

/// Without a lock nothing asks for a PIN.
#[tokio::test(flavor = "current_thread")]
async fn no_pin_needed_without_a_lock() {
    let (_dir, mut app) = fixture();
    app.store.set_lock(None).unwrap();
    let reply = app
        .handle(&json!({"id": 1, "cmd": "deleteAllData"}).to_string())
        .await;
    assert!(reply.contains("\"ok\":true"), "{reply}");
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

/// Creating or restoring an ID doesn't replace the one on this device: that would take it off
/// without the PIN. Refused before anything else happens, with a license or without.
#[tokio::test(flavor = "current_thread")]
async fn an_existing_id_is_not_replaced() {
    let (_dir, mut app) = fixture();
    app.store
        .set_setting(crate::store::keys::LICENSE_KEY, "XXXXX-XXXXX")
        .unwrap();
    for command in [
        json!({"id": 1, "cmd": "createIdentity"}),
        json!({"id": 2, "cmd": "restoreIdentityBackup", "backup": "x", "password": "y"}),
        json!({"id": 3, "cmd": "safeRestore", "identity": "ABCD1234", "password": "y"}),
    ] {
        let reply: serde_json::Value =
            serde_json::from_str(&app.handle(&command.to_string()).await).unwrap();
        assert_eq!(reply["error"]["kind"], "identityExists", "{command}");
    }
    let identity = app.store.identity().unwrap().unwrap();
    assert_eq!(identity.threema_id, "AAAAAAAA");
    assert_eq!(*identity.client_key, [7; 32]);
}

/// Removing a session tells the browser to forget it; a mere disconnect or a refused license
/// leaves it the pairing.
#[test]
fn web_goodbye_says_whether_the_pairing_stays() {
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    app.web_delete(1).unwrap();
    assert!(matches!(rx.try_recv(), Ok(ToWeb::Stop(StopReason::Delete))));

    let mut rx = browser(&mut app);
    app.web_stop(1);
    assert!(matches!(rx.try_recv(), Ok(ToWeb::Stop(StopReason::Stop))));
    app.web_stop_all(StopReason::Stop);
    assert!(matches!(rx.try_recv(), Ok(ToWeb::Stop(StopReason::Stop))));

    for (reason, wire) in [
        (StopReason::Stop, "stop"),
        (StopReason::Delete, "delete"),
        (StopReason::Disable, "disable"),
    ] {
        let message = goodbye(reason);
        let data = crate::web::saltyrtc::field(&message, "data").unwrap();
        assert_eq!(
            crate::web::saltyrtc::field(data, "reason").and_then(Value::as_str),
            Some(wire)
        );
    }
}

/// A server's close text goes nowhere: neither into the log's failure nor to the UI.
#[tokio::test(flavor = "current_thread")]
async fn web_server_close_text_stays_out_of_the_failure() {
    let failure = WebFailure::from(WebSocketError::Closed {
        code: Some(3000),
        reason: "SERVER'S OWN WORDS".into(),
    });
    assert_eq!(failure, WebFailure::Closed(Some(3000)));
    assert!(!format!("{failure:?}").contains("WORDS"));

    let (_dir, mut app) = fixture();
    let mut events = app.take_media_events().unwrap();
    let (_rx, task) = app.web_test_session(1);
    app.on_web_event((
        task,
        FromWeb::Ended {
            session: 1,
            delete: false,
            error: Some(failure),
        },
    ))
    .await;
    let Some(CoreEvent::Call(event)) = events.recv().await else {
        panic!("no event");
    };
    assert_eq!(event["state"], "stopped");
    assert_eq!(event["error"], "closed");
}

/// A made-up Threema Web code for this browser key.
fn web_code(browser: u8, host: &str) -> String {
    let mut bytes = qr::PROTOCOL_VERSION.to_be_bytes().to_vec();
    bytes.push(0);
    bytes.extend([browser; 32]);
    bytes.extend([2; 32]);
    bytes.extend([0; 32]);
    bytes.extend(443u16.to_be_bytes());
    bytes.extend(host.as_bytes());
    data_encoding::BASE64.encode(&bytes)
}

/// A stored session of this browser, without a task.
fn stored_web_session(app: &App, browser: u8, host: &str, persistent: bool) -> i64 {
    app.store
        .add_web_session(&WebSession {
            id: 0,
            initiator_key: [browser; 32],
            private_key: Zeroizing::new([9; 32]),
            server_key: None,
            host: host.into(),
            port: 443,
            self_hosted: false,
            persistent,
            client: String::new(),
            created_at: 1,
            last_connection: None,
        })
        .unwrap()
}

const WEB_HOST: &str = "saltyrtc-01.threema.ch";

/// The same browser's code again is refused while its session works or can come back.
#[test]
fn web_code_of_a_paired_browser_is_refused() {
    let (_dir, mut app) = fixture();
    let code = web_code(1, WEB_HOST);
    let stored = stored_web_session(&app, 1, WEB_HOST, true);
    assert!(matches!(
        app.web_pair(&code, false),
        Err(ApiError::WebAlreadyPaired)
    ));
    assert_eq!(
        app.classify_qr(&code).unwrap()["error"],
        ApiError::WebAlreadyPaired.kind()
    );
    assert_eq!(app.store.web_sessions().unwrap().len(), 1);

    // Running, though the browser hasn't stored it.
    app.store.remove_web_session(stored).unwrap();
    let id = stored_web_session(&app, 1, WEB_HOST, false);
    let _rx = app.web_test_session(id);
    assert!(matches!(
        app.web_pair(&code, false),
        Err(ApiError::WebAlreadyPaired)
    ));
}

/// What a kill during pairing left behind neither stands in the way nor stays.
#[tokio::test(flavor = "current_thread")]
async fn web_code_replaces_a_stale_session() {
    let (_dir, mut app) = fixture();
    let code = web_code(1, WEB_HOST);
    stored_web_session(&app, 1, WEB_HOST, false);
    assert!(app.classify_qr(&code).unwrap().get("error").is_none());
    // Nothing here waits, so the session's task never gets to connect.
    let result = tokio::task::LocalSet::new()
        .run_until(async { app.web_pair(&code, false) })
        .await
        .unwrap();
    let sessions = app.store.web_sessions().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(result["id"], sessions[0].id);
    assert_eq!(sessions[0].initiator_key, [1; 32]);
}

/// Duplicates of one browser from before are merged at the start, keeping the last one used;
/// other browsers are left alone.
#[test]
fn web_duplicates_of_a_browser_merge() {
    let (_dir, mut app) = fixture();
    let older = stored_web_session(&app, 1, WEB_HOST, true);
    let used = stored_web_session(&app, 1, WEB_HOST, true);
    let other = stored_web_session(&app, 2, WEB_HOST, true);
    let never_stored = stored_web_session(&app, 3, WEB_HOST, false);
    app.store.set_web_session_connected(older, "", 5).unwrap();
    app.store.set_web_session_connected(used, "", 9).unwrap();
    app.take_web_events();
    let mut left: Vec<i64> = app
        .store
        .web_sessions()
        .unwrap()
        .iter()
        .map(|session| session.id)
        .collect();
    left.sort_unstable();
    assert_eq!(left, vec![used, other]);
    assert!(app.store.web_session(never_stored).unwrap().is_none());
}

/// Whether a session's server is shown depends on the host, not on what the code claimed.
#[test]
fn web_session_shows_a_foreign_host_whatever_the_code_said() {
    let (_dir, app) = fixture();
    stored_web_session(&app, 1, "web.example.org", false);
    stored_web_session(&app, 2, WEB_HOST, false);
    let sessions = app.web_sessions_json().unwrap();
    let self_hosted = |host: &str| {
        sessions["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|session| session["host"] == host)
            .unwrap()["selfHosted"]
            .clone()
    };
    assert_eq!(self_hosted("web.example.org"), true);
    assert_eq!(self_hosted(WEB_HOST), false);
}

/// Removing an allowed server takes its sessions and pin with it, tells a connected browser to
/// forget the session, and the next code naming the server asks again.
#[test]
fn web_allowed_server_can_be_removed() {
    let (_dir, mut app) = fixture();
    let host = "web.example.org";
    app.store.set_web_host(host, Some("pin")).unwrap();
    app.store.set_web_host("other.example.org", None).unwrap();
    let on_host = stored_web_session(&app, 1, host, true);
    let other = stored_web_session(&app, 2, WEB_HOST, true);
    let (mut rx, _) = app.web_test_session(on_host);
    let listed = app.web_sessions_json().unwrap();
    assert_eq!(listed["servers"], json!(["other.example.org", host]));

    let after = app.web_remove_server(host).unwrap();

    assert_eq!(after["servers"], json!(["other.example.org"]));
    assert!(matches!(rx.try_recv(), Ok(ToWeb::Stop(StopReason::Delete))));
    assert!(app.store.web_session(on_host).unwrap().is_none());
    assert!(app.store.web_session(other).unwrap().is_some());
    assert_eq!(app.store.web_host(host).unwrap(), None);
    assert_eq!(
        app.classify_qr(&web_code(3, host)).unwrap()["confirmHost"],
        true
    );
}

/// A changed certificate reported for this session's running task.
async fn web_certificate_changed(app: &mut App, session: i64, spki: &str) {
    let (_rx, task) = app.web_test_session(session);
    app.on_web_event((
        task,
        FromWeb::CertificateChanged {
            session,
            spki: spki.into(),
        },
    ))
    .await;
    app.on_web_event((
        task,
        FromWeb::Ended {
            session,
            delete: false,
            error: Some(WebFailure::CertificateChanged),
        },
    ))
    .await;
}

/// A freshly scanned session is deleted when it ends, before the user answers: the new pin must
/// still reach the store, or every further scan runs into the same dead end.
#[tokio::test(flavor = "current_thread")]
async fn web_new_certificate_of_a_scanned_session_is_pinned() {
    let (_dir, mut app) = fixture();
    let host = "web.example.org";
    app.store.set_web_host(host, Some("old")).unwrap();
    let id = stored_web_session(&app, 1, host, false);
    web_certificate_changed(&mut app, id, "new").await;
    assert!(app.store.web_session(id).unwrap().is_none());
    assert_eq!(
        app.web_sessions_json().unwrap()["certificates"],
        json!([{ "id": id, "host": host }])
    );

    let answer = app.web_accept_certificate(id).unwrap();

    assert_eq!(answer["scanAgain"], true);
    assert_eq!(app.store.web_host(host).unwrap(), Some(Some("new".into())));
    assert_eq!(app.web_sessions_json().unwrap()["certificates"], json!([]));
    assert!(matches!(
        app.web_accept_certificate(id),
        Err(ApiError::WebSessionNotFound)
    ));
}

/// A session the browser stored connects again with the new pin, and the answer also goes for
/// the same server's other sessions.
#[tokio::test(flavor = "current_thread")]
async fn web_new_certificate_of_a_stored_session_connects_again() {
    let (_dir, mut app) = fixture();
    let host = "web.example.org";
    app.store.set_web_host(host, Some("old")).unwrap();
    let id = stored_web_session(&app, 1, host, true);
    let other = stored_web_session(&app, 2, host, true);
    web_certificate_changed(&mut app, id, "new").await;
    web_certificate_changed(&mut app, other, "new").await;
    // Nothing here waits, so the session's task never gets to connect.
    let answer = tokio::task::LocalSet::new()
        .run_until(async { app.web_accept_certificate(id) })
        .await
        .unwrap();
    assert_eq!(answer, json!({}));
    assert_eq!(app.store.web_host(host).unwrap(), Some(Some("new".into())));
    assert!(app.web.running.contains_key(&id));
    assert!(app.web.changed.is_empty());
}

/// Declining keeps the pin and leaves nothing waiting, and a question doesn't outlive the
/// session's next attempt or reach a later session with the same ID.
#[tokio::test(flavor = "current_thread")]
async fn web_declined_certificate_is_forgotten() {
    let (_dir, mut app) = fixture();
    let host = "web.example.org";
    app.store.set_web_host(host, Some("old")).unwrap();
    let id = stored_web_session(&app, 1, host, true);
    web_certificate_changed(&mut app, id, "new").await;
    app.web_decline_certificate(id);
    assert_eq!(app.store.web_host(host).unwrap(), Some(Some("old".into())));
    assert!(app.web.changed.is_empty());

    web_certificate_changed(&mut app, id, "new").await;
    tokio::task::LocalSet::new()
        .run_until(async { app.web_start(id) })
        .await
        .unwrap();
    assert!(app.web.changed.is_empty());
}

/// The hash a wake-up names this browser by.
fn web_wake_up_hash(browser: u8) -> String {
    use sha2::{Digest as _, Sha256};
    HEXLOWER.encode(&Sha256::digest([browser; 32]))
}

/// Off, nothing connects: no code, no tap, no wake-up.
#[tokio::test(flavor = "current_thread")]
async fn web_switched_off_refuses_everything_that_connects() {
    let (_dir, mut app) = fixture();
    assert_eq!(app.web_sessions_json().unwrap()["enabled"], true);
    let id = stored_web_session(&app, 1, WEB_HOST, true);
    app.web_set_enabled(false).unwrap();
    assert_eq!(app.web_sessions_json().unwrap()["enabled"], false);

    let code = web_code(2, WEB_HOST);
    assert!(matches!(
        app.web_pair(&code, false),
        Err(ApiError::WebDisabled)
    ));
    assert_eq!(
        app.classify_qr(&code).unwrap()["error"],
        ApiError::WebDisabled.kind()
    );
    // A host isn't allowed on the way to being refused.
    assert!(matches!(
        app.web_pair(&web_code(2, "web.example.org"), true),
        Err(ApiError::WebDisabled)
    ));
    assert_eq!(app.store.web_host("web.example.org").unwrap(), None);
    assert!(matches!(app.web_start(id), Err(ApiError::WebDisabled)));
    app.web_resume(&web_wake_up_hash(1));
    assert!(app.web.running.is_empty(), "a wake-up connected nothing");
    assert_eq!(app.store.web_sessions().unwrap().len(), 1);

    // On again: the stored session wakes up.
    app.web_set_enabled(true).unwrap();
    tokio::task::LocalSet::new()
        .run_until(async {
            app.web_resume(&web_wake_up_hash(1));
        })
        .await;
    assert!(app.web.running.contains_key(&id));
}

/// Switching off ends running sessions at once: stored ones are told "disable", what the browser
/// never stored is gone and told "delete", all reported to the window, no more say for the tasks.
#[tokio::test(flavor = "current_thread")]
async fn web_switching_off_stops_running_sessions() {
    let (_dir, mut app) = fixture();
    let mut events = app.take_media_events().unwrap();
    let stored = stored_web_session(&app, 1, WEB_HOST, true);
    let (mut rx, task) = app.web_test_session(stored);
    let never_stored = stored_web_session(&app, 2, WEB_HOST, false);
    let (mut other_rx, _) = app.web_test_session(never_stored);

    let after = app.web_set_enabled(false).unwrap();

    assert_eq!(after["enabled"], false);
    assert!(matches!(
        rx.try_recv(),
        Ok(ToWeb::Stop(StopReason::Disable))
    ));
    assert!(matches!(
        other_rx.try_recv(),
        Ok(ToWeb::Stop(StopReason::Delete))
    ));
    assert!(app.web.running.is_empty());
    assert!(app.store.web_session(stored).unwrap().is_some());
    assert!(app.store.web_session(never_stored).unwrap().is_none());
    let mut stopped = Vec::new();
    while let Ok(CoreEvent::Call(event)) = events.try_recv() {
        assert_eq!(event["state"], "stopped");
        stopped.push(event["id"].as_i64().unwrap());
    }
    stopped.sort_unstable();
    assert_eq!(stopped, vec![stored, never_stored]);

    let message = arp::message("request", "profile", None, None);
    app.on_web_event((
        task,
        FromWeb::Message {
            session: stored,
            message,
        },
    ))
    .await;
    assert!(rx.try_recv().is_err(), "a stopped task's request isn't run");
}

/// The window takes down its notification on this event: removing a connected session has to
/// report it, as its task's own end no longer counts.
#[test]
fn web_removing_a_connected_session_reports_its_end() {
    let (_dir, mut app) = fixture();
    let mut events = app.take_media_events().unwrap();
    let id = stored_web_session(&app, 1, WEB_HOST, true);
    let _rx = app.web_test_session(id);
    app.web_delete(id).unwrap();
    let Ok(CoreEvent::Call(event)) = events.try_recv() else {
        panic!("no event");
    };
    assert_eq!(event["state"], "stopped");
    assert_eq!(event["id"], id);
}

/// A refused license leaves a stored pairing alone, but a session the browser never stored can't
/// survive it, so that browser hears "delete".
#[test]
fn web_refused_license_keeps_only_stored_pairings() {
    let (_dir, mut app) = fixture();
    let stored = stored_web_session(&app, 1, WEB_HOST, true);
    let (mut rx, _) = app.web_test_session(stored);
    let never_stored = stored_web_session(&app, 2, WEB_HOST, false);
    let (mut other_rx, _) = app.web_test_session(never_stored);
    app.web_stop_all(StopReason::Stop);
    assert!(matches!(rx.try_recv(), Ok(ToWeb::Stop(StopReason::Stop))));
    assert!(matches!(
        other_rx.try_recv(),
        Ok(ToWeb::Stop(StopReason::Delete))
    ));
    assert!(app.store.web_session(stored).unwrap().is_some());
    assert!(app.store.web_session(never_stored).unwrap().is_none());
}

/// "Show profile pictures" holds for the browser too.
#[tokio::test(flavor = "current_thread")]
async fn web_gets_no_contact_picture_while_they_are_not_shown() {
    let (_dir, mut app) = fixture();
    let (mut rx, task) = app.web_test_session(1);
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(8, 8))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    app.store
        .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
        .unwrap();
    app.media
        .write_picture(
            &crate::profile_pictures::contact_picture_name("BBBBBBBB"),
            &png,
        )
        .unwrap();
    let request = arp::message(
        "request",
        "avatar",
        Some(arp::map(vec![
            ("type", "contact".into()),
            ("id", "BBBBBBBB".into()),
        ])),
        None,
    );
    let picture_sent = async |app: &mut App, rx: &mut UnboundedReceiver<ToWeb>| {
        while rx.try_recv().is_ok() {}
        app.on_web_event((
            task,
            FromWeb::Message {
                session: 1,
                message: request.clone(),
            },
        ))
        .await;
        let mut sent = false;
        while let Ok(reply) = rx.try_recv() {
            if let ToWeb::Send(wire) = reply
                && crate::web::saltyrtc::field(&wire, "subType")
                    .is_some_and(|sub| sub.as_str() == Some("avatar"))
            {
                sent =
                    crate::web::saltyrtc::field(&wire, "data").is_some_and(|data| !data.is_nil());
            }
        }
        sent
    };
    assert!(picture_sent(&mut app, &mut rx).await);
    app.set_app_setting("showProfilePictures", false).unwrap();
    assert!(!picture_sent(&mut app, &mut rx).await);
}

/// One update of the browser's chat list: its mode and the chat's model.
fn conversation_updates(wires: &[Value]) -> Vec<(String, Value)> {
    wires
        .iter()
        .filter(|wire| saltyrtc::field(wire, "subType") == Some(&Value::from("conversation")))
        .map(|wire| {
            let args = saltyrtc::field(wire, "args").unwrap();
            (
                saltyrtc::field(args, "mode")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_owned(),
                saltyrtc::field(wire, "data").unwrap().clone(),
            )
        })
        .collect()
}

fn sent_values(rx: &mut UnboundedReceiver<ToWeb>) -> Vec<Value> {
    let mut sent = Vec::new();
    while let Ok(message) = rx.try_recv() {
        if let ToWeb::Send(wire) = message {
            sent.push(wire);
        }
    }
    sent
}

/// A muted chat tells the browser so (it would notify of every message otherwise), and changing
/// that on the phone reaches it.
#[test]
fn the_browser_hears_of_a_chats_notification_setting() {
    use crate::chat_notifications::Policy;
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    app.store
        .insert_incoming("BBBBBBBB", 1, "text", "hi", 1)
        .unwrap();
    let list = app.web_conversations(1).unwrap();
    let dnd = |chat: &Value| {
        saltyrtc::field(saltyrtc::field(chat, "notifications").unwrap(), "dnd")
            .unwrap()
            .clone()
    };
    let mode = |dnd: &Value| saltyrtc::field(dnd, "mode").cloned().unwrap();
    assert_eq!(mode(&dnd(&list[0])), Value::from("off"));

    let changed = |app: &mut App, rx: &mut UnboundedReceiver<ToWeb>, setting| {
        app.store
            .set_chat_notifications("BBBBBBBB", setting)
            .unwrap();
        app.web_refresh();
        let mut updates = conversation_updates(&sent_values(rx));
        assert_eq!(updates.len(), 1);
        let (mode, chat) = updates.remove(0);
        assert_eq!(mode, "modified");
        chat
    };
    let on = dnd(&changed(&mut app, &mut rx, Some((Policy::Never, None))));
    assert_eq!(mode(&on), Value::from("on"));
    assert_eq!(
        saltyrtc::field(&on, "mentionOnly"),
        Some(&Value::from(false))
    );

    let until = crate::messages::now_ms() + 60_000;
    let limited = dnd(&changed(
        &mut app,
        &mut rx,
        Some((Policy::Mentioned, Some(until))),
    ));
    assert_eq!(mode(&limited), Value::from("until"));
    assert_eq!(
        saltyrtc::field(&limited, "until"),
        Some(&Value::from(until))
    );
    assert_eq!(
        saltyrtc::field(&limited, "mentionOnly"),
        Some(&Value::from(true))
    );

    assert_eq!(
        mode(&dnd(&changed(&mut app, &mut rx, None))),
        Value::from("off")
    );
}

/// "Mark as unread" shows in the browser, and opening the chat there takes it away, as a chat
/// without unread messages has nothing for the browser to acknowledge.
#[tokio::test(flavor = "current_thread")]
async fn a_chat_opened_in_the_browser_is_no_longer_marked_unread() {
    let (_dir, mut app) = fixture();
    app.store.set_lock(None).unwrap();
    let mut media_events = app.take_media_events().unwrap();
    let mut rx = browser(&mut app);
    app.store
        .insert_incoming("BBBBBBBB", 1, "text", "hi", 1)
        .unwrap();
    app.web_conversations(1).unwrap();
    app.web_refresh();
    sent_values(&mut rx);

    app.store.set_marked_unread("BBBBBBBB", true).unwrap();
    app.web_refresh();
    let updates = conversation_updates(&sent_values(&mut rx));
    assert_eq!(
        saltyrtc::field(&updates[0].1, "isUnread"),
        Some(&Value::from(true))
    );

    let open = |id: &str| {
        arp::message(
            "update",
            "activeConversation",
            Some(arp::map(vec![
                ("type", "contact".into()),
                ("id", id.into()),
            ])),
            None,
        )
    };
    let message = open("BBBBBBBB");
    let incoming = Incoming::parse(&message).unwrap();
    app.web_action(&incoming).await.unwrap().unwrap();
    assert!(app.store.marked_unread_chats().unwrap().is_empty());
    assert!(media_events.try_recv().is_ok(), "our own list hears of it");
    app.web_refresh();
    let updates = conversation_updates(&sent_values(&mut rx));
    assert_eq!(
        saltyrtc::field(&updates[0].1, "isUnread"),
        Some(&Value::from(false))
    );

    // A private chat isn't the browser's to change.
    app.store.set_private("CCCCCCCC", true).unwrap();
    app.store.set_marked_unread("CCCCCCCC", true).unwrap();
    let message = open("CCCCCCCC");
    let incoming = Incoming::parse(&message).unwrap();
    app.web_action(&incoming).await.unwrap().unwrap();
    assert_eq!(app.store.marked_unread_chats().unwrap(), vec!["CCCCCCCC"]);
}

/// The chat server's words reach a browser that is open, which the phone in a pocket doesn't
/// show. What has no words of the server's stays with the phone.
#[test]
fn the_browser_hears_what_the_chat_server_says() {
    use crate::connection::OfflineReason;
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    app.web_subscribe_receivers(1);

    app.web_server_said(&OfflineReason::ServerAlert("Maintenance tonight".into()));
    app.web_server_said(&OfflineReason::Server("Your ID was revoked".into()));
    app.web_server_said(&OfflineReason::LoginRefused);
    let sent = sent_values(&mut rx);
    assert_eq!(sent.len(), 2);
    for (wire, kind, text) in [
        (&sent[0], "warning", "Maintenance tonight"),
        (&sent[1], "error", "Your ID was revoked"),
    ] {
        assert_eq!(saltyrtc::field(wire, "type"), Some(&Value::from("update")));
        assert_eq!(
            saltyrtc::field(wire, "subType"),
            Some(&Value::from("alert"))
        );
        let args = saltyrtc::field(wire, "args").unwrap();
        assert_eq!(saltyrtc::field(args, "type"), Some(&Value::from(kind)));
        assert_eq!(
            saltyrtc::field(args, "source"),
            Some(&Value::from("server"))
        );
        let data = saltyrtc::field(wire, "data").unwrap();
        assert_eq!(saltyrtc::field(data, "message"), Some(&Value::from(text)));
    }
}

/// A real picture, which what the browser gets is scaled from.
fn picture() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::new(64, 64))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    bytes
}

/// Our own picture shows in the browser: in the profile, and when asked for by our ID, which is
/// no contact's.
#[test]
fn the_browser_gets_our_own_picture() {
    use crate::profile_pictures::OWN_PICTURE;
    let (_dir, app) = fixture();
    let ask = |app: &App, kind: &str, id: &str| {
        let message = arp::message(
            "request",
            "avatar",
            Some(arp::map(vec![("type", kind.into()), ("id", id.into())])),
            None,
        );
        let incoming = Incoming::parse(&message).unwrap();
        app.web_avatar(&incoming).unwrap()
    };

    assert!(saltyrtc::field(&app.web_profile().unwrap(), "avatar").is_none());
    assert!(saltyrtc::field(&ask(&app, "me", "AAAAAAAA"), "data").is_none_or(Value::is_nil));

    app.media.write_picture(OWN_PICTURE, &picture()).unwrap();
    assert!(
        saltyrtc::field(&app.web_profile().unwrap(), "avatar").is_some_and(Value::is_bin),
        "in the profile"
    );
    let reply = ask(&app, "me", "AAAAAAAA");
    assert!(saltyrtc::field(&reply, "data").is_some_and(Value::is_bin));
    let args = saltyrtc::field(&reply, "args").unwrap();
    assert_eq!(saltyrtc::field(args, "type"), Some(&Value::from("me")));
    assert_eq!(saltyrtc::field(args, "id"), Some(&Value::from("AAAAAAAA")));
}

/// The updates a browser got for receivers: (subtype, mode, ID).
fn receiver_updates(sent: &[Value]) -> Vec<(String, String, String)> {
    let text = |value: Option<&Value>| value.and_then(Value::as_str).unwrap_or("").to_owned();
    sent.iter()
        .filter(|wire| {
            ["receiver", "receivers", "avatar"]
                .iter()
                .any(|sub| saltyrtc::field(wire, "subType") == Some(&Value::from(*sub)))
        })
        .map(|wire| {
            let args = saltyrtc::field(wire, "args").unwrap();
            (
                text(saltyrtc::field(wire, "subType")),
                text(saltyrtc::field(args, "mode")),
                text(saltyrtc::field(args, "id")),
            )
        })
        .collect()
}

fn subscribed_browser(app: &mut App) -> UnboundedReceiver<ToWeb> {
    app.store.set_lock(None).unwrap();
    let mut rx = browser(app);
    app.web_subscribe_receivers(1);
    app.web_conversations(1).unwrap();
    app.web_refresh();
    sent_values(&mut rx);
    rx
}

/// A contact added on the phone, with a chat: the browser would drop a conversation of a receiver
/// it doesn't know.
#[test]
fn a_contact_added_on_the_phone_reaches_the_browser_before_its_chat() {
    let (_dir, mut app) = fixture();
    let mut rx = subscribed_browser(&mut app);
    app.store
        .upsert_contact(&crate::contacts::test_contact("DDDDDDDD"))
        .unwrap();
    app.store
        .insert_incoming("DDDDDDDD", 1, "text", "hi", 1)
        .unwrap();
    app.web_refresh();
    let sent = sent_values(&mut rx);
    assert_eq!(
        receiver_updates(&sent),
        [(
            "receiver".to_owned(),
            "new".to_owned(),
            "DDDDDDDD".to_owned()
        )]
    );
    assert_eq!(
        saltyrtc::field(&sent[0], "subType"),
        Some(&Value::from("receiver"))
    );
    assert_eq!(conversation_updates(&sent)[0].0, "new");

    // Nothing new: nothing to say.
    app.store
        .insert_incoming("DDDDDDDD", 2, "text", "again", 2)
        .unwrap();
    app.web_refresh();
    assert!(receiver_updates(&sent_values(&mut rx)).is_empty());
}

/// A change on the phone reaches the browser as that one receiver: a whole list makes it forget
/// every picture it has loaded.
#[test]
fn a_changed_contact_goes_to_the_browser_alone_and_a_removed_one_goes() {
    let (_dir, mut app) = fixture();
    let mut rx = subscribed_browser(&mut app);
    let mut contact = crate::contacts::test_contact("BBBBBBBB");
    contact.first_name = Some("Renamed".into());
    app.store.upsert_contact(&contact).unwrap();
    app.web_refresh();
    let sent = sent_values(&mut rx);
    assert_eq!(
        receiver_updates(&sent),
        [(
            "receiver".to_owned(),
            "modified".to_owned(),
            "BBBBBBBB".to_owned()
        )]
    );
    assert!(format!("{:?}", sent[0]).contains("Renamed"));

    app.store.delete_contact("BBBBBBBB").unwrap();
    app.web_refresh();
    assert_eq!(
        receiver_updates(&sent_values(&mut rx)),
        [(
            "receiver".to_owned(),
            "removed".to_owned(),
            "BBBBBBBB".to_owned()
        )]
    );
}

/// Groups and lists made on the phone, and changes of a group, whichever way they come.
#[test]
fn groups_and_lists_made_on_the_phone_reach_the_browser() {
    let (_dir, mut app) = fixture();
    let mut rx = subscribed_browser(&mut app);
    let group = app.create_group("A GROUP", &["BBBBBBBB".into()]).unwrap();
    let key = group["key"].as_str().unwrap().to_owned();
    app.web_refresh();
    let sent = sent_values(&mut rx);
    assert_eq!(
        receiver_updates(&sent)[0],
        ("receiver".to_owned(), "new".to_owned(), key.clone())
    );
    let position = |sub: &str| {
        sent.iter()
            .position(|wire| saltyrtc::field(wire, "subType") == Some(&Value::from(sub)))
    };
    assert!(position("receiver") < position("conversation"), "{sent:?}");

    let list = crate::distribution_lists::DistributionList {
        key: "d:00000000000000ff".into(),
        name: "A LIST".into(),
        members: vec!["BBBBBBBB".into()],
        created_at: 1,
    };
    app.store.upsert_distribution_list(&list).unwrap();
    app.web_refresh();
    assert_eq!(
        receiver_updates(&sent_values(&mut rx)),
        [("receiver".to_owned(), "new".to_owned(), list.key.clone())]
    );
}

/// A chat going private is gone from the browser's list before its receiver goes, and a private
/// contact is marked hidden and loses its picture there.
#[test]
fn a_chat_going_private_leaves_the_browser_in_order() {
    use crate::profile_pictures::contact_picture_name;
    let (_dir, mut app) = fixture();
    let group = app.create_group("A GROUP", &["BBBBBBBB".into()]).unwrap();
    let key = group["key"].as_str().unwrap().to_owned();
    let mut rx = subscribed_browser(&mut app);
    app.store.set_private(&key, true).unwrap();
    app.web_refresh();
    let sent = sent_values(&mut rx);
    let position = |sub: &str| {
        sent.iter()
            .position(|wire| saltyrtc::field(wire, "subType") == Some(&Value::from(sub)))
            .unwrap()
    };
    assert!(position("conversation") < position("receiver"), "{sent:?}");
    assert!(!format!("{sent:?}").contains("A GROUP"));

    app.media
        .write_picture(&contact_picture_name("BBBBBBBB"), &picture())
        .unwrap();
    app.web_refresh();
    sent_values(&mut rx);
    app.store.set_private("BBBBBBBB", true).unwrap();
    app.web_refresh();
    let sent = sent_values(&mut rx);
    let model = sent
        .iter()
        .find(|wire| saltyrtc::field(wire, "subType") == Some(&Value::from("receiver")))
        .and_then(|wire| saltyrtc::field(wire, "data"))
        .unwrap();
    assert_eq!(saltyrtc::field(model, "visible"), Some(&Value::from(false)));
    let avatar = sent
        .iter()
        .find(|wire| saltyrtc::field(wire, "subType") == Some(&Value::from("avatar")))
        .and_then(|wire| saltyrtc::field(wire, "data"))
        .unwrap();
    assert!(avatar.is_nil());
}

/// An import changes more than a browser should get one by one.
#[test]
fn many_changes_at_once_reach_the_browser_as_the_whole_list() {
    let (_dir, mut app) = fixture();
    let mut rx = subscribed_browser(&mut app);
    for n in 0..=crate::app_web_data::RECEIVER_UPDATES_MAX {
        let id = format!("E{n:07}");
        app.store
            .upsert_contact(&crate::contacts::test_contact(&id))
            .unwrap();
    }
    app.web_refresh();
    let updates = receiver_updates(&sent_values(&mut rx));
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].0, "receivers");
    app.web_refresh();
    assert!(sent_values(&mut rx).is_empty(), "and that was all of it");
}

/// A picture that arrives or goes reaches the browser small, without its receiver: the browser
/// keeps the other pictures, and the picture isn't in the database, whose change count would tell.
#[test]
fn a_new_picture_reaches_the_browser_on_its_own() {
    use crate::profile_pictures::contact_picture_name;
    let (_dir, mut app) = fixture();
    let mut rx = subscribed_browser(&mut app);
    let name = contact_picture_name("BBBBBBBB");
    let avatar = |rx: &mut UnboundedReceiver<ToWeb>, app: &mut App| {
        app.web_refresh();
        let sent = sent_values(rx);
        assert_eq!(
            receiver_updates(&sent),
            [("avatar".to_owned(), String::new(), "BBBBBBBB".to_owned())]
        );
        saltyrtc::field(&sent[0], "data").unwrap().clone()
    };

    app.media.write_picture(&name, &picture()).unwrap();
    assert!(avatar(&mut rx, &mut app).is_bin());
    app.media.remove_picture(&name);
    assert!(avatar(&mut rx, &mut app).is_nil());

    // With pictures of contacts not to be shown, none goes out.
    app.set_app_setting("showProfilePictures", false).unwrap();
    app.media.write_picture(&name, &picture()).unwrap();
    app.web_refresh();
    assert!(receiver_updates(&sent_values(&mut rx)).is_empty());
}

/// Our nickname and picture changed from anywhere reach the browser: a nickname on its own leaves
/// the picture alone there, and no nickname is our ID.
#[test]
fn the_browser_hears_of_our_profile_changing() {
    use crate::{profile_pictures::OWN_PICTURE, store::keys};
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    app.web_subscribe_profile(1);
    app.web_refresh();
    sent_values(&mut rx);
    let update = |app: &mut App, rx: &mut UnboundedReceiver<ToWeb>| {
        app.web_refresh();
        let mut sent = sent_values(rx);
        assert_eq!(sent.len(), 1);
        let wire = sent.remove(0);
        assert_eq!(
            saltyrtc::field(&wire, "subType"),
            Some(&Value::from("profile"))
        );
        saltyrtc::field(&wire, "data").unwrap().clone()
    };
    let nickname = |data: &Value| saltyrtc::field(data, "publicNickname").cloned().unwrap();

    app.store.set_setting(keys::NICKNAME, "Someone").unwrap();
    let data = update(&mut app, &mut rx);
    assert_eq!(nickname(&data), Value::from("Someone"));
    assert!(saltyrtc::field(&data, "avatar").is_none());

    app.media.write_picture(OWN_PICTURE, &picture()).unwrap();
    let data = update(&mut app, &mut rx);
    assert_eq!(nickname(&data), Value::from("Someone"));
    assert!(saltyrtc::field(&data, "avatar").is_some_and(Value::is_bin));

    app.media.remove_picture(OWN_PICTURE);
    let data = update(&mut app, &mut rx);
    assert!(saltyrtc::field(&data, "avatar").is_some_and(Value::is_nil));

    app.store.delete_setting(keys::NICKNAME).unwrap();
    let data = update(&mut app, &mut rx);
    assert_eq!(nickname(&data), Value::from("AAAAAAAA"));
    assert!(saltyrtc::field(&data, "avatar").is_none());

    app.web_refresh();
    assert!(sent_values(&mut rx).is_empty(), "nothing more to tell");
}

/// The removals a browser got: chat, message IDs.
fn removed_messages(sent: &[Value]) -> Vec<(String, Vec<String>)> {
    sent.iter()
        .filter(|wire| {
            saltyrtc::field(wire, "subType") == Some(&Value::from("messages"))
                && saltyrtc::field(saltyrtc::field(wire, "args").unwrap(), "mode")
                    == Some(&Value::from("removed"))
        })
        .map(|wire| {
            let args = saltyrtc::field(wire, "args").unwrap();
            let ids = saltyrtc::field(wire, "data").unwrap().as_array().unwrap();
            (
                saltyrtc::field(args, "id")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_owned(),
                ids.iter()
                    .map(|m| {
                        saltyrtc::field(m, "id")
                            .and_then(Value::as_str)
                            .unwrap()
                            .to_owned()
                    })
                    .collect(),
            )
        })
        .collect()
}

/// Messages going from the phone go from the browser's open chat too, however they go, and only
/// those it has: the first page leaves older ones out.
#[test]
fn messages_removed_on_the_phone_leave_the_browser() {
    let (_dir, mut app) = fixture();
    app.store.set_lock(None).unwrap();
    let mut rx = browser(&mut app);
    let incoming = |app: &App, n: u64| {
        app.store
            .insert_incoming("BBBBBBBB", n, "text", "hi", n.cast_signed())
            .unwrap()
            .unwrap()
            .id
    };
    let older = incoming(&app, 1);
    for n in 2..=60 {
        incoming(&app, n);
    }
    let shown = incoming(&app, 61);
    let also_shown = incoming(&app, 62);
    app.web_messages(1, "BBBBBBBB", None).unwrap();
    app.web_refresh();
    sent_values(&mut rx);

    app.store.remove_message(older).unwrap();
    app.web_refresh();
    assert!(
        removed_messages(&sent_values(&mut rx)).is_empty(),
        "not shown there"
    );

    app.store.remove_message(shown).unwrap();
    app.web_refresh();
    assert_eq!(
        removed_messages(&sent_values(&mut rx)),
        [("BBBBBBBB".to_owned(), vec![shown.to_string()])]
    );

    // Not said twice, and a message that came later counts too.
    let later = incoming(&app, 63);
    app.web_refresh();
    sent_values(&mut rx);
    assert!(later > also_shown);
    app.store
        .delete_messages_before(i64::MAX, false, &[])
        .unwrap();
    app.web_refresh();
    let removed = removed_messages(&sent_values(&mut rx));
    assert_eq!(removed.len(), 1);
    assert!(removed[0].1.contains(&later.to_string()));
    assert!(removed[0].1.contains(&also_shown.to_string()));
    assert!(!removed[0].1.contains(&shown.to_string()));
}

/// Emptying a chat, and the next message there: it may get the ID of the newest one removed, as
/// the database gives that out again.
#[test]
fn a_chat_emptied_on_the_phone_empties_in_the_browser_and_goes_on() {
    let (_dir, mut app) = fixture();
    app.store.set_lock(None).unwrap();
    let mut rx = browser(&mut app);
    let first = app
        .store
        .insert_incoming("BBBBBBBB", 1, "text", "first", 1)
        .unwrap()
        .unwrap();
    app.web_messages(1, "BBBBBBBB", None).unwrap();
    app.web_refresh();
    sent_values(&mut rx);

    app.store.empty_conversation("BBBBBBBB").unwrap();
    app.web_refresh();
    assert_eq!(
        removed_messages(&sent_values(&mut rx)),
        [("BBBBBBBB".to_owned(), vec![first.id.to_string()])]
    );

    let next = app
        .store
        .insert_incoming("BBBBBBBB", 2, "text", "next", 2)
        .unwrap()
        .unwrap();
    assert_eq!(next.id, first.id, "what this is about");
    app.web_refresh();
    let sent = sent_values(&mut rx);
    assert!(format!("{sent:?}").contains("next"), "{sent:?}");
}

/// The browser deleting a message itself hears of it once.
#[tokio::test(flavor = "current_thread")]
async fn a_message_deleted_in_the_browser_is_removed_there_once() {
    let (_dir, mut app) = fixture();
    app.store.set_lock(None).unwrap();
    let mut rx = browser(&mut app);
    let message = app
        .store
        .insert_incoming("BBBBBBBB", 1, "text", "hi", 1)
        .unwrap()
        .unwrap();
    app.web_messages(1, "BBBBBBBB", None).unwrap();
    app.web_refresh();
    sent_values(&mut rx);
    let request = arp::message(
        "delete",
        "message",
        Some(arp::map(vec![
            ("type", "contact".into()),
            ("id", "BBBBBBBB".into()),
            ("messageId", message.id.to_string().into()),
        ])),
        None,
    );
    let incoming = Incoming::parse(&request).unwrap();
    let reply = app.web_action(&incoming).await.unwrap().unwrap();
    assert_eq!(
        saltyrtc::field(&reply, "subType"),
        Some(&Value::from("confirm"))
    );
    app.web_refresh();
    assert_eq!(
        removed_messages(&sent_values(&mut rx)),
        [("BBBBBBBB".to_owned(), vec![message.id.to_string()])]
    );
}

/// A private chat's removals are none of the browser's business.
#[test]
fn nothing_of_a_private_chats_removals_reaches_the_browser() {
    let (_dir, mut app) = fixture();
    app.store.set_lock(None).unwrap();
    let mut rx = browser(&mut app);
    let message = app
        .store
        .insert_incoming("BBBBBBBB", 1, "text", "hi", 1)
        .unwrap()
        .unwrap();
    app.web_messages(1, "BBBBBBBB", None).unwrap();
    app.store.set_private("BBBBBBBB", true).unwrap();
    app.store.remove_message(message.id).unwrap();
    app.web_refresh();
    assert!(removed_messages(&sent_values(&mut rx)).is_empty());
}
