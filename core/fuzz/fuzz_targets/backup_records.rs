// SPDX-License-Identifier: AGPL-3.0-only
//! The typed records on top of a data backup's CSV tables, and the conversion of message rows.
//! One text goes through all of them: a table's columns decide how far each gets, and
//! fuzz/seeds/backup_csv holds a real one of each.
#![no_main]
use threesea_core::android_backup::{convert, csv::Table, records};

libfuzzer_sys::fuzz_target!(|text: &str| {
    let Ok(table) = Table::parse(text) else {
        return;
    };
    let _ = records::contacts(&table);
    let (groups, _) = records::groups(&table);
    for group in &groups {
        let _ = group.aliases();
    }
    let _ = records::distribution_lists(&table);
    let _ = records::reactions(&table, false);
    let _ = records::reactions(&table, true);
    let (messages, _) = records::messages(&table);
    for message in &messages {
        let _ = message.unsent();
        let _ = convert::convert(message);
    }
});
