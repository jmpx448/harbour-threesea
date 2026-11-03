// SPDX-License-Identifier: AGPL-3.0-only
//! RTP payloads of our group call video (RFC 7741): an encrypted VP8 frame split into packets,
//! each starting with the VP8 payload descriptor. GStreamer's payloader can't be used: it parses
//! the frame's first partition, which the frame encryption hides.
//!
//! Every packet carries a 15-bit picture ID, TL0PICIDX and the temporal layer (the SFU picks
//! layers by them, docs/research/group-calls.md §2.2). We send one temporal layer, so every frame
//! is layer 0 with the sync bit set and TL0PICIDX counts frames. Each simulcast layer (l, m, h)
//! is a stream of its own with its own numbering.

use std::sync::{LazyLock, Mutex};

/// The descriptor: X, then I, L and T, a 15-bit picture ID, TL0PICIDX, TID/Y/KEYIDX.
const DESCRIPTOR_LEN: usize = 6;

/// Picture ID and TL0PICIDX of our stream.
#[derive(Debug)]
pub struct Packetizer {
    picture_id: u16,
    tl0_pic_idx: u8,
}

impl Packetizer {
    /// Starting at random values, like libwebrtc.
    #[must_use]
    pub fn new() -> Self {
        Self {
            picture_id: rand::random::<u16>() & 0x7fff,
            tl0_pic_idx: rand::random(),
        }
    }

    /// The payloads of one frame, each at most `max_payload` bytes (descriptor included); empty
    /// for an empty frame or a `max_payload` too small to carry any data.
    pub fn payloads(&mut self, frame: &[u8], max_payload: usize) -> Vec<Vec<u8>> {
        if frame.is_empty() || max_payload <= DESCRIPTOR_LEN {
            return Vec::new();
        }
        let [id_high, id_low] = self.picture_id.to_be_bytes();
        let payloads = frame
            .chunks(max_payload - DESCRIPTOR_LEN)
            .enumerate()
            .map(|(index, chunk)| {
                let mut payload = Vec::with_capacity(DESCRIPTOR_LEN + chunk.len());
                payload.extend_from_slice(&[
                    // X, S on the frame's first packet, partition 0.
                    if index == 0 { 0x90 } else { 0x80 },
                    // I, L, T.
                    0xe0,
                    // M: a 15-bit picture ID.
                    0x80 | id_high,
                    id_low,
                    self.tl0_pic_idx,
                    // TID 0, Y (layer sync).
                    0x20,
                ]);
                payload.extend_from_slice(chunk);
                payload
            })
            .collect();
        self.picture_id = (self.picture_id + 1) & 0x7fff;
        self.tl0_pic_idx = self.tl0_pic_idx.wrapping_add(1);
        payloads
    }
}

impl Default for Packetizer {
    fn default() -> Self {
        Self::new()
    }
}

/// The simulcast layers we send (l, m, h).
pub const LAYERS: usize = 3;

static PACKETIZERS: LazyLock<Mutex<[Packetizer; LAYERS]>> =
    LazyLock::new(|| Mutex::new(std::array::from_fn(|_| Packetizer::new())));

/// Encrypt one of our VP8 frames of simulcast `layer` and split it into RTP payloads; `None`:
/// drop the frame (or an unknown layer).
#[must_use]
pub fn packets(layer: usize, frame: &[u8], max_payload: usize) -> Option<Vec<Vec<u8>>> {
    if layer >= LAYERS {
        return None;
    }
    let encrypted = crate::group_call_frames::encrypt(crate::group_call_frames::Codec::Vp8, frame)?;
    let payloads = PACKETIZERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)[layer]
        .payloads(&encrypted, max_payload);
    (!payloads.is_empty()).then_some(payloads)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_is_split_with_descriptors() {
        let mut packetizer = Packetizer {
            picture_id: 0x7fff,
            tl0_pic_idx: 255,
        };
        let frame: Vec<u8> = (0..=24).collect();
        let payloads = packetizer.payloads(&frame, 16);
        assert_eq!(payloads.len(), 3);
        assert_eq!(payloads[0][..6], [0x90, 0xe0, 0xff, 0xff, 255, 0x20]);
        assert_eq!(payloads[0][6..], frame[..10]);
        assert_eq!(payloads[1][0], 0x80);
        assert_eq!(payloads[2][6..], frame[20..]);
        // The next frame: picture ID and TL0PICIDX wrap.
        let next = packetizer.payloads(&[1, 2, 3], 16);
        assert_eq!(next, vec![vec![0x90, 0xe0, 0x80, 0x00, 0, 0x20, 1, 2, 3]]);
    }

    #[test]
    fn nothing_to_send() {
        let mut packetizer = Packetizer::new();
        assert!(packetizer.payloads(&[], 1000).is_empty());
        assert!(packetizer.payloads(&[1, 2, 3], DESCRIPTOR_LEN).is_empty());
    }
}
