// SPDX-License-Identifier: AGPL-3.0-only
//! Contact QR codes like Threema Android (`ContactUrlUtil`): `3mid:<ID>,<public key hex>` with an
//! optional `,<expiry>` (epoch seconds; a past one is refused, one that isn't a number means
//! none). Showing ours lets others verify us; scanning theirs proves their key in person
//! (verification level _fully verified_).

use qrcode::{EcLevel, QrCode};

const PREFIX: &str = "3mid:";

/// Our QR code text.
pub fn contact_text(identity: &str, public_key_hex: &str) -> String {
    format!("{PREFIX}{identity},{public_key_hex}")
}

/// A scanned contact code.
#[derive(Debug)]
pub struct ContactCode {
    pub identity: String,
    pub public_key: [u8; 32],
    /// Epoch seconds, if the code has an expiry (ours and Android's own don't).
    pub expires_at: Option<i64>,
}

impl ContactCode {
    /// Past its expiry at `now` (epoch seconds), by Android's rule (`ContactUrlResult`).
    #[must_use]
    pub fn is_expired(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|at| at < now)
    }
}

/// Parse a scanned contact code. Expiry isn't checked here: see [`ContactCode::is_expired`].
pub fn parse(text: &str) -> Option<ContactCode> {
    let mut parts = text.trim().strip_prefix(PREFIX)?.split(',');
    let identity = parts.next()?;
    if identity.len() != 8 {
        return None;
    }
    let public_key = data_encoding::HEXLOWER_PERMISSIVE
        .decode(parts.next()?.as_bytes())
        .ok()?
        .try_into()
        .ok()?;
    Some(ContactCode {
        identity: identity.to_ascii_uppercase(),
        public_key,
        expires_at: parts.next().and_then(|expiry| expiry.parse().ok()),
    })
}

/// Whether this is an ID backup, Threema for Android's ID export: Base32 in groups of four, in
/// either of the sizes Android accepts from a scan (`WizardIDRestoreActivity`). Whether it can be
/// read takes its password.
#[must_use]
pub fn is_identity_backup(text: &str) -> bool {
    const GROUPS: [usize; 2] = [20, 26];
    let groups = text.trim().split('-');
    GROUPS.contains(&groups.clone().count())
        && groups.into_iter().all(|group| {
            group.len() == 4
                && group
                    .bytes()
                    .all(|c| matches!(c, b'A'..=b'Z' | b'2'..=b'7'))
        })
}

/// The QR code modules: `(size, rows of '0'/'1')`, for the UI to draw.
pub fn modules(text: &str) -> Option<(usize, String)> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).ok()?;
    let size = code.width();
    let bits = code
        .to_colors()
        .iter()
        .map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' })
        .collect();
    Some((size, bits))
}

/// Decode the first QR code in an 8-bit grey image.
pub fn decode(grey: &[u8], width: usize, height: usize) -> Option<String> {
    if width == 0 || height == 0 || grey.len() < width * height {
        return None;
    }
    let mut image =
        rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| grey[y * width + x]);
    image
        .detect_grids()
        .into_iter()
        .find_map(|grid| grid.decode().ok().map(|(_, text)| text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rendered with a quiet zone, 4 px per module, then decoded.
    fn through_pixels(text: &str) -> String {
        let (size, bits) = modules(text).unwrap();
        let scale = 4;
        let side = (size + 8) * scale;
        let mut grey = vec![255_u8; side * side];
        for (i, bit) in bits.chars().enumerate() {
            if bit == '1' {
                let (mx, my) = (i % size + 4, i / size + 4);
                for y in 0..scale {
                    for x in 0..scale {
                        grey[(my * scale + y) * side + mx * scale + x] = 0;
                    }
                }
            }
        }
        decode(&grey, side, side).unwrap()
    }

    #[test]
    fn roundtrip_through_pixels() {
        let text = contact_text("ECHOECHO", &"ab".repeat(32));
        let decoded = through_pixels(&text);
        assert_eq!(decoded, text);
        let code = parse(&decoded).unwrap();
        assert_eq!(code.identity, "ECHOECHO");
        assert_eq!(code.public_key, [0xab; 32]);
        assert_eq!(code.expires_at, None);
        assert!(parse("3mid:SHORT,ab").is_none());
        assert!(parse("https://threema.id/ECHOECHO").is_none());
    }

    fn backup_of(groups: usize) -> String {
        vec!["AB2C"; groups].join("-")
    }

    #[test]
    fn an_id_backup_of_either_size_is_one_and_survives_a_code() {
        for (groups, length) in [(20, 99), (26, 129)] {
            let backup = backup_of(groups);
            assert_eq!(backup.len(), length);
            assert!(is_identity_backup(&backup));
            assert!(is_identity_backup(&format!(" {backup}\n")));
            assert_eq!(through_pixels(&backup), backup);
        }
    }

    #[test]
    fn what_merely_resembles_an_id_backup_is_not_one() {
        assert!(!is_identity_backup(""));
        assert!(!is_identity_backup(&backup_of(19)));
        assert!(!is_identity_backup(&backup_of(21)));
        assert!(!is_identity_backup(&backup_of(20).to_lowercase()));
        assert!(!is_identity_backup(&backup_of(20).replace("AB2C", "AB1C")));
        assert!(!is_identity_backup(&backup_of(20).replace('-', "")));
        assert!(!is_identity_backup(&contact_text(
            "ECHOECHO",
            &"ab".repeat(32)
        )));
    }

    #[test]
    fn expiry_is_seconds_and_a_past_one_counts() {
        let key = "ab".repeat(32);
        let code = parse(&format!("3mid:ABCD1234,{key},1700000000")).unwrap();
        assert_eq!(code.expires_at, Some(1_700_000_000));
        assert!(code.is_expired(1_700_000_001));
        // Android: expired only once the moment has passed.
        assert!(!code.is_expired(1_700_000_000));
        assert!(!code.is_expired(1_699_999_999));
    }

    #[test]
    fn an_unreadable_expiry_means_none() {
        let key = "ab".repeat(32);
        for text in [
            format!("3mid:ABCD1234,{key},soon"),
            format!("3mid:ABCD1234,{key},"),
            format!("3mid:ABCD1234,{key}"),
        ] {
            let code = parse(&text).unwrap();
            assert_eq!(code.expires_at, None, "{text}");
            assert!(!code.is_expired(i64::MAX), "{text}");
        }
    }
}
