// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Web chunks from the browser, in any order and amount: reassembly and its budget.
#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let mut unchunker = threesea_core::web::chunks::Unchunker::default();
    // The first byte says how the rest splits into chunks.
    let Some((&size, mut rest)) = data.split_first() else {
        return;
    };
    let size = usize::from(size).max(9);
    while !rest.is_empty() {
        let take = size.min(rest.len());
        let _ = unchunker.add(&rest[..take]);
        rest = &rest[take..];
    }
});
