// SPDX-License-Identifier: AGPL-3.0-only
// Offline review probes. Defect-test PASS means vulnerable behavior is reproduced.
use super::*;
use crate::{app::StartConfig, store::StoredIdentity};

fn fixture() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(
        &StartConfig {
            data_dir: dir.path().into(),
            app_version: "audit".into(),
            language: "en".into(),
            country: "CH".into(),
            model: "audit".into(),
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
    app.store
        .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
        .unwrap();
    (dir, app)
}

fn stored_session(app: &App) -> i64 {
    app.store
        .add_web_session(&WebSession {
            id: 0,
            initiator_key: [1; 32],
            private_key: Zeroizing::new([2; 32]),
            server_key: None,
            host: "saltyrtc.threema.ch".into(),
            port: 443,
            self_hosted: false,
            persistent: false,
            client: "audit".into(),
            created_at: 1,
            last_connection: None,
        })
        .unwrap()
}

fn running(app: &mut App, id: i64) -> UnboundedReceiver<ToWeb> {
    let (tx, rx) = unbounded_channel();
    app.web.running.insert(id, (tx, "connected"));
    rx
}

#[tokio::test(flavor = "current_thread")]
async fn audit_reused_session_id_accepts_revoked_commands() {
    let (_dir, mut app) = fixture();
    let old = stored_session(&app);
    let _old_rx = running(&mut app, old);
    app.web_delete(old).unwrap();
    let next = stored_session(&app);
    assert_eq!(old, next, "SQLite reuses the row ID");
    let _new_rx = running(&mut app, next);
    let message = arp::message(
        "create",
        "textMessage",
        Some(arp::map(vec![("id", "BBBBBBBB".into())])),
        Some(arp::map(vec![("text", "REVOKED SESSION COMMAND".into())])),
    );
    app.on_web_event(FromWeb::Message {
        session: old,
        message,
    })
    .await;
    assert_eq!(
        app.store.conversation_messages("BBBBBBBB", 1).unwrap()[0].body,
        "REVOKED SESSION COMMAND"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn audit_stale_end_deletes_replacement_session() {
    let (_dir, mut app) = fixture();
    let old = stored_session(&app);
    let _old_rx = running(&mut app, old);
    app.web_delete(old).unwrap();
    let next = stored_session(&app);
    assert_eq!(old, next);
    let _new_rx = running(&mut app, next);
    app.store.set_web_session_persistent(next).unwrap();
    app.on_web_event(FromWeb::Ended {
        session: old,
        delete: true,
        error: None,
    })
    .await;
    assert!(!app.web.running.contains_key(&next));
    assert!(app.store.web_session(next).unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn audit_private_vote_and_reaction_return_content() {
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
    app.store.set_private("BBBBBBBB", true).unwrap();
    assert!(app.check_readable("BBBBBBBB").is_err());
    let read = app
        .handle(&json!({"id":1,"cmd":"message","message":text.id}).to_string())
        .await;
    assert!(read.contains("locked"));
    let vote = app
        .handle(&json!({"id":2,"cmd":"vote","message":poll.id,"choices":[0]}).to_string())
        .await;
    assert!(vote.contains("PRIVATE POLL QUESTION"), "{vote}");
    let react = app.handle(&json!({"id":3,"cmd":"react","identity":"BBBBBBBB","messageId":"000000000000007b","emoji":"👍","apply":true}).to_string()).await;
    assert!(react.contains("PRIVATE REACTION BODY"), "{react}");
}

#[tokio::test(flavor = "current_thread")]
async fn audit_peer_controlled_log_fields() {
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
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || Writer(output.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (_dir, mut app) = fixture();
    let _rx = running(&mut app, 1);
    let message = arp::message("PEER_SECRET_SENTINEL", "ARBITRARY_PEER_TEXT", None, None);
    app.on_web_event(FromWeb::Message {
        session: 1,
        message,
    })
    .await;
    let logged = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(
        logged.contains("PEER_SECRET_SENTINEL") && logged.contains("ARBITRARY_PEER_TEXT"),
        "{logged}"
    );
}

#[test]
fn audit_msgpack_decoded_expansion() {
    let count = 65_536u32;
    let mut bytes = vec![0xdd]; // array32, followed by nil elements.
    bytes.extend_from_slice(&count.to_be_bytes());
    bytes.resize(5 + count as usize, 0xc0);
    let value = rmpv::decode::read_value(&mut bytes.as_slice()).unwrap();
    let values = value.as_array().unwrap();
    assert_eq!(values.len(), count as usize);
    let allocation = values.capacity() * std::mem::size_of::<Value>();
    println!(
        "AUDIT_MSGPACK wire_bytes={} value_size={} array_allocation_bytes={allocation}",
        bytes.len(),
        std::mem::size_of::<Value>()
    );
    assert!(allocation > bytes.len() * 20);
}

#[test]
fn audit_measure_history_scaling() {
    use std::time::Instant;
    fn median(mut f: impl FnMut()) -> f64 {
        f();
        let mut timings = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            f();
            timings.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        timings.sort_by(f64::total_cmp);
        timings[2]
    }
    for (chats, per_chat) in [(10, 100), (100, 100), (100, 1000), (1000, 100)] {
        let (_dir, mut app) = fixture();
        app.store.conn.execute_batch("BEGIN").unwrap();
        for c in 0..chats {
            let who = format!("C{c:07}");
            app.store
                .upsert_contact(&crate::contacts::test_contact(&who))
                .unwrap();
            for n in 0..per_chat {
                app.store
                    .insert_incoming(
                        &who,
                        (n + 1) as u64,
                        "text",
                        "ordinary synthetic message",
                        n,
                    )
                    .unwrap();
            }
        }
        app.store.conn.execute_batch("COMMIT").unwrap();
        let list = median(|| {
            std::hint::black_box(app.conversations_json(false, false).unwrap());
        });
        let sql = median(|| {
            std::hint::black_box(app.store.conversations().unwrap());
        });
        let search = median(|| {
            std::hint::black_box(
                app.store
                    .search_messages("no-match-sentinel", None, 100, |_| None)
                    .unwrap(),
            );
        });
        let mut rx = running(&mut app, 1);
        app.web_conversations(1).unwrap();
        let refresh = median(|| {
            app.web_refresh();
            while rx.try_recv().is_ok() {}
        });
        println!(
            "AUDIT_PERF chats={chats} messages={} list_ms={list:.3} list_sql_ms={sql:.3} absent_search_ms={search:.3} unchanged_web_refresh_ms={refresh:.3}",
            chats * per_chat
        );
        if chats == 1000 {
            let mut plan = app.store.conn.prepare("EXPLAIN QUERY PLAN SELECT id FROM messages m WHERE m.id IN (SELECT max(id) FROM messages GROUP BY conversation)").unwrap();
            for row in plan.query_map([], |r| r.get::<_, String>(3)).unwrap() {
                println!("AUDIT_PLAN {}", row.unwrap());
            }
        }
    }
}
