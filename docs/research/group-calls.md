# Research: Threema group calls on GStreamer webrtcbin

Status: research notes, 2026-10-23. Only public sources were read. Each claim gives its source.
Abbreviations used for sources:

- **[PROTO]** `third_party/libthreema/threema-protocols/src/group-call.proto` (the vendored copy matches upstream 6.5.3)
- **[STRUCT]** `third_party/libthreema/threema-protocols/src/group-call.struct.yml`
- **[CSP-E2E]** `third_party/libthreema/threema-protocols/src/csp-e2e.proto`; **[COMMON]** `.../common.proto`;
  **[DIR-API]** `.../directory.openapi.yml`
- **[AND]** `third_party/threema-android/app/src/main/java/ch/threema/app/voip/groupcall/` (6.5.3-1216);
  **[AND-domain]** `third_party/threema-android/domain/src/main/java/ch/threema/domain/protocol/`
- **[PATCH]** Threema's libwebrtc patch with the frame crypto that Android and iOS use:
  <https://github.com/threema-ch/webrtc-build-docker/blob/master/patches/group-call-frame-crypto.patch>
  (repository HEAD `6362bd0746`, 2026-10-20). It adds `pc/threema_group_call_frame_transformer.{cc,h}`,
  which is the code behind `org.webrtc.ThreemaGroupCallFrameCryptoContext`. The other patches of
  that build are in the same directory. <https://github.com/threema-ch/webrtc-android> holds only the
  prebuilt `.so`/`.jar`.
- **[DESK]** threema-desktop, branch `stable`, <https://github.com/threema-ch/threema-desktop/tree/stable>:
  - `apps/desktop/src/worker/backend/media-crypto-worker.ts` (frame crypto, Insertable Streams)
  - `apps/desktop/src/common/dom/crypto/group-call.ts` (PCMFK derivation)
  - `apps/desktop/src/common/webrtc/group-call.ts` (codecs, encodings, SDP builder)
  - `apps/desktop/src/common/dom/webrtc/group-call.ts` (transceivers)
- **[GST]** GStreamer `1.26` branch, <https://gitlab.freedesktop.org/gstreamer/gstreamer/-/tree/1.26/subprojects>.

The SFU itself is not public. threema-ch's GitHub organisation has no SFU repository, and a web
search found none. So anything about how the SFU behaves is inferred from what the clients send.

---

## 1. Per-frame media encryption

### 1.1 Keys

- `GCKH = BLAKE2b(key=GCK, salt='#', personal='3ma-call')`, 32 bytes. Source: [PROTO] "Key Derivation", [AND] `GroupCallDescription.kt:32`.
- PCMK: 32 random bytes per participant. It is the same towards every other participant. Its starting state is epoch 0, ratchet counter 0. Sources: [PROTO] `MediaKey`, [AND] `sfu/webrtc/LocalParticipantCallMediaKey.kt`.
- Ratchet: `PCMK' = BLAKE2b(key=PCMK, salt="m'", personal='3ma-call')`, with no input. Source: [PROTO]. In [PATCH] it is `derive_next_pcmk`.
- **Frame key:** `PCMFK = BLAKE2b(out=32, key=PCMK, salt='mf', personal='3ma-call', input=GCKH)`. Source: [PROTO], [PATCH] `derive_pcmfk`, and [DESK] `dom/crypto/group-call.ts` (`deriveKey(32, pcmk, {personal, salt:'mf', input: gckh})`).
  - BLAKE2b salt and personal are padded with zeros to 16 bytes. Source: [PATCH] `hash_personal`, `hash_pcmfk_salt`.
  - This is keyed BLAKE2b with salt and personalisation, which `libthreema`'s `blake2b` already provides. Android calls it through `CryptoCallUtils.gcBlake2b256` → `libthreema.blake2bMac256`.
- **Cipher: AES-256-GCM**, 12-byte nonce, 16-byte tag placed after the ciphertext. Source: [STRUCT] `e2e.frame`, [PATCH] `EVP_aead_aes_256_gcm()`, `kNonceLength = 12`, `kTagLength = 16`.
- Note: the general rule "Encryption cipher: XSalsa20-Poly1305" in [PROTO] does **not** apply to media frames. It covers handshakes, P2P envelopes and the call state only.

### 1.2 MFSN (media frame sequence number)

- MFSN is a u32 counter per sender. It is **shared by all of the sender's media streams**: microphone, camera, every simulcast layer, and screen. Sources: [STRUCT] footer `mfsn` note "the MFSN is shared across different media types"; [PATCH] one `GroupCallFrameEncryptorKeyContext` holds `state.mfsn` and is shared by every `GroupCallFrameEncryptor` created from it (`CreateEncryptor`).
- The counter starts at 0. Sources: [PATCH] `state({0})`, [DESK] `SequenceNumberU32`.
- It is **never reset**, not on a ratchet and not on a new epoch. Source: [PROTO] Join/Leave "Do **not** reset the MFSN!"; [PATCH] `SetPcmk` comment.
- It must be incremented under a mutex because streaming threads run concurrently. Source: [STRUCT] "IMPORTANT: The MFSN must be guarded by a mutex"; [PATCH] `webrtc::MutexLock` in `Apply()`.
- Spec quirk: [STRUCT] increments MFSN in step 4 **and** again in step 10 of "When creating a media frame". Both implementations increment once per frame: [PATCH] `state.mfsn++` and [DESK] `state.mfsn.next()`. The receiver takes the MFSN from the footer, so either works. Follow the implementations and increment once. [PATCH] aborts with `RTC_CHECK` before the counter would reach `UINT32_MAX`.

### 1.3 Nonce, AAD, wire layout

```
nonce (12 B) = u32-le(mfsn) || 00 00 00 00 00 00 00 00
ad           = u8(epoch) || u8(ratchet_counter) || u32-le(mfsn) || unencrypted-header
```
Sources: [STRUCT] steps 5 and 7, [PATCH] `Transform` ("Encode nonce", "Encode additional data"), [DESK] `media-crypto-worker.ts` around lines 545–560.

The participant ID is **not** part of the nonce or the AAD. Two senders never share a key, because each sender has its own random PCMK.

Encrypted frame, byte by byte:

```
offset 0                : unencrypted header, H bytes (H = 0 Opus, 3 VP8 delta, 10 VP8 key)
offset H                : AES-256-GCM ciphertext of data[H..], same length as plaintext payload
offset H+P              : GCM tag, 16 bytes
offset H+P+16           : footer, 6 bytes:
        +0  u8      key epoch            (MediaKey.epoch, wraps 255 -> 0)
        +1  u8      key ratchet counter  (MediaKey.ratchet_counter, max 255)
        +2  u32-le  MFSN
total = plaintext length + 22
```
Sources: [STRUCT] `frame.data` field doc and `footer`; [PATCH] "Write the 6 byte footer" (it copies the first 6 bytes of `additional_data`, so the footer equals the AAD prefix).

There is **no prefix** in front of the frame. Everything is appended at the end, and the codec's own header bytes stay at the front in clear.

### 1.4 Unencrypted header per codec

| Codec | Unencrypted bytes | Rule |
|---|---|---|
| Opus | 0 | The whole Opus packet is encrypted, TOC byte included. |
| VP8 | 10 if key frame, 3 otherwise | Key frame ⇔ `(data[0] & 0x01) == 0`. VP8 frames shorter than 3 bytes are dropped, and key frames shorter than 10. |
| H.264 / VP9 / AV1 | — | **Not supported** in group calls. The spec only defines Opus and VP8, [PATCH] `enum class Codec { kOpus, kVp8 }`, and Android only uses VP8 ("We only use VP8 for group calls right now", [AND] `sfu/webrtc/FactoryCtx.kt`). |

Sources: [STRUCT] "Steps to extract the unencrypted header", [PATCH] `kVp8DeltaFrameLength = 3`, `kVp8KeyFrameLength = 10`, [DESK] `getVp8Header`.

The 3 and 10 bytes are the VP8 *bitstream* frame header (RFC 6386 §9.1), **not** the RTP VP8 payload descriptor. The transform runs on encoded frames before packetisation:

- bytes 0–2: frame tag. Bit 0 is `!key_frame`, bits 1–3 are the version, bit 4 is `show_frame`, bits 5–23 are `first_part_size`.
- key frames, bytes 3–5: start code `9d 01 2a`.
- key frames, bytes 6–7 and 8–9: width and height (14 bits each, plus 2 bits scale).

These bytes stay readable so that the SFU and depayloaders can see key frames and the resolution. The RTP VP8 payload descriptor (picture ID, TL0PICIDX, TID) is added later by the packetiser and is protected only by SRTP hop by hop.

Empty frames: with Opus DTX, a 0-byte frame is passed through unencrypted on both sides. Video frames of 0 bytes are dropped. Sources: [PATCH] encryptor and decryptor, "Opus DTX generates empty frames"; [DESK] `_allowEmptyFrame`.

Length limits:
- [STRUCT] says plaintext must be ≤ 65536 − 16 − 6 and received data ≤ 65536.
- **Both implementations use 1 MiB:** `kMaxEncryptedFrameLength = 1048576` in [PATCH] and `MAX_ENCRYPTED_FRAME_LENGTH = 1048576` in [DESK].
- Accept up to 1 MiB. VP8 key frames at 720p can exceed 64 KiB.

### 1.5 Decryption and key selection

Each remote participant has an ordered list of PCMK states, sorted by epoch in ascending order. Entries come from `Handshake.Auth.media_keys` (current key, then the pending one) and from `Envelope.rekey`. Source: [AND] `sfu/connection/Connected.kt:396` (`decryptor.addPcmk`).

For each frame ([PATCH] `AlignWith`, [STRUCT] receive steps 5–7):

1. Read the footer, then find the state with the matching epoch, dropping older-epoch states.
   - Epoch wrap heuristic: a frame with epoch 0 counts as "older" while the current state's epoch is < 128.
   - A key whose epoch is older than the newest known key is ignored ([PATCH] `AddPcmk`).
2. If the frame's ratchet counter is lower than the state's, drop the frame. If it is higher, ratchet the PCMK forward (`PCMK'`) and re-derive PCMFK until the counters match.
3. Decrypt with AES-256-GCM, using the nonce and AAD from the footer and the clear header. Discard the frame on failure.

There is no replay window: the MFSN is not checked on receive ([PATCH] comment).

Sender side ([PROTO] Join/Leave, [AND] `Connected.kt:438-498`):

- When a participant joins: ratchet once and apply immediately. The ratchet counter must not pass 255.
- When a participant leaves: create a new random PCMK with epoch + 1 (255 wraps to 0) and ratchet counter 0, and send it as `rekey` to every authenticated participant. Apply it **2 s later**.
  - If another leave happens while a key is pending, mark that pending key *stale*; after applying it, run the steps again.
- [PATCH] `SetPcmk` enforces that keys only move forward: `RTC_CHECK` epoch > old, or same epoch with ratchet counter > old, or 255 → 0.

### 1.6 Where to put it in our code

- Implement the frame crypto in Rust (`core`), with AES-256-GCM, BLAKE2b and the MFSN mutex. GStreamer calls it from a small C element or an `identity`-style pad probe through the FFI. AES-GCM is new: libthreema only has chacha20/xsalsa20 (`third_party/libthreema/lib/Cargo.toml`). `aes-gcm` from RustCrypto is the obvious crate (§7.7: the commit must justify it).
- One encryptor context is shared by every outgoing stream, because the MFSN is shared. Keep one decryptor context per remote participant ID.
- Test vectors: neither upstream repository ships any for frame crypto. Generate them from [DESK]'s algorithm, or cross-check against a real Android client.

---

## 2. Codecs, RTP setup and SDP

### 2.1 Codecs (fixed; [DESK] says "MUST be kept in sync with the SFU")

| | PT | rtpmap | rtcp-fb | fmtp |
|---|---|---|---|---|
| Microphone | 111 | `opus/48000/2` | `transport-cc` | `minptime=10;useinbandfec=1;usedtx=1` |
| Camera / screen | 96 | `VP8/90000` | `transport-cc`, `ccm fir`, `nack`, `nack pli`, `goog-remb` | — |
| Camera / screen RTX | 97 | `rtx/90000` | — | `apt=96` |

Sources: [AND] `sfu/GroupCallSessionDescription.kt` (`MICROPHONE_CODECS`, `CAMERA_CODECS`), [DESK] `common/webrtc/group-call.ts` (`MEDIA_CODECS`).

Android rewrites the Opus fmtp in its *local* answer to exactly that string (`patchLocalDescription`). No CBR patch is applied here, unlike 1:1 calls.

### 2.2 Simulcast

Camera, identical on Android and Desktop:

| rid | scale down | max bitrate | scalability |
|---|---|---|---|
| `l` | 4 | 100 kbit/s | L1T3 |
| `m` | 2 | 250 kbit/s | L1T3 |
| `h` | 1 | 1.2 Mbit/s | L1T3 |

Sources: [AND] `CAMERA_SEND_ENCODINGS` (marked "TODO(ANDR-1952): Refine parameters"), [DESK] `CAMERA_ENCODINGS`. Android applies them with `degradationPreference = BALANCED` ([AND] `sfu/webrtc/ConnectionCtx.kt`, `setCameraVideoSimulcastEncodingParameters`).

Screen (Desktop only): `l` (scale down 4, 100 kbit/s, L1T2) and `h` (scale down 1, 1.2 Mbit/s, L1T2) ([DESK] `SCREEN_ENCODINGS`).

L1T3 means one spatial layer and three temporal layers per encoding. The SFU picks the temporal layer from the VP8 payload descriptor's TID and TL0PICIDX to meet `ParticipantCamera.Subscribe.desired_fps`, and the simulcast stream to meet `desired_resolution` ([PROTO] `ParticipantCamera.Subscribe`: "The SFU will select the spatial layer … temporal layer that fits best").

**Is simulcast mandatory?** No source says so, and the SFU's code is not available. What is known:

- The SDP is never sent to the SFU. Each client builds the "remote offer" itself ([AND] `generateRemoteDescription`, [DESK] same structure). The SFU learns only the DTLS fingerprint (in `Join`) and then raw RTP. It must therefore classify incoming packets **by header extensions alone**: MID for audio vs. camera, and `rtp-stream-id` (RID) for the simulcast layer.
- So whatever we send must carry the MID and, for video, a RID extension with the ID from `JoinResponse.rtp_header_extension_ids`. Packets without a RID on the camera MID will most likely be dropped. That is untested.
- Sending one layer is probably tolerated if it carries a valid RID: the SFU can only forward layers that exist, and a sender's link can drop layers at any time. Try one encoding tagged `h` (or `m`) first. Test whether receivers asking for small tiles still get video when `l` is missing. Whether a stream without temporal-layer information (no T/TID fields) is forwarded is also unknown, so send L1T3 with TID fields.
- Android notes that VP8 hardware encoders do not handle simulcast and falls back to software for all three layers ([AND] `FactoryCtx.kt`). Mobile clients therefore really do encode three VP8 streams in software.

### 2.3 RTP header extensions

The IDs come from `SfuHttpResponse.Join.rtp_header_extension_ids` ([PROTO] lines 641–663). The same ID is used on every m-line. If the response lacks them, these fallbacks apply ([AND] `sfu/RtpHeaderExtensionIds.kt`):

| URI | Microphone | Camera/screen | Fallback ID |
|---|---|---|---|
| `urn:ietf:params:rtp-hdrext:sdes:mid` | ✓ | ✓ | 1 |
| `urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id` | | ✓ | 2 |
| `urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id` | | ✓ | 3 |
| `http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time` | ✓ | ✓ | 4 |
| `http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01` | ✓ | ✓ | 5 |
| `urn:3gpp:video-orientation` | | ✓ | 11 |
| `urn:ietf:params:rtp-hdrext:toffset` | | ✓ | 12 |

- `ssrc-audio-level` is deliberately left out ("Disabled until we can use cryptex", [DESK] `HEADER_EXTENSION_URIS`). Android disables encrypted header extensions (`setEnableEncryptedRtpHeaderExtensions(false)`) and sets `offerExtmapAllowMixed = true` ("NEVER disable this", [AND] `ConnectionCtx.kt`).
- Threema's libwebrtc **sends `video-orientation` on the last packet of every frame** ([webrtc-build-docker `patches/dont-leak-video-orientation.patch`](https://github.com/threema-ch/webrtc-build-docker/blob/master/patches/dont-leak-video-orientation.patch)). When CVO is negotiated, libwebrtc does not rotate the pixels. Video from Android/iOS phones may therefore arrive in sensor orientation, and the receiver has to rotate it from that extension.

### 2.4 Transport parameters (Android `ConnectionCtx.getPeerConnectionConfiguration`)

- No ICE servers. Bundle policy is max-bundle, RTCP mux is required, TCP candidates are disabled.
- DTLS certificate: ECDSA.
- Unified Plan.
- **SRTP: AES-GCM suites on, AES_CM_128_HMAC_SHA1_80 and _32 off.** Threema's libwebrtc reorders the suites to prefer AEAD_AES_256_GCM, then AES_128_GCM, then CM_SHA1_80 ([webrtc-build-docker `patches/srtp-cipher-suites.patch`](https://github.com/threema-ch/webrtc-build-docker/blob/master/patches/srtp-cipher-suites.patch)). Android's **1:1** calls keep CM_SHA1_80 on (`voip/PeerConnectionClient.java:610-611`); group calls turn it off on purpose.
- Data channel: label `p2s`, `negotiated=true`, `id=0`, `ordered=true`. Every message is a `ParticipantToSfu.Envelope` / `SfuToParticipant.Envelope` protobuf ([PROTO], [AND] `sfu/webrtc/DataChannelType.kt`). `max-message-size` is 131072 ([DESK] `DATA_CHANNEL_PARAMETERS`).
- ICE candidates come from `Join.addresses`, written as `candidate:0 1 udp <1 for IPv4 | 2 for IPv6> <ip> <port> typ host` with mid "" and index 0. IPv6 is filtered unless enabled ([AND] `ConnectionCtx.addIceCandidates`).

### 2.5 The locally generated "remote offer" ([AND] `GroupCallSessionDescription.kt`, [DESK] `createRemoteSessionDescription`)

**MIDs.** The SDP token alphabet ([AND] `SDP_TOKEN_RANGE`) is these 79 characters, in this exact order:

```
! # $ % & ' * + - . 0-9 A-Z ^ _ ` a-z { | } ~
```

- The MID list is every 1-character token, followed by every 2-character token (left-major). That gives 79 + 79² = 6320 MIDs.
- Participant *p* gets the slice `[8p, 8p+8)`. Index 0 is the microphone, 1 the camera, 2 the screen (Android: reserved `_r1`), 3–6 are reserved, and 7 is data. The data MID is used only for the local participant.
- Example: participant 0 → mic `!`, camera `#`, screen `$`, data `+`; participant 1 → mic `-`, camera `.`. Both clients must generate the list in exactly the same order. This also caps a call at 790 participants.

**Session lines:**

```
v=0
o=- <id> <version++> IN IP4 127.0.0.1
s=-
t=0 0
a=group:BUNDLE <all mids in m-line order>
a=msid-semantic: WMS *
a=ice-ufrag:<Join.ice_username_fragment>
a=ice-pwd:<Join.ice_password>
a=ice-options:trickle
a=ice-lite
a=fingerprint:sha-256 <Join.dtls_fingerprint as AA:BB:..>
a=setup:passive
```

**m-line order.** The local participant comes first: audio, then video (camera), then (Desktop only) screen video, then `m=application 9 UDP/DTLS/SCTP webrtc-datachannel` with `a=sctp-port:5000` and `a=max-message-size:131072`.

After that come all remote participants, in the order they were **first seen**. Participants who left are kept as `inactive` lines with port 0 and stay in BUNDLE. m-lines are never removed or reordered (`mLineOrder` is a `LinkedHashSet`).

**Per RTP m-line:**

```
m=<audio|video> <9 | 0 if inactive> UDP/TLS/RTP/SAVPF <pts>
c=IN IP4 0.0.0.0
a=rtcp:9 IN IP4 0.0.0.0
a=mid:<mid>
a=extmap:<id> <uri>          (per 2.3)
a=<recvonly | sendonly | inactive>
a=rtcp-mux
a=rtcp-rsize                 (video only)
a=rtpmap / a=rtcp-fb / a=fmtp per codec (2.1)
```

- Local camera only (and Desktop screen) also gets `a=rid:l recv`, `a=rid:m recv`, `a=rid:h recv` and `a=simulcast:recv l;m;h`.
- Directions are from the SFU's point of view: local lines are `recvonly` (SFU receives) and remote participants' lines are `sendonly`.
- There are **no `a=ssrc` lines**. Remote streams are matched only by the MID header extension.

**Renegotiation.** On every join or leave the client regenerates the offer and applies it as the remote description, then creates and applies its own answer ([AND] `ConnectionCtx.updateCall`). Nothing is sent to the SFU.

**Interop note.** Android has only mic and camera m-lines per participant, while Desktop has mic, camera and screen. Each client's SDP is private, so this works. Screen share is advertised by the SFU through `Join.supported_features & SCREEN_SHARE` ([PROTO] `SupportedFeature`).

---

## 3. SFU HTTP API and credentials

### 3.1 SFU credentials from the directory

- Endpoint: `POST <directory>/identity/sfu_cred`, JSON. It uses the same two-phase challenge as the other authenticated directory calls ([DIR-API] lines 483–530 and 1069–1100; [AND-domain] `domain/src/main/java/ch/threema/domain/protocol/api/APIConnector.java:765-804`).
  1. Send `{"identity": "<ID>"}`. The reply is `{"token": b64, "tokenRespKeyPub": b64}`.
  2. Send `{"identity", "token", "response"}`, where `response` comes from `makeTokenResponse` (X25519 shared secret with `tokenRespKeyPub`, the same as `set_featuremask` and `turn_cred`, which our core presumably already has).
- Success response: `{"success": true, "sfuBaseUrl": str, "allowedSfuHostnameSuffixes": [str], "sfuToken": str, "expiration": int seconds}`. On error: `{"success": false, "error": str}`.
- Android caches the token until `now + expiration`, refreshes it on 401, and allows at most one retry ([AND] `sfu/SfuConnectionImpl.kt`, `sfu/connection/Joining.kt`).
- URL check: the scheme must be `https`, and the host (with `:port` if one is given) must **end with** one of the suffixes. This applies to our own base URL and to every received `GroupCallStart.sfu_base_url` ([PROTO] "Obtain SFU Information", [AND-domain] `api/SfuToken.kt`).

### 3.2 Peek

- `POST <sfu_base_url>/v1/peek/<hex(call_id)>`. Headers: `Authorization: ThreemaSfuToken <sfuToken>`, `Content-Type: application/octet-stream`, and a `User-Agent`. The body is protobuf `SfuHttpRequest.Peek{call_id=1: bytes}`.
- Timeout: 5 s (`ProtocolDefines.GC_PEEK_TIMEOUT_MILLIS`).
- Response codes: 200 with `SfuHttpResponse.Peek{started_at=1 u64 ms, max_participants=2 u32, encrypted_call_state=3 optional bytes}`; 401 for an invalid token; 404 if the call is not running; 400 for bad input. Source: [PROTO] lines 474–586, [AND] `SfuConnectionImpl.postPeek`.
- `encrypted_call_state` is a 24-byte random nonce followed by XSalsa20-Poly1305 under `GCSK = BLAKE2b(key=GCK, salt='s', personal='3ma-call')`, wrapping a `CallState` ([PROTO]).
- `call_id = BLAKE2b(out=32, salt='i', personal='3ma-call', input=creator_identity_utf8 || group_id(8) || u8(protocol_version) || GCK || utf8(sfu_base_url))`. This is unkeyed ([PROTO] "Group Call ID Derivation", [AND] `sfu/CallId.kt`).
- Refresh: repeat every 10 s while the call is running (30 s while the conversation is not visible). Retire a call after a 404, or after ≥ 3 failures once it is older than 10 h ([PROTO] "Periodic Refresh", `ProtocolDefines.java:171-179`).

### 3.3 Join

- `POST <sfu_base_url>/v1/join/<hex(call_id)>`, with the same headers. The body is `SfuHttpRequest.Join{call_id=1 bytes, protocol_version=2 u32 (=1), dtls_fingerprint=3 bytes}`. The fingerprint is the **raw 32-byte SHA-256** of our DTLS certificate, which must therefore exist **before** the SDP ([AND] `sfu/connection/Joining.kt:37-45`).
- Timeout: 20 s on Android (`GC_JOIN_TIMEOUT_MILLIS`); the spec says 10 s.
- Response codes: 200, 401 (refresh the token once), 419 (protocol version unsupported), 503 (call full).
- 200 body, `SfuHttpResponse.Join`:
  - `started_at` = 1
  - `max_participants` = 2
  - `participant_id` = 3 (our ID; it determines our MIDs)
  - `addresses` = 4 (each `{protocol=1 UDP, port=2, ip=3}`; IPv4 is mandatory)
  - `ice_username_fragment` = 5
  - `ice_password` = 6
  - `dtls_fingerprint` = 7 (raw SHA-256)
  - `supported_features` = 8 (bitmask)
  - `rtp_header_extension_ids` = 9
- Then: connect within 30 s, and wait for `SfuToParticipant.Hello{participant_ids}` on the data channel ([PROTO] `Join` steps 6–7, [AND] `ConnectionCtx.setSfuHelloMessageObserver`).
- If fewer than 4 other participants are present, the microphone starts on ([PROTO] Join Steps 6).
- Our P2P handshake runs with each participant, after which we subscribe to their microphone (`ParticipantToSfu.ParticipantMicrophone.subscribe`). Camera subscriptions carry `desired_resolution` and `desired_fps` ([AND] `sfu/messages/P2SMessage.kt:88-100`).
- One participant is designated to post `UpdateCallState` every 10 s ([PROTO] "State Update").

---

## 4. CSP `GroupCallStart`

- Message type **`0x4f`** (`GROUP_CALL_START`). It is a group message: the `csp-e2e.GroupCallStart` protobuf is wrapped in a `group-member-container`, which is `creator-identity (8 B) || group-id (8 B) || inner-data` ([COMMON] lines 212–214, `csp.struct.yml` `group-member-container`, `ProtocolDefines.MSGTYPE_GROUP_CALL_START = 0x4f`).
- Fields ([CSP-E2E] `message GroupCallStart`):
  - `uint32 protocol_version = 1`, currently `1` (`GC_PROTOCOL_VERSION`)
  - `bytes gck = 2`, 32 bytes, length checked in [AND-domain] `GroupCallStartData.kt`
  - `string sfu_base_url = 3`
- Properties ([CSP-E2E]):
  - flags `0x01` (push)
  - user profile distribution: yes
  - **exempt from blocking**: yes
  - protect against replay: yes
  - reflected incoming and outgoing
  - no delivery receipts and no reactions
- Send it only to group members with feature mask `GROUP_CALL_SUPPORT = 0x0080` ([COMMON] line 120). We must also set that bit in our own feature mask once group calls work.
- On receive, validate the URL against the allowed suffixes, ignore a duplicate GCK, add the call even if the protocol version is unsupported, then run the refresh steps ([CSP-E2E] "Common Group Call Start Receive Steps").
- The creator waits about 2 s after the join before sending it ([PROTO] Join Steps 7.1).

---

## 5. Feasibility on GStreamer 1.26 webrtcbin

**Verdict:** feasible. The design is a fit: frame encryption sits between encoder and payloader on the send side and between depayloader and decoder on the receive side, which matches where libwebrtc's frame transformer runs. Simulcast sending is supported by 1.26 webrtcbin. The SDP is entirely local, so webrtcbin's SDP quirks only have to satisfy webrtcbin itself.

Three items need patched or own GStreamer elements. Two of them (2 and 3) are **hard blockers** with stock 1.26.

### Blockers (stock SFOS 5.1 GStreamer 1.26)

1. **`rtpvp8pay` cannot packetise encrypted VP8.**
   - Its `handle_buffer` always calls `gst_rtp_vp8_pay_parse_frame()`, which runs a bool decoder over the first partition and reads the partition size table from the payload. With an encrypted payload, these are garbage. Whenever the decoded partition count is above 1, random 24-bit sizes are read and the parse fails with `GST_ELEMENT_ERROR "Failed to parse VP8 frame"` / `GST_FLOW_ERROR`. That makes it effectively random per frame ([GST] `gst-plugins-good/gst/rtp/gstrtpvp8pay.c`, `parse_frame` ~l.272–440, `handle_buffer` ~l.677).
   - The partition information only matters when there is no `GstVP8Meta`. With temporal-scalability meta the payloader already uses `PID 0` and sets S only on the first packet (l.629–635), which is all libwebrtc does too.
   - **Fix:** ship our own VP8 payloader (RFC 7741 is small): a patched copy of `rtpvp8pay` that skips the parse when it fails or meta is present, or a Rust/C element. It must keep reading `GstVP8Meta` for TID/TL0PICIDX/Y, and our encrypt element must copy that meta onto its output buffer.
   - `rtpvp8depay` is fine. It reads only the descriptor and, on key frames, bytes 0–9 of the frame, which stay in clear (`gstrtpvp8depay.c` ~l.425–460).
   - Opus is fine: `rtpopuspay` and `rtpopusdepay` never look inside the payload. Only `rtpopuspay dtx=true` drops buffers ≤ 2 bytes (l.333). Our encryptor should drop or pass through 0-byte DTX frames the way libwebrtc does.

2. **DTLS-SRTP profile: GStreamer 1.26 offers only `SRTP_AES128_CM_SHA1_80`, and Threema's group-call clients turn that suite off.**
   - In 1.26, `gstdtlsagent.c:200` is `SSL_CTX_set_tlsext_use_srtp(ctx, "SRTP_AES128_CM_SHA1_80")`, and `gstdtlsconnection.c` (~l.923–940) rejects every other profile.
   - Android group calls use `setEnableAes128Sha1_80CryptoCipher(false)` and GCM only (§2.4). The SFU presumably speaks AES-GCM. Whether it also accepts CM_SHA1_80 is **unknown**. It may, since stock Chromium (Desktop) offers it, but that is unverified, and the SFU is the DTLS server that picks the suite.
   - GStreamer `main` added `SRTP_AEAD_AES_128_GCM` and `_256_GCM` on 2026-09-29 (commits `ca6d5449`, `c851d42c`; [GST main] `gstdtlsagent.c:200`). `srtpenc`/`srtpdec` in 1.26 already handle GCM through libsrtp2.
   - **Fix if the SFU refuses CM80:** backport those two commits into a Chum-bundled `libgstdtls.so`. Test first with the stock plugin; this is the first thing to try against the real SFU.

3. **Video orientation (CVO).** GStreamer 1.26 has no `urn:3gpp:video-orientation` (nor `abs-send-time` or `toffset`) header-extension implementation. [GST] `gst-plugins-good/gst/rtpmanager/` has `mid`, `streamid`, `repairedstreamid`, `twcc`, `clientaudiolevel` and `ntp`; `gst/rtp/` has `colorspace`. Threema phones send CVO on every frame (§2.3) and likely do not rotate pixels. **Fix:** a small `GstRTPHeaderExtension` subclass, or an RTP pad probe before the depayloader that reads the extension, plus `videoflip` (or rotation in QML). This is probably the same issue as in 1:1 video and should be checked there. Sending CVO ourselves is optional: rotate before encoding.

### Workable with care (not blockers)

- **Simulcast send.** Since 1.24/1.26, webrtcbin can send simulcast: N payloaders → `rtpfunnel` → one `sink_%u` whose caps carry `rid-<id>=send`, `a-simulcast`, and `extmap-N` for MID/RID/RRID. Each payloader gets `rtphdrextmid` and `rtphdrextstreamid` with `rid=…` ([GST] `gst-plugins-bad/tests/check/elements/webrtcbin.c` `create_simulcast_audio_caps`, `add_simulcast_audio_test_src_harness`, `test_simulcast`; `gst-plugins-base/gst-libs/gst/sdp/gstsdpmessage.c` ~l.4049–4086 turns `rid-*` caps into `a=rid`). Costs:
  - three `vp8enc` instances (software, like Android) with `temporal-scalability-*` set for L1T3 (layer IDs 0,2,1,2, periodicity 4);
  - one shared MFSN across all three encrypt elements;
  - a single-layer fallback to try first (§2.2).
- **No SSRCs in the SDP, MID-only demux.** webrtcbin maps incoming SSRCs to transceivers through the MID header extension. This needs `extmap` for MID in the receive caps (webrtcbin ~l.7690 and l.8130). New SSRCs on one MID (layer switches, rejoins) create extra pads ("duplicate output ssrc?" path). Handle `pad-added` per SSRC.
- **m-line churn.** Every join or leave means a new remote offer with more m-lines, and old ones turn `inactive`/port 0 but stay in BUNDLE. That is unusual (a port-0 line in BUNDLE). webrtcbin's tolerance for repeated renegotiation with growing offers and port-0 bundled lines must be tested. If webrtcbin objects, our SDP builder may write `a=bundle-only` or keep port 9 with `a=inactive`. Only we read it, so this is our choice, provided the MIDs stay stable.
- **ICE lite.** webrtcbin becomes ICE-controlling when the remote description has `a=ice-lite` ([GST] `gstwebrtcbin.c` ~l.6884–6895). As the answerer we are DTLS `active` against `setup:passive`, which is correct.
- **DTLS fingerprint before SDP.** It is required for `Join`. `bridge/callengine.cpp` already creates its own ECDSA P-256 certificate (PEM) and its fingerprint for 1:1 calls (ADR 0012). Reuse it, and send the raw 32 bytes in `Join`.
- **transport-cc.** webrtcbin and rtpbin 1.26 negotiate TWCC (`rtcp-fb-transport-cc`, `rtphdrexttwcc`) and send feedback, so the SFU can estimate our downlink. There is no sender-side bandwidth estimation (`rtpgccbwe` lives in gst-plugins-rs, which SFOS does not ship). Use fixed layer bitrates, as Android's caps do. REMB from the SFU can be read from RTCP (`on-feedback-rtcp`) if needed later.
- **abs-send-time / toffset** are not implemented. We do not send them. That the SFU copes without them is an assumption, since it is not public; it most likely relies on TWCC. Received ones are ignored.
- **RTX (PT 97).** Optional on send (`rtprtxsend` through webrtcbin's `do-nack`). On receive, the SFU may retransmit with `repaired-rtp-stream-id`.
- **Data channel.** `create-data-channel "p2s"` with `negotiated=true, id=0, ordered=true` works the same as the 1:1 in-call channel.

### Found while making audio work (2026-10-26)

- webrtcbin 1.26 does **not** route incoming SSRCs by the MID header extension, only by
  `a=ssrc` lines (which the SFU's offer has none of). Every incoming stream comes out of the first
  receiving transceiver's pad, so `GroupCallEngine` reads the MID from each packet itself.
- `rtphdrextmid` refuses MIDs with characters outside `[A-Za-z0-9]` on write; the SFU's MIDs use
  `-`, so a pad probe writes the one-byte extension.
- libnice 0.1.21 does not send ICE consent freshness checks; the SFU drops us after ~15 s without
  them. We send a STUN binding request every 5 s via `nice_agent_send`.
- A receive branch that returns `not-linked` even once stops `nicesrc` for good (the error
  travels back through the jitter buffer on the next packet): no media, no data channel, the SFU
  drops us. Link every branch completely before starting it; across bins use
  `gst_pad_link_maybe_ghosting`.
- Recalculate latency when a branch joins, or RTCP stays empty ("Generated empty RTCP").
- Video (2026-10-26): a single camera stream tagged RID `h`, every frame TID 0 with the T fields
  present, is accepted by the SFU and forwarded to Android. An `appsrc` feeding webrtcbin must
  state its latency (`min-latency=0`), or latency queries fail ("minimum latency bigger than
  maximum latency").

### Suggested order of experiments

1. Directory `sfu_cred`, peek and join from the core. This needs no media.
2. webrtcbin with the generated offer, the data channel only, and the stock dtls plugin → `SfuToParticipant.Hello`. This settles blocker 2.
3. Opus with frame crypto, then cross-check audio against an Android or Desktop client.
4. VP8 on one layer (rid `h`) with our payloader and CVO handling, then three layers.
