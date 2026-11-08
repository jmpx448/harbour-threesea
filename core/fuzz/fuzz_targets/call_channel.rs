// SPDX-License-Identifier: AGPL-3.0-only
//! The call peer's messages on the WebRTC data channel (camera state, video quality).
#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = threesea_core::call_video::decode(data);
});
