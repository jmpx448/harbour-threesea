// SPDX-License-Identifier: AGPL-3.0-only
//! The optional check for a newer release on GitHub (ADR 0014; a named exception to CLAUDE.md
//! §7.6). Only in builds made for GitHub (not a store build), only once the user agreed, at most
//! once a day: one request for the repository's latest release to `api.github.com`, nothing
//! about the user in it but the app version (User-Agent). GitHub sees the device's IP address.
//! Nothing is downloaded or installed; the UI links to the release page.

use std::{sync::Arc, time::Duration};

use serde_json::Value;

/// The repository whose releases are checked.
pub const REPOSITORY: &str = "penttimus/harbour-threesea";
/// The only host the check contacts.
const HOST: &str = "api.github.com";
/// A release's JSON is a few KiB; more is not read.
const MAX_RESPONSE: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Without a leading `v`, e.g. `0.2.0`.
    pub version: String,
    /// Its page on GitHub (release notes, the RPMs).
    pub url: String,
}

/// Whether `candidate` (a release tag like `v0.2.0`) is newer than `current` (`0.1.0`):
/// numeric parts compared in order, a missing part counting as 0; anything after `-` or `+`
/// (pre-release, build) is ignored. Tags that are no version are never newer.
#[must_use]
pub fn is_newer(current: &str, candidate: &str) -> bool {
    fn parts(version: &str) -> Option<Vec<u64>> {
        let version = version.trim().trim_start_matches(['v', 'V']);
        let core = version.split(['-', '+']).next()?;
        core.split('.').map(|part| part.parse().ok()).collect()
    }
    let (Some(current), Some(candidate)) = (parts(current), parts(candidate)) else {
        return false;
    };
    let length = current.len().max(candidate.len());
    let at = |list: &[u64], i: usize| list.get(i).copied().unwrap_or(0);
    (0..length)
        .map(|i| at(&candidate, i).cmp(&at(&current, i)))
        .find(|order| order.is_ne())
        .is_some_and(std::cmp::Ordering::is_gt)
}

/// The release in GitHub's `releases/latest` answer, if it is one of this repository's.
#[must_use]
pub fn parse_release(json: &Value) -> Option<Release> {
    let tag = json["tag_name"].as_str()?;
    let url = json["html_url"].as_str()?;
    // Only ever a page of this repository's releases is shown as a link.
    let prefix = format!("https://github.com/{REPOSITORY}/releases/");
    if !url.starts_with(&prefix) || json["draft"] == true || json["prerelease"] == true {
        return None;
    }
    Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_owned(),
        url: url.to_owned(),
    })
}

/// A client for `api.github.com` only: ordinary WebPKI certificate checks (GitHub's keys are
/// not pinned, unlike Threema's), HTTPS only, no redirects.
fn client() -> Result<reqwest::Client, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .user_agent(crate::https::user_agent())
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// The repository's latest release (`None`: there is none yet).
///
/// # Errors
/// Network or server errors, as text for the log (no user data in it).
pub async fn latest() -> Result<Option<Release>, String> {
    let url = format!("https://{HOST}/repos/{REPOSITORY}/releases/latest");
    let mut response = client()?
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    match response.status().as_u16() {
        200 => {}
        404 => return Ok(None),
        status => return Err(format!("status {status}")),
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| e.without_url().to_string())?
    {
        if body.len() + chunk.len() > MAX_RESPONSE {
            return Err("response too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    let json: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    Ok(parse_release(&json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.1.0", "v0.2.0"));
        assert!(is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.9.0", "v0.10.0"), "not as text");
        assert!(is_newer("0.1", "0.1.1"));
        assert!(
            is_newer("1.0.0", "v1.0.1-rc1"),
            "the pre-release part is ignored"
        );
        assert!(!is_newer("0.2.0", "v0.2.0"));
        assert!(!is_newer("0.2.0", "v0.1.9"));
        assert!(!is_newer("0.2.0", "latest"));
        assert!(!is_newer("0.2.0", ""));
    }

    #[test]
    fn only_this_repositorys_releases() {
        let release = json!({
            "tag_name": "v0.2.0",
            "html_url": format!("https://github.com/{REPOSITORY}/releases/tag/v0.2.0"),
            "draft": false,
            "prerelease": false,
        });
        assert_eq!(
            parse_release(&release),
            Some(Release {
                version: "0.2.0".into(),
                url: format!("https://github.com/{REPOSITORY}/releases/tag/v0.2.0"),
            })
        );
        let mut elsewhere = release.clone();
        elsewhere["html_url"] = "https://example.com/releases/v0.2.0".into();
        assert_eq!(parse_release(&elsewhere), None);
        let mut lookalike = release.clone();
        lookalike["html_url"] =
            format!("https://github.com/{REPOSITORY}-evil/releases/tag/v0.2.0").into();
        assert_eq!(parse_release(&lookalike), None);
        let mut draft = release;
        draft["draft"] = true.into();
        assert_eq!(parse_release(&draft), None);
        assert_eq!(parse_release(&json!({})), None);
    }
}
