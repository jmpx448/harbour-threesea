// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Safe: encrypted cloud backup of the ID, profile, contacts, groups and settings.
//!
//! libthreema (6.5.3-1216) has no Safe implementation, so this follows upstream Android
//! `app/src/main/java/ch/threema/app/threemasafe/` with the same RustCrypto primitives libthreema
//! uses (scrypt, XSalsa20-Poly1305). See docs/protocol-notes.md.

pub mod backup;
pub mod server;

use std::io::{Read as _, Write as _};

use crypto_secretbox::{
    KeyInit as _, XSalsa20Poly1305,
    aead::{Aead as _, AeadCore as _, OsRng},
};
use zeroize::{Zeroize as _, Zeroizing};

/// Password length limits (`ThreemaSafeServiceImpl.MIN_PW_LENGTH` / `MAX_PW_LENGTH`).
pub const MIN_PASSWORD_LENGTH: usize = 8;
pub const MAX_PASSWORD_LENGTH: usize = 4096;

const NONCE_LENGTH: usize = 24;

/// Safe errors.
#[derive(Debug, thiserror::Error)]
pub enum SafeError {
    #[error("backup could not be decrypted")]
    Decrypt,
    #[error("backup is damaged: {0}")]
    Format(String),
    #[error("backup format version {0} is not supported")]
    UnsupportedVersion(u32),
    #[error("key derivation failed")]
    Kdf,
}

/// The 64-byte Safe master key: `backup_id || encryption_key`. Derived from ID and password; this
/// (not the password) is what gets stored locally, like Android does.
pub struct MasterKey(Zeroizing<[u8; 64]>);

impl MasterKey {
    /// `scrypt(password, salt = identity, N = 2^16, r = 8, p = 1, dkLen = 64)`.
    ///
    /// Slow on purpose (~64 MiB, seconds on a phone): call from a blocking thread.
    pub fn derive(identity: &str, password: &str) -> Result<Self, SafeError> {
        let params = scrypt::Params::new(16, 8, 1, 64).map_err(|_| SafeError::Kdf)?;
        let mut out = Zeroizing::new([0_u8; 64]);
        scrypt::scrypt(
            password.as_bytes(),
            identity.as_bytes(),
            &params,
            &mut out[..],
        )
        .map_err(|_| SafeError::Kdf)?;
        Ok(Self(out))
    }

    /// Restore from the stored bytes.
    pub fn from_bytes(bytes: [u8; 64]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }

    /// Identifies the backup on the server (first half).
    pub fn backup_id(&self) -> &[u8] {
        &self.0[..32]
    }

    /// Lowercase hex backup id, as used in URLs.
    pub fn backup_id_hex(&self) -> String {
        data_encoding::HEXLOWER.encode(self.backup_id())
    }

    fn cipher(&self) -> XSalsa20Poly1305 {
        XSalsa20Poly1305::new(self.0[32..].into())
    }

    /// gzip (stored blocks, no compression — upstream avoids compression oracles), then
    /// XSalsa20-Poly1305 with a random nonce placed in front.
    pub fn encrypt(&self, json: &[u8]) -> Result<Vec<u8>, SafeError> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::none());
        gz.write_all(json)
            .map_err(|e| SafeError::Format(e.to_string()))?;
        let mut compressed = gz.finish().map_err(|e| SafeError::Format(e.to_string()))?;
        let nonce = XSalsa20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher()
            .encrypt(&nonce, compressed.as_slice())
            .map_err(|_| SafeError::Format("encryption failed".into()))?;
        compressed.zeroize();
        let mut blob = Vec::with_capacity(NONCE_LENGTH + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(blob)
    }

    /// Inverse of [`Self::encrypt`]; returns the JSON bytes.
    pub fn decrypt(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, SafeError> {
        if blob.len() < NONCE_LENGTH + 16 {
            return Err(SafeError::Format("too short".into()));
        }
        let (nonce, ciphertext) = blob.split_at(NONCE_LENGTH);
        let compressed = Zeroizing::new(
            self.cipher()
                .decrypt(nonce.into(), ciphertext)
                .map_err(|_| SafeError::Decrypt)?,
        );
        // Bounded (security audit 2026-10-27): a backup is contacts, groups and settings, a few
        // MB at most; gzip can expand a thousandfold.
        let mut json = Zeroizing::new(Vec::new());
        flate2::read::MultiGzDecoder::new(compressed.as_slice())
            .take(MAX_BACKUP_JSON + 1)
            .read_to_end(&mut json)
            .map_err(|e| SafeError::Format(e.to_string()))?;
        if json.len() as u64 > MAX_BACKUP_JSON {
            return Err(SafeError::Format("backup too large".into()));
        }
        Ok(json)
    }
}

/// Largest decompressed backup we read.
const MAX_BACKUP_JSON: u64 = 64 * 1024 * 1024;

/// Parse a decrypted backup, rejecting future format versions.
pub fn parse_backup(json: &[u8]) -> Result<backup::Backup, SafeError> {
    let value: serde_json::Value =
        serde_json::from_slice(json).map_err(|e| SafeError::Format(e.to_string()))?;
    let version = value["info"]["version"].as_u64().unwrap_or(0);
    if version > u64::from(backup::VERSION) {
        return Err(SafeError::UnsupportedVersion(
            u32::try_from(version).unwrap_or(u32::MAX),
        ));
    }
    serde_json::from_value(value).map_err(|e| SafeError::Format(e.to_string()))
}

/// Upstream's list of common passwords (`app/assets/passwords/bad_passwords.txt`).
const BAD_PASSWORDS: &str =
    include_str!("../../../third_party/threema-android/app/assets/passwords/bad_passwords.txt");

/// Password quality, like `CheckBadPasswordUseCase` (without MDM): a weak password is a warning,
/// the user may continue anyway.
#[derive(Debug, PartialEq, Eq)]
pub enum PasswordCheck {
    TooShort,
    TooLong,
    Weak,
    Ok,
}

pub fn check_password(password: &str) -> PasswordCheck {
    let length = password.chars().count();
    if length < MIN_PASSWORD_LENGTH {
        return PasswordCheck::TooShort;
    }
    if length > MAX_PASSWORD_LENGTH {
        return PasswordCheck::TooLong;
    }
    let mut chars = password.chars();
    let first = chars.next();
    let single_repeated = chars.all(|c| Some(c) == first);
    let short_numeric = length <= 15 && password.chars().all(|c| c.is_ascii_digit());
    let lower = password.to_lowercase();
    let listed = BAD_PASSWORDS
        .lines()
        .any(|line| line.eq_ignore_ascii_case(&lower));
    if single_repeated || short_numeric || listed {
        PasswordCheck::Weak
    } else {
        PasswordCheck::Ok
    }
}

/// Upload policy (`ThreemaSafeServiceImpl` ~L469–500): when unchanged, refresh before half the
/// retention period passes; when changed, at most about once a day. Errors or `force` always
/// upload. Times in seconds since the epoch.
pub fn upload_due(
    changed: bool,
    last_upload: Option<i64>,
    last_failed: bool,
    retention_days: i64,
    now: i64,
    force: bool,
) -> bool {
    let Some(last) = last_upload else { return true };
    if force || last_failed {
        return true;
    }
    let age = now.saturating_sub(last);
    if changed {
        age >= 23 * 3600
    } else {
        let retention = if retention_days > 0 {
            retention_days
        } else {
            180
        };
        age >= retention * 86_400 / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivation_matches_upstream_test_vector() {
        // ThreemaSafeServiceTest.kt: password "shootdeathstar", identity "ECHOECHO".
        let key = MasterKey::derive("ECHOECHO", "shootdeathstar").unwrap();
        assert_eq!(
            key.backup_id_hex(),
            "066384d3695fbbd9f31a7d533900fd0cd8d1373beb6a28678522d2a49980c9c3"
        );
        assert_eq!(
            data_encoding::HEXLOWER.encode(&key.as_bytes()[32..]),
            "51c3d8d752fb6e1fd3199ead7f0895d6e3893ff691f2a5ee1976ed0897fc2f66"
        );
    }

    #[test]
    fn encrypt_decrypt_roundtrip_and_tamper() {
        let key = MasterKey::from_bytes([7; 64]);
        let json = br#"{"info":{"version":1}}"#;
        let blob = key.encrypt(json).unwrap();
        assert_eq!(&*key.decrypt(&blob).unwrap(), json);
        // Different nonce each time.
        assert_ne!(blob, key.encrypt(json).unwrap());
        let mut bad = blob.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(matches!(key.decrypt(&bad), Err(SafeError::Decrypt)));
        assert!(matches!(
            MasterKey::from_bytes([8; 64]).decrypt(&blob),
            Err(SafeError::Decrypt)
        ));
    }

    #[test]
    fn future_versions_are_rejected() {
        let json = br#"{"info":{"version":2},"user":{"privatekey":"x"},"settings":{}}"#;
        assert!(matches!(
            parse_backup(json),
            Err(SafeError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn password_checks() {
        assert_eq!(check_password("short"), PasswordCheck::TooShort);
        assert_eq!(check_password("aaaaaaaaaa"), PasswordCheck::Weak);
        assert_eq!(check_password("1234567890"), PasswordCheck::Weak);
        assert_eq!(
            check_password("password1"),
            PasswordCheck::Weak,
            "in the list"
        );
        assert_eq!(check_password("Kq7!sailfish-harbour"), PasswordCheck::Ok);
    }

    #[test]
    fn upload_policy() {
        let day = 86_400;
        assert!(upload_due(false, None, false, 180, 0, false));
        assert!(!upload_due(true, Some(0), false, 180, 22 * 3600, false));
        assert!(upload_due(true, Some(0), false, 180, day, false));
        assert!(!upload_due(false, Some(0), false, 180, 89 * day, false));
        assert!(upload_due(false, Some(0), false, 180, 90 * day, false));
        assert!(upload_due(false, Some(0), true, 180, 1, false));
        assert!(upload_due(false, Some(0), false, 180, 1, true));
    }
}
