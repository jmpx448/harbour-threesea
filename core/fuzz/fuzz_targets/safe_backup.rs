// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Safe: a backup blob from the server (or the user's own server), and the JSON inside.
#![no_main]
use threesea_core::safe::{self, MasterKey};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = MasterKey::from_bytes([3; 64]).decrypt(data);
    let _ = safe::parse_backup(data);
});
