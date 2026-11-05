// SPDX-License-Identifier: AGPL-3.0-only
//! Media frame encryption of group calls (docs/research/group-calls.md §1, Threema's
//! `group-call-frame-crypto.patch`): each encoded audio/video frame is encrypted with AES-256-GCM
//! under the sender's frame key before it is packetised.
//!
//! ```text
//! PCMFK  = BLAKE2b(key=PCMK, salt='mf', personal='3ma-call', input=GCKH)
//! nonce  = u32-le(MFSN) || 8 zero bytes
//! ad     = u8(epoch) || u8(ratchet) || u32-le(MFSN) || clear header
//! frame  = clear header || ciphertext || tag (16) || epoch || ratchet || u32-le(MFSN)
//! ```
//!
//! The clear header: nothing for Opus, 3 bytes of VP8 (10 on key frames). MFSN is one counter for
//! all of a sender's streams, never reset. The state is process-wide (GStreamer's streaming
//! threads call in through the FFI); the app sets the keys.

use std::{
    collections::{BTreeSet, HashMap},
    sync::Mutex,
};

use aes_gcm::{
    Aes256Gcm, KeyInit as _,
    aead::{AeadInPlace as _, generic_array::GenericArray},
};
use zeroize::Zeroizing;

use crate::{group_call_handshake::MediaKey, group_calls::blake2b};

const TAG_LEN: usize = 16;
const FOOTER_LEN: usize = 6;
/// What encryption adds to a frame.
pub const OVERHEAD: usize = TAG_LEN + FOOTER_LEN;
/// Both implementations accept up to 1 MiB.
const MAX_FRAME: usize = 1 << 20;

/// The codec of a frame (its clear header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Opus,
    Vp8,
}

/// Bytes left in clear; `None`: a frame too short to be valid (dropped).
fn clear_header(codec: Codec, frame: &[u8]) -> Option<usize> {
    match codec {
        Codec::Opus => Some(0),
        Codec::Vp8 => {
            let first = *frame.first()?;
            // Bit 0 clear: a key frame (frame tag, start code, size).
            let length = if first & 1 == 0 { 10 } else { 3 };
            (frame.len() >= length).then_some(length)
        }
    }
}

/// How far back a frame number may lie behind the newest one accepted under a key: audio and
/// the video layers share the sender's counter and reach decryption with different delays.
const REPLAY_WINDOW: u32 = 1024;

/// A key state with its derived frame key.
struct KeyState {
    key: MediaKey,
    cipher: Aes256Gcm,
    /// Frame numbers accepted under this key within the window (a frame is taken only once).
    seen: BTreeSet<u32>,
}

impl KeyState {
    fn new(key: MediaKey, gckh: &[u8; 32]) -> Self {
        let pcmfk = Zeroizing::new(blake2b(key.pcmk.as_slice(), b"mf", &[gckh]));
        Self {
            cipher: Aes256Gcm::new(GenericArray::from_slice(pcmfk.as_slice())),
            key,
            seen: BTreeSet::new(),
        }
    }

    /// Whether `mfsn` is new under this key and not too old.
    fn fresh(&self, mfsn: u32) -> bool {
        let newest = self.seen.last().copied().unwrap_or(0);
        !self.seen.contains(&mfsn) && mfsn.saturating_add(REPLAY_WINDOW) > newest
    }

    fn accept(&mut self, mfsn: u32) {
        self.seen.insert(mfsn);
        let newest = self.seen.last().copied().unwrap_or(0);
        while self
            .seen
            .first()
            .is_some_and(|&oldest| oldest.saturating_add(REPLAY_WINDOW) <= newest)
        {
            self.seen.pop_first();
        }
    }
}

fn nonce(mfsn: u32) -> [u8; 12] {
    let mut nonce = [0; 12];
    nonce[..4].copy_from_slice(&mfsn.to_le_bytes());
    nonce
}

#[derive(Default)]
struct State {
    gckh: [u8; 32],
    local: Option<KeyState>,
    mfsn: u32,
    /// Per remote participant: its keys, ascending by epoch.
    peers: HashMap<u32, Vec<KeyState>>,
}

#[cfg(not(test))]
static STATE: std::sync::LazyLock<Mutex<State>> = std::sync::LazyLock::new(Mutex::default);

#[cfg(not(test))]
fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// Each test has a thread of its own, and its own keys: a test that ends a call elsewhere (a leave
// resets the keys) can't take them away from one that is encrypting.
#[cfg(test)]
thread_local! {
    static STATE: &'static Mutex<State> = {
        let state: &'static Mutex<State> = Box::leak(Box::default());
        TEST_STATES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(state);
        state
    };
}

/// Every test thread's keys, so that a leak checker (scripts/test-ffi-asan.sh) sees them as
/// still in use rather than lost.
#[cfg(test)]
static TEST_STATES: Mutex<Vec<&'static Mutex<State>>> = Mutex::new(Vec::new());

#[cfg(test)]
fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .with(|state| *state)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A new call: nothing encrypts or decrypts until keys are set.
pub fn reset(gckh: [u8; 32]) {
    let mut state = state();
    *state = State {
        gckh,
        ..State::default()
    };
}

/// The key our frames go out with now (on joining, ratcheted on joins, replaced 2 s after a
/// rekey). MFSN keeps counting.
pub fn set_local(key: &MediaKey) {
    let mut state = state();
    let gckh = state.gckh;
    state.local = Some(KeyState::new(key.clone(), &gckh));
}

/// Keys of a participant (from its Auth or a rekey): kept by epoch, older epochs than the newest
/// known one ignored.
pub fn add_peer_keys(participant: u32, keys: &[MediaKey]) {
    let mut state = state();
    let gckh = state.gckh;
    let list = state.peers.entry(participant).or_default();
    for key in keys {
        if let Some(newest) = list.last()
            && key.epoch != newest.key.epoch
            && !epoch_newer(key.epoch, newest.key.epoch)
        {
            continue;
        }
        list.retain(|state| state.key.epoch != key.epoch);
        list.push(KeyState::new(key.clone(), &gckh));
    }
}

pub fn remove_peer(participant: u32) {
    state().peers.remove(&participant);
}

/// `candidate` is a newer epoch than `current` (epochs wrap 255 → 0; 0 counts as newer than
/// anything from 128 on, like Threema's `AlignWith`).
fn epoch_newer(candidate: u8, current: u8) -> bool {
    candidate > current || (candidate < current && current - candidate >= 128)
}

/// Encrypt one of our frames; `None`: no key yet, or not a valid frame (drop it). Empty frames
/// (Opus DTX) pass unencrypted.
#[must_use]
pub fn encrypt(codec: Codec, frame: &[u8]) -> Option<Vec<u8>> {
    if frame.is_empty() {
        return (codec == Codec::Opus).then(Vec::new);
    }
    let header = clear_header(codec, frame)?;
    if frame.len() + OVERHEAD > MAX_FRAME {
        return None;
    }
    let mut state = state();
    let mfsn = state.mfsn;
    let local = state.local.as_ref()?;
    let footer = [
        local.key.epoch,
        local.key.ratchet,
        mfsn.to_le_bytes()[0],
        mfsn.to_le_bytes()[1],
        mfsn.to_le_bytes()[2],
        mfsn.to_le_bytes()[3],
    ];
    let mut ad = footer.to_vec();
    ad.extend_from_slice(&frame[..header]);
    let mut out = frame.to_vec();
    let tag = local
        .cipher
        .encrypt_in_place_detached(
            GenericArray::from_slice(&nonce(mfsn)),
            &ad,
            &mut out[header..],
        )
        .ok()?;
    // Before UINT32_MAX (Threema's implementation stops there too).
    state.mfsn = mfsn.checked_add(1)?;
    out.extend_from_slice(&tag);
    out.extend_from_slice(&footer);
    Some(out)
}

/// Decrypt a participant's frame; `None`: unknown sender or key, or not authentic (drop it).
#[must_use]
pub fn decrypt(participant: u32, codec: Codec, frame: &[u8]) -> Option<Vec<u8>> {
    if frame.is_empty() {
        return (codec == Codec::Opus).then(Vec::new);
    }
    if frame.len() > MAX_FRAME || frame.len() < OVERHEAD {
        return None;
    }
    let (body, footer) = frame.split_at(frame.len() - FOOTER_LEN);
    let (epoch, ratchet) = (footer[0], footer[1]);
    let mfsn = u32::from_le_bytes([footer[2], footer[3], footer[4], footer[5]]);
    let (sealed, tag) = body.split_at(body.len() - TAG_LEN);
    let header = clear_header(codec, sealed)?;
    let mut state = state();
    let gckh = state.gckh;
    let keys = state.peers.get_mut(&participant)?;
    // The key of this epoch (older ones are dropped once a newer one is used) at the frame's
    // ratchet step. The footer is not authenticated yet: work on a copy and change the state
    // only for an authentic frame, so a forged one can't spoil the keys (security audit
    // 2026-10-27, S05).
    let position = keys.iter().position(|state| state.key.epoch == epoch)?;
    let stored = &keys[position];
    if ratchet < stored.key.ratchet {
        return None;
    }
    let advanced = if ratchet > stored.key.ratchet {
        let mut key = stored.key.clone();
        while key.ratchet < ratchet {
            key = key.ratcheted()?;
        }
        Some(KeyState::new(key, &gckh))
    } else {
        None
    };
    let current = advanced.as_ref().unwrap_or(stored);
    if !current.fresh(mfsn) {
        return None;
    }
    let mut ad = footer.to_vec();
    ad.extend_from_slice(&sealed[..header]);
    let mut out = sealed.to_vec();
    current
        .cipher
        .decrypt_in_place_detached(
            GenericArray::from_slice(&nonce(mfsn)),
            &ad,
            &mut out[header..],
            GenericArray::from_slice(tag),
        )
        .ok()?;
    if let Some(advanced) = advanced {
        keys[position] = advanced;
    }
    keys.drain(..position);
    keys[0].accept(mfsn);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test runs the whole sequence: the keys go on from one step to the next.
    #[test]
    fn frames_round_trip_with_keys_and_ratchets() {
        reset([7; 32]);
        let ours = MediaKey::random(0);
        assert!(encrypt(Codec::Opus, b"opus").is_none(), "no key yet");
        set_local(&ours);
        // We are participant 5 to ourselves (same process).
        add_peer_keys(5, std::slice::from_ref(&ours));

        let sealed = encrypt(Codec::Opus, b"opus frame").unwrap();
        assert_eq!(sealed.len(), 10 + OVERHEAD);
        assert_eq!(&sealed[sealed.len() - 6..], &[0, 0, 0, 0, 0, 0]);
        assert_eq!(decrypt(5, Codec::Opus, &sealed).unwrap(), b"opus frame");
        // Tampered: dropped.
        let mut bad = sealed.clone();
        bad[0] ^= 1;
        assert!(decrypt(5, Codec::Opus, &bad).is_none());
        // An unknown participant: dropped.
        assert!(decrypt(6, Codec::Opus, &sealed).is_none());

        // VP8 key frame: 10 bytes in clear (and authenticated), delta frame: 3.
        let key_frame = [0x10, 1, 2, 0x9d, 0x01, 0x2a, 9, 9, 9, 9, 42, 43];
        let sealed = encrypt(Codec::Vp8, &key_frame).unwrap();
        assert_eq!(&sealed[..10], &key_frame[..10]);
        assert_eq!(sealed[sealed.len() - 4], 1, "MFSN counts on");
        assert_eq!(decrypt(5, Codec::Vp8, &sealed).unwrap(), key_frame);
        let mut header_changed = sealed.clone();
        header_changed[4] ^= 1;
        assert!(decrypt(5, Codec::Vp8, &header_changed).is_none());
        let delta = [0x11, 1, 2, 3, 4];
        assert_eq!(&encrypt(Codec::Vp8, &delta).unwrap()[..3], &delta[..3]);
        assert!(encrypt(Codec::Vp8, &[0x10, 1]).is_none(), "too short");

        // Someone joined: we ratchet; the receiver catches up from the footer.
        set_local(&ours.ratcheted().unwrap().ratcheted().unwrap());
        let sealed = encrypt(Codec::Opus, b"after joins").unwrap();
        assert_eq!(sealed[sealed.len() - 5], 2);
        assert_eq!(decrypt(5, Codec::Opus, &sealed).unwrap(), b"after joins");
        // An older ratchet step is gone now.
        set_local(&ours);
        assert!(decrypt(5, Codec::Opus, &encrypt(Codec::Opus, b"old").unwrap()).is_none());

        // Someone left: a new epoch; frames of the new key decrypt, the old epoch is dropped.
        let next = MediaKey::random(1);
        add_peer_keys(5, std::slice::from_ref(&next));
        set_local(&next);
        assert_eq!(
            decrypt(5, Codec::Opus, &encrypt(Codec::Opus, b"new epoch").unwrap()).unwrap(),
            b"new epoch"
        );

        // Forged footers change nothing (audit S05): a frame claiming a later ratchet step but
        // without a valid tag is dropped, and the real frames still decrypt.
        let first = encrypt(Codec::Opus, b"first").unwrap();
        let second = encrypt(Codec::Opus, b"second").unwrap();
        let mut forged = second.clone();
        let ratchet = forged.len() - 5;
        forged[ratchet] = 255;
        assert!(decrypt(5, Codec::Opus, &forged).is_none());
        assert_eq!(decrypt(5, Codec::Opus, &second).unwrap(), b"second");
        // Out of order within the window: fine; the same frame again: not.
        assert_eq!(decrypt(5, Codec::Opus, &first).unwrap(), b"first");
        assert!(decrypt(5, Codec::Opus, &first).is_none(), "replayed");
        assert!(decrypt(5, Codec::Opus, &second).is_none(), "replayed");
        // Too far behind the newest: not.
        let old = encrypt(Codec::Opus, b"old").unwrap();
        for _ in 0..REPLAY_WINDOW {
            encrypt(Codec::Opus, b"x").unwrap();
        }
        assert_eq!(
            decrypt(5, Codec::Opus, &encrypt(Codec::Opus, b"newest").unwrap()).unwrap(),
            b"newest"
        );
        assert!(
            decrypt(5, Codec::Opus, &old).is_none(),
            "outside the window"
        );

        // Opus DTX: empty frames pass.
        assert_eq!(encrypt(Codec::Opus, b"").unwrap(), b"");
        assert_eq!(decrypt(5, Codec::Opus, b"").unwrap(), b"");
        assert!(epoch_newer(0, 200) && !epoch_newer(0, 100) && epoch_newer(3, 2));
    }
}
