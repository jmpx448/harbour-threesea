// SPDX-License-Identifier: AGPL-3.0-only
//! GIFs about to leave, from whoever made them. What was cleaned stays a file that can be cleaned,
//! and doing so again changes nothing.
#![no_main]
use zeroize::Zeroizing;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let mut file = Zeroizing::new(data.to_vec());
    let Ok(size) = threesea_core::gif::clean(&mut file) else {
        assert_eq!(file.as_slice(), data, "a refusal leaves the file as it was");
        return;
    };
    let cleaned = file.clone();
    assert_eq!(threesea_core::gif::clean(&mut file), Ok(size));
    assert_eq!(file, cleaned);
});
