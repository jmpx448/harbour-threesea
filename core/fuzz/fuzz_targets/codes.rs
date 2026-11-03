// SPDX-License-Identifier: AGPL-3.0-only
//! Scanned or pasted codes: Threema Web QR codes, Threema Desktop join codes.
#![no_main]
libfuzzer_sys::fuzz_target!(|text: &str| {
    let _ = threesea_core::web::qr::parse(text);
    let _ = threesea_core::rendezvous::parse_join_code(text);
});
