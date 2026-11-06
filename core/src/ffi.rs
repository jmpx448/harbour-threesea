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

/// What the bundled map renderer may fetch, as JSON `{"origin": …, "pins": …}` (ADR 0032;
/// `https::map_policy`). Needs no running core. Free with `threesea_string_free`.
#[unsafe(no_mangle)]
pub extern "C" fn threesea_map_policy() -> *mut c_char {
    into_c_string(crate::https::map_policy().to_string())
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

/// Stop the core thread for the end of the process, waiting only a few seconds. True: the thread
/// has ended, as after [`threesea_core_stop`]. False: it still runs and can still call back, so
/// the caller keeps `user_data` alive and leaves the process with `_exit`. Either way the handle
/// is gone.
///
/// # Safety
/// `core` must come from `threesea_core_start` and not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn threesea_core_quit(core: *mut ThreeseaCore) -> bool {
    if core.is_null() {
        return true;
    }
    // SAFETY: caller contract.
    unsafe { Box::from_raw(core) }.0.quit()
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

#[cfg(test)]
mod tests {
    //! Every function of the C ABI as the bridge calls it: NULLs, wrong lengths, buffers that are
    //! too small, and what comes back released the way the bridge releases it. A length that
    //! disagrees between a buffer and its release is undefined behaviour the tests here can't see;
    //! `scripts/test-ffi-asan.sh` runs them under AddressSanitizer for that.

    use std::{
        ffi::{CString, c_char, c_void},
        ptr::{null, null_mut},
        sync::{Mutex, mpsc},
        time::Duration,
    };

    use super::*;
    use crate::{group_call_frames, group_call_handshake::MediaKey};

    /// Far beyond what any answer takes: only a core that never answers gets here.
    const NO_ANSWER: Duration = Duration::from_secs(600);
    /// Bytes past a buffer's room, which nothing may write.
    const GUARD: u8 = 0xa5;
    /// A VP8 key frame of 16×16 (its first 10 bytes stay in clear).
    const KEY_FRAME: [u8; 12] = [0x10, 1, 2, 0x9d, 0x01, 0x2a, 16, 0, 16, 0, 42, 43];

    fn c(text: &str) -> CString {
        CString::new(text).unwrap()
    }

    extern "C" fn forward(user_data: *mut c_void, json: *const c_char) {
        // SAFETY: `user_data` is the test's `Mutex<Sender>`, alive until the core has stopped;
        // `json` is valid during the call.
        let (tx, json) = unsafe {
            (
                &*user_data.cast::<Mutex<mpsc::Sender<String>>>(),
                CStr::from_ptr(json),
            )
        };
        let _ = tx.lock().unwrap().send(json.to_str().unwrap().to_owned());
    }

    extern "C" fn ignore(_: *mut c_void, _: *const c_char) {}

    fn config(dir: &std::path::Path) -> CString {
        c(&serde_json::json!({
            "dataDir": dir,
            "appVersion": "0.1.0",
            "language": "en",
            "country": "US",
            "model": "test",
            "osVersion": "5.1",
        })
        .to_string())
    }

    /// The answer to request `id`, skipping the events in between.
    fn reply(rx: &mpsc::Receiver<String>, id: u64) -> serde_json::Value {
        loop {
            let message: serde_json::Value =
                serde_json::from_str(&rx.recv_timeout(NO_ANSWER).unwrap()).unwrap();
            if message["reply"] == id {
                return message;
            }
        }
    }

    /// What a returned buffer holds, released as the bridge releases it.
    fn taken(buffer: *mut u8, len: usize) -> Option<Vec<u8>> {
        if buffer.is_null() {
            return None;
        }
        // SAFETY: a buffer of `len` bytes from this library.
        let copy = unsafe { std::slice::from_raw_parts(buffer, len) }.to_vec();
        // SAFETY: as above; not used afterwards.
        unsafe { threesea_buffer_free(buffer, len) };
        Some(copy)
    }

    fn untouched(guard: &[u8]) -> bool {
        guard.iter().all(|&byte| byte == GUARD)
    }

    #[test]
    fn the_version_is_a_string_to_free() {
        let version = threesea_core_version();
        // SAFETY: from this library, freed once.
        unsafe {
            assert_eq!(CStr::from_ptr(version).to_str().unwrap(), crate::VERSION);
            threesea_string_free(version);
            threesea_string_free(null_mut());
        }
    }

    #[test]
    fn nulls_and_wrong_lengths_are_refused_everywhere() {
        let key = [3_u8; 32];
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        let mut len = 0;
        let (mut width, mut height) = (0, 0);
        // SAFETY: NULLs where the functions check for them; everything else valid.
        unsafe {
            threesea_core_log(null(), null());
            threesea_core_log(c("info").as_ptr(), null());
            let start = |config: *const c_char, key: *const u8, key_len| {
                threesea_core_start(config, key, key_len, ignore, null_mut())
            };
            assert!(start(null(), key.as_ptr(), 32).is_null());
            assert!(start(config.as_ptr(), null(), 32).is_null());
            assert!(start(config.as_ptr(), key.as_ptr(), 31).is_null());
            assert!(start(config.as_ptr(), key.as_ptr(), 33).is_null());
            assert!(start(c("not json").as_ptr(), key.as_ptr(), 32).is_null());
            let latin1 = [0xff_u8, 0];
            assert!(start(latin1.as_ptr().cast(), key.as_ptr(), 32).is_null());
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

            threesea_core_send(null_mut(), c("{}").as_ptr());
            threesea_core_send_data(null_mut(), c("{}").as_ptr(), key.as_ptr(), 32, null(), 0);
            assert!(threesea_media_read(null(), 1, false, &raw mut len).is_null());
            assert!(threesea_picture_read(null(), c("x").as_ptr(), &raw mut len).is_null());
            assert!(threesea_voice_decode(null(), 1, &raw mut len).is_null());
            assert!(!threesea_video_size(
                null(),
                &raw mut width,
                &raw mut height
            ));
            assert!(!threesea_video_size(
                c("/nowhere").as_ptr(),
                null_mut(),
                &raw mut height
            ));
            assert!(threesea_qr_decode(null(), 4, 4).is_null());
            assert!(threesea_qr_decode(key.as_ptr(), 0, 4).is_null());
            assert!(
                threesea_key_wrap(null(), 32, c("1234").as_ptr(), false, &raw mut len).is_null()
            );
            assert!(
                threesea_key_wrap(key.as_ptr(), 31, c("1234").as_ptr(), false, &raw mut len)
                    .is_null()
            );
            assert!(threesea_key_wrap(key.as_ptr(), 32, null(), false, &raw mut len).is_null());
            assert!(
                threesea_key_wrap(key.as_ptr(), 32, c("1234").as_ptr(), false, null_mut())
                    .is_null()
            );
            assert!(threesea_key_unwrap(null(), 0, c("1234").as_ptr(), &raw mut len).is_null());
            assert_eq!(threesea_key_wrapped(null(), 0), -1);
            threesea_buffer_free(null_mut(), 5);
            threesea_core_stop(null_mut());
            assert!(threesea_core_quit(null_mut()));
            assert_eq!(threesea_data_removal_mark(null()), -1);
            assert!(!threesea_data_remove(null()));
        }
        assert_eq!(len, 0, "nothing was written through out_len");
    }

    /// The core's whole life through the C ABI: requests answered through the callback, media read
    /// out and released, and stopped by either way the bridge stops it.
    #[test]
    fn a_core_starts_answers_hands_out_media_and_stops() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        let key = [3_u8; 32];
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let user_data = (&raw const tx).cast_mut().cast::<c_void>();
        // SAFETY: valid arguments; `tx` outlives the core.
        let core =
            unsafe { threesea_core_start(config.as_ptr(), key.as_ptr(), 32, forward, user_data) };
        assert!(!core.is_null());
        // SAFETY: `core` is live until it is stopped below.
        unsafe {
            threesea_core_send(core, c(r#"{"id":1,"cmd":"status"}"#).as_ptr());
            assert_eq!(reply(&rx, 1)["ok"], true);
            let data = [1_u8, 2, 3];
            threesea_core_send_data(
                core,
                c(r#"{"id":2,"cmd":"status"}"#).as_ptr(),
                data.as_ptr(),
                3,
                null(),
                0,
            );
            assert_eq!(reply(&rx, 2)["ok"], true);

            let media = (*core).0.media();
            let mut len = 0;
            assert!(threesea_media_read(core, 7, false, &raw mut len).is_null());
            media
                .write(7, crate::media::MediaPart::File, b"decrypted media")
                .unwrap();
            let read = threesea_media_read(core, 7, false, &raw mut len);
            assert_eq!(taken(read, len).unwrap(), b"decrypted media");
            assert!(threesea_media_read(core, 7, true, &raw mut len).is_null());
            media.set_hidden(Some([7].into()));
            assert!(
                threesea_media_read(core, 7, false, &raw mut len).is_null(),
                "a hidden private chat's"
            );
            assert!(threesea_voice_decode(core, 7, &raw mut len).is_null());
            media.set_hidden(None);
            assert!(
                threesea_voice_decode(core, 7, &raw mut len).is_null(),
                "not Opus"
            );

            let group = c("g:ABCD1234:0000000000000001");
            assert!(threesea_picture_read(core, group.as_ptr(), &raw mut len).is_null());
            media
                .write_picture("g:ABCD1234:0000000000000001", b"picture")
                .unwrap();
            let read = threesea_picture_read(core, group.as_ptr(), &raw mut len);
            assert_eq!(taken(read, len).unwrap(), b"picture");

            threesea_core_stop(core);
        }
        let core =
            unsafe { threesea_core_start(config.as_ptr(), key.as_ptr(), 32, forward, user_data) };
        assert!(!core.is_null(), "the same data opens again");
        // SAFETY: as above.
        unsafe {
            threesea_core_send(core, c(r#"{"id":3,"cmd":"status"}"#).as_ptr());
            assert_eq!(reply(&rx, 3)["ok"], true);
            assert!(threesea_core_quit(core));
        }
    }

    #[test]
    fn the_data_directory_is_emptied_and_its_mark_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("store.db"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("media")).unwrap();
        std::fs::write(dir.path().join("media").join("1.file"), b"x").unwrap();
        let path = c(dir.path().to_str().unwrap());
        // SAFETY: a valid C string.
        unsafe {
            assert_eq!(threesea_data_removal_mark(path.as_ptr()), -1);
            assert!(threesea_data_remove(path.as_ptr()));
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    /// The database key wrapped with a passphrase and back, as `SecretStore` does it.
    #[test]
    fn a_key_wrapped_comes_back_only_with_its_passphrase() {
        let key: [u8; 32] = core::array::from_fn(|i| u8::try_from(i).unwrap());
        let (right, wrong) = (c("correct horse"), c("wrong horse"));
        let mut len = 0;
        // SAFETY: valid arguments; buffers released once with their own length.
        unsafe {
            let wrapped = threesea_key_wrap(key.as_ptr(), 32, right.as_ptr(), true, &raw mut len);
            let blob = taken(wrapped, len).unwrap();
            assert_eq!(threesea_key_wrapped(blob.as_ptr(), blob.len()), 1);
            assert_eq!(threesea_key_wrapped(blob.as_ptr(), 3), -1);

            let mut key_len = 0;
            let unwrapped =
                threesea_key_unwrap(blob.as_ptr(), blob.len(), right.as_ptr(), &raw mut key_len);
            assert_eq!(taken(unwrapped, key_len).unwrap(), key);
            assert!(
                threesea_key_unwrap(blob.as_ptr(), blob.len(), wrong.as_ptr(), &raw mut key_len)
                    .is_null()
            );
            assert!(
                threesea_key_unwrap(blob.as_ptr(), 3, right.as_ptr(), &raw mut key_len).is_null()
            );
            let latin1 = [0xff_u8, 0];
            assert!(
                threesea_key_unwrap(
                    blob.as_ptr(),
                    blob.len(),
                    latin1.as_ptr().cast(),
                    &raw mut key_len
                )
                .is_null()
            );

            let digits =
                threesea_key_wrap(key.as_ptr(), 32, c("1234").as_ptr(), false, &raw mut len);
            let blob = taken(digits, len).unwrap();
            assert_eq!(threesea_key_wrapped(blob.as_ptr(), blob.len()), 0);
        }
    }

    #[test]
    fn what_is_not_a_video_or_a_code_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-video.mp4");
        std::fs::write(&file, b"no index here").unwrap();
        let (mut width, mut height) = (0, 0);
        let grey = [255_u8; 64 * 64];
        // SAFETY: valid arguments, `grey` holds width × height bytes.
        unsafe {
            let path = c(file.to_str().unwrap());
            assert!(!threesea_video_size(
                path.as_ptr(),
                &raw mut width,
                &raw mut height
            ));
            assert!(!threesea_video_size(
                c("/nowhere").as_ptr(),
                &raw mut width,
                &raw mut height
            ));
            assert!(threesea_qr_decode(grey.as_ptr(), 64, 64).is_null());
        }
        assert_eq!((width, height), (0, 0));
    }

    /// A QR code in a camera's grey picture, its text a string to free.
    #[test]
    fn a_code_is_read_from_grey_pixels() {
        let text = crate::qr::contact_text("ABCD1234", &"ab".repeat(32));
        let (size, bits) = crate::qr::modules(&text).unwrap();
        let (scale, side) = (4, (size + 8) * 4);
        let mut grey = vec![255_u8; side * side];
        for (i, bit) in bits.chars().enumerate() {
            if bit == '1' {
                let (x, y) = ((i % size + 4) * scale, (i / size + 4) * scale);
                for row in y..y + scale {
                    grey[row * side + x..row * side + x + scale].fill(0);
                }
            }
        }
        // SAFETY: `grey` holds side × side bytes; the string is freed once.
        unsafe {
            let decoded = threesea_qr_decode(grey.as_ptr(), side, side);
            assert_eq!(CStr::from_ptr(decoded).to_str().unwrap(), text);
            threesea_string_free(decoded);
        }
    }

    /// Frames go into the caller's buffer only if they fit: a byte too few is a dropped frame,
    /// and nothing is written past the room given.
    #[test]
    fn group_call_frames_fit_the_room_given_or_are_dropped() {
        group_call_frames::reset([7; 32]);
        let ours = MediaKey::random(0);
        group_call_frames::set_local(&ours);
        group_call_frames::add_peer_keys(5, core::slice::from_ref(&ours));
        let frame = b"opus frame";
        let mut out = [GUARD; 64];
        // SAFETY: `frame` valid for its length, `out` for the capacities given (at most 64).
        unsafe {
            let sealed_len =
                threesea_group_call_encrypt(0, frame.as_ptr(), frame.len(), out.as_mut_ptr(), 64);
            let sealed_len = usize::try_from(sealed_len).unwrap();
            assert!(
                sealed_len <= frame.len() + 22,
                "the room the header promises"
            );
            assert!(untouched(&out[sealed_len..]));
            let sealed = out[..sealed_len].to_vec();

            let mut short = [GUARD; 64];
            let refused = threesea_group_call_encrypt(
                0,
                frame.as_ptr(),
                frame.len(),
                short.as_mut_ptr(),
                sealed_len - 1,
            );
            assert_eq!(refused, -1);
            assert!(untouched(&short));
            assert_eq!(
                threesea_group_call_encrypt(9, frame.as_ptr(), frame.len(), short.as_mut_ptr(), 64),
                -1
            );
            assert_eq!(
                threesea_group_call_encrypt(0, frame.as_ptr(), frame.len(), null_mut(), 64),
                -1
            );
            // Opus DTX: an empty frame passes, with nowhere to write it.
            assert_eq!(threesea_group_call_encrypt(0, null(), 0, null_mut(), 0), 0);

            let mut plain = [GUARD; 64];
            let opened = threesea_group_call_decrypt(
                5,
                0,
                sealed.as_ptr(),
                sealed.len(),
                plain.as_mut_ptr(),
                frame.len(),
            );
            assert_eq!(usize::try_from(opened).unwrap(), frame.len());
            assert_eq!(&plain[..frame.len()], frame);
            assert!(untouched(&plain[frame.len()..]));
            // Another frame (the same one again would be a replay), into too little room.
            let next_len =
                threesea_group_call_encrypt(0, frame.as_ptr(), frame.len(), out.as_mut_ptr(), 64);
            let next = out[..usize::try_from(next_len).unwrap()].to_vec();
            let mut short = [GUARD; 64];
            let refused = threesea_group_call_decrypt(
                5,
                0,
                next.as_ptr(),
                next.len(),
                short.as_mut_ptr(),
                frame.len() - 1,
            );
            assert_eq!(refused, -1);
            assert!(untouched(&short));
            assert_eq!(
                threesea_group_call_decrypt(
                    6,
                    0,
                    sealed.as_ptr(),
                    sealed.len(),
                    plain.as_mut_ptr(),
                    64
                ),
                -1
            );
        }
    }

    #[test]
    fn video_packets_fit_both_buffers_or_are_dropped() {
        group_call_frames::reset([7; 32]);
        group_call_frames::set_local(&MediaKey::random(0));
        let frame: Vec<u8> = KEY_FRAME.iter().copied().chain(0..40).collect();
        let mut out = [GUARD; 256];
        let mut lengths = [usize::MAX; 16];
        // SAFETY: `frame` valid for its length, `out` and `lengths` for the room given.
        unsafe {
            let packets = threesea_group_call_vp8_packets(
                0,
                frame.as_ptr(),
                frame.len(),
                16,
                out.as_mut_ptr(),
                256,
                lengths.as_mut_ptr(),
                16,
            );
            let packets = usize::try_from(packets).unwrap();
            assert!(packets > 1);
            let total: usize = lengths[..packets].iter().sum();
            assert!(lengths[..packets].iter().all(|&length| length <= 16));
            assert!(untouched(&out[total..]));
            assert!(
                lengths[packets..]
                    .iter()
                    .all(|&length| length == usize::MAX)
            );

            let mut short = [GUARD; 256];
            let mut few = [usize::MAX; 16];
            let call = |out: *mut u8, out_cap, lengths: *mut usize, max_packets| {
                threesea_group_call_vp8_packets(
                    0,
                    frame.as_ptr(),
                    frame.len(),
                    16,
                    out,
                    out_cap,
                    lengths,
                    max_packets,
                )
            };
            assert_eq!(
                call(short.as_mut_ptr(), total - 1, few.as_mut_ptr(), 16),
                -1
            );
            assert_eq!(
                call(short.as_mut_ptr(), 256, few.as_mut_ptr(), packets - 1),
                -1
            );
            assert_eq!(call(null_mut(), 256, few.as_mut_ptr(), 16), -1);
            assert_eq!(call(short.as_mut_ptr(), 256, null_mut(), 16), -1);
            assert!(untouched(&short));
            assert!(few.iter().all(|&length| length == usize::MAX));
            let layer = threesea_group_call_vp8_packets(
                3,
                frame.as_ptr(),
                frame.len(),
                16,
                short.as_mut_ptr(),
                256,
                few.as_mut_ptr(),
                16,
            );
            assert_eq!(layer, -1);
        }
    }

    #[test]
    fn a_video_stream_waits_for_a_key_frame_that_fits() {
        let mut waiting = true;
        let delta = [0x11_u8, 1, 2];
        let mut huge = KEY_FRAME;
        huge[6..10].copy_from_slice(&[0xff, 0x3f, 0xff, 0x3f]);
        // SAFETY: frames valid for their lengths, `waiting` a live bool.
        unsafe {
            assert_eq!(
                threesea_call_vp8_filter(delta.as_ptr(), delta.len(), &raw mut waiting),
                0
            );
            assert_eq!(threesea_call_vp8_filter(null(), 0, &raw mut waiting), 0);
            assert_eq!(
                threesea_call_vp8_filter(huge.as_ptr(), huge.len(), &raw mut waiting),
                -1
            );
            assert!(waiting);
            assert_eq!(
                threesea_call_vp8_filter(KEY_FRAME.as_ptr(), KEY_FRAME.len(), &raw mut waiting),
                1
            );
            assert!(!waiting);
            assert_eq!(
                threesea_call_vp8_filter(delta.as_ptr(), delta.len(), &raw mut waiting),
                1
            );
        }
    }

    /// The copy into the caller's buffer, by itself: exactly the bytes, never past the room.
    #[test]
    fn a_frame_is_written_whole_or_not_at_all() {
        let mut out = [GUARD; 8];
        // SAFETY: `out` valid for 8 bytes; NULL only with nothing to write.
        unsafe {
            assert_eq!(write_frame(Some(vec![1, 2, 3]), out.as_mut_ptr(), 2), -1);
            assert!(untouched(&out));
            assert_eq!(write_frame(None, out.as_mut_ptr(), 8), -1);
            assert_eq!(write_frame(Some(vec![1, 2, 3]), null_mut(), 8), -1);
            assert_eq!(write_frame(Some(Vec::new()), null_mut(), 0), 0);
            assert_eq!(write_frame(Some(vec![1, 2, 3]), out.as_mut_ptr(), 3), 3);
            assert_eq!(out[..3], [1, 2, 3]);
            assert!(untouched(&out[3..]));
            assert!(frame_slice(null(), 5).is_empty());
            assert!(frame_slice(out.as_ptr(), 0).is_empty());
        }
    }
}
