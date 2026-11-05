// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Web (ADR 0013, docs/research/threema-web.md): this phone serves the browser over
//! SaltyRTC's relayed-data task. `saltyrtc` is the responder, `chunks` the message chunking,
//! `arp` the app remote protocol's connection layer; the app answers the browser's requests
//! (`app_web`). Sessions (our permanent key, the browser's key, the server) are stored here.

pub mod arp;
pub mod battery;
pub mod budget;
pub mod chunks;
pub mod images;
pub mod qr;
pub mod saltyrtc;

use rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::store::{Store, StoreError};

pub(crate) const MIGRATION: &str = "
    CREATE TABLE web_sessions (
        id              INTEGER PRIMARY KEY,
        initiator_key   BLOB NOT NULL,
        private_key     BLOB NOT NULL,
        server_key      BLOB,
        host            TEXT NOT NULL,
        port            INTEGER NOT NULL,
        self_hosted     INTEGER NOT NULL DEFAULT 0,
        persistent      INTEGER NOT NULL DEFAULT 0,
        client          TEXT NOT NULL DEFAULT '',
        created_at      INTEGER NOT NULL,
        last_connection INTEGER
    );
    CREATE TABLE web_hosts (
        host TEXT PRIMARY KEY,
        spki TEXT
    );
";

/// The name the user gave a browser (sessions of the same browser program look alike).
pub(crate) const MIGRATION_LABEL: &str =
    "ALTER TABLE web_sessions ADD COLUMN label TEXT NOT NULL DEFAULT '';";

/// A Threema Web session (one browser).
pub struct WebSession {
    pub id: i64,
    /// The browser's permanent public key (also its SaltyRTC path).
    pub initiator_key: [u8; 32],
    /// Our permanent secret key towards this browser.
    pub private_key: Zeroizing<[u8; 32]>,
    pub server_key: Option<[u8; 32]>,
    pub host: String,
    pub port: u16,
    /// What the code claimed (its options). Not for anything that matters: the host itself says
    /// whether the server is Threema's.
    pub self_hosted: bool,
    /// The browser stored the session (`keyPersisted`): it can come back without a QR code.
    pub persistent: bool,
    /// What the browser said it is (`clientInfo`).
    pub client: String,
    /// What the user calls it; empty: `client` is its name.
    pub label: String,
    pub created_at: i64,
    pub last_connection: Option<i64>,
}

impl WebSession {
    /// For the UI (no keys).
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "host": self.host,
            "selfHosted": !qr::is_threema_host(&self.host),
            "persistent": self.persistent,
            "client": self.client,
            "label": self.label,
            "createdAt": self.created_at,
            "lastConnection": self.last_connection,
        })
    }
}

fn key32(bytes: Vec<u8>) -> [u8; 32] {
    bytes.try_into().unwrap_or([0; 32])
}

const COLUMNS: &str =
    "id, initiator_key, private_key, server_key, host, port, self_hosted, persistent,
    client, created_at, last_connection, label";

fn session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WebSession> {
    Ok(WebSession {
        id: row.get(0)?,
        initiator_key: key32(row.get(1)?),
        private_key: Zeroizing::new(key32(row.get(2)?)),
        server_key: row.get::<_, Option<Vec<u8>>>(3)?.map(key32),
        host: row.get(4)?,
        port: u16::try_from(row.get::<_, i64>(5)?).unwrap_or(443),
        self_hosted: row.get(6)?,
        persistent: row.get(7)?,
        client: row.get(8)?,
        created_at: row.get(9)?,
        last_connection: row.get(10)?,
        label: row.get(11)?,
    })
}

impl Store {
    /// A new session from a scanned code; its ID.
    pub fn add_web_session(&self, session: &WebSession) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO web_sessions (initiator_key, private_key, server_key, host, port, self_hosted,
                 persistent, client, created_at, label)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                session.initiator_key.as_slice(),
                session.private_key.as_slice(),
                session.server_key.map(|key| key.to_vec()),
                session.host,
                session.port,
                session.self_hosted,
                session.persistent,
                session.client,
                session.created_at,
                session.label,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn web_sessions(&self) -> Result<Vec<WebSession>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM web_sessions ORDER BY created_at DESC"
        ))?;
        let rows = statement.query_map([], session_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn web_session(&self, id: i64) -> Result<Option<WebSession>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM web_sessions WHERE id = ?1"),
                [id],
                session_row,
            )
            .optional()?)
    }

    pub fn set_web_session_persistent(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("UPDATE web_sessions SET persistent = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_web_session_connected(
        &self,
        id: i64,
        client: &str,
        at: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE web_sessions SET last_connection = ?2,
                 client = CASE WHEN ?3 = '' THEN client ELSE ?3 END
             WHERE id = ?1",
            params![id, at, client],
        )?;
        Ok(())
    }

    /// Name a session (empty: none); whether it exists.
    pub fn set_web_session_label(&self, id: i64, label: &str) -> Result<bool, StoreError> {
        let changed = self.conn.execute(
            "UPDATE web_sessions SET label = ?2 WHERE id = ?1",
            params![id, label],
        )?;
        Ok(changed > 0)
    }

    pub fn remove_web_session(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM web_sessions WHERE id = ?1", [id])?;
        Ok(())
    }

    /// A self-hosted server the user allowed: `Some(pin)` (SPKI SHA-256, Base64; `None` until
    /// the first connection pinned it), or `None` if not allowed.
    pub fn web_host(&self, host: &str) -> Result<Option<Option<String>>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT spki FROM web_hosts WHERE host = ?1",
                [host],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Allow a self-hosted server, pinned to `spki` (`None`: pin on the next connection).
    pub fn set_web_host(&self, host: &str, spki: Option<&str>) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO web_hosts (host, spki) VALUES (?1, ?2)
             ON CONFLICT(host) DO UPDATE SET spki = excluded.spki",
            params![host, spki],
        )?;
        Ok(())
    }

    /// The self-hosted servers the user allowed.
    pub fn web_hosts(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT host FROM web_hosts ORDER BY host")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Take a self-hosted server's permission and its certificate pin back.
    pub fn remove_web_host(&self, host: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM web_hosts WHERE host = ?1", [host])?;
        Ok(())
    }
}
