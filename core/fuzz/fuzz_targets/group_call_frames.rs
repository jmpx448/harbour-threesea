// SPDX-License-Identifier: AGPL-3.0-only
//! Group call media frames from the network, and frames that pass the check: what we encrypted
//! ourselves, with the input's last bytes changing the footer (key epoch, ratchet, counter).
#![no_main]
use threesea_core::{
    group_call_frames::{self, Codec},
    group_call_handshake::MediaKey,
};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&mode, frame)) = data.split_first() else {
        return;
    };
    let codec = if mode & 1 == 0 { Codec::Opus } else { Codec::Vp8 };
    let key = MediaKey {
        epoch: 0,
        ratchet: 0,
        pcmk: [5; 32].into(),
    };
    group_call_frames::reset([8; 32]);
    group_call_frames::set_local(&key);
    group_call_frames::add_peer_keys(1, std::slice::from_ref(&key));
    let _ = group_call_frames::decrypt(1, codec, frame);
    let _ = group_call_frames::decrypt(2, codec, frame);
    if let Some(mut sealed) = group_call_frames::encrypt(codec, frame) {
        let _ = group_call_frames::decrypt(1, codec, &sealed);
        // Once more, the replay window's way.
        let _ = group_call_frames::decrypt(1, codec, &sealed);
        let length = sealed.len();
        if length >= 6 {
            sealed[length - 6..length - 4].copy_from_slice(&[mode >> 4, mode]);
            let _ = group_call_frames::decrypt(1, codec, &sealed);
        }
    }
});
