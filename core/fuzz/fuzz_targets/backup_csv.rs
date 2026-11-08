// SPDX-License-Identifier: AGPL-3.0-only
//! A data backup's CSV tables (the file comes from wherever the user found it), and the
//! readers on top of them.
#![no_main]
use threesea_core::android_backup::{csv::Table, nonces, polls::ArchivePolls};

libfuzzer_sys::fuzz_target!(|text: &str| {
    let Ok(table) = Table::parse(text) else {
        return;
    };
    let _ = nonces::read(&table, "ABCD1234");
    let (polls, _) = ArchivePolls::read(&table, Some(&table), Some(&table));
    let _ = polls.message(text);
});
