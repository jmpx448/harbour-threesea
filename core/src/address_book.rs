// SPDX-License-Identifier: AGPL-3.0-only
//! Address book matching (opt-in), like Threema Android's `SynchronizeContactsRoutine`: e-mail
//! addresses and phone numbers from the device's address book are normalised and hashed
//! (HMAC-SHA256 with Threema's fixed keys); only the hashes go to the directory, which answers
//! with the Threema IDs linked to them. Found people become contacts named as in the address
//! book, server verified (two dots).

use std::collections::HashMap;

use data_encoding::BASE64;
use hmac::{Hmac, Mac as _};
use sha2::Sha256;

/// HMAC keys from Threema Android (`APIConnector.EMAIL_HMAC_KEY` / `MOBILENO_HMAC_KEY`).
const EMAIL_HMAC_KEY: [u8; 32] = [
    0x30, 0xa5, 0x50, 0x0f, 0xed, 0x97, 0x01, 0xfa, 0x6d, 0xef, 0xdb, 0x61, 0x08, 0x41, 0x90, 0x0f,
    0xeb, 0xb8, 0xe4, 0x30, 0x88, 0x1f, 0x7a, 0xd8, 0x16, 0x82, 0x62, 0x64, 0xec, 0x09, 0xba, 0xd7,
];
const MOBILE_HMAC_KEY: [u8; 32] = [
    0x85, 0xad, 0xf8, 0x22, 0x69, 0x53, 0xf3, 0xd9, 0x6c, 0xfd, 0x5d, 0x09, 0xbf, 0x29, 0x55, 0x5e,
    0xb9, 0x55, 0xfc, 0xd8, 0xaa, 0x5e, 0xc4, 0xf9, 0xfc, 0xd8, 0x69, 0xe2, 0x58, 0x37, 0x07, 0x23,
];

/// An address book entry (from the bridge).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: String,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub phones: Vec<String>,
}

fn hmac(key: &[u8; 32], data: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data.as_bytes());
    BASE64.encode(&mac.finalize().into_bytes())
}

/// Hashes of an e-mail address (also the other Gmail domain, like Android).
pub fn email_hashes(email: &str) -> Vec<String> {
    let normalized = email.trim().to_lowercase();
    if normalized.is_empty() || !normalized.contains('@') {
        return Vec::new();
    }
    let mut hashes = vec![hmac(&EMAIL_HMAC_KEY, &normalized)];
    let alternative = if let Some(user) = normalized.strip_suffix("@gmail.com") {
        Some(format!("{user}@googlemail.com"))
    } else {
        normalized
            .strip_suffix("@googlemail.com")
            .map(|user| format!("{user}@gmail.com"))
    };
    hashes.extend(alternative.map(|a| hmac(&EMAIL_HMAC_KEY, &a)));
    hashes
}

/// Calling codes for national numbers (without international prefix) by region; unknown
/// regions send the digits as they are (Android uses libphonenumber's E.164 formatting).
fn calling_code(country: &str) -> Option<&'static str> {
    Some(match country.to_ascii_uppercase().as_str() {
        "LU" => "352",
        "DE" => "49",
        "AT" => "43",
        "CH" | "LI" => "41",
        "FR" => "33",
        "BE" => "32",
        "NL" => "31",
        "IT" => "39",
        "ES" => "34",
        "PT" => "351",
        "GB" => "44",
        "IE" => "353",
        "DK" => "45",
        "SE" => "46",
        "NO" => "47",
        "FI" => "358",
        "PL" => "48",
        "CZ" => "420",
        "SK" => "421",
        "HU" => "36",
        "US" | "CA" => "1",
        _ => return None,
    })
}

/// `+49 (0) 171 …` is a common way to write a number: the zero is what one dials at home, it isn't
/// part of it. Other brackets are: `(415) 555-0100`.
fn without_bracketed_zero(number: &str) -> String {
    let mut kept = String::with_capacity(number.len());
    let mut rest = number;
    while let Some(open) = rest.find('(') {
        kept.push_str(&rest[..open]);
        let inside = &rest[open + 1..];
        let after_zero = inside
            .trim_start()
            .strip_prefix('0')
            .and_then(|after| after.trim_start().strip_prefix(')'));
        rest = after_zero.unwrap_or_else(|| {
            kept.push('(');
            inside
        });
    }
    kept.push_str(rest);
    kept
}

/// A phone number in E.164 without `+` (what Threema hashes), given the device's region.
pub fn normalize_phone(number: &str, country: &str) -> Option<String> {
    let trimmed = number.trim();
    let digits: String = without_bracketed_zero(trimmed)
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    if digits.len() < 5 {
        return None;
    }
    if trimmed.starts_with('+') {
        return Some(digits);
    }
    if let Some(international) = digits.strip_prefix("00") {
        return Some(international.to_owned());
    }
    match calling_code(country) {
        // North America: national numbers have no trunk prefix.
        Some("1") => Some(format!("1{digits}")),
        Some(code) => {
            let trunk_prefix = if code == "36" { "06" } else { "0" };
            Some(format!(
                "{code}{}",
                digits.strip_prefix(trunk_prefix).unwrap_or(&digits)
            ))
        }
        None => Some(digits),
    }
}

/// A number that says its own country (`+` or `00`), in E.164 without `+`; anything else is
/// `None`. For the user's own number, which goes to Threema in plain and gets an SMS: a national
/// number would need a country, and the device's language says nothing about the SIM.
pub fn international_phone(number: &str) -> Option<String> {
    let trimmed = number.trim();
    if !trimmed.starts_with('+') && !trimmed.starts_with("00") {
        return None;
    }
    normalize_phone(trimmed, "")
        .filter(|digits| (6..=15).contains(&digits.len()) && !digits.starts_with('0'))
}

pub fn phone_hash(normalized: &str) -> String {
    hmac(&MOBILE_HMAC_KEY, normalized)
}

/// All hashes of the entries, each mapped to the entry's index.
pub fn hash_entries(
    entries: &[Entry],
    country: &str,
) -> (HashMap<String, usize>, HashMap<String, usize>) {
    let mut emails = HashMap::new();
    let mut phones = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        for email in &entry.emails {
            for hash in email_hashes(email) {
                emails.insert(hash, index);
            }
        }
        for phone in &entry.phones {
            if let Some(normalized) = normalize_phone(phone, country) {
                phones.insert(phone_hash(&normalized), index);
            }
        }
    }
    (emails, phones)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation() {
        assert_eq!(
            normalize_phone("+352 621 123 456", "DE").as_deref(),
            Some("352621123456")
        );
        assert_eq!(
            normalize_phone("0043 6641234567", "DE").as_deref(),
            Some("436641234567")
        );
        assert_eq!(
            normalize_phone("0171 1234567", "DE").as_deref(),
            Some("491711234567")
        );
        assert_eq!(
            normalize_phone("0664 123 4567", "AT").as_deref(),
            Some("436641234567")
        );
        assert_eq!(
            normalize_phone("(415) 555-0100", "US").as_deref(),
            Some("14155550100")
        );
        assert_eq!(normalize_phone("112", "DE"), None);
        assert_eq!(email_hashes("  Someone@GMAIL.com ").len(), 2);
        assert_eq!(email_hashes("not an address"), Vec::<String>::new());
        assert_eq!(email_hashes("a@b.ch"), email_hashes("A@B.CH"));
    }

    #[test]
    fn a_bracketed_zero_is_not_part_of_the_number() {
        for number in [
            "+49 (0) 171 1234567",
            "+49 ( 0 ) 171 1234567",
            "+49(0)1711234567",
        ] {
            assert_eq!(
                normalize_phone(number, "").as_deref(),
                Some("491711234567"),
                "{number}"
            );
        }
        assert_eq!(
            normalize_phone("0049 (0) 171 1234567", "").as_deref(),
            Some("491711234567")
        );
        assert_eq!(
            normalize_phone("(0) 171 1234567", "DE").as_deref(),
            Some("491711234567")
        );
        // Other brackets are part of the number.
        assert_eq!(
            normalize_phone("(0171) 1234567", "DE").as_deref(),
            Some("491711234567")
        );
        assert_eq!(
            normalize_phone("+1 (415) 555-0100", "").as_deref(),
            Some("14155550100")
        );
        assert_eq!(
            normalize_phone("+1 (800) 555-0100", "").as_deref(),
            Some("18005550100")
        );
    }

    #[test]
    fn hungary_has_a_two_digit_national_prefix() {
        assert_eq!(
            normalize_phone("06 20 123 4567", "HU").as_deref(),
            Some("36201234567")
        );
        assert_eq!(
            normalize_phone("20 123 4567", "HU").as_deref(),
            Some("36201234567")
        );
    }

    #[test]
    fn international_numbers_only() {
        assert_eq!(
            international_phone(" +41 79 123 45 67 ").as_deref(),
            Some("41791234567")
        );
        assert_eq!(
            international_phone("0041 79 123 45 67").as_deref(),
            Some("41791234567")
        );
        assert_eq!(
            international_phone("+49 (0) 171 1234567").as_deref(),
            Some("491711234567")
        );
        // National: no country to go by.
        assert_eq!(international_phone("0664 1234567"), None);
        assert_eq!(international_phone("079 123 45 67"), None);
        assert_eq!(international_phone("(415) 555-0100"), None);
        // No calling code starts with a zero.
        assert_eq!(international_phone("+0 79 123 45 67"), None);
        assert_eq!(international_phone("000 79 123 45 67"), None);
        // E.164's limits.
        assert_eq!(international_phone("+41 123"), None);
        assert_eq!(international_phone("+41 79123456789012"), None);
        assert_eq!(international_phone(""), None);
    }

    #[test]
    fn hmac_is_sha256_with_threemas_key() {
        // 32-byte HMAC-SHA256, base64 (44 chars), stable.
        let hash = phone_hash("41791234567");
        assert_eq!(hash.len(), 44);
        assert_eq!(hash, phone_hash("41791234567"));
        assert_ne!(hash, hmac(&EMAIL_HMAC_KEY, "41791234567"));
    }
}
