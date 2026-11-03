// SPDX-License-Identifier: AGPL-3.0-only
//! Interleaving imported history into a conversation's existing messages.
//!
//! Live messages keep their arrival order (`sort_key = id << 20`, so adjacent live messages
//! leave about a million free keys between them). Imported messages are processed oldest
//! first; each one goes after the last existing message whose time is not later — scanning
//! monotonically, so a live message with a skewed clock acts as a barrier exactly once and
//! the placement stays deterministic.

/// Allocates `sort_key`s for one conversation's import.
pub struct Placer {
    /// `(sort_key, created_at)` of the existing messages, in history order.
    existing: Vec<(i64, i64)>,
    cursor: usize,
    next_key: i64,
}

impl Placer {
    /// `existing` from [`crate::store::Store::conversation_positions`]; `incoming` is how many
    /// messages will be placed (bounds the room reserved before the first existing message).
    pub fn new(existing: Vec<(i64, i64)>, incoming: usize) -> Self {
        let first_key = existing.first().map_or(1 << 20, |(key, _)| *key);
        let incoming = i64::try_from(incoming).unwrap_or(i64::MAX / 4);
        let start = if existing.is_empty() {
            1 << 20
        } else {
            first_key.saturating_sub(incoming.saturating_add(1))
        };
        Self {
            existing,
            cursor: 0,
            next_key: start,
        }
    }

    /// The key for the next imported message; call with ascending `created_at`.
    pub fn place(&mut self, created_at: i64) -> i64 {
        while self
            .existing
            .get(self.cursor)
            .is_some_and(|(_, at)| *at <= created_at)
        {
            // Entering the gap after this existing message.
            self.next_key = self.next_key.max(self.existing[self.cursor].0 + 1);
            self.cursor += 1;
        }
        let upper = self
            .existing
            .get(self.cursor)
            .map_or(i64::MAX, |(key, _)| *key);
        // A full gap (about a million imported messages between two adjacent live ones) falls
        // back to the gap's last key; ties keep insertion order via the row id.
        let mut key = self.next_key.min(upper - 1);
        if key == 0 {
            // 0 is reserved for the live-insert trigger.
            key = if upper > 1 { 1 } else { -1 };
        }
        self.next_key = key.saturating_add(1);
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_conversation() {
        let mut placer = Placer::new(Vec::new(), 3);
        let keys: Vec<i64> = [10, 20, 30].iter().map(|t| placer.place(*t)).collect();
        assert_eq!(keys, vec![1 << 20, (1 << 20) + 1, (1 << 20) + 2]);
    }

    #[test]
    fn interleaves_with_existing() {
        // Live messages (key = id << 20) at t = 5000 and 6000.
        let existing = vec![(1 << 20, 5000), (2 << 20, 6000)];
        let mut placer = Placer::new(existing, 4);
        let before = placer.place(100);
        let before2 = placer.place(200);
        let between = placer.place(5500);
        let after = placer.place(9000);
        assert!(before < before2, "ascending");
        assert!(before2 < (1 << 20), "before the first live message");
        assert!((1 << 20) < between && between < (2 << 20), "in the gap");
        assert!(after > (2 << 20), "after the last live message");
        // A later live insert (id 3) still lands after everything imported.
        assert!(after < (3 << 20));
    }

    #[test]
    fn equal_times_go_after_the_live_message() {
        let existing = vec![(1 << 20, 5000)];
        let mut placer = Placer::new(existing, 1);
        assert!(placer.place(5000) > (1 << 20), "not before an equal time");
    }

    #[test]
    fn skewed_live_clock_is_a_barrier_once() {
        // Arrival order: t=5000, then one with a future clock (9999), then t=6000.
        let existing = vec![(1 << 20, 5000), (2 << 20, 9999), (3 << 20, 6000)];
        let mut placer = Placer::new(existing, 2);
        let a = placer.place(5500);
        let b = placer.place(7000);
        // Both stop at the future-clocked message and never jump past it.
        assert!((1 << 20) < a && a < (2 << 20));
        assert!(a < b && b < (2 << 20));
    }

    #[test]
    fn full_gap_clamps_and_keeps_order_by_id() {
        let existing = vec![(10, 100), (12, 200)];
        let mut placer = Placer::new(existing, 3);
        let a = placer.place(150);
        let b = placer.place(160);
        let c = placer.place(170);
        assert_eq!(a, 11, "the only free key in the gap");
        assert_eq!((b, c), (11, 11), "clamped; row ids break the tie in order");
    }

    #[test]
    fn key_zero_is_never_produced() {
        let existing = vec![(2, 100)];
        // Reserved room drives the start key below zero; crossing it must skip 0.
        let mut placer = Placer::new(existing, 5);
        for t in [1, 2, 3, 4, 5] {
            assert_ne!(placer.place(t), 0);
        }
    }
}
