// SPDX-License-Identifier: AGPL-3.0-only
//! A call peer's SDP offer or answer, through the patcher in every mode.
#![no_main]
use threesea_core::sdp::{HeaderExtensions, SdpKind, SdpPatcher};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let Ok(sdp) = std::str::from_utf8(rest) else {
        return;
    };
    let extensions = match mode % 3 {
        0 => HeaderExtensions::Disable,
        1 => HeaderExtensions::OneByteOnly,
        _ => HeaderExtensions::OneAndTwoByte,
    };
    let kind = if mode & 0x10 == 0 {
        SdpKind::RemoteOffer
    } else {
        SdpKind::RemoteAnswer
    };
    let _ = SdpPatcher::new(extensions).patch(kind, sdp);
});
