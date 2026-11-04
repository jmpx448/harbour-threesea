// SPDX-License-Identifier: AGPL-3.0-only
//! Used nonces in the backup (`nonces.csv` for CSP, `nonces_d2d.csv`, `nonce_counts.csv`):
//! one column `nonces`, one lowercase-hex nonce per row (`BackupService.writeNonces`). Android
//! writes them hashed (HMAC-SHA256, keyed with the Threema ID string); its restore also takes
//! the unhashed 24-byte form older databases may still hold, and several nonces per cell split
//! by `;` (5.1-alpha3), and hashes the raw ones first (`NonceFactory.toHashedNonces`). A hash
//! cannot become a nonce again: the store keeps the hashes beside its raw nonces and checks
//! both (`Store::has_nonce`).

use super::csv::{Table, write_row};

/// A nonce's length on the wire and its hash's.
const NONCE_LEN: usize = 24;
const HASHED_LEN: usize = 32;

/// All nonces of one file as hashes, plus the count of cells that are neither form.
pub fn read(table: &Table, identity: &str) -> (Vec<[u8; HASHED_LEN]>, usize) {
    let mut hashed = Vec::with_capacity(table.rows.len());
    let mut skipped = 0;
    for row in &table.rows {
        for cell in table.get_list(row, "nonces") {
            let bytes = data_encoding::HEXLOWER_PERMISSIVE.decode(cell.trim().as_bytes());
            match bytes {
                Ok(bytes) if bytes.len() == NONCE_LEN || bytes.len() == HASHED_LEN => {
                    let hash = crate::messages::hash_nonce(identity, &bytes);
                    let mut fixed = [0; HASHED_LEN];
                    fixed.copy_from_slice(&hash);
                    hashed.push(fixed);
                }
                _ => skipped += 1,
            }
        }
    }
    (hashed, skipped)
}

/// One nonce file as Android writes it: every nonce hashed, as hex.
pub fn write(identity: &str, nonces: &[Vec<u8>]) -> String {
    let mut text = write_row(&["nonces"]);
    for nonce in nonces {
        let hash = crate::messages::hash_nonce(identity, nonce);
        text.push_str(&write_row(&[&data_encoding::HEXLOWER.encode(&hash)]));
    }
    text
}

/// `nonce_counts.csv`: lets Android's restore size its progress without reading the files.
pub fn write_counts(csp: usize, d2d: usize) -> String {
    write_row(&["csp", "d2d"]) + &write_row(&[&csp.to_string(), &d2d.to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `DatabaseNonceStoreTest.hashNonce`: identity "01234567", 23 zero bytes + counter.
    const ANDROID_HASH_OF_ZERO: &str =
        "dcba29bd5f74197131c0e645a30299ba833263af68ecf6fb7fdf3988f9f3ce4e";

    #[test]
    fn hash_matches_android() {
        assert_eq!(
            data_encoding::HEXLOWER.encode(&crate::messages::hash_nonce("01234567", &[0; 24])),
            ANDROID_HASH_OF_ZERO
        );
    }

    #[test]
    fn reads_hashed_raw_and_several_per_cell() {
        let raw = "00".repeat(24);
        let text = write_row(&["nonces"])
            + &write_row(&[ANDROID_HASH_OF_ZERO])
            + &write_row(&[&raw])
            + &write_row(&[&format!("{};{}", "11".repeat(24), "22".repeat(32))])
            + &write_row(&["abcd"])
            + &write_row(&["not hex"]);
        let (hashed, skipped) = read(&Table::parse(&text).unwrap(), "01234567");
        assert_eq!(skipped, 2);
        assert_eq!(hashed.len(), 4);
        // The raw zero nonce hashes to the same value Android wrote.
        assert_eq!(hashed[0], hashed[1]);
        assert_eq!(hashed[3], [0x22; 32], "already hashed stays");
    }

    #[test]
    fn writes_what_it_reads() {
        let nonces = vec![vec![0; 24], vec![0x33; 32]];
        let text = write("01234567", &nonces);
        let (hashed, skipped) = read(&Table::parse(&text).unwrap(), "01234567");
        assert_eq!(skipped, 0);
        assert_eq!(
            data_encoding::HEXLOWER.encode(&hashed[0]),
            ANDROID_HASH_OF_ZERO
        );
        assert_eq!(hashed[1], [0x33; 32]);
        assert_eq!(write_counts(2, 0), "\"csp\",\"d2d\"\n\"2\",\"0\"\n");
    }

    #[test]
    fn imported_hashes_catch_a_replay() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "01234567".into(),
                client_key: zeroize::Zeroizing::new([7; 32]),
                server_group: "a".into(),
            })
            .unwrap();
        let (hashed, _) = read(
            &Table::parse(&(write_row(&["nonces"]) + &write_row(&[ANDROID_HASH_OF_ZERO]))).unwrap(),
            "01234567",
        );
        assert_eq!(store.add_hashed_nonces(false, &hashed).unwrap(), 1);
        assert_eq!(
            store.add_hashed_nonces(false, &hashed).unwrap(),
            0,
            "idempotent"
        );
        assert!(store.has_nonce(&[0; 24]).unwrap(), "seen on Android");
        assert!(!store.has_nonce(&[1; 24]).unwrap());
        assert!(!store.has_d2d_nonce(&[0; 24]).unwrap(), "scopes stay apart");
        assert_eq!(store.add_hashed_nonces(true, &hashed).unwrap(), 1);
        assert!(store.has_d2d_nonce(&[0; 24]).unwrap());
        // Our own raw nonces still match.
        store.add_nonces(&[&[5; 24]]).unwrap();
        assert!(store.has_nonce(&[5; 24]).unwrap());
    }
}
