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

/// A country whose rules for writing numbers we know.
pub struct Country {
    /// ISO 3166 region, what the locale says and the setting stores.
    pub region: &'static str,
    /// Its calling code, without `+`. They are prefix-free.
    pub code: &'static str,
    /// What is dialled before a national number at home and isn't part of it internationally.
    /// Italy has none: its landlines keep their `0` (`+39 06 …`).
    trunk: &'static str,
    /// What is dialled to call abroad from there, besides `00`; `x` is any digit.
    international: &'static [&'static str],
}

const fn country(region: &'static str, code: &'static str, trunk: &'static str) -> Country {
    Country {
        region,
        code,
        trunk,
        international: &[],
    }
}

/// Italy: a zero belongs to the number, also in brackets (`+39 (0)6 …`).
const ITALY: &str = "39";

impl Country {
    const fn calling_abroad_with(self, international: &'static [&'static str]) -> Self {
        Self {
            international,
            ..self
        }
    }

    /// `digits` without the prefix for calling abroad from this country, if they start with it.
    fn after_international_prefix<'a>(&self, digits: &'a str) -> Option<&'a str> {
        self.international.iter().find_map(|prefix| {
            let matches = digits.len() > prefix.len()
                && digits
                    .bytes()
                    .zip(prefix.bytes())
                    .all(|(digit, wanted)| wanted == b'x' || digit == wanted);
            matches.then(|| &digits[prefix.len()..])
        })
    }

    /// A national number with what is dialled before it at home taken off.
    fn national<'a>(&self, digits: &'a str) -> &'a str {
        // In Russia an 8 also starts real numbers, (812) …: only all 11 digits have the prefix.
        if self.code == "7" && digits.len() != 11 {
            return digits;
        }
        digits.strip_prefix(self.trunk).unwrap_or(digits)
    }
}

/// The countries the settings offer. Android formats numbers with libphonenumber; this is the part
/// of its rules we need. A country not listed isn't guessed at: only numbers that say their country
/// count there.
pub const COUNTRIES: &[Country] = &[
    country("AE", "971", "0"),
    country("AT", "43", "0"),
    country("AU", "61", "0").calling_abroad_with(&["0011", "0014", "0015", "0016", "0018", "0019"]),
    country("BE", "32", "0"),
    country("BG", "359", "0"),
    country("BR", "55", "0"),
    country("CA", "1", "1").calling_abroad_with(&["011"]),
    country("CH", "41", "0"),
    country("CN", "86", "0"),
    country("CY", "357", ""),
    country("CZ", "420", ""),
    country("DE", "49", "0"),
    country("DK", "45", ""),
    country("EE", "372", ""),
    country("ES", "34", ""),
    country("FI", "358", "0"),
    country("FR", "33", "0"),
    country("GB", "44", "0"),
    country("GR", "30", ""),
    country("HR", "385", "0"),
    country("HU", "36", "06"),
    country("IE", "353", "0"),
    country("IL", "972", "0"),
    country("IN", "91", "0"),
    country("IS", "354", ""),
    country("IT", "39", ""),
    country("JP", "81", "0").calling_abroad_with(&["010"]),
    country("KR", "82", "0"),
    country("LI", "423", "0"),
    country("LU", "352", ""),
    country("LV", "371", ""),
    country("MT", "356", ""),
    country("MX", "52", ""),
    country("NL", "31", "0"),
    country("NO", "47", ""),
    country("NZ", "64", "0"),
    country("PL", "48", ""),
    country("PT", "351", ""),
    country("RO", "40", "0"),
    country("RS", "381", "0"),
    country("RU", "7", "8").calling_abroad_with(&["810"]),
    country("SE", "46", "0"),
    country("SG", "65", "").calling_abroad_with(&["00x", "01x", "02x", "03x"]),
    country("SI", "386", "0"),
    country("SK", "421", "0"),
    country("TR", "90", "0"),
    country("UA", "380", "0"),
    country("US", "1", "1").calling_abroad_with(&["011"]),
    country("ZA", "27", "0"),
];

/// The country of a region code, in any case.
pub fn country_of(region: &str) -> Option<&'static Country> {
    COUNTRIES
        .iter()
        .find(|country| country.region.eq_ignore_ascii_case(region))
}

/// The country that "Automatic" stands for: that of the linked number, else the locale's.
pub fn automatic_country(linked: Option<&str>, locale: &str) -> Option<&'static Country> {
    linked
        .map(|number| {
            number
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>()
        })
        .and_then(|digits| country_calling(&digits))
        .or_else(|| country_of(locale))
}

/// The country whose calling code an international number (digits only) starts with.
fn country_calling(digits: &str) -> Option<&'static Country> {
    COUNTRIES
        .iter()
        .filter(|country| digits.starts_with(country.code))
        .max_by_key(|country| country.code.len())
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

/// The first digit of the digit sets address books use, each followed by the other nine.
const DIGIT_ZEROS: [u32; 18] = [
    0x0660, 0x06f0, 0x0966, 0x09e6, 0x0a66, 0x0ae6, 0x0b66, 0x0be6, 0x0c66, 0x0ce6, 0x0d66, 0x0e50,
    0x0ed0, 0x0f20, 0x1040, 0x17e0, 0x1810, 0xff10,
];

fn ascii_digit(c: char) -> Option<char> {
    let code = u32::from(c);
    DIGIT_ZEROS
        .iter()
        .find(|zero| (**zero..**zero + 10).contains(&code))
        .and_then(|zero| char::from_digit(code - zero, 10))
}

/// In plain characters: ASCII digits and `+`, and without what only says which way text runs.
/// Address books copy these from web pages and calling apps, the marks in front of the `+`
/// (`U+202A +49 … U+202C`).
fn plain_form(number: &str) -> String {
    number
        .chars()
        .filter_map(|c| match c {
            '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => None,
            '\u{ff0b}' => Some('+'),
            c => Some(ascii_digit(c).unwrap_or(c)),
        })
        .collect()
}

/// Without an extension or tones to dial after connecting (`+49 30 123456;12`, `… ext. 12`): they
/// start at the first sign or letter after a digit that can't be part of the number.
fn without_extension(number: &str) -> &str {
    let mut digit_seen = false;
    for (at, c) in number.char_indices() {
        if digit_seen && (matches!(c, ';' | ',' | '#' | '~') || c.is_alphabetic()) {
            return &number[..at];
        }
        digit_seen |= c.is_ascii_digit();
    }
    number
}

/// `+49 …`, or `(+49) …` as some write it.
fn starts_with_plus(number: &str) -> bool {
    number
        .trim_start_matches(|c: char| c == '(' || c.is_whitespace())
        .starts_with('+')
}

/// An international number without the national prefix someone wrote after the calling code,
/// `+49 0171 …`. Only `0` and Hungary's `06`, only countries known to have it: Italy keeps its
/// zero, and a country not listed stays as it is.
fn without_trunk_after_code(digits: &str) -> String {
    let Some(country) = country_calling(digits).filter(|c| c.trunk.starts_with('0')) else {
        return digits.to_owned();
    };
    let national = &digits[country.code.len()..];
    format!(
        "{}{}",
        country.code,
        national.strip_prefix(country.trunk).unwrap_or(national)
    )
}

fn only_digits(number: &str) -> String {
    number.chars().filter(char::is_ascii_digit).collect()
}

/// A number that says its country, without what says so: `+`, `00` or the country's own prefix for
/// calling abroad, which comes first as it can start with `00`.
fn international_part<'a>(
    digits: &'a str,
    has_plus: bool,
    country: Option<&Country>,
) -> Option<&'a str> {
    if has_plus {
        return Some(digits);
    }
    country
        .and_then(|country| country.after_international_prefix(digits))
        .or_else(|| digits.strip_prefix("00"))
}

/// A phone number in E.164 without `+` (what Threema hashes). `region` is the country of numbers
/// that don't say their own; without one those have no E.164 form and are `None`.
pub fn normalize_phone(number: &str, region: &str) -> Option<String> {
    let plain = plain_form(number);
    let trimmed = without_extension(plain.trim()).trim_end();
    let digits = only_digits(&without_bracketed_zero(trimmed));
    if digits.len() < 5 {
        return None;
    }
    let as_written = only_digits(trimmed);
    let has_plus = starts_with_plus(trimmed);
    let country = country_of(region);
    if let Some(italian) = international_part(&as_written, has_plus, country)
        .filter(|international| international.starts_with(ITALY))
    {
        return Some(italian.to_owned());
    }
    if let Some(international) = international_part(&digits, has_plus, country) {
        return Some(without_trunk_after_code(international));
    }
    let country = country?;
    if country.code == ITALY {
        return Some(format!("{}{as_written}", country.code));
    }
    Some(format!("{}{}", country.code, country.national(&digits)))
}

/// A number that says its own country (`+` or `00`), in E.164 without `+`; anything else is
/// `None`. For the user's own number, which goes to Threema in plain and gets an SMS: a national
/// number would need a country, and the device's language says nothing about the SIM.
pub fn international_phone(number: &str) -> Option<String> {
    let plain = plain_form(number);
    let trimmed = plain.trim();
    if !starts_with_plus(trimmed) && !trimmed.starts_with("00") {
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
    fn a_national_zero_after_the_calling_code_is_dropped() {
        for number in [
            "+49 0171 1234567",
            "+49 (0171) 1234567",
            "0049 0171 1234567",
            "(+49) 171 1234567",
            "(+49) 0171 1234567",
            "( +49 ) 171 1234567",
        ] {
            assert_eq!(
                normalize_phone(number, "AT").as_deref(),
                Some("491711234567"),
                "{number}"
            );
        }
        assert_eq!(
            normalize_phone("+36 06 20 123 4567", "").as_deref(),
            Some("36201234567")
        );
        // Only the prefix the country has.
        assert_eq!(
            normalize_phone("+36 0 20 123 4567", "").as_deref(),
            Some("360201234567")
        );
        // A country not listed stays as written.
        assert_eq!(
            normalize_phone("+370 0612 34567", "").as_deref(),
            Some("370061234567")
        );
    }

    #[test]
    fn italian_numbers_keep_their_zero() {
        for number in [
            "+39 06 1234 5678",
            "0039 06 1234 5678",
            "06 1234 5678",
            "+39 (0)6 1234 5678",
            "0039 (0) 6 1234 5678",
            "(0)6 1234 5678",
        ] {
            assert_eq!(
                normalize_phone(number, "IT").as_deref(),
                Some("390612345678"),
                "{number}"
            );
        }
        assert_eq!(
            normalize_phone("333 123 4567", "IT").as_deref(),
            Some("393331234567")
        );
        // From anywhere, also what the link dialog would show.
        assert_eq!(
            normalize_phone("+39 (0)6 1234 5678", "DE").as_deref(),
            Some("390612345678")
        );
        assert_eq!(
            international_phone("+39 (0)6 1234 5678").as_deref(),
            Some("390612345678")
        );
    }

    #[test]
    fn north_american_numbers_get_one_leading_1() {
        for region in ["US", "CA"] {
            for number in ["(415) 555-0100", "1 (415) 555-0100", "1-415-555-0100"] {
                assert_eq!(
                    normalize_phone(number, region).as_deref(),
                    Some("14155550100"),
                    "{number}"
                );
            }
        }
        assert_eq!(
            normalize_phone("+1 415 555 0100", "DE").as_deref(),
            Some("14155550100")
        );
    }

    #[test]
    fn countries_without_a_national_prefix() {
        assert_eq!(
            normalize_phone("621 123 456", "LU").as_deref(),
            Some("352621123456")
        );
        assert_eq!(
            normalize_phone("612 34 56", "LI").as_deref(),
            Some("4236123456")
        );
        // Russia's national prefix isn't a zero: not touched after the calling code.
        assert_eq!(
            normalize_phone("8 916 123 45 67", "RU").as_deref(),
            Some("79161234567")
        );
        assert_eq!(
            normalize_phone("+7 916 123 45 67", "").as_deref(),
            Some("79161234567")
        );
        // Without the prefix, an 8 is the number's own.
        assert_eq!(
            normalize_phone("(812) 123-45-67", "RU").as_deref(),
            Some("78121234567")
        );
        assert_eq!(
            normalize_phone("8 (812) 123-45-67", "RU").as_deref(),
            Some("78121234567")
        );
        // Liechtenstein has one, unlike Luxembourg.
        assert_eq!(
            normalize_phone("0612 34 56", "LI").as_deref(),
            Some("4236123456")
        );
    }

    #[test]
    fn prefixes_for_calling_abroad_besides_00() {
        for (number, region) in [
            ("011 49 171 1234567", "US"),
            ("011 49 171 1234567", "CA"),
            ("010 49 171 1234567", "JP"),
            ("810 49 171 1234567", "RU"),
            ("0011 49 171 1234567", "AU"),
            ("001 49 171 1234567", "SG"),
            ("023 49 171 1234567", "SG"),
            ("0049 171 1234567", "US"),
            // A national prefix after the calling code goes here too.
            ("011 49 0171 1234567", "US"),
        ] {
            assert_eq!(
                normalize_phone(number, region).as_deref(),
                Some("491711234567"),
                "{number} {region}"
            );
        }
        // Only there: elsewhere it is a national number.
        assert_eq!(
            normalize_phone("011 49 171 1234567", "DE").as_deref(),
            Some("4911491711234567")
        );
        // And not for the own number, which has to say its country.
        assert_eq!(international_phone("011 49 171 1234567"), None);
    }

    #[test]
    fn extensions_are_not_part_of_the_number() {
        for number in [
            "+49 30 123456;12",
            "+49 30 123456,12",
            "+49 30 123456#12",
            "+49 30 123456 x12",
            "+49 30 123456 ext. 12",
            "+49 30 123456 p12",
            "030 123456;ext=12",
            "030/123456 w12",
        ] {
            assert_eq!(
                normalize_phone(number, "DE").as_deref(),
                Some("4930123456"),
                "{number}"
            );
        }
        assert_eq!(
            international_phone("+49 30 123456;12").as_deref(),
            Some("4930123456")
        );
    }

    #[test]
    fn digits_of_other_scripts_count() {
        for number in [
            "＋４９ １７１ １２３４５６７",
            "٠١٧١ ١٢٣٤٥٦٧",
            "۰۱۷۱ ۱۲۳۴۵۶۷",
            "०१७१ १२३४५६७",
        ] {
            assert_eq!(
                normalize_phone(number, "DE").as_deref(),
                Some("491711234567"),
                "{number}"
            );
        }
    }

    #[test]
    fn direction_marks_do_not_hide_the_plus() {
        for number in ["\u{202a}+49 171 1234567\u{202c}", "\u{200e}+49 171 1234567"] {
            assert_eq!(
                normalize_phone(number, "AT").as_deref(),
                Some("491711234567"),
                "{number}"
            );
            assert_eq!(
                international_phone(number).as_deref(),
                Some("491711234567"),
                "{number}"
            );
        }
    }

    #[test]
    fn a_region_not_listed_finds_only_numbers_that_say_their_country() {
        // Their digits could be someone else's number somewhere else.
        assert_eq!(normalize_phone("0612 34567", "LT"), None);
        assert_eq!(normalize_phone("415 555 0100", ""), None);
        assert_eq!(
            normalize_phone("+370 612 34567", "LT").as_deref(),
            Some("37061234567")
        );
        assert_eq!(
            normalize_phone("00370 612 34567", "").as_deref(),
            Some("37061234567")
        );
    }

    #[test]
    fn the_country_table_is_prefix_free_between_different_codes() {
        for one in COUNTRIES {
            assert!(country_of(one.region).is_some_and(|c| c.code == one.code));
            for other in COUNTRIES {
                assert!(
                    one.code == other.code || !one.code.starts_with(other.code),
                    "{} and {}",
                    one.region,
                    other.region
                );
            }
        }
    }

    #[test]
    fn automatic_goes_by_the_linked_number_then_the_locale() {
        let region = |linked, locale| automatic_country(linked, locale).map(|c| c.region);
        assert_eq!(region(Some("436641234567"), "DE"), Some("AT"));
        assert_eq!(region(Some("+41791234567"), "DE"), Some("CH"));
        assert_eq!(region(None, "de"), Some("DE"));
        // A calling code we have no rules for: the locale's are the next best.
        assert_eq!(region(Some("37061234567"), "DE"), Some("DE"));
        assert_eq!(region(Some("37061234567"), "LT"), None);
        assert_eq!(region(None, ""), None);
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
        assert_eq!(
            international_phone("(+49) 171 1234567").as_deref(),
            Some("491711234567")
        );
        // What the link dialog shows and sends: the number as it is called from abroad.
        assert_eq!(
            international_phone("+49 0171 1234567").as_deref(),
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
