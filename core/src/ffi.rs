// SPDX-License-Identifier: AGPL-3.0-only
//! C ABI consumed by the Qt bridge (`bridge/`). Header: `core/include/threesea_core.h`.
//! Message protocol: docs/ffi.md.
//!
//! Rules: plain data only across the boundary. Strings returned to C are owned by Rust and must be
//! released with [`threesea_string_free`]. Callbacks run on the core thread.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_char, c_void};

use zeroize::Zeroizing;

use crate::{app::StartConfig, runtime::CoreHandle};

fn into_c_string(s: String) -> *mut c_char {
    // Our strings never contain interior NULs; fall back to an empty string rather than panic.
    CString::new(s).unwrap_or_default().into_raw()
}

/// Returns the core version. Free with `threesea_string_free`.
#[unsafe(no_mangle)]
pub extern "C" fn threesea_core_version() -> *mut c_char {
    into_c_string(crate::VERSION.to_owned())
}

/// A log line from the bridge (Qt warnings, QML errors), into the core's outputs (journal
/// and the in-app log file). `level`: "debug", "info" or anything else for warning. Safe to
/// call at any time; lines before `threesea_core_start` go to the journal only.
///
/// # Safety
/// `level` and `message` must be valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_core_log(level: *const c_char, message: *const c_char) {
    if level.is_null() || message.is_null() {
        return;
    }
    // SAFETY: caller contract.
    let (level, message) = unsafe { (CStr::from_ptr(level), CStr::from_ptr(message)) };
    crate::runtime::log_bridge(
        level.to_str().unwrap_or("warn"),
        message.to_str().unwrap_or_default(),
    );
}

/// Releases a string previously returned by this library. Passing NULL is a no-op.
///
/// # Safety
/// `s` must be NULL or a pointer obtained from this library that was not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_string_free(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: guaranteed by the caller contract above.
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Callback receiving one NUL-terminated UTF-8 JSON message. The string is only valid during the
/// call. Called on the core thread.
pub type ThreeseaEventCallback = extern "C" fn(user_data: *mut c_void, json: *const c_char);

struct Callback {
    f: ThreeseaEventCallback,
    user_data: *mut c_void,
}
// SAFETY: the bridge guarantees `user_data` may be used from the core thread until
// `threesea_core_stop` returns (it only posts queued events to the GUI thread).
unsafe impl Send for Callback {}
impl Callback {
    fn call(&self, message: &CStr) {
        (self.f)(self.user_data, message.as_ptr());
    }
}

/// Opaque core handle.
pub struct ThreeseaCore(CoreHandle);

/// Start the core thread.
///
/// `config_json`: see `StartConfig` / docs/ffi.md. `db_key` must point to exactly 32 bytes; it is
/// copied, the caller should wipe its copy. Returns NULL on invalid arguments.
///
/// # Safety
/// `config_json` must be a valid C string, `db_key` valid for 32 bytes, `callback`/`user_data`
/// valid until `threesea_core_stop` returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_core_start(
    config_json: *const c_char,
    db_key: *const u8,
    db_key_len: usize,
    callback: ThreeseaEventCallback,
    user_data: *mut c_void,
) -> *mut ThreeseaCore {
    if config_json.is_null() || db_key.is_null() || db_key_len != 32 {
        crate::runtime::init_logging(None);
        return std::ptr::null_mut();
    }
    // SAFETY: caller contract.
    let config = unsafe { CStr::from_ptr(config_json) };
    let Ok(config) = config
        .to_str()
        .map_err(|_| ())
        .and_then(|s| serde_json::from_str::<StartConfig>(s).map_err(|_| ()))
    else {
        crate::runtime::init_logging(None);
        return std::ptr::null_mut();
    };
    let _ = std::fs::create_dir_all(&config.data_dir);
    crate::runtime::init_logging(Some(&config.data_dir));
    crate::runtime::set_log_level(config.log_level.as_deref().unwrap_or("warn"));
    let mut key = Zeroizing::new([0_u8; 32]);
    // SAFETY: caller contract, length checked above.
    key.copy_from_slice(unsafe { std::slice::from_raw_parts(db_key, 32) });

    let callback = Callback {
        f: callback,
        user_data,
    };
    let emit = Box::new(move |message: String| {
        callback.call(&CString::new(message).unwrap_or_default());
    });
    match CoreHandle::start(config, key, emit) {
        Ok(handle) => Box::into_raw(Box::new(ThreeseaCore(handle))),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Queue a JSON request. Never blocks.
///
/// # Safety
/// `core` must come from `threesea_core_start`, `request_json` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_core_send(core: *mut ThreeseaCore, request_json: *const c_char) {
    if core.is_null() || request_json.is_null() {
        return;
    }
    // SAFETY: caller contract.
    let (core, request) = unsafe { (&*core, CStr::from_ptr(request_json)) };
    if let Ok(request) = request.to_str() {
        core.0.send(request.to_owned());
    }
}

/// Read a message's decrypted media (`thumbnail` or the file) from the encrypted media store.
/// Thread-safe, may be called from any thread while the core runs. Returns NULL if not on the
/// device (or unreadable); otherwise a buffer of `*out_len` bytes to release with
/// `threesea_buffer_free`.
///
/// # Safety
/// `core` must come from `threesea_core_start`, `out_len` must be valid for writing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_media_read(
    core: *const ThreeseaCore,
    message: i64,
    thumbnail: bool,
    out_len: *mut usize,
) -> *mut u8 {
    if core.is_null() || out_len.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller contract.
    let core = unsafe { &*core };
    let part = if thumbnail {
        crate::media::MediaPart::Thumbnail
    } else {
        crate::media::MediaPart::File
    };
    // Not while its private chat is hidden.
    if !core.0.media().shown(message) {
        return std::ptr::null_mut();
    }
    match core.0.media().read(message, part) {
        Ok(Some(data)) => {
            let boxed: Box<[u8]> = data.to_vec().into_boxed_slice();
            // SAFETY: caller contract.
            unsafe { *out_len = boxed.len() };
            Box::into_raw(boxed).cast::<u8>()
        }
        Ok(None) => std::ptr::null_mut(),
        Err(error) => {
            tracing::warn!(%error, "reading media failed");
            std::ptr::null_mut()
        }
    }
}

/// Read a picture that isn't a message's (a group picture: `name` is the group key) from the
/// encrypted media store. Thread-safe; NULL if none. Free with `threesea_buffer_free`.
///
/// # Safety
/// `core` from `threesea_core_start`, `name` a valid C string, `out_len` valid for writing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_picture_read(
    core: *const ThreeseaCore,
    name: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    if core.is_null() || name.is_null() || out_len.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller contract.
    let (core, name) = unsafe { (&*core, CStr::from_ptr(name)) };
    let Ok(name) = name.to_str() else {
        return std::ptr::null_mut();
    };
    match core.0.media().read_picture_for_ui(name) {
        Ok(Some(data)) => {
            let boxed: Box<[u8]> = data.to_vec().into_boxed_slice();
            // SAFETY: caller contract.
            unsafe { *out_len = boxed.len() };
            Box::into_raw(boxed).cast::<u8>()
        }
        _ => std::ptr::null_mut(),
    }
}

/// The size of the picture of the video at `path` as a player shows it, read from the file's index
/// (MP4 and QuickTime). False if it can't be told. Any thread, no core needed.
///
/// # Safety
/// `path` a valid C string, `width` and `height` valid for writing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_video_size(
    path: *const c_char,
    width: *mut u32,
    height: *mut u32,
) -> bool {
    if path.is_null() || width.is_null() || height.is_null() {
        return false;
    }
    // SAFETY: caller contract.
    let Ok(path) = unsafe { CStr::from_ptr(path) }.to_str() else {
        return false;
    };
    let Some((found_width, found_height)) =
        crate::mp4::video_size_of_file(std::path::Path::new(path))
    else {
        return false;
    };
    // SAFETY: caller contract.
    unsafe {
        *width = found_width;
        *height = found_height;
    }
    true
}

/// Decode a message's Ogg Opus voice message to 16-bit mono PCM at 48 kHz (little endian), for
/// playing without a platform player. Thread-safe; NULL if not on the device or not Opus. Free
/// with `threesea_buffer_free`.
///
/// # Safety
/// As `threesea_media_read`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_voice_decode(
    core: *const ThreeseaCore,
    message: i64,
    out_len: *mut usize,
) -> *mut u8 {
    if core.is_null() || out_len.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller contract.
    let core = unsafe { &*core };
    if !core.0.media().shown(message) {
        return std::ptr::null_mut();
    }
    let Ok(Some(file)) = core.0.media().read(message, crate::media::MediaPart::File) else {
        return std::ptr::null_mut();
    };
    match crate::voice::decode(&file) {
        Ok(pcm) => {
            // An exact copy: shrinking the buffer could leave the audio in freed memory, and
            // `pcm` is wiped when it goes.
            let boxed: Box<[u8]> = pcm.to_vec().into_boxed_slice();
            // SAFETY: caller contract.
            unsafe { *out_len = boxed.len() };
            Box::into_raw(boxed).cast::<u8>()
        }
        Err(error) => {
            tracing::warn!(%error, "decoding voice message failed");
            std::ptr::null_mut()
        }
    }
}

/// Decode the first QR code in an 8-bit grey image (`width * height` bytes); a string to free
/// with `threesea_string_free`, or NULL. Needs no core; thread-safe.
///
/// # Safety
/// `grey` must be valid for `width * height` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_qr_decode(
    grey: *const u8,
    width: usize,
    height: usize,
) -> *mut c_char {
    if grey.is_null() || width == 0 || height == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: caller contract.
    let grey = unsafe { std::slice::from_raw_parts(grey, width.saturating_mul(height)) };
    match crate::qr::decode(grey, width, height) {
        Some(text) => CString::new(text).map_or(std::ptr::null_mut(), CString::into_raw),
        None => std::ptr::null_mut(),
    }
}

/// Group call media frames (ADR 0012): encrypt one of our encoded frames (`codec` 0 Opus,
/// 1 VP8) into `out` (capacity `out_cap`, at least `len + 22`); the bytes written, or -1 to drop
/// the frame. Needs no core; thread-safe (GStreamer's streaming threads).
///
/// # Safety
/// `frame` valid for `len` bytes (or NULL with `len` 0), `out` writable for `out_cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_group_call_encrypt(
    codec: u8,
    frame: *const u8,
    len: usize,
    out: *mut u8,
    out_cap: usize,
) -> isize {
    // SAFETY: caller contract.
    let frame = unsafe { frame_slice(frame, len) };
    let result =
        frame_codec(codec).and_then(|codec| crate::group_call_frames::encrypt(codec, frame));
    // SAFETY: caller contract.
    unsafe { write_frame(result, out, out_cap) }
}

/// Decrypt a frame of `participant` into `out` (capacity `out_cap`, at least `len`); the bytes
/// written, or -1 to drop it.
///
/// # Safety
/// As `threesea_group_call_encrypt`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_group_call_decrypt(
    participant: u32,
    codec: u8,
    frame: *const u8,
    len: usize,
    out: *mut u8,
    out_cap: usize,
) -> isize {
    // SAFETY: caller contract.
    let frame = unsafe { frame_slice(frame, len) };
    let result = frame_codec(codec)
        .and_then(|codec| crate::group_call_frames::decrypt(participant, codec, frame));
    // SAFETY: caller contract.
    unsafe { write_frame(result, out, out_cap) }
}

/// Whether a received VP8 frame may reach the decoder: 1 yes, 0 no, -1 no and it was a key frame
/// that is too large or broken. `waiting` is the stream's state (starts out true), used on the
/// stream's thread only.
///
/// # Safety
/// `frame` valid for `len` bytes (or NULL with `len` 0), `waiting` valid for reads and writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_call_vp8_filter(
    frame: *const u8,
    len: usize,
    waiting: *mut bool,
) -> i32 {
    // SAFETY: caller contract.
    let frame = unsafe { frame_slice(frame, len) };
    // SAFETY: caller contract.
    match crate::call_video::vp8_filter(frame, unsafe { &mut *waiting }) {
        crate::call_video::Vp8Verdict::Pass => 1,
        crate::call_video::Vp8Verdict::Drop => 0,
        crate::call_video::Vp8Verdict::Refuse => -1,
    }
}

/// Encrypt one of our VP8 frames of simulcast `layer` (0 l, 1 m, 2 h) and split it into RTP
/// payloads of at most `max_payload` bytes:
/// written one after the other into `out`, their lengths into `lengths` (room for `max_packets`).
/// The number of payloads, or -1 to drop the frame (no key, invalid, no room).
///
/// # Safety
/// `frame` valid for `len` bytes, `out` writable for `out_cap` bytes, `lengths` for
/// `max_packets` entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_group_call_vp8_packets(
    layer: u8,
    frame: *const u8,
    len: usize,
    max_payload: usize,
    out: *mut u8,
    out_cap: usize,
    lengths: *mut usize,
    max_packets: usize,
) -> isize {
    // SAFETY: caller contract.
    let frame = unsafe { frame_slice(frame, len) };
    let Some(payloads) = crate::group_call_vp8::packets(usize::from(layer), frame, max_payload)
    else {
        return -1;
    };
    let total: usize = payloads.iter().map(Vec::len).sum();
    if payloads.len() > max_packets || total > out_cap || out.is_null() || lengths.is_null() {
        return -1;
    }
    let mut offset = 0;
    for (index, payload) in payloads.iter().enumerate() {
        // SAFETY: caller contract, room checked above.
        unsafe {
            std::ptr::copy_nonoverlapping(payload.as_ptr(), out.add(offset), payload.len());
            *lengths.add(index) = payload.len();
        }
        offset += payload.len();
    }
    isize::try_from(payloads.len()).unwrap_or(-1)
}

fn frame_codec(codec: u8) -> Option<crate::group_call_frames::Codec> {
    match codec {
        0 => Some(crate::group_call_frames::Codec::Opus),
        1 => Some(crate::group_call_frames::Codec::Vp8),
        _ => None,
    }
}

/// # Safety
/// `frame` valid for `len` bytes or NULL.
unsafe fn frame_slice<'a>(frame: *const u8, len: usize) -> &'a [u8] {
    if frame.is_null() || len == 0 {
        return &[];
    }
    // SAFETY: caller contract.
    unsafe { std::slice::from_raw_parts(frame, len) }
}

/// # Safety
/// `out` writable for `out_cap` bytes.
unsafe fn write_frame(result: Option<Vec<u8>>, out: *mut u8, out_cap: usize) -> isize {
    let Some(data) = result else {
        return -1;
    };
    if data.len() > out_cap || (out.is_null() && !data.is_empty()) {
        return -1;
    }
    if !data.is_empty() {
        // SAFETY: caller contract, length checked.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), out, data.len()) };
    }
    isize::try_from(data.len()).unwrap_or(-1)
}

/// A passphrase from C, or `None` (NULL or not UTF-8).
///
/// # Safety
/// `passphrase` NULL or a valid C string.
unsafe fn passphrase_arg<'a>(passphrase: *const c_char) -> Option<&'a str> {
    if passphrase.is_null() {
        return None;
    }
    // SAFETY: caller contract.
    unsafe { std::ffi::CStr::from_ptr(passphrase) }
        .to_str()
        .ok()
}

fn into_buffer(data: &[u8], out_len: *mut usize) -> *mut u8 {
    let boxed: Box<[u8]> = data.into();
    // SAFETY: callers checked `out_len`.
    unsafe { *out_len = boxed.len() };
    Box::into_raw(boxed).cast::<u8>()
}

/// The 32-byte database key encrypted with `passphrase` (ADR 0009), to keep in Sailfish Secrets
/// instead of the key. Slow (scrypt): not on the GUI thread. Free with `threesea_buffer_free`;
/// NULL on bad arguments.
///
/// # Safety
/// `key` valid for `key_len` bytes, `passphrase` a C string, `out_len` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_key_wrap(
    key: *const u8,
    key_len: usize,
    passphrase: *const c_char,
    keyboard: bool,
    out_len: *mut usize,
) -> *mut u8 {
    if key.is_null() || key_len != 32 || out_len.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller contract.
    let Some(passphrase) = (unsafe { passphrase_arg(passphrase) }) else {
        return std::ptr::null_mut();
    };
    // SAFETY: caller contract.
    let key = Zeroizing::new(
        <[u8; 32]>::try_from(unsafe { std::slice::from_raw_parts(key, key_len) })
            .expect("checked length"),
    );
    into_buffer(
        &crate::applock::wrap_key(&key, passphrase, keyboard),
        out_len,
    )
}

/// The database key from a wrapped one, or NULL (wrong passphrase). Slow (scrypt). Free with
/// `threesea_buffer_free`.
///
/// # Safety
/// `blob` valid for `blob_len` bytes, `passphrase` a C string, `out_len` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_key_unwrap(
    blob: *const u8,
    blob_len: usize,
    passphrase: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    if blob.is_null() || out_len.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller contract.
    let Some(passphrase) = (unsafe { passphrase_arg(passphrase) }) else {
        return std::ptr::null_mut();
    };
    // SAFETY: caller contract.
    let blob = unsafe { std::slice::from_raw_parts(blob, blob_len) };
    match crate::applock::unwrap_key(blob, passphrase) {
        Some(key) => into_buffer(key.as_ref(), out_len),
        None => std::ptr::null_mut(),
    }
}

/// -1 if `blob` is not a wrapped key, else 1 if its passphrase is typed on the keyboard, 0 for
/// digits. Fast.
///
/// # Safety
/// `blob` valid for `blob_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_key_wrapped(blob: *const u8, blob_len: usize) -> i32 {
    if blob.is_null() {
        return -1;
    }
    // SAFETY: caller contract.
    let blob = unsafe { std::slice::from_raw_parts(blob, blob_len) };
    crate::applock::wrapped_keyboard(blob).map_or(-1, i32::from)
}

/// Release a buffer from `threesea_media_read` (wiped first: it holds decrypted media).
///
/// # Safety
/// `buffer`/`len` must come from `threesea_media_read`, and not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_buffer_free(buffer: *mut u8, len: usize) {
    if !buffer.is_null() {
        // SAFETY: caller contract: allocated as a boxed slice of `len` bytes.
        let mut boxed = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(buffer, len)) };
        zeroize::Zeroize::zeroize(&mut *boxed);
    }
}

/// Queue a JSON request with up to two binary attachments (copied; NULL/0 for none), e.g.
/// `sendFile` with the file and its thumbnail. Never blocks.
///
/// # Safety
/// `core` from `threesea_core_start`, `request_json` a valid C string, `data`/`data_len` and
/// `extra`/`extra_len` valid byte ranges (or NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_core_send_data(
    core: *mut ThreeseaCore,
    request_json: *const c_char,
    data: *const u8,
    data_len: usize,
    extra: *const u8,
    extra_len: usize,
) {
    if core.is_null() || request_json.is_null() {
        return;
    }
    let copy = |pointer: *const u8, len: usize| {
        if pointer.is_null() || len == 0 {
            Zeroizing::new(Vec::new())
        } else {
            // SAFETY: caller contract.
            Zeroizing::new(unsafe { std::slice::from_raw_parts(pointer, len) }.to_vec())
        }
    };
    // SAFETY: caller contract.
    let (core, request) = unsafe { (&*core, CStr::from_ptr(request_json)) };
    if let Ok(request) = request.to_str() {
        core.0.send_with(
            request.to_owned(),
            vec![copy(data, data_len), copy(extra, extra_len)],
        );
    }
}

/// Stop the core thread and free the handle. Blocks until the thread has finished; no callbacks
/// happen after this returns.
///
/// # Safety
/// `core` must come from `threesea_core_start` and not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_core_stop(core: *mut ThreeseaCore) {
    if !core.is_null() {
        // SAFETY: caller contract.
        drop(unsafe { Box::from_raw(core) });
    }
}

/// The core's mark of a removal of the ID that never finished (ADR 0018): -1 none, else whether
/// linked devices remain (0 or 1).
///
/// # Safety
/// `data_dir` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_data_removal_mark(data_dir: *const c_char) -> i32 {
    if data_dir.is_null() {
        return -1;
    }
    // SAFETY: caller contract.
    let Ok(dir) = unsafe { CStr::from_ptr(data_dir) }.to_str() else {
        return -1;
    };
    crate::runtime::removal_mark(std::path::Path::new(dir)).map_or(-1, i32::from)
}

/// Remove everything in the data directory (the ID is being removed, ADR 0018). The core must
/// have stopped. False if something stayed.
///
/// # Safety
/// `data_dir` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_data_remove(data_dir: *const c_char) -> bool {
    if data_dir.is_null() {
        return false;
    }
    // SAFETY: caller contract.
    let Ok(data_dir) = unsafe { CStr::from_ptr(data_dir) }.to_str() else {
        return false;
    };
    match crate::runtime::remove_data_dir(std::path::Path::new(data_dir)) {
        Ok(()) => true,
        Err(error) => {
            tracing::error!(%error, "the data directory could not be emptied");
            false
        }
    }
}
