// SPDX-License-Identifier: AGPL-3.0-only
//! WebP pictures from the browser or forwarded, from whoever made them. What was cleaned stays a
//! file that can be cleaned, and doing so again changes nothing.
#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = threesea_core::webp::is_animated(data);
    let mut file = data.to_vec();
    let Ok(size) = threesea_core::webp::clean(&mut file) else {
        assert_eq!(file, data, "a refusal leaves the file as it was");
        return;
    };
    let cleaned = file.clone();
    assert_eq!(threesea_core::webp::clean(&mut file), Ok(size));
    assert_eq!(file, cleaned);
});
