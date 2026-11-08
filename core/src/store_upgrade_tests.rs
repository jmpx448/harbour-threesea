// SPDX-License-Identifier: AGPL-3.0-only
//! Stores from older versions, with data in them, brought up to date by [`Store::open`]: from every
//! schema version the migration list has passed through, and from the schema the released 0.2.0
//! wrote (`core/tests/fixtures/store-schema-0.2.0.sql`). The made-up rows go in as soon as their
//! tables exist, so every migration that rewrites rows runs on them.
//!
//! A release whose schema differs from the last one adds its fixture (see docs/release.md).

use libthreema::{
    common::{GroupIdentity, ThreemaId},
    csp_e2e::fs::FsSessionStore as _,
};
use rusqlite::{Connection, params};

use super::*;

const USER: &str = "USER0001";
const PEER: &str = "ABCD1234";
const CREATOR: &str = "CREATOR1";
const KEY: [u8; 32] = [1; 32];

/// The schemas releases wrote: the version, its migration count, its schema.
const RELEASED: &[(&str, usize, &str)] = &[
    (
        "0.2.0",
        33,
        include_str!("../tests/fixtures/store-schema-0.2.0.sql"),
    ),
    (
        "0.3.0",
        46,
        include_str!("../tests/fixtures/store-schema-0.3.0.sql"),
    ),
];

fn group(id: u64) -> String {
    crate::groups::key(&GroupIdentity {
        creator_identity: ThreemaId::try_from(CREATOR).unwrap(),
        group_id: id,
    })
}

fn bytes(byte: u8, length: usize) -> Vec<u8> {
    vec![byte; length]
}

/// A message with the columns every schema version has.
fn message(
    conn: &Connection,
    id: i64,
    conversation: &str,
    outgoing: i64,
    kind: &str,
    body: &str,
    state: i64,
) {
    conn.execute(
        "INSERT INTO messages (id, conversation, message_id, outgoing, kind, body, created_at,
            updated_at, state) VALUES (?1, ?2, ?1 + 100, ?3, ?4, ?5, ?1 * 1000, ?1 * 1000, ?6)",
        params![id, conversation, outgoing, kind, body, state],
    )
    .unwrap();
}

/// The made-up rows that go in once schema version `version` is there, written with that
/// version's columns. Message ids are fixed: the checks refer to them.
fn seed(conn: &Connection, version: usize) {
    let run = |sql: &str, values: &[&dyn rusqlite::ToSql]| {
        conn.execute(sql, values)
            .unwrap_or_else(|error| panic!("seed for version {version}: {error}"));
    };
    let (crew, old_crew) = (group(7), group(8));
    match version {
        1 => {
            run(
                "INSERT INTO identity VALUES (1, ?1, ?2, 'a', 1)",
                params![USER, bytes(7, 32)],
            );
            run(
                "INSERT INTO settings VALUES ('app_settings', '{\"blockedContacts\":[\"EVIL0001\"]}'),
                    ('nickname', 'Tester'), ('pictureRequested:ABCD1234', '1000')",
                params![],
            );
        }
        2 => {
            for (id, verification) in [(PEER, 2), (CREATOR, 0)] {
                run(
                    "INSERT INTO contacts VALUES (?1, ?2, 1, 'Anna', NULL, NULL, ?3, 0, 0, 0, 0, 255,
                        0, NULL, NULL, 0, 0)",
                    params![id, "11".repeat(32), verification],
                );
            }
        }
        3 => {
            for (id, outgoing, body, state) in [
                (1, 0, "hello there", 0),
                (2, 1, "hi back", 2),
                (3, 0, "are you there", 0),
            ] {
                message(conn, id, PEER, outgoing, "text", body, state);
            }
            run("INSERT INTO nonces VALUES (?1)", params![bytes(1, 24)]);
        }
        4 => run(
            "INSERT INTO fs_sessions VALUES (?1, ?2, NULL, ?3, NULL, NULL, 1000, NULL, NULL, NULL,
                NULL, ?4, 3, NULL, NULL)",
            params![PEER, bytes(5, 16), bytes(6, 32), bytes(8, 32)],
        ),
        5 => {
            run(
                "INSERT INTO reactions VALUES (1, ?1, '👍', 1500)",
                params![PEER],
            );
            run(
                "INSERT INTO outbox (conversation, message_id, created_at, kind, body)
                 VALUES (?1, 201, 1000, 'deliveryReceipt', '{}')",
                params![PEER],
            );
        }
        7 => {
            // A voice message heard (read) and one not yet.
            for (id, state) in [(4, 2), (5, 0)] {
                let voice = r#"{"mediaType":"audio/aac","rendering":"voice"}"#;
                message(conn, id, PEER, 0, "file", voice, state);
                run(
                    "INSERT INTO files VALUES (?1, ?2, ?3, NULL)",
                    params![id, bytes(9, 32), bytes(10, 16)],
                );
            }
        }
        8 => {
            for (key, id, left) in [(&crew, 7, 0), (&old_crew, 8, 1)] {
                // Without their creator, as group syncs left them before migration 35.
                run(
                    "INSERT INTO groups (key, creator, group_id, name, members, left_group,
                        created_at) VALUES (?1, ?2, ?3, 'Crew', ?4, ?5, 900)",
                    params![key, CREATOR, id, format!("[\"{PEER}\"]"), left],
                );
            }
            message(conn, 6, &crew, 0, "text", "group hello", 0);
            run(
                "UPDATE messages SET sender = ?1 WHERE id = 6",
                params![CREATOR],
            );
            // Deleted for everyone and still waiting, and a text on its way.
            message(conn, 7, &crew, 1, "deleted", "", 0);
            run(
                "INSERT INTO pending_receivers VALUES (?1, 107, ?2)",
                params![crew, PEER],
            );
            message(conn, 8, PEER, 1, "text", "on its way", 0);
        }
        13 => run("INSERT INTO pinned_chats VALUES (?1, 0)", params![PEER]),
        16 => run("INSERT INTO archived_chats VALUES (?1)", params![crew]),
        19 => run("INSERT INTO private_chats VALUES (?1)", params![PEER]),
        22 => run("INSERT INTO d2d_nonces VALUES (?1)", params![bytes(2, 24)]),
        25 => run(
            "INSERT INTO web_sessions (initiator_key, private_key, host, port, persistent,
                created_at) VALUES (?1, ?2, 'saltyrtc.example.org', 443, 1, 1000)",
            params![bytes(11, 32), bytes(12, 32)],
        ),
        29 => {
            // Imported history, older than everything live, by its position alone.
            run(
                "INSERT INTO messages (id, conversation, message_id, outgoing, kind, body,
                    created_at, updated_at, state, sort_key)
                 VALUES (20, ?1, 120, 0, 'text', 'from the old phone', 500, 500, 2, ?2)",
                params![PEER, (1_i64 << 20) - 1],
            );
        }
        _ => {}
    }
}

/// A store at schema version `version`, its rows put in as their tables came.
fn store_at(path: &Path, version: usize) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(&format!(
        "PRAGMA key = \"x'{}'\"; PRAGMA synchronous = OFF;",
        data_encoding::HEXLOWER.encode(&KEY)
    ))
    .unwrap();
    let mut store = Store {
        conn,
        held_notifications: std::cell::RefCell::default(),
        invalid_ids: std::cell::RefCell::default(),
        member_lookups: std::cell::Cell::default(),
    };
    for applied in 1..=version {
        store.migrate(&MIGRATIONS[..applied], &|| {}).unwrap();
        seed(&store.conn, applied);
    }
}

/// A store with a release's schema, and every row it can hold.
fn released_store(path: &Path, migrations: usize, schema: &str) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(&format!(
        "PRAGMA key = \"x'{}'\"; PRAGMA synchronous = OFF;",
        data_encoding::HEXLOWER.encode(&KEY)
    ))
    .unwrap();
    conn.execute_batch(schema).unwrap();
    conn.execute_batch(&format!("PRAGMA user_version = {migrations}"))
        .unwrap();
    // A fixture is only the schema; this row a migration puts in, and a release past it has it.
    if migrations >= 41 {
        conn.execute_batch("INSERT INTO receiver_generation (value) VALUES (0)")
            .unwrap();
    }
    for version in 1..=migrations {
        seed(&conn, version);
    }
}

/// Tables, indexes, triggers and views in the order they were made, without SQLite's own and
/// without the tables full text search keeps for itself.
fn schema(conn: &Connection) -> Vec<String> {
    let mut statement = conn
        .prepare(
            "SELECT sql FROM sqlite_master m
             WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'
               AND NOT EXISTS (SELECT 1 FROM sqlite_master v
                               WHERE v.sql LIKE 'CREATE VIRTUAL TABLE%'
                                 AND m.type = 'table' AND m.name LIKE v.name || '\\_%' ESCAPE '\\')
             ORDER BY rowid",
        )
        .unwrap();
    let rows = statement.query_map([], |row| row.get(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

fn fresh_schema() -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    schema(&Store::open(&dir.path().join("t.db"), &KEY).unwrap().conn)
}

fn ids(messages: &[crate::messages::Message]) -> Vec<i64> {
    messages.iter().map(|m| m.id).collect()
}

/// The upgraded store keeps what was in it, in its order, with what the migrations added.
/// `seeded`: the last version whose rows went in; `walked`: whether each went in before the
/// migrations after its version ran (else they came at the end, into the last schema).
fn check(path: &Path, seeded: usize, walked: bool, fresh: &[String]) {
    let store = std::rc::Rc::new(Store::open(path, &KEY).unwrap());
    let at = |version: usize| seeded >= version;
    let version: usize = store
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, MIGRATIONS.len());
    assert_eq!(schema(&store.conn), fresh, "the same schema as a new store");
    assert!(store.receiver_generation().is_ok());

    if at(1) {
        assert_eq!(store.identity().unwrap().unwrap().threema_id, USER);
        assert_eq!(
            store.setting(keys::NICKNAME).unwrap().as_deref(),
            Some("Tester")
        );
        // Moved into a table of its own by migration 27, which migration 38 dropped.
        let requested = store.settings_with_prefix("pictureRequested:").unwrap();
        assert_eq!(requested.is_empty(), walked);
    }
    if at(2) {
        let contact = store.contact(PEER).unwrap().unwrap();
        assert_eq!(contact.first_name.as_deref(), Some("Anna"));
        assert_eq!(
            contact.verification_level,
            libthreema::protobuf::d2d_sync::contact::VerificationLevel::FullyVerified
        );
    }
    if !at(3) {
        assert!(store.conversations().unwrap().is_empty());
        return;
    }
    assert!(store.has_nonce(&[1; 24]).unwrap());

    let mut expected = vec![1, 2, 3];
    if at(7) {
        expected.extend([4, 5]);
    }
    if at(8) {
        expected.push(8);
    }
    if at(29) {
        expected.insert(0, 20);
    }
    let messages = store.conversation_messages(PEER, 100).unwrap();
    assert_eq!(ids(&messages), expected, "in history order");
    for message in messages.iter().filter(|m| m.id != 20) {
        assert_eq!(message.sort_key, message.id << 20, "{}", message.id);
    }
    assert_eq!(store.message(1).unwrap().unwrap().body, "hello there");

    // The chat list's summary is what a count over the messages says.
    let unread = 2 + usize::from(at(7));
    let mut chats: Vec<(String, i64, i64)> = store
        .conversations()
        .unwrap()
        .into_iter()
        .filter(|(message, _)| message.id > 0)
        .map(|(message, unread)| (message.conversation, message.id, unread))
        .collect();
    chats.sort();
    let mut wanted = vec![(
        PEER.to_owned(),
        if at(8) {
            8
        } else if at(7) {
            5
        } else {
            3
        },
        i64::try_from(unread).unwrap(),
    )];
    if at(8) {
        wanted.push((group(7), 7, 1));
    }
    wanted.sort();
    assert_eq!(chats, wanted);

    let found = store
        .search_messages("hello there", None, 10, |_| true, |_| None)
        .unwrap();
    assert_eq!(
        ids(&found.into_iter().map(|(m, _)| m).collect::<Vec<_>>()),
        [1]
    );

    if at(4) {
        let sessions: i64 = store
            .conn
            .query_row("SELECT count(*) FROM fs_sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sessions, 1);
        let fs = crate::fs_store::StoreFs {
            store: std::rc::Rc::clone(&store),
            my_identity: ThreemaId::try_from(USER).unwrap(),
            events: crate::providers::EventSink::default(),
        };
        let session = fs.get_best(ThreemaId::try_from(PEER).unwrap()).unwrap();
        assert_eq!(session.unwrap().id.0, [5; 16]);
    }
    if at(5) {
        assert_eq!(
            store
                .message(1)
                .unwrap()
                .unwrap()
                .reactions
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(store.outbox().unwrap().len(), 1);
    }
    if at(7) {
        let listened = |id| {
            let body: serde_json::Value =
                serde_json::from_str(&store.message(id).unwrap().unwrap().body).unwrap();
            body["listened"] == true
        };
        assert_eq!(listened(4), walked, "heard once read, by migration 30");
        assert!(!listened(5));
    }
    if at(8) {
        let crew = store.group(&group(7)).unwrap().unwrap();
        // Migration 35 adds them, to a store that hadn't had it.
        assert_eq!(
            crew.members.contains(&CREATOR.to_owned()),
            walked || seeded < 35,
            "{:?}",
            crew.members
        );
        assert!(crew.members.contains(&PEER.to_owned()));
        let members: String = store
            .conn
            .query_row(
                "SELECT members FROM groups WHERE key = ?1",
                [group(8)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            members,
            format!("[\"{PEER}\"]"),
            "a group we left stays as it was"
        );
        assert_eq!(
            store.message(6).unwrap().unwrap().sender.as_deref(),
            Some(CREATOR)
        );
        // Migration 36 settles what was waiting, in a store that hadn't had it.
        if walked || seeded < 36 {
            let deleted = store.message(7).unwrap().unwrap();
            assert_eq!(deleted.state, crate::messages::OutgoingState::Sent as i64);
            assert!(store.pending_receivers(&group(7), 107).unwrap().is_empty());
            let on_its_way = store.message(8).unwrap().unwrap();
            assert!(on_its_way.written_out && on_its_way.state == 0);
        }
        let found = store
            .search_messages("group hello", None, 10, |_| true, |_| None)
            .unwrap();
        assert_eq!(found.len(), 1);
    }
    if seeded < 35 {
        assert_eq!(
            store
                .setting(keys::GROUP_CONTACTS_TO_REFLECT)
                .unwrap()
                .as_deref(),
            Some("1")
        );
    }
    if at(13) {
        assert_eq!(store.pinned_chats().unwrap(), [PEER]);
    }
    if at(16) {
        assert_eq!(store.archived_chats().unwrap(), [group(7)]);
    }
    if at(19) {
        assert!(
            store.is_private(PEER).unwrap(),
            "a private chat stays private"
        );
    }
    if at(22) {
        assert!(store.has_d2d_nonce(&[2; 24]).unwrap());
    }
    if at(25) {
        let sessions = store.web_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].label, "");
    }
}

#[test]
fn a_store_of_every_older_schema_with_data_in_it_upgrades() {
    let fresh = fresh_schema();
    for version in 1..MIGRATIONS.len() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        store_at(&path, version);
        check(&path, version, true, &fresh);
    }
}

#[test]
fn a_store_of_each_release_with_data_in_it_upgrades() {
    let fresh = fresh_schema();
    for (release, migrations, schema) in RELEASED {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        released_store(&path, *migrations, schema);
        eprintln!("from {release}");
        check(&path, *migrations, false, &fresh);
    }
}

/// A migration that has shipped is never changed: stores out there already ran it as it was.
#[test]
fn the_migrations_still_make_each_releases_schema() {
    for (release, migrations, fixture) in RELEASED {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        store_at(&path, *migrations);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!(
            "PRAGMA key = \"x'{}'\";",
            data_encoding::HEXLOWER.encode(&KEY)
        ))
        .unwrap();
        let shipped = Connection::open_in_memory().unwrap();
        shipped.execute_batch(fixture).unwrap();
        assert_eq!(schema(&conn), schema(&shipped), "{release}");
    }
}

/// Writes the schema of the first `THREESEA_MIGRATIONS` migrations (all of them if unset) to
/// `THREESEA_SCHEMA_OUT`, a release's fixture (docs/release.md).
#[test]
#[ignore = "writes a fixture, on request"]
fn write_a_releases_schema() {
    let out = std::env::var("THREESEA_SCHEMA_OUT").expect("THREESEA_SCHEMA_OUT");
    let migrations = std::env::var("THREESEA_MIGRATIONS")
        .map_or(MIGRATIONS.len(), |count| count.parse().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    store_at(&path, migrations);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(&format!(
        "PRAGMA key = \"x'{}'\";",
        data_encoding::HEXLOWER.encode(&KEY)
    ))
    .unwrap();
    let mut text = format!(
        "-- The store's schema after its first {migrations} migrations, as a release wrote it\n\
         -- (core/src/store_upgrade_tests.rs; written by its test write_a_releases_schema).\n"
    );
    for statement in schema(&conn) {
        text.push_str(&statement);
        text.push_str(";\n");
    }
    std::fs::write(out, text).unwrap();
}
