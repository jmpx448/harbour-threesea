// SPDX-License-Identifier: AGPL-3.0-only
//! Images a Threema Web browser sends: decoding, scaling, thumbnails.
#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = threesea_core::web::images::thumbnail_info(data);
    let mut picture = zeroize::Zeroizing::new(data.to_vec());
    let _ = threesea_core::web::images::sent_picture(&mut picture, "image/jpeg", Some(1600));
});
