// SPDX-License-Identifier: AGPL-3.0-only
//! Chunking of Threema Web messages (saltyrtc-meta Chunking.md, unreliable/unordered mode, as
//! Threema Web uses it over the relayed-data task): a 9-byte header (options, message ID u32,
//! serial number u32; big endian), then up to 64 KiB of the message. The last chunk of a message
//! has options bit 0 set.

use std::collections::HashMap;

/// Chunk size, header included (Threema Web's `RELAYED_DATA_CHUNK_SIZE`).
pub const CHUNK_SIZE: usize = 64 * 1024;
const HEADER: usize = 9;
const END: u8 = 0x01;
/// Incomplete messages kept at most (lost chunks must not grow memory forever).
const MAX_PENDING: usize = 64;
/// Largest message we put together (a file in `create/fileMessage` included).
const MAX_MESSAGE: usize = 128 * 1024 * 1024;
/// Chunks of one message at most: a largest message in full chunks.
const MAX_CHUNKS: u32 = (MAX_MESSAGE / (CHUNK_SIZE - HEADER)) as u32 + 1;
/// All incomplete messages together; every chunk counts at least `MIN_CHARGE`, so empty
/// chunks can't pile up either (security audit 2026-10-27, S06).
const MAX_TOTAL: usize = MAX_MESSAGE + 32 * 1024 * 1024;
const MIN_CHARGE: usize = 1024;

/// Split `message` (ID `id`) into chunks.
#[must_use]
pub fn chunk(id: u32, message: &[u8]) -> Vec<Vec<u8>> {
    let parts: Vec<&[u8]> = if message.is_empty() {
        vec![&[][..]]
    } else {
        message.chunks(CHUNK_SIZE - HEADER).collect()
    };
    let last = parts.len() - 1;
    parts
        .into_iter()
        .enumerate()
        .map(|(serial, part)| {
            let mut chunk = Vec::with_capacity(HEADER + part.len());
            chunk.push(if serial == last { END } else { 0 });
            chunk.extend_from_slice(&id.to_be_bytes());
            chunk.extend_from_slice(&u32::try_from(serial).unwrap_or(u32::MAX).to_be_bytes());
            chunk.extend_from_slice(part);
            chunk
        })
        .collect()
}

#[derive(Default)]
struct Pending {
    parts: HashMap<u32, Vec<u8>>,
    /// Number of chunks, once the last one arrived.
    count: Option<u32>,
    /// Bytes of its chunks so far, and what they count against the budgets.
    size: usize,
    charged: usize,
    /// Arrival order, to drop the oldest incomplete message first.
    age: u64,
}

/// Puts chunks back together, in any order.
#[derive(Default)]
pub struct Unchunker {
    pending: HashMap<u32, Pending>,
    arrivals: u64,
    /// What all incomplete messages count together.
    charged: usize,
}

impl Unchunker {
    /// A chunk; the whole message once its last missing chunk arrived. Invalid chunks are
    /// dropped (`None`).
    pub fn add(&mut self, chunk: &[u8]) -> Option<Vec<u8>> {
        if chunk.len() < HEADER {
            return None;
        }
        let options = chunk[0];
        let id = u32::from_be_bytes(chunk[1..5].try_into().ok()?);
        let serial = u32::from_be_bytes(chunk[5..9].try_into().ok()?);
        if serial >= MAX_CHUNKS {
            self.drop_message(id);
            return None;
        }
        self.arrivals += 1;
        let payload = chunk.len() - HEADER;
        let charge = payload.max(MIN_CHARGE);
        let duplicate = self
            .pending
            .get(&id)
            .is_some_and(|pending| pending.parts.contains_key(&serial));
        // Room for it: the oldest other incomplete messages go first.
        while (!self.pending.contains_key(&id) && self.pending.len() >= MAX_PENDING)
            || (!duplicate && self.charged + charge > MAX_TOTAL)
        {
            let Some(oldest) = self
                .pending
                .iter()
                .filter(|(other, _)| **other != id)
                .min_by_key(|(_, pending)| pending.age)
                .map(|(id, _)| *id)
            else {
                break;
            };
            self.drop_message(oldest);
        }
        if !duplicate && self.charged + charge > MAX_TOTAL {
            self.drop_message(id);
            return None;
        }
        let pending = self.pending.entry(id).or_default();
        pending.age = self.arrivals;
        if options & END != 0 {
            pending.count = Some(serial.checked_add(1)?);
        }
        if !duplicate {
            pending.size += payload;
            pending.charged += charge;
            self.charged += charge;
            pending.parts.insert(serial, chunk[HEADER..].to_vec());
        }
        if pending.size > MAX_MESSAGE || pending.count.is_some_and(|count| count > MAX_CHUNKS) {
            self.drop_message(id);
            return None;
        }
        let count = pending.count?;
        if usize::try_from(count).ok()? != pending.parts.len() {
            return None;
        }
        let mut pending = self.pending.remove(&id)?;
        self.charged -= pending.charged;
        let mut message = Vec::with_capacity(pending.size);
        for serial in 0..count {
            message.extend(pending.parts.remove(&serial)?);
        }
        Some(message)
    }

    fn drop_message(&mut self, id: u32) {
        if let Some(pending) = self.pending.remove(&id) {
            self.charged -= pending.charged;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_example_and_round_trip() {
        let message: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let mut chunks = chunk(42, &message);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0][..9], [0, 0, 0, 0, 42, 0, 0, 0, 0]);
        assert_eq!(chunks[3][0], END);
        chunks.reverse();
        let mut unchunker = Unchunker::default();
        let mut whole = None;
        for chunk in &chunks {
            whole = unchunker.add(chunk);
        }
        assert_eq!(whole, Some(message));
    }

    #[test]
    fn single_and_duplicated_chunks() {
        let mut unchunker = Unchunker::default();
        let chunks = chunk(1, b"hello");
        assert_eq!(
            chunks,
            vec![[&[1, 0, 0, 0, 1, 0, 0, 0, 0][..], b"hello"].concat()]
        );
        assert_eq!(unchunker.add(&chunks[0]), Some(b"hello".to_vec()));
        let two = chunk(2, &[7; CHUNK_SIZE]);
        assert_eq!(unchunker.add(&two[0]), None);
        assert_eq!(unchunker.add(&two[0]), None);
        assert_eq!(unchunker.add(&two[1]).map(|m| m.len()), Some(CHUNK_SIZE));
        assert_eq!(unchunker.add(&[0, 1]), None);
        assert_eq!(unchunker.charged, 0, "nothing pending");
    }

    /// Empty chunks of messages that never end count against the budget too: they can't pile
    /// up, and a real message still gets through afterwards.
    #[test]
    fn budgets_bound_incomplete_messages() {
        let mut unchunker = Unchunker::default();
        for id in 0..200u32 {
            for serial in 0..2_000u32 {
                let mut header = vec![0];
                header.extend_from_slice(&id.to_be_bytes());
                header.extend_from_slice(&serial.to_be_bytes());
                assert_eq!(unchunker.add(&header), None);
            }
        }
        assert!(unchunker.pending.len() <= MAX_PENDING);
        assert!(unchunker.charged <= MAX_TOTAL);
        let entries: usize = unchunker.pending.values().map(|p| p.parts.len()).sum();
        assert!(entries <= MAX_TOTAL / MIN_CHARGE);
        // Beyond the largest message's chunk count: dropped.
        let mut far = vec![END];
        far.extend_from_slice(&7u32.to_be_bytes());
        far.extend_from_slice(&MAX_CHUNKS.to_be_bytes());
        assert_eq!(unchunker.add(&far), None);
        let message = vec![5; 3 * CHUNK_SIZE];
        let mut whole = None;
        for chunk in chunk(9_999, &message) {
            whole = unchunker.add(&chunk);
        }
        assert_eq!(whole, Some(message));
    }
}
