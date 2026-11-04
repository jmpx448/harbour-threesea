// SPDX-License-Identifier: AGPL-3.0-only
//! Multi-device (Threema Desktop, ADR 0011): the persisted device group membership.
//!
//! Upstream: `PersistedMultiDeviceProperties` (Android). Present only while multi-device is
//! active; then the connection goes through the mediator (D2M) instead of directly to the chat
//! server, and forward security is off (SE-199). Kept in the encrypted store's settings; the
//! device group key (DGK) never leaves the core.

use data_encoding::HEXLOWER;
use rusqlite::OptionalExtension as _;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::store::{Store, StoreError};

/// Schema (store migration 22): nonces of D2D envelopes (reflection), apart from CSP's.
pub(crate) const MIGRATION: &str = "
    CREATE TABLE d2d_nonces (
        nonce BLOB PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;";

/// Schema (store migration 23): sync envelopes waiting for the mediator (changed while offline or
/// before the reflection went through); removed once committed.
pub(crate) const MIGRATION_SYNC_QUEUE: &str = "
    CREATE TABLE d2d_sync_queue (
        id       INTEGER PRIMARY KEY,
        scope    INTEGER NOT NULL,
        content  BLOB NOT NULL
    );";

/// D2D nonce storage for libthreema's D2X context.
pub struct StoreD2dNonces(pub std::rc::Rc<Store>);

impl libthreema::model::provider::NonceStorage for StoreD2dNonces {
    fn has(
        &self,
        nonce: &libthreema::common::Nonce,
    ) -> Result<bool, libthreema::model::provider::ProviderError> {
        self.0
            .has_d2d_nonce(&nonce.0)
            .map_err(crate::providers::foreign)
    }

    fn add_many(
        &mut self,
        nonces: Vec<libthreema::common::Nonce>,
    ) -> Result<(), libthreema::model::provider::ProviderError> {
        for nonce in nonces {
            self.0
                .add_d2d_nonce(&nonce.0)
                .map_err(crate::providers::foreign)?;
        }
        Ok(())
    }
}

/// Settings key of [`Properties`] (JSON).
pub const SETTING_PROPERTIES: &str = "md_properties";

/// Settings key: "1" while forward security sessions still have to be terminated after turning
/// multi-device on (done by the connection once logged in, so it survives being offline).
pub const SETTING_FS_TERMINATE: &str = "md_fs_terminate";

/// Our label in other devices' lists (Android: "Android Client"), the default of the setting
/// [`SETTING_DEVICE_LABEL`] the user can change (Settings → Linked devices).
pub const DEVICE_LABEL: &str = "Sailfish OS";

/// Settings key: this device's label, as other devices show it. It travels in the D2M
/// client-hello, so a change needs a reconnect.
pub const SETTING_DEVICE_LABEL: &str = "md_device_label";

/// Settings key prefix of a name the user gave another device of the group, for our lists
/// only: a device's label is its own (encrypted with the device group key), no device can
/// rename another one.
pub const SETTING_ALIAS_PREFIX: &str = "md_device_alias:";

/// The device group membership of this device.
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct Properties {
    /// Device group key, hex.
    dgk: String,
    /// Our D2X (mediator) device id.
    #[zeroize(skip)]
    pub mediator_device_id: u64,
    /// Our CSP device id (login extension).
    #[zeroize(skip)]
    pub csp_device_id: u64,
    /// The mediator knows this device (after the first successful handshake).
    #[zeroize(skip)]
    #[serde(default)]
    pub registered: bool,
}

impl Properties {
    /// New random ids and key (Android's `generateMdProperties`).
    #[must_use]
    pub fn generate() -> Self {
        let dgk = Zeroizing::new(rand::random::<[u8; 32]>());
        Self {
            dgk: HEXLOWER.encode(dgk.as_slice()),
            mediator_device_id: rand::random(),
            csp_device_id: rand::random(),
            registered: false,
        }
    }

    /// The device group key's bytes.
    #[must_use]
    pub fn dgk(&self) -> Option<Zeroizing<[u8; 32]>> {
        let bytes = Zeroizing::new(HEXLOWER.decode(self.dgk.as_bytes()).ok()?);
        let mut key = Zeroizing::new([0_u8; 32]);
        if bytes.len() != key.len() {
            return None;
        }
        key.copy_from_slice(&bytes);
        Some(key)
    }

    #[must_use]
    pub fn device_group_key(&self) -> Option<libthreema::common::keys::DeviceGroupKey> {
        self.dgk()
            .map(|key| libthreema::common::keys::DeviceGroupKey::from(*key))
    }
}

impl Store {
    /// Used before? Raw or (Android backup import) hashed, like [`Store::has_nonce`].
    pub fn has_d2d_nonce(&self, nonce: &[u8]) -> Result<bool, StoreError> {
        self.has_nonce_in("d2d_nonces", nonce)
    }

    /// Queue an encoded `d2d.Envelope` content (`Envelope` with only `content` set) and its
    /// transaction scope.
    pub fn queue_sync(&self, scope: i32, content: &[u8]) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO d2d_sync_queue (scope, content) VALUES (?1, ?2)",
            rusqlite::params![scope, content],
        )?;
        Ok(())
    }

    /// Queue something for the other devices. Part of the caller's store step, if it is in one.
    pub(crate) fn queue_sync_content(
        &self,
        scope: libthreema::protobuf::d2d::transaction_scope::Scope,
        content: libthreema::protobuf::d2d::envelope::Content,
    ) -> Result<(), StoreError> {
        use prost::Message as _;
        #[expect(deprecated, reason = "Padding is still part of the envelope")]
        let envelope = libthreema::protobuf::d2d::Envelope {
            padding: Vec::new(),
            device_id: 0,
            protocol_version: 0,
            content: Some(content),
        };
        self.queue_sync(scope as i32, &envelope.encode_to_vec())
    }

    /// The oldest queued sync: (id, scope, content).
    pub fn next_sync(&self) -> Result<Option<(i64, i32, Vec<u8>)>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, scope, content FROM d2d_sync_queue ORDER BY id LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?)
    }

    pub fn sync_done(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM d2d_sync_queue WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Multi-device off: nothing left to reflect.
    pub fn clear_sync_queue(&self) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM d2d_sync_queue", [])?;
        Ok(())
    }

    /// Every used D2D nonce (device join).
    pub fn all_d2d_nonces(&self) -> Result<Vec<Vec<u8>>, StoreError> {
        let mut statement = self.conn.prepare("SELECT nonce FROM d2d_nonces")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn add_d2d_nonce(&self, nonce: &[u8]) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO d2d_nonces (nonce) VALUES (?1)",
            [nonce],
        )?;
        Ok(())
    }

    /// The device group membership, if multi-device is active.
    pub fn multi_device(&self) -> Result<Option<Properties>, StoreError> {
        Ok(self
            .setting(SETTING_PROPERTIES)?
            .and_then(|json| serde_json::from_str(&Zeroizing::new(json)).ok()))
    }

    /// Store (or, with `None`, forget) the device group membership.
    pub fn set_multi_device(&self, properties: Option<&Properties>) -> Result<(), StoreError> {
        match properties {
            Some(properties) => {
                // Plain strings and numbers: serializing cannot fail.
                let json = Zeroizing::new(serde_json::to_string(properties).unwrap_or_default());
                self.set_setting(SETTING_PROPERTIES, &json)
            }
            None => {
                self.clear_sync_queue()?;
                self.delete_setting(SETTING_PROPERTIES)
            }
        }
    }
}

/// The 128 emojis of the rendezvous path hash check (upstream `RendezvousEmojis.kt`, same order;
/// the RPH's first three bytes mod 128 pick them).
#[rustfmt::skip]
pub const RENDEZVOUS_EMOJIS: [&str; 128] = [
    "\u{1f435}", "\u{1f436}", "\u{1f429}", "\u{1f98a}", "\u{1f408}\u{200d}\u{2b1b}", "\u{1f434}",
    "\u{1f9ac}", "\u{1f42e}", "\u{1f437}", "\u{1f42a}", "\u{1f992}", "\u{1f418}", "\u{1f42d}",
    "\u{1f430}", "\u{1f43f}\u{fe0f}", "\u{1f43c}", "\u{1f998}", "\u{1f414}", "\u{1f54a}\u{fe0f}",
    "\u{1f986}", "\u{1f9a2}", "\u{1f989}", "\u{1f9a4}", "\u{1fab6}", "\u{1f9a9}", "\u{1f438}",
    "\u{1f40b}", "\u{1f42c}", "\u{1f9ad}", "\u{1f41f}", "\u{1f41a}", "\u{1fab2}",
    "\u{1f578}\u{fe0f}", "\u{1f3f5}\u{fe0f}", "\u{1f33b}", "\u{1f332}", "\u{1f33f}",
    "\u{2618}\u{fe0f}", "\u{1f349}", "\u{1f34b}", "\u{1f34e}", "\u{1f352}", "\u{1f353}",
    "\u{1fad0}", "\u{1f345}", "\u{1f951}", "\u{1f955}", "\u{1f966}", "\u{1f344}", "\u{1fad8}",
    "\u{1f9c2}", "\u{1f36a}", "\u{1f36b}", "\u{2615}", "\u{1f9ca}", "\u{1f962}",
    "\u{1f5fa}\u{fe0f}", "\u{1f30b}", "\u{1f3d5}\u{fe0f}", "\u{1f3dd}\u{fe0f}",
    "\u{1f3db}\u{fe0f}", "\u{1f682}", "\u{1f69a}", "\u{1f69c}", "\u{1f6e4}\u{fe0f}", "\u{2693}",
    "\u{1f6f0}\u{fe0f}", "\u{1f680}", "\u{1f319}", "\u{2600}\u{fe0f}", "\u{2b50}", "\u{1f308}",
    "\u{2602}\u{fe0f}", "\u{2744}\u{fe0f}", "\u{2603}\u{fe0f}", "\u{1f525}", "\u{1f4a7}",
    "\u{2728}", "\u{1f388}", "\u{1f380}", "\u{1f947}", "\u{1f3c0}", "\u{1f3d0}", "\u{1f3b3}",
    "\u{1f3d3}", "\u{26f3}", "\u{1f3af}", "\u{1f579}\u{fe0f}", "\u{1f9e9}", "\u{1f9f8}",
    "\u{2660}\u{fe0f}", "\u{2665}\u{fe0f}", "\u{1f457}", "\u{1f451}", "\u{1f514}", "\u{1f3b7}",
    "\u{1f3b8}", "\u{1f5a8}\u{fe0f}", "\u{1f4f8}", "\u{1f56f}\u{fe0f}", "\u{1f4d6}", "\u{1f4e6}",
    "\u{1f4ee}", "\u{1f4dd}", "\u{1f4bc}", "\u{1f4cb}", "\u{1f512}", "\u{1f511}",
    "\u{2692}\u{fe0f}", "\u{1fa83}", "\u{2696}\u{fe0f}", "\u{1f517}", "\u{1fa9d}", "\u{1f52c}",
    "\u{1fa91}", "\u{1f6bd}", "\u{1f9f9}", "\u{1faa3}", "\u{1fae7}", "\u{26ab}", "\u{1f7e8}",
    "\u{25b6}\u{fe0f}", "\u{1f4f6}", "\u{1f4a5}", "\u{1f4ac}", "\u{1f4ab}", "\u{1f440}",
    "\u{1f463}",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_round_trip_through_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db"), &[3; 32]).unwrap();
        assert!(store.multi_device().unwrap().is_none());
        let properties = Properties::generate();
        let dgk = properties.dgk().unwrap();
        store.set_multi_device(Some(&properties)).unwrap();
        let read = store.multi_device().unwrap().unwrap();
        assert_eq!(read.dgk().unwrap().as_slice(), dgk.as_slice());
        assert_eq!(read.mediator_device_id, properties.mediator_device_id);
        assert_eq!(read.csp_device_id, properties.csp_device_id);
        store.queue_sync(1, b"a").unwrap();
        store.queue_sync(2, b"b").unwrap();
        let (first, scope, content) = store.next_sync().unwrap().unwrap();
        assert_eq!((scope, content.as_slice()), (1, &b"a"[..]));
        store.sync_done(first).unwrap();
        assert_eq!(store.next_sync().unwrap().unwrap().1, 2);
        store.set_multi_device(None).unwrap();
        assert!(store.multi_device().unwrap().is_none());
        assert!(
            store.next_sync().unwrap().is_none(),
            "the queue goes with it"
        );
    }
}
