/* SPDX-License-Identifier: AGPL-3.0-only */
/* C ABI of threesea-core. Keep in sync with core/src/ffi.rs; message protocol in docs/ffi.md. */
#ifndef THREESEA_CORE_H
#define THREESEA_CORE_H

#include <stddef.h>
#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ThreeseaCore ThreeseaCore;

/* Receives one UTF-8 JSON message, valid only during the call. Runs on the core thread. */
typedef void (*ThreeseaEventCallback)(void *user_data, const char *json);

char *threesea_core_version(void);

/* A log line from the bridge (Qt warnings, QML errors) into the core's outputs (journal and
 * the in-app log file). level: "debug", "info" or anything else for warning. */
void threesea_core_log(const char *level, const char *message);
void threesea_string_free(char *s);

/* db_key: exactly 32 bytes (copied). Returns NULL on invalid arguments. */
ThreeseaCore *threesea_core_start(const char *config_json, const uint8_t *db_key, size_t db_key_len,
                                  ThreeseaEventCallback callback, void *user_data);
void threesea_core_send(ThreeseaCore *core, const char *request_json);
/* Like threesea_core_send, with up to two binary attachments (copied; NULL/0 for none). */
void threesea_core_send_data(ThreeseaCore *core, const char *request_json, const uint8_t *data, size_t data_len,
                             const uint8_t *extra, size_t extra_len);
/* Blocks until the core thread has stopped; no callbacks after it returns. */
void threesea_core_stop(ThreeseaCore *core);
/* For the end of the process: waits a few seconds at most. True: as threesea_core_stop. False: the
 * core thread still runs and can still call back, so keep user_data and leave with _exit. */
bool threesea_core_quit(ThreeseaCore *core);
/* The core's mark of a removal of the ID that never finished (the app was killed in it):
 * -1 none, else whether linked devices remain (0 or 1). */
int threesea_data_removal_mark(const char *data_dir);
/* Removes everything in the data directory (database, media, logs): the ID is being removed. The
 * core must have stopped. False if something stayed. */
bool threesea_data_remove(const char *data_dir);

/* Decrypted media of a message (thumbnail or file) from the encrypted media store; thread-safe.
 * NULL if not on the device. Free with threesea_buffer_free(buffer, *out_len). */
uint8_t *threesea_media_read(const ThreeseaCore *core, int64_t message, bool thumbnail, size_t *out_len);
/* Ogg Opus voice message decoded to 16-bit mono 48 kHz PCM (little endian); NULL if not possible.
 * Free with threesea_buffer_free. */
uint8_t *threesea_voice_decode(const ThreeseaCore *core, int64_t message, size_t *out_len);
/* A picture that isn't a message's (group picture: name = group key); NULL if none. */
uint8_t *threesea_picture_read(const ThreeseaCore *core, const char *name, size_t *out_len);
void threesea_buffer_free(uint8_t *buffer, size_t len);

/* The size of the picture of the video at `path` as a player shows it, from the file's index (MP4 and
 * QuickTime). False if it can't be told. Any thread, no core needed. */
bool threesea_video_size(const char *path, uint32_t *width, uint32_t *height);

/* Database key encryption with the app lock passphrase (ADR 0009); scrypt, so slow: call off the
 * GUI thread. Buffers are freed with threesea_buffer_free; NULL on failure (wrong passphrase). */
uint8_t *threesea_key_wrap(const uint8_t *key, size_t key_len, const char *passphrase, bool keyboard,
                           size_t *out_len);
uint8_t *threesea_key_unwrap(const uint8_t *blob, size_t blob_len, const char *passphrase, size_t *out_len);
/* -1: not a wrapped key; 1: passphrase typed on the keyboard; 0: digits. */
int threesea_key_wrapped(const uint8_t *blob, size_t blob_len);

/* First QR code in an 8-bit grey image; free with threesea_string_free; NULL if none. */
char *threesea_qr_decode(const uint8_t *grey, size_t width, size_t height);

/* Group call media frames (ADR 0012), thread-safe, no core needed: codec 0 Opus, 1 VP8. Encrypt
 * needs out_cap >= len + 22, decrypt out_cap >= len. Returns the bytes written into `out`, or -1:
 * drop the frame (no key, not authentic, invalid). */
ptrdiff_t threesea_group_call_encrypt(uint8_t codec, const uint8_t *frame, size_t len, uint8_t *out, size_t out_cap);
ptrdiff_t threesea_group_call_decrypt(uint32_t participant, uint8_t codec, const uint8_t *frame, size_t len,
                                      uint8_t *out, size_t out_cap);
/* Whether a received VP8 frame may reach the decoder: 1 yes, 0 no, -1 no and it was a key frame
 * that is too large or broken. `*waiting` is the stream's state (starts out true), used on the
 * stream's thread only. */
int32_t threesea_call_vp8_filter(const uint8_t *frame, size_t len, bool *waiting);

/* Encrypt one of our VP8 frames of simulcast layer `layer` (0 l, 1 m, 2 h) and split it into RTP
 * payloads (VP8 descriptor included) of at most max_payload bytes, written one after the other
 * into `out`, their lengths into `lengths`. Returns the number of payloads, or -1: drop it. */
ptrdiff_t threesea_group_call_vp8_packets(uint8_t layer, const uint8_t *frame, size_t len, size_t max_payload,
                                          uint8_t *out, size_t out_cap, size_t *lengths, size_t max_packets);

#ifdef __cplusplus
}
#endif

#endif
