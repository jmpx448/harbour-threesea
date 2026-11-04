// SPDX-License-Identifier: AGPL-3.0-only
//! Contact QR codes and links, as text and (first the width, then the pixels) as a camera image.
#![no_main]
use threesea_core::{links, qr};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = qr::parse(text);
        if let Some(link) = links::parse(text) {
            let _ = link.to_json();
        }
    }
    if let Some((&width, pixels)) = data.split_first()
        && width > 0
    {
        let width = usize::from(width);
        let _ = qr::decode(pixels, width, pixels.len() / width);
    }
});
