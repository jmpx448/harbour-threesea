// SPDX-License-Identifier: AGPL-3.0-only
//! Tests against synthetic fixtures (scripts/make-backup-fixtures.py, written by pyzipper:
//! an independent implementation of the WinZip AES format Zip4j writes) and against archives
//! built in-test with the zip crate's writer.

use std::io::Write as _;

use super::{csv::Table, *};

/// The text scripts/make-backup-fixtures.py buries in the fixture's CSVs.
const TRICKY: &str = "Tricky \"quote\", semi;colon, back\\slash, emoji \u{1f389},\nsecond line";

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// One-off generator for the fixture's `identity` entry (prints an encrypted ID backup for
/// the synthetic test identity; pasted into scripts/make-backup-fixtures.py).
#[test]
#[ignore = "generator, not a check"]
fn generate_identity_backup_string() {
    let data = libthreema::id_backup::IdentityBackupData {
        threema_id: "ABCD1234".parse().unwrap(),
        client_key: libthreema::common::keys::ClientKey::from([7; 32]),
    };
    let backup = libthreema::id_backup::encrypt_identity_backup("test-password", &data).unwrap();
    eprintln!("identity backup: {backup}");
}

#[test]
fn reads_a_winzip_aes_archive() {
    let mut archive = Archive::open(&fixture("android-backup.zip"), "test-password").unwrap();
    assert_eq!(read_version(&mut archive).unwrap(), 27);

    let inventory = inventory(&archive);
    assert_eq!(inventory.contact_chats, 1);
    assert_eq!(inventory.group_chats, 1);
    assert_eq!(inventory.list_chats, 0);
    assert_eq!(inventory.media_entries, 2);
    assert_eq!(inventory.media_bytes, 4096);
    assert_eq!(inventory.thumbnails, 1);
    assert_eq!(inventory.contact_avatars, 1, "me is not a contact avatar");
    assert_eq!(inventory.contact_profile_pictures, 1);
    assert_eq!(inventory.group_avatars, 1);
    assert!(inventory.own_avatar);
    assert!(inventory.has_contacts && inventory.has_groups && inventory.has_distribution_lists);
    assert!(inventory.has_polls);
    assert!(inventory.has_reactions);
    assert!(inventory.has_nonces, "the ZIP64 entry is readable");
    assert_eq!(inventory.unknown_entries, 1, "future_stuff.csv");

    // The identity entry is a normal encrypted ID backup, same password.
    let encrypted = read_identity_backup(&mut archive).unwrap();
    let (_, identity) =
        libthreema::id_backup::decrypt_identity_backup("test-password", &encrypted).unwrap();
    assert_eq!(identity.threema_id.to_string(), "ABCD1234");

    // The CSV dialect round-trips OpenCSV's escaping, by header name, not position.
    let contacts = Table::parse(&archive.read_text("contacts.csv", 1 << 20).unwrap());
    assert_eq!(contacts.rows.len(), 3);
    assert_eq!(
        contacts.get(&contacts.rows[0], "identity"),
        Some("B0B0B0B0")
    );
    assert_eq!(contacts.get(&contacts.rows[0], "nick_name"), Some(TRICKY));
    assert_eq!(contacts.get(&contacts.rows[0], "identity_id"), Some("17"));
    assert_eq!(
        contacts.get_ms(&contacts.rows[0], "last_update"),
        Some(1_700_000_000_000)
    );
    assert_eq!(contacts.get_ms(&contacts.rows[1], "last_update"), None);

    // The typed records follow RestoreService's reading (hex keys, enum names, gates).
    let (parsed, invalid) = records::contacts(&contacts);
    assert_eq!((parsed.len(), invalid), (3, 0));
    assert_eq!(parsed[0].public_key, [0xaa; 32]);
    assert_eq!(parsed[0].verification, 1);
    assert_eq!(parsed[0].nickname.as_deref(), Some(TRICKY));
    assert_eq!(parsed[1].verification, 2);
    assert!(parsed[1].archived);
    assert!(parsed[2].hidden);
    assert_eq!(parsed[2].alias.as_deref(), Some("99"));

    let groups = Table::parse(&archive.read_text("groups.csv", 1 << 20).unwrap());
    assert_eq!(
        groups.get_list(&groups.rows[0], "members"),
        vec!["ABCD1234", "B0B0B0B0", "ANNANNAN", "CCCCCCCC"]
    );
    assert_eq!(groups.get(&groups.rows[0], "group_uid"), Some("g-17"));
    let (parsed, invalid) = records::groups(&groups);
    assert_eq!((parsed.len(), invalid), (2, 0));
    assert_eq!(parsed[0].group_id, 0x0102_0304_0506_0708);
    assert_eq!(parsed[0].creator, "B0B0B0B0");
    assert_eq!(parsed[0].user_state, Some(0));
    assert_eq!(parsed[0].aliases(), vec!["g-17".to_owned()]);
    assert_eq!(parsed[1].user_state, Some(2));
    assert!(parsed[1].archived);

    let lists = Table::parse(&archive.read_text("distribution_list.csv", 1 << 20).unwrap());
    let (parsed, invalid) = records::distribution_lists(&lists);
    assert_eq!((parsed.len(), invalid), (1, 0));
    assert_eq!(parsed[0].id, 7);
    assert_eq!(parsed[0].members, vec!["B0B0B0B0", "ANNANNAN"]);

    let messages = Table::parse(&archive.read_text("message_17.csv", 1 << 20).unwrap());
    assert_eq!(messages.get(&messages.rows[0], "body"), Some(TRICKY));
    assert_eq!(messages.get(&messages.rows[0], "type"), Some("TEXT"));
    assert_eq!(
        messages.get(&messages.rows[0], "quoted_message_apiid"),
        Some("")
    );
    assert_eq!(
        messages.get_ms(&messages.rows[0], "delivered_at"),
        Some(1_700_000_001_500)
    );

    // A DEFLATE entry and a STORED one both read fully (authenticated at their end).
    let media = archive.read("message_media_m-0002", 1 << 20).unwrap();
    let expected: Vec<u8> = (0..=255_u8).cycle().take(2048).collect();
    assert_eq!(media.as_slice(), expected.as_slice());
    assert!(
        archive
            .read("contact_avatar_me", 1 << 20)
            .unwrap()
            .starts_with(&[0xff, 0xd8])
    );
}

#[test]
fn wrong_password_is_told_apart() {
    let mut archive = Archive::open(&fixture("android-backup.zip"), "not-the-password").unwrap();
    match read_version(&mut archive) {
        Err(ArchiveError::WrongPassword) => {}
        other => panic!("expected WrongPassword, got {other:?}"),
    }
}

#[test]
fn unicode_passwords_work() {
    // Zip4j on Android derives the AES key from the password's UTF-8 bytes; so do pyzipper
    // (this fixture) and the zip crate.
    let mut archive = Archive::open(
        &fixture("android-backup-unicode.zip"),
        "pässwörter\u{1f511}",
    )
    .unwrap();
    assert_eq!(read_version(&mut archive).unwrap(), 27);
    assert!(matches!(
        read_version(
            &mut Archive::open(&fixture("android-backup-unicode.zip"), "pässwörter").unwrap()
        ),
        Err(ArchiveError::WrongPassword)
    ));
}

#[test]
fn tampering_fails_the_entry() {
    // Flip one ciphertext byte of the media entry: the read must fail (HMAC), not hand out
    // altered bytes.
    let original = std::fs::read(fixture("android-backup.zip")).unwrap();
    let name = b"message_media_m-0002";
    let position = original
        .windows(name.len())
        .position(|w| w == name)
        .unwrap();
    let mut altered = original;
    // Past the name and the AES extra field, well inside the 2 KiB ciphertext.
    altered[position + name.len() + 150] ^= 0x01;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("altered.zip");
    std::fs::write(&path, &altered).unwrap();
    let mut archive = Archive::open(&path, "test-password").unwrap();
    match archive.read("message_media_m-0002", 1 << 20) {
        Err(ArchiveError::Corrupt(_) | ArchiveError::Invalid(_)) => {}
        Err(other) => panic!("unexpected error {other:?}"),
        Ok(_) => panic!("altered entry was accepted"),
    }
    // The rest of the archive is still fine.
    assert_eq!(read_version(&mut archive).unwrap(), 27);
}

#[test]
fn oversized_entries_are_bounded_while_reading() {
    let mut archive = Archive::open(&fixture("android-backup.zip"), "test-password").unwrap();
    assert!(matches!(
        archive.read("message_media_m-0002", 100),
        Err(ArchiveError::TooLarge)
    ));
}

/// A small AES archive built with the zip crate's writer (the second implementation the
/// reader is checked against).
fn build_zip(dir: &std::path::Path, entries: &[(&str, &[u8], bool)]) -> std::path::PathBuf {
    let path = dir.join("built.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for (name, data, encrypted) in entries {
        let mut options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        if *encrypted {
            options = options.with_aes_encryption(zip::AesMode::Aes256, "test-password");
        }
        writer.start_file(*name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap();
    path
}

#[test]
fn version_gates() {
    let dir = tempfile::tempdir().unwrap();
    let version = |settings: &str| {
        let path = build_zip(dir.path(), &[("settings", settings.as_bytes(), true)]);
        read_version(&mut Archive::open(&path, "test-password").unwrap())
    };
    assert_eq!(version("\"version\",\"27\"\n").unwrap(), 27);
    assert_eq!(version("\"version\",\"15\"\n").unwrap(), 15);
    // Newer than this importer, ancient, or absent (Android defaults to 1): refused up front.
    assert!(matches!(
        version("\"version\",\"99\"\n"),
        Err(ArchiveError::UnsupportedVersion(99))
    ));
    assert!(matches!(
        version("\"version\",\"5\"\n"),
        Err(ArchiveError::UnsupportedVersion(5))
    ));
    assert!(matches!(
        version("\"other\",\"x\"\n"),
        Err(ArchiveError::UnsupportedVersion(1))
    ));
    assert!(matches!(
        version("\"version\",\"soon\"\n"),
        Err(ArchiveError::Invalid(_))
    ));
    // No settings entry at all.
    let path = build_zip(dir.path(), &[("identity", b"x", true)]);
    assert!(matches!(
        read_version(&mut Archive::open(&path, "test-password").unwrap()),
        Err(ArchiveError::Invalid(_))
    ));
}

#[test]
fn bad_containers_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    // Path traversal and directories don't belong in a backup.
    for name in ["../evil", "dir/inside", ".hidden"] {
        let path = build_zip(dir.path(), &[(name, b"x", true)]);
        assert!(
            matches!(
                Archive::open(&path, "test-password"),
                Err(ArchiveError::Invalid(_))
            ),
            "{name} was accepted"
        );
    }
    // Unencrypted entries mean this is not an Android backup.
    let path = build_zip(dir.path(), &[("settings", b"\"version\",\"27\"\n", false)]);
    assert!(matches!(
        Archive::open(&path, "test-password"),
        Err(ArchiveError::Invalid(_))
    ));
    // Not a ZIP at all.
    let path = dir.path().join("junk.zip");
    std::fs::write(&path, b"junk").unwrap();
    assert!(matches!(
        Archive::open(&path, "test-password"),
        Err(ArchiveError::Invalid(_))
    ));
}

#[test]
fn entry_names_classify() {
    use EntryKind::*;
    for (name, kind) in [
        ("settings", Settings),
        ("identity", Identity),
        ("contacts.csv", Contacts),
        ("groups.csv", Groups),
        ("distribution_list.csv", DistributionLists),
        ("message_17.csv", ContactMessages("17".into())),
        ("group_message_g-17.csv", GroupMessages("g-17".into())),
        ("distribution_list_message_5.csv", ListMessages("5".into())),
        ("message_media_m-1", ContactMedia("m-1".into())),
        ("group_message_media_m-1", GroupMedia("m-1".into())),
        (
            "distribution_list_message_media_m-1",
            ListMedia("m-1".into()),
        ),
        ("message_thumbnail_m-1", ContactThumbnail("m-1".into())),
        ("group_message_thumbnail_m-1", GroupThumbnail("m-1".into())),
        (
            "distribution_list_thumbnail_m-1",
            ListThumbnail("m-1".into()),
        ),
        ("contact_avatar_me", OwnAvatar),
        ("contact_avatar_17", ContactAvatar("17".into())),
        ("contact_profile_pic_17", ContactProfilePicture("17".into())),
        ("group_avatar_g-17", GroupAvatar("g-17".into())),
        ("ballot.csv", Ballots),
        ("ballot_choice.csv", BallotChoices),
        ("ballot_vote.csv", BallotVotes),
        ("contact_reactions.csv", ContactReactions),
        ("group_reactions.csv", GroupReactions),
        ("reaction_counts.csv", ReactionCounts),
        ("nonces.csv", NoncesCsp),
        ("nonces_d2d.csv", NoncesD2d),
        ("nonce_counts.csv", NonceCounts),
        ("future_stuff.csv", Unknown),
        ("message_17", Unknown),
    ] {
        assert_eq!(EntryKind::classify(name), kind, "{name}");
    }
}
