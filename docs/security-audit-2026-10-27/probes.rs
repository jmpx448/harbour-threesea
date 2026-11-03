// SPDX-License-Identifier: AGPL-3.0-only
// Vulnerability demonstrations: PASS means the reported vulnerable behavior was reproduced.
// The separate parser smoke test only checks for panics on a small malformed-input corpus.
// Loaded as a child module of app_web in a disposable copy by run-probes.sh.
use super::*;
use crate::{app::StartConfig, media::MediaPart, store::StoredIdentity};

fn fixture() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(
        &StartConfig {
            data_dir: dir.path().to_owned(),
            app_version: "audit".into(),
            language: "en".into(),
            country: "CH".into(),
            model: "audit".into(),
            os_version: "test".into(),
            pictures_dir: None,
            calls: false,
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

fn browser(app: &mut App) -> UnboundedReceiver<ToWeb> {
    let (tx, rx) = unbounded_channel();
    app.web.running.insert(1, (tx, "connected"));
    rx
}

#[test]
fn audit_invalid_frame_poisoning_and_replay() {
    use crate::{
        group_call_frames as frames, group_call_frames::Codec, group_call_handshake::MediaKey,
    };
    frames::reset([1; 32]);
    let key = MediaKey::random(0);
    frames::set_local(&key);
    frames::add_peer_keys(7, &[key]);
    let valid = frames::encrypt(Codec::Opus, b"authenticated audio").unwrap();
    assert_eq!(
        frames::decrypt(7, Codec::Opus, &valid).unwrap(),
        b"authenticated audio"
    );
    assert!(
        frames::decrypt(7, Codec::Opus, &valid).is_some(),
        "same frame accepted twice"
    );
    let mut forged = valid.clone();
    let ratchet_index = forged.len() - 5;
    forged[ratchet_index] = 255; // No valid tag for this footer.
    assert!(frames::decrypt(7, Codec::Opus, &forged).is_none());
    assert!(
        frames::decrypt(7, Codec::Opus, &valid).is_none(),
        "rejected frame destroyed valid key state"
    );
    frames::reset([0; 32]);
}

#[test]
fn audit_web_subscription_discloses_edit_after_chat_becomes_private() {
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    let original = app
        .store
        .insert_incoming("BBBBBBBB", 1, "text", "old text", 1)
        .unwrap()
        .unwrap();
    app.web_messages(1, "BBBBBBBB", None).unwrap();
    app.store.set_private("BBBBBBBB", true).unwrap();
    app.web_refresh(); // Production runs this after setPrivate; it does not revoke the subscription.
    while rx.try_recv().is_ok() {}
    assert!(app.web_receiver_key(Some("BBBBBBBB")).is_err());
    let changed = app
        .store
        .edit_message(original.id, "NEW PRIVATE CONTENT", Some(2))
        .unwrap()
        .unwrap();
    app.web_on_core_event(&CoreEvent::MessagesChanged(vec![changed]));
    let ToWeb::Send(wire) = rx.try_recv().unwrap() else {
        panic!("no browser message")
    };
    assert!(wire.to_string().contains("NEW PRIVATE CONTENT"));
}

#[test]
fn audit_pending_web_download_discloses_blob_after_chat_becomes_private() {
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    let message = app.store.insert_incoming("BBBBBBBB", 2, "file",
        &json!({"fileState": 1, "mediaType": "application/octet-stream", "name": "private.bin"}).to_string(), 1)
        .unwrap().unwrap();
    // Model the state created by an authorized request/blob while its download is pending.
    assert!(app.web_receiver_key(Some("BBBBBBBB")).is_ok());
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
        panic!("no browser message")
    };
    let data = saltyrtc::field(&wire, "data").unwrap();
    assert_eq!(
        saltyrtc::field(data, "blob").unwrap().as_slice(),
        Some(&b"NEW PRIVATE FILE"[..])
    );
}

#[tokio::test(flavor = "current_thread")]
async fn audit_browser_can_add_member_to_private_group() {
    let (_dir, mut app) = fixture();
    let group = app
        .create_group("private group", &["BBBBBBBB".into()])
        .unwrap();
    let key = group["key"].as_str().unwrap();
    app.store.set_private(key, true).unwrap();
    assert!(app.web_receiver_key(Some(key)).is_err());
    let wire = arp::message(
        "update",
        "group",
        Some(arp::map(vec![("id", key.into())])),
        Some(arp::map(vec![(
            "members",
            Value::Array(vec!["BBBBBBBB".into(), "CCCCCCCC".into()]),
        )])),
    );
    let incoming = Incoming::parse(&wire).unwrap();
    assert!(app.web_manage(&incoming).await.unwrap().is_ok());
    assert!(
        app.store
            .group(key)
            .unwrap()
            .unwrap()
            .members
            .contains(&"CCCCCCCC".to_owned())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn audit_private_core_commands_bypass_read_gate() {
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
    let read: serde_json::Value = serde_json::from_str(
        &app.handle(&json!({"id": 1, "cmd": "message", "message": message.id}).to_string())
            .await,
    )
    .unwrap();
    assert_eq!(read["error"]["kind"], "locked");
    let star = app
        .handle(
            &json!({"id": 2, "cmd": "setStarred", "message": message.id, "starred": true})
                .to_string(),
        )
        .await;
    assert!(star.contains("PRIVATE BODY"));
    let draft = app
        .handle(r#"{"id":3,"cmd":"draft","identity":"BBBBBBBB"}"#)
        .await;
    assert!(draft.contains("PRIVATE DRAFT"));
    let forwarded: serde_json::Value = serde_json::from_str(&app.handle(&json!({"id": 4, "cmd": "forwardMessage", "message": message.id, "identity": "CCCCCCCC"}).to_string()).await).unwrap();
    assert_eq!(forwarded["ok"], true);
    assert_eq!(
        app.store.conversation_messages("CCCCCCCC", 10).unwrap()[0].body,
        "PRIVATE BODY"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn audit_browser_survives_identity_deletion() {
    let (_dir, mut app) = fixture();
    let mut rx = browser(&mut app);
    app.delete_all_data().unwrap();
    assert!(
        app.web.running.contains_key(&1),
        "old authenticated browser remains connected"
    );
    assert!(rx.try_recv().is_err(), "no Stop was sent");
    // Simulate the persistence step of restoring/creating an identity in the same process.
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
        .insert_incoming("BBBBBBBB", 10, "text", "NEW ID PRIVATE DATA", 10)
        .unwrap();
    let message = arp::message(
        "request",
        "messages",
        Some(arp::map(vec![("id", "BBBBBBBB".into())])),
        None,
    );
    app.on_web_event(FromWeb::Message {
        session: 1,
        message,
    })
    .await;
    let ToWeb::Send(wire) = rx.try_recv().unwrap() else {
        panic!("no reply")
    };
    assert!(wire.to_string().contains("NEW ID PRIVATE DATA"));
}

#[tokio::test(flavor = "current_thread")]
async fn audit_queued_browser_command_runs_after_session_deletion() {
    let (_dir, mut app) = fixture();
    let _rx = browser(&mut app);
    app.web_delete(1).unwrap();
    let message = arp::message(
        "create",
        "textMessage",
        Some(arp::map(vec![("id", "BBBBBBBB".into())])),
        Some(arp::map(vec![("text", "AFTER REVOCATION".into())])),
    );
    // A request already queued by the old session task, delivered after webDelete.
    app.on_web_event(FromWeb::Message {
        session: 1,
        message,
    })
    .await;
    assert_eq!(
        app.store.conversation_messages("BBBBBBBB", 10).unwrap()[0].body,
        "AFTER REVOCATION"
    );
}

#[test]
fn audit_parser_smoke_no_panic() {
    // A small deterministic malformed-input corpus, not coverage-guided fuzzing.
    use libthreema::model::message::IncomingMessageBody;
    let sender = "BBBBBBBB".parse().unwrap();
    let mut seed = 0x12345678u32;
    for length in [0, 1, 2, 7, 8, 9, 16, 24, 32, 64, 255, 1024, 4096] {
        for _ in 0..8 {
            let bytes: Vec<u8> = (0..length)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    seed as u8
                })
                .collect();
            for kind in [
                0x01, 0x10, 0x15, 0x16, 0x17, 0x18, 0x19, 0x41, 0x42, 0x4a, 0x4b, 0x50, 0x52, 0x60,
                0x61, 0x62, 0x80, 0x82, 0x91, 0x92, 0xa0,
            ] {
                let _ = IncomingMessageBody::decode_reflected(sender, kind, &bytes);
            }
            let _ = crate::voice::decode(&bytes);
            let text = String::from_utf8_lossy(&bytes);
            let _ = crate::links::parse(&text);
            let _ = crate::rendezvous::parse_join_code(&text);
            let _ = crate::web::qr::parse(&text);
        }
    }
}
