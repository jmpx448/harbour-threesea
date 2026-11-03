// SPDX-License-Identifier: AGPL-3.0-only
//! Images a Threema Web browser sends: decoding, scaling, thumbnails.
#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = threesea_core::web::images::thumbnail_info(data);
    let _ = threesea_core::web::images::sent_image(data);
});
