// SPDX-License-Identifier: AGPL-3.0-only
//! Reading Android Threema's full data backup (the encrypted ZIP the app exports), for the
//! import into an existing identity. References (read-only, `third_party/threema-android`):
//! `app/backuprestore/csv/{BackupService,RestoreService,Tags,RestoreSettings}.java`,
//! `app/utils/{CSVReader,CSVWriter,CSVRow}.java`, `app/utils/FileHandlingZipOutputStream.kt`;
//! see docs/android-mapping.md. This module only reads and validates; nothing here touches the
//! store or the network.

pub mod archive;
pub mod convert;
pub mod csv;
pub mod export;
pub mod import;
pub mod nonces;
pub mod place;
pub mod polls;
pub mod records;

pub use archive::{Archive, ArchiveError};

/// `RestoreSettings.CURRENT_VERSION`: the newest backup format this importer knows. Anything
/// newer is rejected before any data is touched.
pub const CURRENT_VERSION: i64 = 27;

/// The oldest format this importer accepts. Android defaults a missing version to 1 and reads
/// everything since, but formats this old (no captions, no quote ids, no delivery dates) were
/// never produced by an app the user could still have run against today's servers; they get a
/// clear "unsupported" answer instead of a silently incomplete import.
pub const MIN_VERSION: i64 = 15;

/// Reading bounds for the small structural entries.
const MAX_SETTINGS_BYTES: usize = 64 * 1024;
const MAX_IDENTITY_BYTES: usize = 16 * 1024;

/// What an archive entry is, by its exact name or prefix (`Tags.java`). `alias` values are
/// archive-local: a contact's `identity_id` or a group's `group_uid`, never a Threema ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    /// Two-column CSV without a header; carries the format version.
    Settings,
    /// The encrypted ID-backup string (same password as the ZIP).
    Identity,
    Contacts,
    Groups,
    DistributionLists,
    /// `message_<alias>.csv` and friends.
    ContactMessages(String),
    GroupMessages(String),
    ListMessages(String),
    /// `message_media_<uid>` and friends: bytes keyed by the Android message `uid`.
    ContactMedia(String),
    GroupMedia(String),
    ListMedia(String),
    ContactThumbnail(String),
    GroupThumbnail(String),
    ListThumbnail(String),
    /// `contact_avatar_me`: the user's own picture.
    OwnAvatar,
    /// `contact_avatar_<alias>`: the picture the user chose locally (address book).
    ContactAvatar(String),
    /// `contact_profile_pic_<alias>`: the picture the contact distributes.
    ContactProfilePicture(String),
    GroupAvatar(String),
    Ballots,
    BallotChoices,
    BallotVotes,
    ContactReactions,
    GroupReactions,
    ReactionCounts,
    NoncesCsp,
    NoncesD2d,
    NonceCounts,
    /// Not a name this format version knows; counted, reported, ignored.
    Unknown,
}

impl EntryKind {
    /// Longest prefixes first: `group_message_media_` is not a `group_message_…` chat file.
    pub fn classify(name: &str) -> Self {
        let csv = |name: &str, exact: &str| name == format!("{exact}.csv");
        let chat_csv = |name: &str, prefix: &str| -> Option<String> {
            name.strip_prefix(prefix)?
                .strip_suffix(".csv")
                .map(str::to_owned)
        };
        let tail = |name: &str, prefix: &str| name.strip_prefix(prefix).map(str::to_owned);
        match name {
            "settings" => return Self::Settings,
            "identity" => return Self::Identity,
            "contact_avatar_me" => return Self::OwnAvatar,
            _ => {}
        }
        if csv(name, "contacts") {
            Self::Contacts
        } else if csv(name, "groups") {
            Self::Groups
        } else if csv(name, "distribution_list") {
            Self::DistributionLists
        } else if csv(name, "ballot") {
            Self::Ballots
        } else if csv(name, "ballot_choice") {
            Self::BallotChoices
        } else if csv(name, "ballot_vote") {
            Self::BallotVotes
        } else if csv(name, "contact_reactions") {
            Self::ContactReactions
        } else if csv(name, "group_reactions") {
            Self::GroupReactions
        } else if csv(name, "reaction_counts") {
            Self::ReactionCounts
        } else if csv(name, "nonces") {
            Self::NoncesCsp
        } else if csv(name, "nonces_d2d") {
            Self::NoncesD2d
        } else if csv(name, "nonce_counts") {
            Self::NonceCounts
        } else if let Some(uid) = tail(name, "distribution_list_message_media_") {
            Self::ListMedia(uid)
        } else if let Some(uid) = tail(name, "distribution_list_thumbnail_") {
            Self::ListThumbnail(uid)
        } else if let Some(alias) = chat_csv(name, "distribution_list_message_") {
            Self::ListMessages(alias)
        } else if let Some(uid) = tail(name, "group_message_media_") {
            Self::GroupMedia(uid)
        } else if let Some(uid) = tail(name, "group_message_thumbnail_") {
            Self::GroupThumbnail(uid)
        } else if let Some(alias) = chat_csv(name, "group_message_") {
            Self::GroupMessages(alias)
        } else if let Some(uid) = tail(name, "message_media_") {
            Self::ContactMedia(uid)
        } else if let Some(uid) = tail(name, "message_thumbnail_") {
            Self::ContactThumbnail(uid)
        } else if let Some(alias) = chat_csv(name, "message_") {
            Self::ContactMessages(alias)
        } else if let Some(alias) = tail(name, "contact_profile_pic_") {
            Self::ContactProfilePicture(alias)
        } else if let Some(alias) = tail(name, "contact_avatar_") {
            Self::ContactAvatar(alias)
        } else if let Some(alias) = tail(name, "group_avatar_") {
            Self::GroupAvatar(alias)
        } else {
            Self::Unknown
        }
    }
}

/// The format version from the `settings` entry (`RestoreSettings.parse`): rows of two cells,
/// `version` decides; absent means 1. A version this importer does not support is an error
/// before anything else is read.
pub fn read_version(archive: &mut Archive) -> Result<i64, ArchiveError> {
    let text = archive.read_text("settings", MAX_SETTINGS_BYTES)?;
    let mut version = 1;
    for row in csv::parse(&text)?.iter() {
        if row.len() == 2 && row[0] == "version" {
            version = row[1]
                .parse()
                .map_err(|_| ArchiveError::Invalid("settings version not a number".into()))?;
        }
    }
    if !(MIN_VERSION..=CURRENT_VERSION).contains(&version) {
        return Err(ArchiveError::UnsupportedVersion(version));
    }
    Ok(version)
}

/// What a quick look at the archive finds, before anything is imported. Counts are entry
/// counts; sizes are the central directory's declarations — estimates until each entry is
/// read and authenticated at import time.
#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    pub contact_chats: usize,
    pub group_chats: usize,
    pub list_chats: usize,
    pub media_entries: usize,
    pub media_bytes: u64,
    pub thumbnails: usize,
    pub contact_avatars: usize,
    pub contact_profile_pictures: usize,
    pub group_avatars: usize,
    pub own_avatar: bool,
    pub has_contacts: bool,
    pub has_groups: bool,
    pub has_distribution_lists: bool,
    pub has_polls: bool,
    pub has_reactions: bool,
    pub has_nonces: bool,
    pub unknown_entries: usize,
}

/// Count what the archive holds (names only; nothing decrypted).
pub fn inventory(archive: &Archive) -> Inventory {
    let mut inventory = Inventory::default();
    for entry in archive.entries() {
        match &entry.kind {
            EntryKind::ContactMessages(_) => inventory.contact_chats += 1,
            EntryKind::GroupMessages(_) => inventory.group_chats += 1,
            EntryKind::ListMessages(_) => inventory.list_chats += 1,
            EntryKind::ContactMedia(_) | EntryKind::GroupMedia(_) | EntryKind::ListMedia(_) => {
                inventory.media_entries += 1;
                inventory.media_bytes += entry.declared_size;
            }
            EntryKind::ContactThumbnail(_)
            | EntryKind::GroupThumbnail(_)
            | EntryKind::ListThumbnail(_) => inventory.thumbnails += 1,
            EntryKind::ContactAvatar(_) => inventory.contact_avatars += 1,
            EntryKind::ContactProfilePicture(_) => inventory.contact_profile_pictures += 1,
            EntryKind::GroupAvatar(_) => inventory.group_avatars += 1,
            EntryKind::OwnAvatar => inventory.own_avatar = true,
            EntryKind::Contacts => inventory.has_contacts = true,
            EntryKind::Groups => inventory.has_groups = true,
            EntryKind::DistributionLists => inventory.has_distribution_lists = true,
            EntryKind::Ballots | EntryKind::BallotChoices | EntryKind::BallotVotes => {
                inventory.has_polls = true;
            }
            EntryKind::ContactReactions | EntryKind::GroupReactions | EntryKind::ReactionCounts => {
                inventory.has_reactions = true
            }
            EntryKind::NoncesCsp | EntryKind::NoncesD2d | EntryKind::NonceCounts => {
                inventory.has_nonces = true;
            }
            EntryKind::Settings | EntryKind::Identity => {}
            EntryKind::Unknown => inventory.unknown_entries += 1,
        }
    }
    inventory
}

/// The encrypted ID-backup string from the `identity` entry (decrypted elsewhere, with the
/// same password; it carries the private key, so it stays zeroizing).
pub fn read_identity_backup(
    archive: &mut Archive,
) -> Result<zeroize::Zeroizing<String>, ArchiveError> {
    let text = archive.read_text("identity", MAX_IDENTITY_BYTES)?;
    Ok(zeroize::Zeroizing::new(text.trim().to_owned()))
}

#[cfg(test)]
mod tests;
