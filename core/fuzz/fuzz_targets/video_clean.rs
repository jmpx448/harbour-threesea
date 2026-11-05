// SPDX-License-Identifier: AGPL-3.0-only
//! Videos the user picks or forwards, from whoever made them. What was cleaned stays a file that
//! can be cleaned, and doing so again changes nothing.
#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let mut file = data.to_vec();
    let Ok(size) = threesea_core::mp4::clean(&mut file) else {
        assert_eq!(file, data, "a refusal leaves the file as it was");
        return;
    };
    let cleaned = file.clone();
    assert_eq!(threesea_core::mp4::clean(&mut file), Ok(size));
    assert_eq!(file, cleaned);
});
