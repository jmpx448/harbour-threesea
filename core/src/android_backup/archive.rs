// SPDX-License-Identifier: AGPL-3.0-only
//! The container of Android's data backup: a ZIP whose entries Zip4j encrypted with AES-256
//! (WinZip AE-2) and DEFLATE at level normal or "no compression" (which is still the DEFLATE
//! method; `FileHandlingZipOutputStream.kt`). The archive is untrusted local input: names,
//! methods and sizes are checked before anything else, every entry is read to its end so the
//! AES authentication code is verified (it can only fail at end-of-entry), and byte counts are
//! enforced while reading, never taken from the central directory.

use std::{io::Read as _, path::Path};

use zeroize::Zeroizing;

use super::{EntryKind, csv::Table};

/// Hard bounds for the container (an archive beyond them is rejected as a whole).
const MAX_ENTRIES: usize = 400_000;
const MAX_NAME_BYTES: usize = 160;
/// What a read reserves at most on the central directory's word alone.
const MAX_INITIAL_CAPACITY: usize = 1 << 20;

/// What can go wrong with an archive; the preview tells these apart for the user.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("cannot open: {0}")]
    Io(#[from] std::io::Error),
    /// Not a readable ZIP (or not one of ours): truncated, split/multidisk, bad headers.
    #[error("not a readable backup: {0}")]
    Invalid(String),
    /// The password does not fit the archive's verifier.
    #[error("wrong password")]
    WrongPassword,
    /// An entry failed decryption/authentication or decompression: altered or damaged.
    #[error("damaged entry: {0}")]
    Corrupt(String),
    /// An entry is bigger than the bound its reader allows.
    #[error("entry too large")]
    TooLarge,
    /// The backup declares a format newer than this importer knows.
    #[error("unsupported backup version {0}")]
    UnsupportedVersion(i64),
}

impl From<super::csv::TooLarge> for ArchiveError {
    fn from(_: super::csv::TooLarge) -> Self {
        Self::TooLarge
    }
}

/// Append without leaving a freed copy behind: `Vec` would move to a bigger block and free the
/// old one as it is, this wipes it.
fn append_wiping(data: &mut Zeroizing<Vec<u8>>, bytes: &[u8]) {
    let needed = data.len() + bytes.len();
    if needed > data.capacity() {
        let mut bigger = Zeroizing::new(Vec::with_capacity(needed.max(data.capacity() * 2)));
        bigger.extend_from_slice(data);
        *data = bigger;
    }
    data.extend_from_slice(bytes);
}

/// One entry as listed (nothing decrypted yet): `declared_size` comes from the central
/// directory and is an estimate until the entry is read.
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    pub declared_size: u64,
}

/// An opened, validated archive (read-only).
pub struct Archive {
    zip: zip::ZipArchive<std::fs::File>,
    password: Zeroizing<Vec<u8>>,
    entries: Vec<Entry>,
}

impl Archive {
    /// Open and validate the container: entry count, names (flat, short, ASCII, no traversal),
    /// methods (stored/deflate only), everything encrypted, no duplicate names and no two
    /// entries sharing bytes. The password is only checked when an entry is read. Reads every
    /// local header, so this belongs on a blocking thread.
    pub fn open(path: &Path, password: &str) -> Result<Self, ArchiveError> {
        let file = std::fs::File::open(path)?;
        let mut zip =
            zip::ZipArchive::new(file).map_err(|error| ArchiveError::Invalid(error.to_string()))?;
        if zip.len() > MAX_ENTRIES {
            return Err(ArchiveError::Invalid(format!(
                "too many entries ({})",
                zip.len()
            )));
        }
        // The reader's list is keyed by name and keeps the last record of a name, so a second
        // record would silently replace an authentic entry; only this count still shows it.
        // Records sharing bytes would let many names inflate the same small entry. Android and
        // our export write neither.
        if zip.has_duplicate_names() {
            return Err(ArchiveError::Invalid("duplicate entry".into()));
        }
        if zip
            .has_overlapping_entries()
            .map_err(|error| ArchiveError::Invalid(error.to_string()))?
        {
            return Err(ArchiveError::Invalid("overlapping entries".into()));
        }
        let mut entries = Vec::with_capacity(zip.len());
        for index in 0..zip.len() {
            let entry = zip
                .by_index_raw(index)
                .map_err(|error| ArchiveError::Invalid(error.to_string()))?;
            let name = entry
                .name()
                .map_err(|error| ArchiveError::Invalid(error.to_string()))?
                .into_owned();
            // ASCII like all of Android's names: the check above compares raw bytes, so two
            // encodings of one non-ASCII name would pass it and still be one name here.
            if name.is_empty()
                || name.len() > MAX_NAME_BYTES
                || !name.is_ascii()
                || name.contains(['/', '\\'])
                || name.starts_with('.')
                || name.chars().any(char::is_control)
            {
                // log-ok: entry names are structural (Tags.java patterns), not user content.
                return Err(ArchiveError::Invalid("entry name not allowed".into()));
            }
            if !matches!(
                entry.compression(),
                zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
            ) {
                return Err(ArchiveError::Invalid("unsupported entry method".into()));
            }
            if !entry.encrypted() {
                return Err(ArchiveError::Invalid("unencrypted entry".into()));
            }
            entries.push(Entry {
                kind: EntryKind::classify(&name),
                declared_size: entry.size(),
                name,
            });
        }
        Ok(Self {
            zip,
            password: Zeroizing::new(password.as_bytes().to_vec()),
            entries,
        })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn has_entry(&self, name: &str) -> bool {
        self.entries.iter().any(|entry| entry.name == name)
    }

    /// Decrypt and read one entry completely (the AES authentication code is checked at its
    /// end), enforcing `max_bytes` while reading. The bytes are returned zeroizing and this
    /// function leaves no other copy: several entries carry key material or private content.
    /// The zip crate's own buffers (the inflate window) are out of reach.
    pub fn read(
        &mut self,
        name: &str,
        max_bytes: usize,
    ) -> Result<Zeroizing<Vec<u8>>, ArchiveError> {
        let mut entry = match self.zip.by_name_decrypt(name, &self.password) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::InvalidPassword) => return Err(ArchiveError::WrongPassword),
            Err(zip::result::ZipError::FileNotFound) => {
                return Err(ArchiveError::Invalid("entry missing".into()));
            }
            Err(error) => return Err(ArchiveError::Invalid(error.to_string())),
        };
        let capacity = usize::try_from(entry.size())
            .unwrap_or(usize::MAX)
            .min(max_bytes)
            .min(MAX_INITIAL_CAPACITY);
        let mut data = Zeroizing::new(Vec::with_capacity(capacity));
        let mut buffer = Zeroizing::new([0_u8; 64 * 1024]);
        loop {
            match entry.read(&mut buffer[..]) {
                Ok(0) => break,
                Ok(read) => {
                    if data.len() + read > max_bytes {
                        return Err(ArchiveError::TooLarge);
                    }
                    append_wiping(&mut data, &buffer[..read]);
                }
                // Authentication, decryption or decompression failed: never trust what came
                // out before. (With AE, a wrong password can also surface here: the 2-byte
                // verifier lets 1 in 65536 wrong passwords through to the HMAC check.)
                Err(error) => return Err(ArchiveError::Corrupt(error.kind().to_string())),
            }
        }
        Ok(data)
    }

    /// An entry that must be UTF-8 text (the CSVs, the identity string).
    pub fn read_text(
        &mut self,
        name: &str,
        max_bytes: usize,
    ) -> Result<Zeroizing<String>, ArchiveError> {
        let data = self.read(name, max_bytes)?;
        let text = std::str::from_utf8(&data)
            .map_err(|_| ArchiveError::Invalid("entry is not UTF-8".into()))?;
        Ok(Zeroizing::new(text.to_owned()))
    }

    /// A CSV entry as a table.
    pub fn read_table(&mut self, name: &str, max_bytes: usize) -> Result<Table, ArchiveError> {
        Ok(Table::parse(&self.read_text(name, max_bytes)?)?)
    }
}
