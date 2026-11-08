// SPDX-License-Identifier: AGPL-3.0-only
//! Threema links, as the Android app handles them (`AndroidManifest.xml`, `AppLinksActivity`,
//! `RecipientListBaseActivity`, `AddContactActivity`, `EnterSerialActivity`):
//!
//! - `https://threema.id/<ID>[?text=…]`: add the contact, then open the chat (with the text) or
//!   the contact.
//! - `threema://compose?id=<ID>&text=…`, `https://go.threema.ch/compose?…`,
//!   `https://threema.id/compose?…`: write to someone (no `id`: choose the chat).
//! - `threema://add?id=<ID>`, `https://go.threema.ch/add?id=<ID>`: add a contact.
//! - `threema://license?key=…`, `https://go.threema.ch/license?key=…`: license key (setup only).
//! - `threema://link_mobileno?code=…`: the code of a pending phone link, from Threema's SMS
//!   (`SMSVerificationLinkActivity`, `VerifyMobileNumberUseCase`).

use serde_json::{Value, json};
use url::Url;

use crate::contacts;

const CONTACT_HOST: &str = "threema.id";
const ACTION_HOST: &str = "go.threema.ch";
const SCHEME: &str = "threema";

#[derive(Debug, PartialEq, Eq)]
pub enum Link {
    /// `threema.id/<ID>`.
    Contact {
        identity: String,
        text: Option<String>,
    },
    Compose {
        identity: Option<String>,
        text: Option<String>,
    },
    Add {
        identity: String,
    },
    License {
        key: String,
    },
    VerifyMobile {
        code: String,
    },
}

/// `VerifyMobileNumberUseCase.isValidCodeFormat`.
const MAX_MOBILE_CODE_DIGITS: usize = 8;

fn mobile_code(code: &str) -> Option<String> {
    let valid = code.len() <= MAX_MOBILE_CODE_DIGITS && code.bytes().all(|b| b.is_ascii_digit());
    valid.then(|| code.to_owned())
}

impl Link {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Contact { identity, text } => {
                json!({ "action": "contact", "identity": identity, "text": text })
            }
            Self::Compose { identity, text } => {
                json!({ "action": "compose", "identity": identity, "text": text })
            }
            Self::Add { identity } => json!({ "action": "add", "identity": identity }),
            Self::License { key } => json!({ "action": "license", "key": key }),
            Self::VerifyMobile { code } => json!({ "action": "verifyMobile", "code": code }),
        }
    }
}

pub fn parse(url: &str) -> Option<Link> {
    let url = Url::parse(url.trim()).ok()?;
    let query = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let identity = |value: &str| contacts::parse_identity(value).map(|id| id.to_string());
    let host = url.host_str()?.to_ascii_lowercase();
    let path = url.path().trim_end_matches('/');
    // threema://<action>?… has the action as the host; https://go.threema.ch/<action>?… as
    // the path.
    let action = match url.scheme() {
        SCHEME if path.is_empty() => host.as_str(),
        "https" if host == ACTION_HOST => path.trim_start_matches('/'),
        "https" if host == CONTACT_HOST => {
            let segment = path.trim_start_matches('/');
            if segment.eq_ignore_ascii_case("compose") {
                "compose"
            } else {
                return Some(Link::Contact {
                    identity: identity(segment)?,
                    text: query("text"),
                });
            }
        }
        _ => return None,
    };
    match action {
        "compose" => Some(Link::Compose {
            // An invalid ID makes the link invalid (it would write to nobody).
            identity: match query("id") {
                Some(id) => Some(identity(&id)?),
                None => None,
            },
            text: query("text"),
        }),
        "add" => Some(Link::Add {
            identity: identity(&query("id")?)?,
        }),
        "license" => Some(Link::License { key: query("key")? }),
        // Only as Threema's own scheme, as in Android's manifest.
        "link_mobileno" if url.scheme() == SCHEME => Some(Link::VerifyMobile {
            code: mobile_code(&query("code")?)?,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_links() {
        assert_eq!(
            parse("https://threema.id/ABCD1234"),
            Some(Link::Contact {
                identity: "ABCD1234".into(),
                text: None
            })
        );
        assert_eq!(
            parse("https://threema.id/abcd1234/?text=Hello%20there%21"),
            Some(Link::Contact {
                identity: "ABCD1234".into(),
                text: Some("Hello there!".into())
            })
        );
        assert_eq!(parse("https://threema.id/TOOLONGID"), None);
        assert_eq!(parse("https://threema.id/"), None);
        assert_eq!(parse("http://threema.id/ABCD1234"), None);
    }

    #[test]
    fn compose_links() {
        let expected = Link::Compose {
            identity: Some("ABCD1234".into()),
            text: Some("Hi + more".into()),
        };
        assert_eq!(
            parse("threema://compose?id=ABCD1234&text=Hi+%2B+more").as_ref(),
            Some(&expected)
        );
        assert_eq!(
            parse("https://go.threema.ch/compose?id=ABCD1234&text=Hi%20%2B%20more").as_ref(),
            Some(&expected)
        );
        assert_eq!(
            parse("https://threema.id/compose?text=Hello"),
            Some(Link::Compose {
                identity: None,
                text: Some("Hello".into())
            })
        );
        assert_eq!(parse("threema://compose?id=nope&text=x"), None);
    }

    #[test]
    fn phone_verification_links() {
        assert_eq!(
            parse("threema://link_mobileno?code=12345678"),
            Some(Link::VerifyMobile {
                code: "12345678".into()
            })
        );
        assert_eq!(
            parse("threema://link_mobileno?code=123456")
                .unwrap()
                .to_json()["action"],
            "verifyMobile"
        );
        assert_eq!(parse("threema://link_mobileno?code=123456789"), None);
        assert_eq!(parse("threema://link_mobileno?code=12a4"), None);
        assert_eq!(parse("threema://link_mobileno?code="), None);
        assert_eq!(parse("threema://link_mobileno"), None);
        assert_eq!(parse("https://go.threema.ch/link_mobileno?code=1234"), None);
    }

    #[test]
    fn add_and_license_links() {
        assert_eq!(
            parse("threema://add?id=ABCD1234"),
            Some(Link::Add {
                identity: "ABCD1234".into()
            })
        );
        assert_eq!(
            parse("https://go.threema.ch/add?id=ABCD1234"),
            Some(Link::Add {
                identity: "ABCD1234".into()
            })
        );
        assert_eq!(parse("threema://add"), None);
        assert_eq!(
            parse("threema://license?key=ABCDE-FGHIJ"),
            Some(Link::License {
                key: "ABCDE-FGHIJ".into()
            })
        );
        assert_eq!(parse("https://example.com/add?id=ABCD1234"), None);
        assert_eq!(parse("threema://unknown"), None);
        assert_eq!(parse("https://threema.ch/en"), None);
    }
}
