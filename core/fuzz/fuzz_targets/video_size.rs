// SPDX-License-Identifier: AGPL-3.0-only
//! The size of a video about to be scaled, read from the file itself.
#![no_main]
use std::sync::LazyLock;

// One file per fuzzing process, in the system's temporary directory.
static PATH: LazyLock<std::path::PathBuf> = LazyLock::new(|| {
    std::env::temp_dir().join(format!("threesea-fuzz-video-size-{}", std::process::id()))
});

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    std::fs::write(&*PATH, data).unwrap();
    let _ = threesea_core::mp4::video_size_of_file(&PATH);
});
