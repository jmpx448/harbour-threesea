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

/// Fuzzing seeds from the synthetic fixture, so the fuzzers start past the header row and the
/// container's structure: `cargo test -p threesea-core --lib write_backup_seeds -- --ignored`
#[test]
#[ignore = "writes fuzzing seeds"]
fn write_backup_seeds() {
    let seeds = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds");
    let csv = seeds.join("backup_csv");
    std::fs::create_dir_all(&csv).unwrap();
    let mut archive = Archive::open(&fixture("android-backup.zip"), "test-password").unwrap();
    let names: Vec<String> = archive
        .entries()
        .iter()
        .map(|entry| entry.name.clone())
        .filter(|name| name.ends_with(".csv"))
        .collect();
    for name in names {
        let text = archive.read_text(&name, 1 << 20).unwrap();
        std::fs::write(csv.join(name), text.as_bytes()).unwrap();
    }
    let zip = seeds.join("backup_archive");
    std::fs::create_dir_all(&zip).unwrap();
    std::fs::copy(
        fixture("android-backup.zip"),
        zip.join("android-backup.zip"),
    )
    .unwrap();
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
    let contacts = Table::parse(&archive.read_text("contacts.csv", 1 << 20).unwrap()).unwrap();
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

    let groups = Table::parse(&archive.read_text("groups.csv", 1 << 20).unwrap()).unwrap();
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

    let lists =
        Table::parse(&archive.read_text("distribution_list.csv", 1 << 20).unwrap()).unwrap();
    let (parsed, invalid) = records::distribution_lists(&lists);
    assert_eq!((parsed.len(), invalid), (1, 0));
    assert_eq!(parsed[0].id, 7);
    assert_eq!(parsed[0].members, vec!["B0B0B0B0", "ANNANNAN"]);

    let messages = Table::parse(&archive.read_text("message_17.csv", 1 << 20).unwrap()).unwrap();
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

/// A ZIP64 end: its record (with `padding` bytes of extensible data), locator and end record.
fn zip64_end(prefix: &[u8], entries: u64, padding: usize) -> Vec<u8> {
    let mut file = prefix.to_vec();
    let record = file.len() as u64;
    file.extend_from_slice(b"PK\x06\x06");
    file.extend_from_slice(&(44 + padding as u64).to_le_bytes());
    file.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    file.extend_from_slice(&entries.to_le_bytes());
    file.extend_from_slice(&entries.to_le_bytes());
    file.extend_from_slice(&0_u64.to_le_bytes());
    file.extend_from_slice(&0_u64.to_le_bytes());
    file.extend_from_slice(&vec![0; padding]);
    file.extend_from_slice(b"PK\x06\x07");
    file.extend_from_slice(&0_u32.to_le_bytes());
    file.extend_from_slice(&record.to_le_bytes());
    file.extend_from_slice(&1_u32.to_le_bytes());
    file.extend_from_slice(b"PK\x05\x06");
    file.extend_from_slice(&[0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
    file.extend_from_slice(&[0xff; 8]);
    file.extend_from_slice(&[0, 0]);
    file
}

fn open_bytes(bytes: &[u8]) -> Result<Archive, ArchiveError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crafted.zip");
    std::fs::write(&path, bytes).unwrap();
    Archive::open(&path, "test-password")
}

fn too_many(result: Result<Archive, ArchiveError>) -> bool {
    matches!(result, Err(ArchiveError::Invalid(text)) if text.starts_with("too many entries"))
}

/// Refused from the end records alone: the zip crate would build all of them in memory first.
#[test]
fn the_entry_count_is_checked_before_the_central_directory_is_parsed() {
    assert!(too_many(open_bytes(&zip64_end(&[], 2_000_000, 0))));
    assert!(too_many(open_bytes(&zip64_end(&[1; 300], 2_000_000, 0))));
    assert!(too_many(open_bytes(&zip64_end(&[], 2_000_000, 100))));
    // A record that is only found by searching: the locator names an earlier position.
    let mut hidden = zip64_end(&[0; 64], 2_000_000, 0);
    let locator = hidden.len() - 42;
    hidden[locator + 8..locator + 16].copy_from_slice(&0_u64.to_le_bytes());
    assert!(too_many(open_bytes(&hidden)));
    // Far from the end, past the first chunk of the search.
    let mut far = zip64_end(&vec![0; 3 << 20], 2_000_000, 0);
    let locator = far.len() - 42;
    far[locator + 8..locator + 16].copy_from_slice(&0_u64.to_le_bytes());
    assert!(too_many(open_bytes(&far)));

    // Below the bound it isn't this check that speaks, and our own archives still open.
    assert!(!too_many(open_bytes(&zip64_end(&[], 100, 0))));
    assert!(!too_many(open_bytes(b"too short")));
    assert!(!too_many(open_bytes(b"")));
    assert!(Archive::open(&fixture("android-backup.zip"), "test-password").is_ok());
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
fn entries_larger_than_the_first_reservation_read_whole() {
    // The buffer moves several times on the way: everything arrives, in order.
    let dir = tempfile::tempdir().unwrap();
    let data: Vec<u8> = (0..(5 << 20)).map(|i: u32| (i % 251) as u8).collect();
    let path = build_zip(dir.path(), &[("message_media_a", &data, true)]);
    let mut archive = Archive::open(&path, "test-password").unwrap();
    assert_eq!(
        archive.read("message_media_a", 8 << 20).unwrap().as_slice(),
        data.as_slice()
    );
    assert!(matches!(
        archive.read("message_media_a", (5 << 20) - 1),
        Err(ArchiveError::TooLarge)
    ));
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
fn duplicate_names_are_refused() {
    // Someone without the password appends an older entry and a second central directory
    // record: same name, different bytes, both authentic. Patching one name reproduces that.
    let dir = tempfile::tempdir().unwrap();
    let path = build_zip(
        dir.path(),
        &[
            ("settings", b"\"version\",\"27\"\n", true),
            ("settingz", b"\"version\",\"15\"\n", true),
        ],
    );
    let mut bytes = std::fs::read(&path).unwrap();
    let mut patched = 0;
    for start in 0..bytes.len() - 8 {
        if &bytes[start..start + 8] == b"settingz" {
            bytes[start..start + 8].copy_from_slice(b"settings");
            patched += 1;
        }
    }
    assert_eq!(patched, 2, "local header and central directory");
    std::fs::write(&path, &bytes).unwrap();
    assert!(matches!(
        Archive::open(&path, "test-password"),
        Err(ArchiveError::Invalid(reason)) if reason == "duplicate entry"
    ));
}

#[test]
fn entries_sharing_bytes_are_refused() {
    // Many names over one highly compressed entry: each read would inflate it again.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .with_aes_encryption(zip::AesMode::Aes256, "test-password");
    writer.start_file("message_media_a", options).unwrap();
    writer.write_all(&[0; 4096]).unwrap();
    writer
        .shallow_copy_file("message_media_a", "message_media_b")
        .unwrap();
    writer.finish().unwrap();
    assert!(matches!(
        Archive::open(&path, "test-password"),
        Err(ArchiveError::Invalid(reason)) if reason == "overlapping entries"
    ));
}

/// The archive rewritten the way a streaming writer lays it out: sizes and CRC zero in the local
/// header, flag bit 3 set, and a data descriptor after each entry's data. Offsets and sizes in the
/// central directory stay true.
fn with_data_descriptors(zip: &[u8]) -> Vec<u8> {
    let u16_at = |at: usize| usize::from(u16::from_le_bytes([zip[at], zip[at + 1]]));
    let u32_at = |at: usize| u32::from_le_bytes(zip[at..at + 4].try_into().unwrap()) as usize;
    let eocd = zip
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .unwrap();
    let central_start = u32_at(eocd + 16);
    let (mut out, mut central) = (Vec::new(), Vec::new());
    let mut record = central_start;
    for _ in 0..u16_at(eocd + 10) {
        let compressed = u32_at(record + 20);
        let local = u32_at(record + 42);
        let data = local + 30 + u16_at(local + 26) + u16_at(local + 28);
        let new_local = u32::try_from(out.len()).unwrap();
        out.extend_from_slice(&zip[local..data + compressed]);
        let header = &mut out[new_local as usize..];
        header[6] |= 0x08;
        header[14..26].fill(0);
        out.extend_from_slice(b"PK\x07\x08");
        out.extend_from_slice(&zip[record + 16..record + 28]);

        let end = record + 46 + u16_at(record + 28) + u16_at(record + 30) + u16_at(record + 32);
        let central_entry = central.len();
        central.extend_from_slice(&zip[record..end]);
        central[central_entry + 8] |= 0x08;
        central[central_entry + 42..central_entry + 46].copy_from_slice(&new_local.to_le_bytes());
        record = end;
    }
    let new_central = u32::try_from(out.len()).unwrap();
    out.extend_from_slice(&central);
    out.extend_from_slice(&zip[eocd..]);
    let new_eocd = out.len() - (zip.len() - eocd);
    out[new_eocd + 16..new_eocd + 20].copy_from_slice(&new_central.to_le_bytes());
    out
}

#[test]
fn data_descriptors_between_entries_are_no_overlap() {
    // The gap they leave lies outside what the central directory claims, so a streamed backup
    // has to pass the refusal above and read as before.
    let dir = tempfile::tempdir().unwrap();
    let streamed = dir.path().join("streamed.zip");
    let original = fixture("android-backup.zip");
    std::fs::write(
        &streamed,
        with_data_descriptors(&std::fs::read(&original).unwrap()),
    )
    .unwrap();
    let mut plain = Archive::open(&original, "test-password").unwrap();
    let mut archive = Archive::open(&streamed, "test-password").unwrap();
    let names: Vec<String> = plain.entries().iter().map(|e| e.name.clone()).collect();
    assert!(names.len() > 3);
    assert_eq!(archive.entries().len(), names.len());
    for name in names {
        assert_eq!(
            archive.read(&name, 8 << 20).unwrap().as_slice(),
            plain.read(&name, 8 << 20).unwrap().as_slice(),
            "{name}"
        );
    }
}

#[test]
fn non_ascii_names_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = build_zip(dir.path(), &[("message_m\u{e4}dia", b"x", true)]);
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
