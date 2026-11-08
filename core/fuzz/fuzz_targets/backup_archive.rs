// SPDX-License-Identifier: AGPL-3.0-only
//! A data backup's ZIP container, from wherever the user found it: everything the reader does
//! before and while decrypting. Seeded with the test fixture, whose password is used here.
#![no_main]
use threesea_core::android_backup::{self, Archive};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    // The reader takes a path. One file per process, in the temporary directory.
    let path = std::env::temp_dir().join(format!("threesea-fuzz-{}.zip", std::process::id()));
    if std::fs::write(&path, data).is_err() {
        return;
    }
    if let Ok(mut archive) = Archive::open(&path, "test-password") {
        let _ = android_backup::inventory(&archive);
        let _ = android_backup::read_version(&mut archive);
        let names: Vec<String> = archive
            .entries()
            .iter()
            .take(16)
            .map(|entry| entry.name.clone())
            .collect();
        for name in names {
            let _ = archive.read_table(&name, 1 << 20);
        }
    }
    let _ = std::fs::remove_file(&path);
});
