// SPDX-License-Identifier: AGPL-3.0-only
//! Group calls (ADR 0012, docs/research/group-calls.md): key and call ID derivation, the SFU
//! credentials from the directory, the SFU's base URL check, and its HTTP API (peek, join),
//! following `group-call.proto` and Android's `voip/groupcall`.

use blake2::{Blake2bMac, digest::Mac as _};
use libthreema::{
    https::{HttpsHeader, HttpsMethod, HttpsRequest},
    protobuf::group_call::{sfu_http_request, sfu_http_response},
};
use prost::Message as _;
use serde::Deserialize;
use zeroize::Zeroizing;

use rusqlite::params;

use crate::{
    https::HttpsClient,
    store::{Store, StoreError},
};

/// A participant as the call state lists it.
pub struct StateParticipant<'a> {
    pub id: u32,
    pub identity: &'a str,
    pub nickname: &'a str,
}

/// The designated participant (the lowest ID of everyone authenticated, us included) keeps the
/// call state at the SFU current (`group-call.proto` "State Update").
#[must_use]
pub fn designated(local: u32, authenticated: impl IntoIterator<Item = u32>) -> bool {
    authenticated.into_iter().all(|id| local < id)
}

/// `UpdateCallState.encrypted_call_state`: the `CallState`, secretbox under GCSK with the random
/// nonce in front (Android's `GroupCallDescription.encryptWithGcsk`).
#[must_use]
pub fn sealed_call_state(
    gcsk: &[u8; 32],
    created_by: u32,
    created_at: u64,
    participants: &[StateParticipant<'_>],
) -> Vec<u8> {
    use crypto_secretbox::{
        KeyInit as _, XSalsa20Poly1305,
        aead::{Aead as _, AeadCore as _, OsRng},
    };
    use libthreema::protobuf::group_call::{CallState, call_state};
    #[expect(
        deprecated,
        reason = "padding as the protocol describes it, random length"
    )]
    let state = CallState {
        padding: vec![0; usize::from(rand::random::<u8>() % 32)],
        state_created_by: created_by,
        state_created_at: created_at,
        participants: participants
            .iter()
            .map(|participant| {
                (
                    participant.id,
                    call_state::Participant {
                        participant: Some(call_state::participant::Participant::Threema(
                            call_state::participant::Normal {
                                identity: participant.identity.to_owned(),
                                nickname: participant.nickname.to_owned(),
                            },
                        )),
                    },
                )
            })
            .collect(),
    };
    let nonce = XSalsa20Poly1305::generate_nonce(&mut OsRng);
    let sealed = XSalsa20Poly1305::new(gcsk.into())
        .encrypt(&nonce, state.encode_to_vec().as_slice())
        .expect("secretbox encryption does not fail");
    [nonce.as_slice(), &sealed].concat()
}

/// Who is in a call by its call state (from a peek); `None` if it doesn't open.
#[must_use]
pub fn opened_call_state(gcsk: &[u8; 32], sealed: &[u8]) -> Option<Vec<(String, String)>> {
    use crypto_secretbox::{KeyInit as _, XSalsa20Poly1305, aead::Aead as _};
    use libthreema::protobuf::group_call::{CallState, call_state::participant::Participant};
    if sealed.len() < 24 {
        return None;
    }
    let plain = XSalsa20Poly1305::new(gcsk.into())
        .decrypt(sealed[..24].into(), &sealed[24..])
        .ok()?;
    let state = CallState::decode(plain.as_slice()).ok()?;
    let mut participants: Vec<(u32, (String, String))> = state
        .participants
        .into_iter()
        .filter_map(|(id, participant)| match participant.participant? {
            Participant::Threema(normal) => Some((id, (normal.identity, normal.nickname))),
            Participant::Guest(_) => None,
        })
        .collect();
    participants.sort_by_key(|(id, _)| *id);
    Some(participants.into_iter().map(|(_, who)| who).collect())
}

/// Group call protocol version (`GroupCallStart.protocol_version`).
pub const PROTOCOL_VERSION: u32 = 1;

const PERSONAL: &[u8] = b"3ma-call";

/// `BLAKE2b(out=32, key, salt, personal='3ma-call', input)`; unkeyed with an empty key (like
/// Android's `CryptoCallUtils.gcBlake2b256`).
pub(crate) fn blake2b(key: &[u8], salt: &[u8], input: &[&[u8]]) -> [u8; 32] {
    let mut mac = Blake2bMac::<blake2::digest::consts::U32>::new_with_salt_and_personal(
        (!key.is_empty()).then_some(key),
        salt,
        PERSONAL,
    )
    .expect("salt and personal fit BLAKE2b");
    for part in input {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

/// The keys of a call, derived from its Group Call Key.
pub struct CallKeys {
    /// `GCKH`: input to every per-participant key derivation.
    pub gckh: [u8; 32],
    /// `GCHK`: encrypts the participants' handshakes.
    pub gchk: Zeroizing<[u8; 32]>,
    /// `GCSK`: encrypts the call state kept at the SFU.
    pub gcsk: Zeroizing<[u8; 32]>,
}

impl CallKeys {
    #[must_use]
    pub fn derive(gck: &[u8; 32]) -> Self {
        Self {
            gckh: blake2b(gck, b"#", &[]),
            gchk: Zeroizing::new(blake2b(gck, b"h", &[])),
            gcsk: Zeroizing::new(blake2b(gck, b"s", &[])),
        }
    }
}

/// The call ID (`group-call.proto` "Group Call ID Derivation").
#[must_use]
pub fn call_id(
    creator: &str,
    group_id: &[u8; 8],
    protocol_version: u32,
    gck: &[u8; 32],
    sfu_base_url: &str,
) -> [u8; 32] {
    // `u8(protocol_version)`: the protocol only has small versions.
    let version = [protocol_version as u8];
    blake2b(
        &[],
        b"i",
        &[
            creator.as_bytes(),
            group_id,
            &version,
            gck,
            sfu_base_url.as_bytes(),
        ],
    )
}

/// An SFU base URL is only used with `https` and a host ending with one of the suffixes the
/// directory allows (from the user's own SFU credentials).
#[must_use]
pub fn sfu_url_allowed(url: &str, allowed_suffixes: &[String]) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    // The host (with its port, like Android's check), without a path.
    let host = rest.split('/').next().unwrap_or("");
    !host.is_empty()
        && allowed_suffixes
            .iter()
            .any(|suffix| !suffix.is_empty() && host.ends_with(suffix.as_str()))
}

/// The SFU credentials (`identity/sfu_cred`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SfuCredentials {
    pub sfu_base_url: String,
    pub allowed_sfu_hostname_suffixes: Vec<String>,
    pub sfu_token: String,
    /// Seconds from now.
    pub expiration: u64,
}

/// What a peek found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peek {
    Running {
        started_at: u64,
        max_participants: u32,
        encrypted_call_state: Option<Vec<u8>>,
    },
    /// 404: the call is over.
    NotRunning,
    /// 401: the token needs a refresh.
    Unauthorized,
}

/// What a join returned.
#[derive(Debug, Clone)]
pub enum Join {
    Joined(sfu_http_response::Join),
    Unauthorized,
    /// 419: our protocol version is not supported.
    UnsupportedVersion,
    /// 503: the call is full.
    Full,
}

#[derive(Debug, thiserror::Error)]
pub enum SfuError {
    #[error("SFU request failed: {0}")]
    Network(String),
    #[error("SFU answered {0}")]
    Status(u16),
    #[error("invalid SFU response: {0}")]
    Decode(String),
}

fn sfu_request(
    base: &str,
    action: &str,
    call_id: &[u8; 32],
    token: &str,
    body: Vec<u8>,
    timeout_s: u64,
) -> HttpsRequest {
    let base = base.trim_end_matches('/');
    HttpsRequest {
        timeout: std::time::Duration::from_secs(timeout_s),
        url: format!("{base}/v1/{action}/{}", hex(call_id)),
        method: HttpsMethod::Post,
        headers: vec![
            HttpsHeader {
                name: "authorization".into(),
                value: format!("ThreemaSfuToken {token}"),
            },
            HttpsHeader {
                name: "content-type".into(),
                value: "application/octet-stream".into(),
            },
        ],
        body,
    }
}

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// `POST <base>/v1/peek/<call id>` (5 s, like Android).
pub async fn peek(
    client: &HttpsClient,
    base: &str,
    token: &str,
    call_id: &[u8; 32],
) -> Result<Peek, SfuError> {
    let body = sfu_http_request::Peek {
        call_id: call_id.to_vec(),
    }
    .encode_to_vec();
    let response = client
        .send(sfu_request(base, "peek", call_id, token, body, 5))
        .await
        .map_err(|e| SfuError::Network(e.to_string()))?;
    match response.status {
        200 => {
            let peek = sfu_http_response::Peek::decode(response.body.as_slice())
                .map_err(|e| SfuError::Decode(e.to_string()))?;
            Ok(Peek::Running {
                started_at: peek.started_at,
                max_participants: peek.max_participants,
                encrypted_call_state: peek.encrypted_call_state,
            })
        }
        401 => Ok(Peek::Unauthorized),
        404 => Ok(Peek::NotRunning),
        status => Err(SfuError::Status(status)),
    }
}

/// `POST <base>/v1/join/<call id>`; `dtls_fingerprint`: the raw SHA-256 of our DTLS certificate.
pub async fn join(
    client: &HttpsClient,
    base: &str,
    token: &str,
    call_id: &[u8; 32],
    dtls_fingerprint: &[u8],
) -> Result<Join, SfuError> {
    let body = sfu_http_request::Join {
        call_id: call_id.to_vec(),
        protocol_version: PROTOCOL_VERSION,
        dtls_fingerprint: dtls_fingerprint.to_vec(),
    }
    .encode_to_vec();
    let response = client
        .send(sfu_request(base, "join", call_id, token, body, 20))
        .await
        .map_err(|e| SfuError::Network(e.to_string()))?;
    match response.status {
        200 => sfu_http_response::Join::decode(response.body.as_slice())
            .map(Join::Joined)
            .map_err(|e| SfuError::Decode(e.to_string())),
        401 => Ok(Join::Unauthorized),
        419 => Ok(Join::UnsupportedVersion),
        503 => Ok(Join::Full),
        status => Err(SfuError::Status(status)),
    }
}

/// Schema (store migration 24): group calls considered running, per group (the
/// `GroupCallStart` messages received or sent), until a peek says they ended.
pub(crate) const MIGRATION: &str = "
    CREATE TABLE group_calls (
        call_id          BLOB PRIMARY KEY NOT NULL,
        conversation     TEXT NOT NULL,
        gck              BLOB NOT NULL,
        sfu_base_url     TEXT NOT NULL,
        protocol_version INTEGER NOT NULL,
        received_at      INTEGER NOT NULL,
        failed           INTEGER NOT NULL DEFAULT 0
    ) WITHOUT ROWID;";

/// A group call considered running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningCall {
    pub call_id: [u8; 32],
    pub conversation: String,
    pub gck: [u8; 32],
    pub sfu_base_url: String,
    pub protocol_version: u32,
    /// ms
    pub received_at: i64,
    /// Peeks that failed in a row.
    pub failed: u32,
}

impl Store {
    /// Add a call; `false` if the group already has a call with this GCK (ignored, like the
    /// spec says).
    pub fn add_group_call(&self, call: &RunningCall) -> Result<bool, StoreError> {
        let duplicate: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM group_calls WHERE conversation = ?1 AND gck = ?2)",
            params![call.conversation, call.gck.as_slice()],
            |row| row.get(0),
        )?;
        if duplicate {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO group_calls
                 (call_id, conversation, gck, sfu_base_url, protocol_version, received_at, failed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![
                call.call_id.as_slice(),
                call.conversation,
                call.gck.as_slice(),
                call.sfu_base_url,
                call.protocol_version,
                call.received_at,
            ],
        )?;
        Ok(true)
    }

    /// The running calls of a group.
    pub fn group_calls(&self, conversation: &str) -> Result<Vec<RunningCall>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT call_id, conversation, gck, sfu_base_url, protocol_version, received_at, failed
             FROM group_calls WHERE conversation = ?1 ORDER BY received_at",
        )?;
        let rows = statement.query_map(params![conversation], |row| {
            let call_id: Vec<u8> = row.get(0)?;
            let gck: Vec<u8> = row.get(2)?;
            Ok(RunningCall {
                call_id: call_id.try_into().unwrap_or([0; 32]),
                conversation: row.get(1)?,
                gck: gck.try_into().unwrap_or([0; 32]),
                sfu_base_url: row.get(3)?,
                protocol_version: row.get(4)?,
                received_at: row.get(5)?,
                failed: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Groups with calls considered running.
    pub fn groups_with_calls(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT DISTINCT conversation FROM group_calls")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Whether it was still there: refreshes run side by side, and only one of them removes it.
    pub fn remove_group_call(&self, call_id: &[u8; 32]) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "DELETE FROM group_calls WHERE call_id = ?1",
            params![call_id.as_slice()],
        )? > 0)
    }

    /// A peek failed (`false`) or worked again (`true`).
    pub fn set_group_call_reachable(
        &self,
        call_id: &[u8; 32],
        reachable: bool,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            if reachable {
                "UPDATE group_calls SET failed = 0 WHERE call_id = ?1"
            } else {
                "UPDATE group_calls SET failed = failed + 1 WHERE call_id = ?1"
            },
            params![call_id.as_slice()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_id_matches_android() {
        // Android's CallIdTest.
        let id = call_id("ABCD1234", &[1, 2, 3, 4, 5, 6, 7, 8], 1, &[0; 32], "abcdef");
        assert_eq!(
            hex(&id),
            "198bb21c1254116a5b420e4744ad5681a955fbbfbab2cdc2a83fbf1751eb8ae0"
        );
    }

    #[test]
    fn sfu_urls_are_checked() {
        let suffixes = vec![".threema.ch".to_owned()];
        assert!(sfu_url_allowed("https://sfu.threema.ch", &suffixes));
        assert!(sfu_url_allowed("https://sfu.threema.ch/", &suffixes));
        assert!(!sfu_url_allowed("http://sfu.threema.ch", &suffixes));
        assert!(!sfu_url_allowed(
            "https://sfu.threema.ch.evil.example",
            &suffixes
        ));
        assert!(!sfu_url_allowed(
            "https://evil.example/sfu.threema.ch",
            &suffixes
        ));
        assert!(!sfu_url_allowed("https://", &suffixes));
        assert!(!sfu_url_allowed("https://sfu.threema.ch", &[String::new()]));
    }

    #[test]
    fn keys_differ_by_salt() {
        let keys = CallKeys::derive(&[9; 32]);
        assert_ne!(keys.gckh, *keys.gchk);
        assert_ne!(*keys.gchk, *keys.gcsk);
    }

    #[test]
    fn running_calls_are_stored() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let call = RunningCall {
            call_id: [1; 32],
            conversation: "g:ECHOECHO:0000000000000001".into(),
            gck: [2; 32],
            sfu_base_url: "https://sfu.threema.ch".into(),
            protocol_version: 1,
            received_at: 5,
            failed: 0,
        };
        assert!(store.add_group_call(&call).unwrap());
        // The same GCK again: ignored.
        assert!(
            !store
                .add_group_call(&RunningCall {
                    call_id: [3; 32],
                    ..call.clone()
                })
                .unwrap()
        );
        store
            .set_group_call_reachable(&call.call_id, false)
            .unwrap();
        let calls = store.group_calls(&call.conversation).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].failed, 1);
        assert_eq!(
            store.groups_with_calls().unwrap(),
            vec![call.conversation.clone()]
        );
        assert!(store.remove_group_call(&call.call_id).unwrap());
        assert!(
            !store.remove_group_call(&call.call_id).unwrap(),
            "a second refresh has nothing to say"
        );
        assert!(store.group_calls(&call.conversation).unwrap().is_empty());
    }

    #[test]
    fn call_state_opens_with_gcsk() {
        use crypto_secretbox::{KeyInit as _, XSalsa20Poly1305, aead::Aead as _};
        use libthreema::protobuf::group_call::{CallState, call_state};
        let keys = CallKeys::derive(&[7; 32]);
        let sealed = sealed_call_state(
            &keys.gcsk,
            3,
            1234,
            &[
                StateParticipant {
                    id: 3,
                    identity: "ABCD1234",
                    nickname: "Me",
                },
                StateParticipant {
                    id: 5,
                    identity: "EFGH5678",
                    nickname: "",
                },
            ],
        );
        assert_eq!(
            opened_call_state(&keys.gcsk, &sealed).unwrap(),
            vec![
                ("ABCD1234".to_owned(), "Me".to_owned()),
                ("EFGH5678".to_owned(), String::new())
            ]
        );
        assert!(opened_call_state(&[0; 32], &sealed).is_none());
        let plain = XSalsa20Poly1305::new(keys.gcsk.as_ref().into())
            .decrypt(sealed[..24].into(), &sealed[24..])
            .unwrap();
        let state = CallState::decode(plain.as_slice()).unwrap();
        assert_eq!((state.state_created_by, state.state_created_at), (3, 1234));
        assert_eq!(state.participants.len(), 2);
        assert!(matches!(
            &state.participants[&5].participant,
            Some(call_state::participant::Participant::Threema(normal)) if normal.identity == "EFGH5678"
        ));
    }

    #[test]
    fn lowest_id_is_designated() {
        assert!(designated(2, [3, 9]));
        assert!(designated(2, []));
        assert!(!designated(4, [3, 9]));
    }
}
