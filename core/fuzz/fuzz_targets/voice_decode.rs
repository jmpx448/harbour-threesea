// SPDX-License-Identifier: AGPL-3.0-only
//! A voice message from a contact: Ogg/Opus decoding (the emulator's fallback player).
#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = threesea_core::voice::decode(data);
});
