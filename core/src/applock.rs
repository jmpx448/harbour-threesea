// SPDX-License-Identifier: AGPL-3.0-only
//! App lock and private chats (ADR 0009; Android `LockAppService`, `ConversationCategoryService`).
//!
//! - The lock (`lock` setting, JSON): the PIN or passphrase (stored only as an scrypt hash),
//!   whether it is typed on the keyboard, whether the whole app locks, the grace time after
//!   leaving the app, and whether the database key is encrypted with it.
//! - Key encryption: Sailfish Secrets then holds [`wrap_key`]'s blob instead of the key; the
//!   bridge unwraps it with the passphrase before the core can start. (The device lock code was
//!   probed as a method and is not usable by apps: ADR 0009.)
//! - Private chats: table `private_chats` (migration 19). While the app is locked (the core's
//!   in-memory flag) they are left out of every list and their messages are refused.

use crypto_secretbox::{
    KeyInit as _, XSalsa20Poly1305,
    aead::{Aead as _, AeadCore as _, OsRng},
};
use data_encoding::HEXLOWER;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::store::{Store, StoreError};

/// Schema (store migration 19).
pub(crate) const MIGRATION: &str = "
    CREATE TABLE private_chats (
        conversation  TEXT PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;";

/// Settings key of [`Lock`].
pub const SETTING_LOCK: &str = "lock";

/// Grace times offered (seconds), as Android's `list_pin_grace_time_values` plus "immediately".
pub const GRACE_TIMES: [u32; 7] = [0, 30, 60, 120, 300, 600, 1800];

/// The stored lock configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lock {
    /// Typed on the keyboard (a passphrase) rather than the keypad.
    #[serde(default)]
    pub keyboard: bool,
    /// Lock the whole app (otherwise only private chats need unlocking).
    pub app_lock: bool,
    /// Seconds after leaving the app before it locks again.
    pub grace_secs: u32,
    /// Hex salt and scrypt hash of the PIN.
    pub salt: String,
    pub hash: String,
}

/// What `setLock` asks for (see [`crate::app::Command::SetLock`]): `pin` is the new PIN if
/// `new_pin`, otherwise the current one (if given).
#[derive(Debug, Clone, Copy)]
pub struct LockOptions<'a> {
    pub pin: Option<&'a str>,
    pub new_pin: bool,
    pub keyboard: bool,
    pub app_lock: bool,
    pub grace_secs: u32,
}

/// Shortest PIN/passphrase accepted (Android's PIN: 4 digits).
pub const MIN_PIN_LEN: usize = 4;

/// Start of a wrapped database key; the next byte is 1 if the passphrase is typed on the keyboard
/// (the unlock screen needs to know before anything else can be read).
const WRAPPED_MAGIC: &[u8; 4] = b"TSK1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;

fn key_encryption_key(passphrase: &str, salt: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0_u8; 32]);
    // Like Threema Safe's (64 MiB): a few seconds on a phone, once per start.
    let params = scrypt::Params::new(16, 8, 1, 32).expect("valid scrypt parameters");
    scrypt::scrypt(passphrase.as_bytes(), salt, &params, out.as_mut())
        .expect("valid output length");
    out
}

/// The database key encrypted with the passphrase: magic, keyboard flag, salt, nonce, then
/// XSalsa20-Poly1305 of the key under scrypt(passphrase).
pub fn wrap_key(key: &[u8; 32], passphrase: &str, keyboard: bool) -> Vec<u8> {
    let salt: [u8; SALT_LEN] = rand::random();
    let kek = key_encryption_key(passphrase, &salt);
    let nonce = XSalsa20Poly1305::generate_nonce(&mut OsRng);
    let sealed = XSalsa20Poly1305::new(kek.as_ref().into())
        .encrypt(&nonce, key.as_slice())
        .expect("encrypting 32 bytes");
    let mut blob = Vec::with_capacity(5 + SALT_LEN + NONCE_LEN + sealed.len());
    blob.extend_from_slice(WRAPPED_MAGIC);
    blob.push(u8::from(keyboard));
    blob.extend_from_slice(&salt);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&sealed);
    blob
}

/// Whether `blob` is a wrapped key, and then whether its passphrase is typed on the keyboard.
pub fn wrapped_keyboard(blob: &[u8]) -> Option<bool> {
    (blob.len() == 5 + SALT_LEN + NONCE_LEN + 48 && blob.starts_with(WRAPPED_MAGIC))
        .then(|| blob[4] == 1)
}

/// The database key from [`wrap_key`]'s blob, or `None` (wrong passphrase, not a wrapped key).
pub fn unwrap_key(blob: &[u8], passphrase: &str) -> Option<Zeroizing<[u8; 32]>> {
    wrapped_keyboard(blob)?;
    let (salt, rest) = blob[5..].split_at(SALT_LEN);
    let (nonce, sealed) = rest.split_at(NONCE_LEN);
    let kek = key_encryption_key(passphrase, salt);
    let key = Zeroizing::new(
        XSalsa20Poly1305::new(kek.as_ref().into())
            .decrypt(nonce.into(), sealed)
            .ok()?,
    );
    Some(Zeroizing::new(key.as_slice().try_into().ok()?))
}

fn pin_hash(pin: &str, salt: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0_u8; 32]);
    // 16 MiB, well under a second on a phone; the hash only lives inside SQLCipher.
    let params = scrypt::Params::new(14, 8, 1, 32).expect("valid scrypt parameters");
    scrypt::scrypt(pin.as_bytes(), salt, &params, out.as_mut()).expect("valid output length");
    out
}

impl Lock {
    /// A lock with a fresh salt (not key-encrypted yet).
    pub fn with_pin(pin: &str, keyboard: bool, app_lock: bool, grace_secs: u32) -> Self {
        let salt: [u8; 16] = rand::random();
        Self {
            keyboard,
            app_lock,
            grace_secs,
            hash: HEXLOWER.encode(pin_hash(pin, &salt).as_ref()),
            salt: HEXLOWER.encode(&salt),
        }
    }

    /// Whether `pin` is right (constant-time comparison of the hashes).
    pub fn check_pin(&self, pin: &str) -> bool {
        let (Ok(salt), Ok(expected)) = (
            HEXLOWER.decode(self.salt.as_bytes()),
            HEXLOWER.decode(self.hash.as_bytes()),
        ) else {
            return false;
        };
        let actual = pin_hash(pin, &salt);
        expected.len() == actual.len()
            && expected
                .iter()
                .zip(actual.iter())
                .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
                == 0
    }
}

impl Store {
    pub fn lock(&self) -> Result<Option<Lock>, StoreError> {
        Ok(self
            .setting(SETTING_LOCK)?
            .and_then(|json| serde_json::from_str(&json).ok()))
    }

    pub fn set_lock(&self, lock: Option<&Lock>) -> Result<(), StoreError> {
        match lock {
            Some(lock) => self.set_setting(
                SETTING_LOCK,
                &serde_json::to_string(lock).unwrap_or_default(),
            ),
            None => self.delete_setting(SETTING_LOCK),
        }
    }

    pub fn private_chats(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT conversation FROM private_chats")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The messages of all private chats (to hide their media while the chats are hidden).
    pub fn private_message_ids(&self) -> Result<std::collections::HashSet<i64>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id FROM messages WHERE conversation IN (SELECT conversation FROM private_chats)",
        )?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn is_private(&self, conversation: &str) -> Result<bool, StoreError> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM private_chats WHERE conversation = ?1",
            [conversation],
            |row| row.get::<_, i64>(0),
        )? > 0)
    }

    /// Mark a chat private (also unpins and unarchives it: it has its own list) or not.
    pub fn set_private(&self, conversation: &str, private: bool) -> Result<(), StoreError> {
        if private {
            self.conn.execute(
                "INSERT OR IGNORE INTO private_chats (conversation) VALUES (?1)",
                [conversation],
            )?;
            self.set_pinned(conversation, false)?;
            self.set_archived(conversation, false)?;
        } else {
            self.conn.execute(
                "DELETE FROM private_chats WHERE conversation = ?1",
                [conversation],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_hash_checks() {
        let lock = Lock::with_pin("1234", false, true, 30);
        assert!(lock.check_pin("1234"));
        assert!(!lock.check_pin("1235"));
        assert!(!lock.check_pin(""));
        // Salted: the same PIN hashes differently.
        assert_ne!(Lock::with_pin("1234", false, true, 30).hash, lock.hash);
    }

    #[test]
    fn key_wrapping() {
        let key = [7_u8; 32];
        let blob = wrap_key(&key, "correct horse", true);
        assert_eq!(wrapped_keyboard(&blob), Some(true));
        assert_eq!(
            wrapped_keyboard(&wrap_key(&key, "12345678", false)),
            Some(false)
        );
        assert_eq!(*unwrap_key(&blob, "correct horse").unwrap(), key);
        assert!(unwrap_key(&blob, "correct hors").is_none());
        // A plain key is not a wrapped one.
        assert_eq!(wrapped_keyboard(&key), None);
        assert!(unwrap_key(&key, "correct horse").is_none());
        // Tampering fails authentication.
        let mut bad = blob.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(unwrap_key(&bad, "correct horse").is_none());
    }

    #[test]
    fn stored_lock_and_private_chats() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        assert_eq!(store.lock().unwrap(), None);
        let lock = Lock::with_pin("secret", true, false, 60);
        store.set_lock(Some(&lock)).unwrap();
        assert_eq!(store.lock().unwrap(), Some(lock));
        store.set_lock(None).unwrap();
        assert_eq!(store.lock().unwrap(), None);

        store.set_pinned("A", true).unwrap();
        store.set_archived("B", true).unwrap();
        store.set_private("A", true).unwrap();
        store.set_private("B", true).unwrap();
        assert!(store.is_private("A").unwrap());
        assert!(store.pinned_chats().unwrap().is_empty());
        assert!(store.archived_chats().unwrap().is_empty());
        store.set_private("A", false).unwrap();
        assert_eq!(store.private_chats().unwrap(), ["B"]);
    }

    #[test]
    fn private_chat_events_are_redacted_while_locked() {
        use crate::providers::CoreEvent;

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        let message = store
            .insert_incoming("BBBBBBBB", 1, "text", "secret words", 1)
            .unwrap()
            .unwrap();
        store.set_private("BBBBBBBB", true).unwrap();

        // Locked: only what the generic notification needs, never the content.
        let event = CoreEvent::MessageAdded {
            message: message.clone(),
            send_delivery_receipt: false,
        };
        let locked = event.to_json(&store, false);
        assert_eq!(locked["private"], true);
        assert_eq!(locked["message"]["conversation"], "BBBBBBBB");
        assert_eq!(locked["message"]["createdAt"], 1);
        assert!(!locked.to_string().contains("secret"), "{locked}");
        let unlocked = event.to_json(&store, true);
        assert_eq!(unlocked["message"]["text"], "secret words");

        let changed = CoreEvent::MessagesChanged(vec![message]);
        assert!(
            changed.to_json(&store, false)["messages"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            changed.to_json(&store, true)["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let typing = CoreEvent::Typing {
            identity: "BBBBBBBB".into(),
            typing: true,
        };
        assert_eq!(typing.to_json(&store, false)["event"], "internal");
        assert_eq!(typing.to_json(&store, true)["event"], "typing");

        // A missed call in a private chat still keeps its status code (no user content).
        let status = store
            .insert_status("BBBBBBBB", "call|missed|0|0")
            .unwrap()
            .unwrap();
        let event = CoreEvent::MessageAdded {
            message: status,
            send_delivery_receipt: false,
        };
        assert_eq!(
            event.to_json(&store, false)["message"]["body"],
            "call|missed|0|0"
        );
    }
}

#[cfg(test)]
mod app_tests {
    use serde_json::{Value, json};

    use crate::app::{App, StartConfig};

    async fn call(app: &mut App, mut command: Value) -> Value {
        command["id"] = json!(1);
        serde_json::from_str(&app.handle(&command.to_string()).await).unwrap()
    }

    fn error(reply: &Value) -> &str {
        reply["error"]["kind"].as_str().unwrap_or("")
    }

    /// Private marks arrive without a lock (Safe restore, a linked device): no PIN opens them
    /// until one is set, then that one does and the marks can be taken off.
    #[tokio::test(flavor = "current_thread")]
    async fn any_pin_opens_nothing_without_a_lock() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(
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
            &[1; 32],
        )
        .unwrap();
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        app.store
            .insert_incoming("BBBBBBBB", 1, "text", "private text", 1)
            .unwrap();
        app.store.set_private("BBBBBBBB", true).unwrap();

        for command in [
            json!({"cmd": "unlock", "pin": "0000", "privateChats": true}),
            json!({"cmd": "unlock", "pin": "0000"}),
        ] {
            assert_eq!(error(&call(&mut app, command).await), "lockNeeded");
        }
        let private = json!({"cmd": "conversations", "private": true});
        assert_eq!(error(&call(&mut app, private.clone()).await), "locked");
        assert!(
            !call(&mut app, json!({"cmd": "lockStatus"})).await["result"]["unlocked"]
                .as_bool()
                .unwrap()
        );

        let set = json!({"cmd": "setLock", "enabled": true, "pin": "2468",
                         "keyboard": false, "appLock": false, "graceSecs": 0});
        assert_eq!(error(&call(&mut app, set).await), "");
        let open = json!({"cmd": "unlock", "pin": "0000", "privateChats": true});
        assert_eq!(error(&call(&mut app, open).await), "wrongPin");
        let open = json!({"cmd": "unlock", "pin": "2468", "privateChats": true});
        assert_eq!(call(&mut app, open).await["result"]["unlocked"], true);
        let unmark = json!({"cmd": "setPrivate", "identity": "BBBBBBBB", "private": false});
        assert_eq!(error(&call(&mut app, unmark).await), "");

        // Removing the lock ends what it opened: a later mark is hidden again.
        let remove = json!({"cmd": "setLock", "enabled": false, "currentPin": "2468"});
        assert_eq!(call(&mut app, remove).await["result"]["unlocked"], false);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn private_chats_are_hidden_while_locked() {
        let dir = tempfile::tempdir().unwrap();
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
        for id in ["BBBBBBBB", "CCCCCCCC"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(id))
                .unwrap();
            app.store
                .insert_incoming(id, 1, "text", "secret words", 1)
                .unwrap();
        }
        app.store.set_starred(1, true).unwrap();

        // No lock yet: no private chats.
        let mark = json!({"cmd": "setPrivate", "identity": "BBBBBBBB", "private": true});
        assert_eq!(error(&call(&mut app, mark.clone()).await), "lockNeeded");
        let short = json!({"cmd": "setLock", "enabled": true, "pin": "12"});
        assert_eq!(error(&call(&mut app, short).await), "pinTooShort");
        let set = json!({"cmd": "setLock", "enabled": true, "pin": "2468", "appLock": true,
                         "graceSecs": 30});
        let reply = call(&mut app, set).await;
        assert_eq!(reply["result"]["lock"]["keyboard"], false);
        // Setting up the lock doesn't open the private chats.
        assert_eq!(reply["result"]["unlocked"], false);
        assert_eq!(call(&mut app, mark).await["result"]["private"], true);

        let list = call(&mut app, json!({"cmd": "conversations"})).await;
        let ids: Vec<&str> = list["result"]["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["identity"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["CCCCCCCC"]);
        assert_eq!(list["result"]["privateCount"], 1);
        for command in [
            json!({"cmd": "conversations", "private": true}),
            json!({"cmd": "messages", "identity": "BBBBBBBB"}),
            json!({"cmd": "chatMedia", "identity": "BBBBBBBB"}),
            json!({"cmd": "message", "message": 1}),
            json!({"cmd": "setPrivate", "identity": "BBBBBBBB", "private": false}),
        ] {
            assert_eq!(
                error(&call(&mut app, command.clone()).await),
                "locked",
                "{command}"
            );
        }
        let search = json!({"cmd": "searchMessages", "query": "secret"});
        let results = call(&mut app, search.clone()).await;
        assert_eq!(results["result"]["results"].as_array().unwrap().len(), 1);
        let starred = call(&mut app, json!({"cmd": "starredMessages"})).await;
        assert!(starred["result"]["results"].as_array().unwrap().is_empty());

        // Changing the lock needs the current PIN.
        let remove = json!({"cmd": "setLock", "enabled": false});
        assert_eq!(error(&call(&mut app, remove).await), "wrongPin");
        let wrong = json!({"cmd": "unlock", "pin": "1111"});
        assert_eq!(error(&call(&mut app, wrong).await), "wrongPin");
        // The app lock's check doesn't open the private chats.
        let reply = call(&mut app, json!({"cmd": "unlock", "pin": "2468"})).await;
        assert_eq!(reply["result"]["unlocked"], false);
        let private = json!({"cmd": "conversations", "private": true});
        assert_eq!(error(&call(&mut app, private.clone()).await), "locked");

        let open = json!({"cmd": "unlock", "pin": "2468", "privateChats": true});
        assert_eq!(call(&mut app, open).await["result"]["unlocked"], true);
        let list = call(&mut app, private).await;
        assert_eq!(list["result"]["conversations"][0]["identity"], "BBBBBBBB");
        let results = call(&mut app, search).await;
        assert_eq!(results["result"]["results"].as_array().unwrap().len(), 2);

        // The lock stays while there are private chats.
        let remove = json!({"cmd": "setLock", "enabled": false, "currentPin": "2468"});
        assert_eq!(
            error(&call(&mut app, remove.clone()).await),
            "privateChatsExist"
        );
        // Other options, same PIN.
        let change = json!({"cmd": "setLock", "enabled": true, "appLock": false, "graceSecs": 60,
                            "currentPin": "2468"});
        assert_eq!(
            call(&mut app, change).await["result"]["lock"]["graceSecs"],
            60
        );
        // A passphrase instead of the PIN.
        let longer = json!({"cmd": "setLock", "enabled": true, "pin": "correct horse",
                            "keyboard": true, "currentPin": "2468"});
        assert_eq!(
            call(&mut app, longer).await["result"]["lock"]["keyboard"],
            true
        );
        let back = json!({"cmd": "setLock", "enabled": true, "pin": "2468",
                          "currentPin": "correct horse"});
        assert_eq!(
            call(&mut app, back).await["result"]["lock"]["keyboard"],
            false
        );
        call(&mut app, json!({"cmd": "lock"})).await;
        let unmark = json!({"cmd": "setPrivate", "identity": "BBBBBBBB", "private": false});
        assert_eq!(error(&call(&mut app, unmark.clone()).await), "locked");
        call(
            &mut app,
            json!({"cmd": "unlock", "pin": "2468", "privateChats": true}),
        )
        .await;
        call(&mut app, unmark).await;
        assert_eq!(call(&mut app, remove).await["result"]["lock"], Value::Null);
    }
}
