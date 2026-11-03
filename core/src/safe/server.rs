// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Safe server API (`ThreemaSafeApiClient.kt`, `ThreemaSafeServerInfo.java`).
//!
//! Only Threema's default Safe servers (`https://safe-XX.threema.ch/`) are supported: custom
//! servers would be third-party endpoints (CLAUDE.md §7 rule 6) and are not reachable through
//! our pinned client anyway.

use std::time::Duration;

use libthreema::https::{HttpsError, HttpsHeader, HttpsMethod, HttpsRequest, HttpsResult};
use serde::Deserialize;

use super::MasterKey;
use crate::https::HttpsClient;

/// Server errors.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("network error: {0}")]
    Network(#[from] HttpsError),
    #[error("no backup found")]
    NotFound,
    #[error("backup too large ({size} of {max} bytes)")]
    TooLarge { size: usize, max: u64 },
    #[error("unexpected HTTP status {0}")]
    Status(u16),
    #[error("invalid server response: {0}")]
    InvalidResponse(String),
}

/// `GET config` response.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub max_backup_bytes: u64,
    pub retention_days: i64,
}

/// Default server base URL for a backup id. The shard is the first byte as two lowercase hex
/// digits (zero-padded, like Android; libthreema's unused helper does not pad).
pub fn base_url(key: &MasterKey) -> String {
    format!("https://safe-{:02x}.threema.ch/", key.backup_id()[0])
}

fn header(name: &str, value: &str) -> HttpsHeader {
    HttpsHeader {
        name: name.to_owned(),
        value: value.to_owned(),
    }
}

async fn call(
    client: &HttpsClient,
    method: HttpsMethod,
    url: String,
    headers: Vec<HttpsHeader>,
    body: Vec<u8>,
) -> HttpsResult {
    client
        .send(HttpsRequest {
            timeout: Duration::from_secs(30),
            url,
            method,
            headers,
            body,
        })
        .await
}

fn ok_body(result: HttpsResult) -> Result<Vec<u8>, ServerError> {
    let response = result?;
    match response.status {
        200..=299 => Ok(response.body),
        404 => Err(ServerError::NotFound),
        status => Err(ServerError::Status(status)),
    }
}

/// `GET <base>config`.
pub async fn config(client: &HttpsClient, key: &MasterKey) -> Result<ServerConfig, ServerError> {
    let body = ok_body(
        call(
            client,
            HttpsMethod::Get,
            format!("{}config", base_url(key)),
            vec![header("accept", "application/json")],
            Vec::new(),
        )
        .await,
    )?;
    serde_json::from_slice(&body).map_err(|e| ServerError::InvalidResponse(e.to_string()))
}

fn backup_url(key: &MasterKey) -> String {
    format!("{}backups/{}", base_url(key), key.backup_id_hex())
}

/// `PUT <base>backups/<id>` with the encrypted blob.
pub async fn upload(
    client: &HttpsClient,
    key: &MasterKey,
    blob: Vec<u8>,
) -> Result<(), ServerError> {
    ok_body(
        call(
            client,
            HttpsMethod::Put,
            backup_url(key),
            vec![header("content-type", "application/octet-stream")],
            blob,
        )
        .await,
    )
    .map(drop)
}

/// `GET <base>backups/<id>`.
pub async fn download(client: &HttpsClient, key: &MasterKey) -> Result<Vec<u8>, ServerError> {
    ok_body(
        call(
            client,
            HttpsMethod::Get,
            backup_url(key),
            vec![header("accept", "application/octet-stream")],
            Vec::new(),
        )
        .await,
    )
}

/// `DELETE <base>backups/<id>`. A missing backup counts as deleted.
pub async fn delete(client: &HttpsClient, key: &MasterKey) -> Result<(), ServerError> {
    match ok_body(
        call(
            client,
            HttpsMethod::Delete,
            backup_url(key),
            Vec::new(),
            Vec::new(),
        )
        .await,
    ) {
        Ok(_) | Err(ServerError::NotFound) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_is_zero_padded_hex() {
        let mut bytes = [0_u8; 64];
        bytes[0] = 0x0a;
        let key = MasterKey::from_bytes(bytes);
        assert_eq!(base_url(&key), "https://safe-0a.threema.ch/");
        assert!(backup_url(&key).starts_with("https://safe-0a.threema.ch/backups/0a00"));
        assert_eq!(
            backup_url(&key).len(),
            "https://safe-0a.threema.ch/backups/".len() + 64
        );
    }
}

/// Live test against the real Safe server; run with `cargo test -- --ignored`.
#[cfg(test)]
mod live_tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "contacts the real Threema Safe server"]
    async fn config_is_reachable_over_pinned_tls() {
        let client = HttpsClient::new().unwrap();
        let key = MasterKey::from_bytes([0x42; 64]);
        let config = config(&client, &key).await.unwrap();
        assert!(config.max_backup_bytes > 0);
        eprintln!("safe config: {config:?}");
        assert!(matches!(
            download(&client, &key).await,
            Err(ServerError::NotFound)
        ));
    }
}
