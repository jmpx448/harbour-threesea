// SPDX-License-Identifier: AGPL-3.0-only
//! The update check's reply from GitHub, and version strings compared.
#![no_main]
use threesea_core::updates;

libfuzzer_sys::fuzz_target!(|text: &str| {
    if let Ok(json) = serde_json::from_str(text) {
        let _ = updates::parse_release(&json);
    }
    let (current, candidate) = text.split_at(text.floor_char_boundary(text.len() / 2));
    let _ = updates::is_newer(current, candidate);
});
