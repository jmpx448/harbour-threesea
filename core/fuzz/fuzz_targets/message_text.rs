// SPDX-License-Identifier: AGPL-3.0-only
//! Message text from contacts: quotes and mentions.
#![no_main]
libfuzzer_sys::fuzz_target!(|text: &str| {
    let _ = threesea_core::messages::split_quote(text);
    let _ = threesea_core::mentions::identities(text);
});
