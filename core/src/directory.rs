// SPDX-License-Identifier: AGPL-3.0-only
//! Directory server calls that libthreema has no task for (yet). Mirrors upstream
//! `domain/src/main/java/ch/threema/domain/protocol/api/APIConnector.java`. Crypto (the
//! challenge response) stays in libthreema via `ClientKey::solve_directory_challenge`
//! (patch 0002).

use data_encoding::BASE64;
use libthreema::{
    common::{
        ClientInfo,
        keys::{ClientKey, PublicKey},
    },
    https::HttpsError,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::https::HttpsClient;

/// Our platform code, in Threema Safe's device field and (through libthreema's
/// `ClientInfo::Threesea`) in the `client-info` string. Not one of Threema's documented codes on
/// purpose: we are none of those clients (see docs/protocol-notes.md).
pub const PLATFORM_CODE: &str = "S";

/// Production directory base URL, same as libthreema's `Config::production()`.
pub const DIRECTORY_URL: &str = "https://ds-apip.threema.ch/";

/// Errors from directory calls.
#[derive(Debug, thiserror::Error)]
pub enum DirectoryError {
    /// Network/TLS problem.
    #[error("network error: {0}")]
    Network(#[from] HttpsError),
    /// The server answered with an error message meant for the user (often localized).
    #[error("{0}")]
    Server(String),
    /// Unexpected HTTP status.
    #[error("unexpected HTTP status {0}")]
    Status(u16),
    /// Malformed response.
    #[error("invalid response: {0}")]
    InvalidResponse(String),
}

impl From<serde_json::Error> for DirectoryError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidResponse(error.to_string())
    }
}

/// Information about the device/app sent with license checks and in the servers' handshakes.
pub struct ClientVersion {
    /// App version, e.g. `0.1.0`.
    pub app_version: String,
    /// e.g. `de`.
    pub language: String,
    /// e.g. `DE`.
    pub country: String,
    /// Device model, e.g. `Jolla C2`.
    pub model: String,
    /// OS version, e.g. `5.1.0.11`.
    pub os_version: String,
    /// CPU architecture, e.g. `aarch64`.
    pub arch: String,
}

impl ClientVersion {
    /// `<appVer>;<platform>;<lang>/<country>;<model>;<osver>` like `AppVersion.getFullVersionString`.
    pub fn full_version_string(&self) -> String {
        self.client_info().to_semicolon_separated()
    }

    /// Who we are to Threema's servers (see docs/protocol-notes.md).
    #[must_use]
    pub fn client_info(&self) -> ClientInfo {
        ClientInfo::Threesea {
            version: self.app_version.clone(),
            locale: format!("{}/{}", self.language, self.country),
            device_model: self.model.clone(),
            os_version: self.os_version.clone(),
        }
    }
}

/// Outcome of a successful license check.
#[derive(Debug, Default)]
pub struct LicenseOk {
    /// Optional message about an available update.
    pub update_message: Option<String>,
}

fn ensure_ok(
    result: Result<libthreema::https::HttpsResponse, HttpsError>,
) -> Result<Vec<u8>, DirectoryError> {
    let response = result?;
    match response.status {
        200 => Ok(response.body),
        429 => Err(DirectoryError::Server(
            "Too many requests, please try again later.".into(),
        )),
        status => Err(DirectoryError::Status(status)),
    }
}

#[derive(Deserialize)]
struct SuccessResponse {
    success: bool,
    #[serde(default)]
    error: Option<String>,
}

fn check_success(body: &[u8]) -> Result<(), DirectoryError> {
    let response: SuccessResponse = serde_json::from_slice(body)?;
    if response.success {
        Ok(())
    } else {
        Err(DirectoryError::Server(
            response
                .error
                .unwrap_or_else(|| "Unknown server error".into()),
        ))
    }
}

/// `POST check_license` (`APIConnector.checkLicense`).
pub async fn check_license(
    client: &HttpsClient,
    license_key: &str,
    device_id: &str,
    version: &ClientVersion,
) -> Result<LicenseOk, DirectoryError> {
    #[derive(Deserialize)]
    struct Response {
        #[serde(rename = "updateMessage", default)]
        update_message: Option<String>,
    }
    let body = ensure_ok(
        client
            .post_json(
                &format!("{DIRECTORY_URL}check_license"),
                &json!({
                    "licenseKey": license_key,
                    "deviceId": device_id,
                    "version": version.full_version_string(),
                    "arch": version.arch,
                }),
            )
            .await,
    )?;
    check_success(&body)?;
    let response: Response = serde_json::from_slice(&body)?;
    Ok(LicenseOk {
        update_message: response.update_message,
    })
}

/// Private identity data (`APIConnector.fetchIdentityPrivate`).
#[derive(Debug)]
pub struct PrivateIdentityData {
    /// Chat server group.
    pub server_group: String,
    /// Linked e-mail address, if any.
    pub email: Option<String>,
    /// Linked mobile number, if any.
    pub mobile_no: Option<String>,
}

#[derive(Deserialize)]
struct Challenge {
    #[serde(rename = "tokenRespKeyPub")]
    token_resp_key_pub: String,
    token: String,
}

/// Solve the challenge from a phase-1 response and add `token`/`response` to `request`.
fn answer_challenge(
    client_key: &ClientKey,
    phase1: &[u8],
    request: &mut serde_json::Value,
) -> Result<(), DirectoryError> {
    let challenge: Challenge = serde_json::from_slice(phase1)?;
    let bad = |_| DirectoryError::InvalidResponse("invalid challenge encoding".into());
    let public_key: [u8; PublicKey::LENGTH] = BASE64
        .decode(challenge.token_resp_key_pub.as_bytes())
        .map_err(bad)?
        .try_into()
        .map_err(|_| DirectoryError::InvalidResponse("invalid challenge key".into()))?;
    let token = BASE64.decode(challenge.token.as_bytes()).map_err(bad)?;
    let response = client_key
        .solve_directory_challenge(&PublicKey::from(public_key), &token)
        .ok_or_else(|| DirectoryError::InvalidResponse("non-contributory challenge key".into()))?;
    request["token"] = json!(challenge.token);
    request["response"] = json!(BASE64.encode(&response));
    Ok(())
}

/// `POST identity/fetch_priv`: needed after restoring an ID to learn its server group.
pub async fn fetch_identity_private(
    client: &HttpsClient,
    threema_id: &str,
    client_key: &ClientKey,
) -> Result<PrivateIdentityData, DirectoryError> {
    #[derive(Deserialize)]
    struct Response {
        #[serde(rename = "serverGroup")]
        server_group: String,
        #[serde(default)]
        email: Option<String>,
        #[serde(rename = "mobileNo", default)]
        mobile_no: Option<String>,
    }
    let url = format!("{DIRECTORY_URL}identity/fetch_priv");
    let mut request = json!({ "identity": threema_id, "appVariant": "consumer" });

    let phase1 = ensure_ok(client.post_json(&url, &request).await)?;
    let maybe_error: serde_json::Value = serde_json::from_slice(&phase1)?;
    if maybe_error.get("success") == Some(&json!(false)) {
        check_success(&phase1)?;
    }
    answer_challenge(client_key, &phase1, &mut request)?;

    let phase2 = ensure_ok(client.post_json(&url, &request).await)?;
    check_success(&phase2)?;
    let response: Response = serde_json::from_slice(&phase2)?;
    Ok(PrivateIdentityData {
        server_group: response.server_group,
        email: response.email.filter(|s| !s.is_empty()),
        mobile_no: response.mobile_no.filter(|s| !s.is_empty()),
    })
}

/// `POST identity/set_featuremask` (`APIConnector.setFeatureMask`): announce what this client
/// supports. Other clients use it to decide e.g. whether to use forward security with us.
pub async fn set_feature_mask(
    client: &HttpsClient,
    threema_id: &str,
    client_key: &ClientKey,
    feature_mask: u64,
) -> Result<(), DirectoryError> {
    let url = format!("{DIRECTORY_URL}identity/set_featuremask");
    let mut request = json!({ "identity": threema_id, "featureMask": feature_mask });
    let phase1 = ensure_ok(client.post_json(&url, &request).await)?;
    answer_challenge(client_key, &phase1, &mut request)?;
    let phase2 = ensure_ok(client.post_json(&url, &request).await)?;
    check_success(&phase2)
}

/// Phase 1 of a link request: `linked: true` (already linked to this ID, nothing to verify) or
/// a challenge; anything else carries the server's error.
fn link_phase1(body: &[u8]) -> Result<bool, DirectoryError> {
    #[derive(Deserialize)]
    struct Phase1 {
        linked: Option<bool>,
        error: Option<String>,
    }
    let phase1: Phase1 = serde_json::from_slice(body)?;
    phase1.linked.ok_or_else(|| {
        DirectoryError::Server(
            phase1
                .error
                .unwrap_or_else(|| "Unknown server error".into()),
        )
    })
}

/// `POST identity/link_email` (`APIConnector.linkEmail`): link `email` (empty: unlink). The
/// address itself goes to Threema, which mails a confirmation link. Returns whether that is
/// pending (`false`: already linked).
pub async fn link_email(
    client: &HttpsClient,
    threema_id: &str,
    client_key: &ClientKey,
    email: &str,
    language: &str,
) -> Result<bool, DirectoryError> {
    let url = format!("{DIRECTORY_URL}identity/link_email");
    let mut request = json!({ "identity": threema_id, "email": email, "lang": language });
    let phase1 = ensure_ok(client.post_json(&url, &request).await)?;
    if link_phase1(&phase1)? {
        return Ok(false);
    }
    answer_challenge(client_key, &phase1, &mut request)?;
    let phase2 = ensure_ok(client.post_json(&url, &request).await)?;
    check_success(&phase2)?;
    Ok(true)
}

/// Whether `email` is linked (confirmed) by now (`APIConnector.linkEmailCheckStatus`).
pub async fn email_linked(
    client: &HttpsClient,
    threema_id: &str,
    email: &str,
) -> Result<bool, DirectoryError> {
    let url = format!("{DIRECTORY_URL}identity/link_email");
    let request = json!({ "identity": threema_id, "email": email });
    link_phase1(&ensure_ok(client.post_json(&url, &request).await)?)
}

/// `POST identity/link_mobileno` (`APIConnector.linkMobileNo`): link `mobile_no` (E.164 without
/// `+`; empty: unlink). Threema sends an SMS with a code. Returns the verification id, or `None`
/// if it is linked already (or was unlinked).
pub async fn link_mobile(
    client: &HttpsClient,
    threema_id: &str,
    client_key: &ClientKey,
    mobile_no: &str,
    language: &str,
) -> Result<Option<String>, DirectoryError> {
    #[derive(Deserialize)]
    struct Phase2 {
        #[serde(rename = "verificationId")]
        verification_id: Option<String>,
    }
    let url = format!("{DIRECTORY_URL}identity/link_mobileno");
    let mut request = json!({
        "identity": threema_id, "mobileNo": mobile_no, "lang": language, "httpsUrl": true,
    });
    let phase1 = ensure_ok(client.post_json(&url, &request).await)?;
    if link_phase1(&phase1)? {
        return Ok(None);
    }
    answer_challenge(client_key, &phase1, &mut request)?;
    let phase2 = ensure_ok(client.post_json(&url, &request).await)?;
    check_success(&phase2)?;
    if mobile_no.is_empty() {
        return Ok(None);
    }
    let phase2: Phase2 = serde_json::from_slice(&phase2)?;
    phase2
        .verification_id
        .map(Some)
        .ok_or_else(|| DirectoryError::InvalidResponse("no verification id".into()))
}

/// The SMS code for a pending phone link (`APIConnector.linkMobileNoVerify`).
pub async fn verify_mobile(
    client: &HttpsClient,
    verification_id: &str,
    code: &str,
) -> Result<(), DirectoryError> {
    let url = format!("{DIRECTORY_URL}identity/link_mobileno");
    let request = json!({ "verificationId": verification_id, "code": code });
    check_success(&ensure_ok(client.post_json(&url, &request).await)?)
}

/// Ask for a phone call that reads the code instead (`APIConnector.linkMobileNoCall`).
pub async fn call_mobile(
    client: &HttpsClient,
    verification_id: &str,
) -> Result<(), DirectoryError> {
    let url = format!("{DIRECTORY_URL}identity/link_mobileno_call");
    let request = json!({ "verificationId": verification_id });
    check_success(&ensure_ok(client.post_json(&url, &request).await)?)
}

/// The revocation key the server stores: base64 of the first 4 bytes of SHA-256 of the UTF-8
/// password (`APIConnector.setRevocationKey`). The password itself never leaves the device.
pub fn revocation_key(password: &str) -> String {
    use sha2::Digest as _;
    BASE64.encode(&sha2::Sha256::digest(password.as_bytes())[..4])
}

/// `POST identity/set_revocation_key`: lets the user revoke the ID at myid.threema.ch/revoke with
/// this password.
pub async fn set_revocation_key(
    client: &HttpsClient,
    threema_id: &str,
    client_key: &ClientKey,
    password: &str,
) -> Result<(), DirectoryError> {
    let url = format!("{DIRECTORY_URL}identity/set_revocation_key");
    let mut request = json!({ "identity": threema_id, "revocationKey": revocation_key(password) });
    let phase1 = ensure_ok(client.post_json(&url, &request).await)?;
    answer_challenge(client_key, &phase1, &mut request)?;
    let phase2 = ensure_ok(client.post_json(&url, &request).await)?;
    check_success(&phase2)
}

/// The request of `identity/report_identity`: who reports whom. Threema's servers want the nickname
/// as an explicit null when there is none. No content of any message.
fn report_identity_request(reporter: &str, peer: &str, nickname: Option<&str>) -> Value {
    json!({
        "reporter_identity": reporter,
        "peer_identity": peer,
        "peer_nickname": nickname.filter(|name| !name.is_empty()),
    })
}

/// The request of `identity/report_group`: who reports which group of whom. The group's name stays
/// on this device.
fn report_group_request(
    reporter: &str,
    creator: &str,
    nickname: Option<&str>,
    group_id: u64,
    member_count: usize,
) -> Value {
    json!({
        "reporter_identity": reporter,
        "group_creator_identity": creator,
        "group_creator_nickname": nickname.filter(|name| !name.is_empty()),
        "group_name": Value::Null,
        "group_id": group_id.to_string(),
        "group_member_count": member_count,
    })
}

/// These two endpoints answer `{"status": "success"}`, not `{"success": true}`.
fn check_report_status(body: &[u8]) -> Result<(), DirectoryError> {
    #[derive(Deserialize)]
    struct Response {
        status: Option<String>,
    }
    let response: Response = serde_json::from_slice(body)?;
    if response.status.as_deref() == Some("success") {
        Ok(())
    } else {
        Err(DirectoryError::Server(
            "The report was not accepted.".into(),
        ))
    }
}

async fn send_report(
    client: &HttpsClient,
    client_key: &ClientKey,
    path: &str,
    mut request: Value,
) -> Result<(), DirectoryError> {
    let url = format!("{DIRECTORY_URL}{path}");
    let phase1 = ensure_ok(client.post_json(&url, &request).await)?;
    answer_challenge(client_key, &phase1, &mut request)?;
    let phase2 = ensure_ok(client.post_json(&url, &request).await)?;
    check_report_status(&phase2)
}

/// Report a contact as sending spam (iOS's `reportSpamForIdentity`).
pub async fn report_identity(
    client: &HttpsClient,
    client_key: &ClientKey,
    reporter: &str,
    peer: &str,
    nickname: Option<&str>,
) -> Result<(), DirectoryError> {
    let request = report_identity_request(reporter, peer, nickname);
    send_report(client, client_key, "identity/report_identity", request).await
}

/// Report a group as spam (iOS's `reportSpamForGroupWithGroupID`).
pub async fn report_group(
    client: &HttpsClient,
    client_key: &ClientKey,
    reporter: &str,
    creator: &str,
    nickname: Option<&str>,
    group_id: u64,
    member_count: usize,
) -> Result<(), DirectoryError> {
    let request = report_group_request(reporter, creator, nickname, group_id, member_count);
    send_report(client, client_key, "identity/report_group", request).await
}

/// TURN servers for calls (`type` `voip`) or Threema Web (`web`): `identity/turn_cred`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnServers {
    pub turn_urls: Vec<String>,
    pub turn_urls_dual_stack: Vec<String>,
    pub turn_username: String,
    pub turn_password: String,
    /// Seconds they stay valid.
    pub expiration: u64,
}

/// `POST identity/turn_cred` (Android's `obtainTurnServers`).
pub async fn turn_servers(
    client: &HttpsClient,
    threema_id: &str,
    client_key: &ClientKey,
    kind: &str,
) -> Result<TurnServers, DirectoryError> {
    let url = format!("{DIRECTORY_URL}identity/turn_cred");
    let mut request = json!({ "identity": threema_id, "type": kind });
    let phase1 = ensure_ok(client.post_json(&url, &request).await)?;
    answer_challenge(client_key, &phase1, &mut request)?;
    let phase2 = ensure_ok(client.post_json(&url, &request).await)?;
    check_success(&phase2)?;
    Ok(serde_json::from_slice(&phase2)?)
}

/// `POST identity/sfu_cred`: the SFU for group calls (Android's `obtainSfuToken`).
pub async fn sfu_credentials(
    client: &HttpsClient,
    threema_id: &str,
    client_key: &ClientKey,
) -> Result<crate::group_calls::SfuCredentials, DirectoryError> {
    let url = format!("{DIRECTORY_URL}identity/sfu_cred");
    let mut request = json!({ "identity": threema_id });
    let phase1 = ensure_ok(client.post_json(&url, &request).await)?;
    answer_challenge(client_key, &phase1, &mut request)?;
    let phase2 = ensure_ok(client.post_json(&url, &request).await)?;
    check_success(&phase2)?;
    Ok(serde_json::from_slice(&phase2)?)
}

/// `POST identity/check_revocation_key`: when a revocation password was set (ms), or `None`.
pub async fn check_revocation_key(
    client: &HttpsClient,
    threema_id: &str,
    client_key: &ClientKey,
) -> Result<Option<i64>, DirectoryError> {
    #[derive(Deserialize)]
    struct Response {
        #[serde(rename = "revocationKeySet")]
        set: bool,
        #[serde(rename = "lastChanged", default)]
        last_changed: Option<String>,
    }
    let url = format!("{DIRECTORY_URL}identity/check_revocation_key");
    let mut request = json!({ "identity": threema_id });
    let phase1 = ensure_ok(client.post_json(&url, &request).await)?;
    answer_challenge(client_key, &phase1, &mut request)?;
    let phase2 = ensure_ok(client.post_json(&url, &request).await)?;
    let response: Response = serde_json::from_slice(&phase2)?;
    if !response.set {
        return Ok(None);
    }
    // Set, but an unreadable date: still set (shown without the date).
    Ok(Some(
        response
            .last_changed
            .as_deref()
            .and_then(parse_timestamp)
            .unwrap_or(0),
    ))
}

/// `yyyy-MM-ddTHH:mm:ss` followed by `Z` or `±HHMM` / `±HH:MM` (the directory's dates) as ms.
fn parse_timestamp(text: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let zone = text.get(19..)?.replace(':', "");
    let offset_minutes = match zone.as_str() {
        "" | "Z" => 0,
        _ => {
            let sign = match zone.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let hhmm: i64 = zone.get(1..5)?.parse().ok()?;
            sign * (hhmm / 100 * 60 + hhmm % 100)
        }
    };
    // Days since 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_minutes * 60;
    Some(seconds * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_phase1_answers() {
        assert!(link_phase1(br#"{"linked":true}"#).unwrap());
        assert!(!link_phase1(br#"{"linked":false,"token":"x","tokenRespKeyPub":"y"}"#).unwrap());
        assert!(matches!(
            link_phase1(br#"{"success":false,"error":"Invalid e-mail address"}"#),
            Err(DirectoryError::Server(message)) if message == "Invalid e-mail address"
        ));
    }

    #[test]
    fn revocation_key_and_dates() {
        // SHA-256("abc") = ba7816bf…
        assert_eq!(
            revocation_key("abc"),
            BASE64.encode(&[0xba, 0x78, 0x16, 0xbf])
        );
        assert_eq!(parse_timestamp("1970-01-01T00:00:00+0000"), Some(0));
        assert_eq!(
            parse_timestamp("2026-10-22T15:00:00Z"),
            Some(1_792_681_200_000)
        );
        assert_eq!(
            parse_timestamp("2026-10-22T17:00:00+0200"),
            parse_timestamp("2026-10-22T15:00:00Z")
        );
        assert_eq!(
            parse_timestamp("2000-03-01T00:00:00-01:30"),
            Some((951_868_800 + 5400) * 1000)
        );
        assert_eq!(parse_timestamp("2026-10-22"), None);
    }

    #[test]
    fn report_requests() {
        assert_eq!(
            report_identity_request("ABCD1234", "EFGH5678", Some("Spammer")),
            json!({"reporter_identity": "ABCD1234", "peer_identity": "EFGH5678",
                   "peer_nickname": "Spammer"})
        );
        // No nickname: an explicit null, empty counts as none.
        for nickname in [None, Some("")] {
            let request = report_identity_request("ABCD1234", "EFGH5678", nickname);
            assert_eq!(request["peer_nickname"], Value::Null);
            assert!(request.as_object().unwrap().contains_key("peer_nickname"));
        }
        // The group's ID in decimal, its name never.
        assert_eq!(
            report_group_request("ABCD1234", "EFGH5678", None, u64::MAX, 4),
            json!({"reporter_identity": "ABCD1234", "group_creator_identity": "EFGH5678",
                   "group_creator_nickname": null, "group_name": null,
                   "group_id": "18446744073709551615", "group_member_count": 4})
        );
    }

    #[test]
    fn report_answers() {
        assert!(check_report_status(br#"{"status":"success"}"#).is_ok());
        for body in [&br#"{"status":"error"}"#[..], br"{}"] {
            assert!(matches!(
                check_report_status(body),
                Err(DirectoryError::Server(_))
            ));
        }
        assert!(matches!(
            check_report_status(b"nope"),
            Err(DirectoryError::InvalidResponse(_))
        ));
    }

    #[test]
    fn version_string_format() {
        let v = ClientVersion {
            app_version: "0.1.0".into(),
            language: "de".into(),
            country: "CH".into(),
            model: "Jolla;C2".into(),
            os_version: "5.1.0.11".into(),
            arch: "aarch64".into(),
        };
        assert_eq!(
            v.full_version_string(),
            "Threesea/0.1.0;S;de/CH;Jolla_C2;5.1.0.11"
        );
    }

    #[test]
    fn challenge_answer_adds_token_and_response() {
        let client_key = ClientKey::from([7; 32]);
        let server_key = ClientKey::from([9; 32]).public_key();
        let phase1 = br#"{"tokenRespKeyPub":"!!!"}"#;
        // Malformed challenge is rejected, not panicking.
        let mut request = json!({});
        assert!(answer_challenge(&client_key, phase1, &mut request).is_err());

        let token = BASE64.encode(b"token");
        let phase1 = serde_json::to_vec(&json!({
            "tokenRespKeyPub": BASE64.encode(&hex32(&server_key.to_string())),
            "token": token,
        }))
        .unwrap();
        let mut request = json!({ "identity": "ECHOECHO" });
        answer_challenge(&client_key, &phase1, &mut request).unwrap();
        assert_eq!(request["token"], token);
        let expected = client_key
            .solve_directory_challenge(&server_key, b"token")
            .unwrap();
        assert_eq!(request["response"], BASE64.encode(&expected));
    }

    fn hex32(hex: &str) -> [u8; 32] {
        data_encoding::HEXLOWER
            .decode(hex.as_bytes())
            .unwrap()
            .try_into()
            .unwrap()
    }

    #[test]
    fn server_error_message_is_surfaced() {
        let err = check_success(br#"{"success":false,"error":"License key invalid"}"#).unwrap_err();
        assert_eq!(err.to_string(), "License key invalid");
        assert!(check_success(br#"{"success":true}"#).is_ok());
    }
}

/// Live tests against the real Threema servers; run with `cargo test -- --ignored`.
#[cfg(test)]
mod live_tests {
    use super::*;

    /// Never gets past phase 1: `EFGH5678` doesn't exist, which the server only says after accepting
    /// the path and every field of the request and finding the reporter (a public ID). Without an
    /// answer to a challenge nothing could be reported anyway.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "contacts the real Threema directory server"]
    async fn report_endpoints_know_the_request() {
        let client = HttpsClient::new().unwrap();
        for (path, request) in [
            (
                "identity/report_identity",
                report_identity_request("ECHOECHO", "EFGH5678", None),
            ),
            (
                "identity/report_group",
                report_group_request("ECHOECHO", "EFGH5678", None, 1, 3),
            ),
        ] {
            let url = format!("{DIRECTORY_URL}{path}");
            let error = ensure_ok(client.post_json(&url, &request).await).unwrap_err();
            assert!(
                matches!(error, DirectoryError::Status(404)),
                "{path}: {error:?}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "contacts the real Threema directory server"]
    async fn bogus_license_is_rejected_by_server_over_pinned_tls() {
        let client = HttpsClient::new().unwrap();
        let version = ClientVersion {
            app_version: crate::VERSION.into(),
            language: "en".into(),
            country: "US".into(),
            model: "test".into(),
            os_version: "test".into(),
            arch: std::env::consts::ARCH.into(),
        };
        let error = check_license(
            &client,
            "AAAAA-AAAAA",
            "00000000-0000-4000-8000-000000000000",
            &version,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, DirectoryError::Server(_)), "{error:?}");
        eprintln!("server said: {error}");
    }
}

/// `POST identity/match_token`: a token authenticating `identity/match` requests.
pub async fn match_token(
    client: &HttpsClient,
    threema_id: &str,
    client_key: &ClientKey,
) -> Result<String, DirectoryError> {
    let url = format!("{DIRECTORY_URL}identity/match_token");
    let mut request = json!({ "identity": threema_id });
    let phase1 = ensure_ok(client.post_json(&url, &request).await)?;
    answer_challenge(client_key, &phase1, &mut request)?;
    let phase2 = ensure_ok(client.post_json(&url, &request).await)?;
    check_success(&phase2)?;
    let value: serde_json::Value = serde_json::from_slice(&phase2)?;
    value["matchToken"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| DirectoryError::InvalidResponse("no match token".into()))
}

/// One identity found by `identity/match`.
#[derive(Debug, Clone)]
pub struct MatchedIdentity {
    pub identity: String,
    pub public_key: [u8; 32],
    pub email_hash: Option<String>,
    pub mobile_hash: Option<String>,
}

/// `POST identity/match` with hashed e-mail addresses and phone numbers (never in clear text).
pub async fn match_identities(
    client: &HttpsClient,
    token: &str,
    email_hashes: &[String],
    mobile_hashes: &[String],
) -> Result<Vec<MatchedIdentity>, DirectoryError> {
    let url = format!("{DIRECTORY_URL}identity/match");
    let request = json!({
        "matchToken": token,
        "emailHashes": email_hashes,
        "mobileNoHashes": mobile_hashes,
    });
    let body = ensure_ok(client.post_json(&url, &request).await)?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    let identities = value["identities"].as_array().cloned().unwrap_or_default();
    Ok(identities
        .iter()
        .filter_map(|entry| {
            let key: [u8; 32] = BASE64
                .decode(entry["publicKey"].as_str()?.as_bytes())
                .ok()?
                .try_into()
                .ok()?;
            Some(MatchedIdentity {
                identity: entry["identity"].as_str()?.to_owned(),
                public_key: key,
                email_hash: entry["emailHash"].as_str().map(str::to_owned),
                mobile_hash: entry["mobileNoHash"].as_str().map(str::to_owned),
            })
        })
        .collect())
}
