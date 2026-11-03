//! KDF ratchet of a forward security session (`KDFRatchet` in the Android implementation).
use educe::Educe;
use zeroize::{Zeroize as _, ZeroizeOnDrop};

use crate::crypto::{
    blake2b::Blake2bMac256,
    digest::{FixedOutput as _, Mac as _},
};

/// BLAKE2b personal used for all forward security key derivations.
pub(crate) const KDF_PERSONAL: &[u8] = b"3ma-e2e";

const KDF_SALT_CHAIN_KEY: &[u8] = b"kdf-ck";
const KDF_SALT_ENCRYPTION_KEY: &[u8] = b"kdf-aek";

/// Maximum amount of ratchet turns allowed to catch up with an incoming message.
pub const MAX_COUNTER_INCREMENT: u64 = 25_000;

/// Derive a 32 byte BLAKE2b MAC as used by the forward security key derivations.
pub(crate) fn blake2b_256(key: &[u8], salt: &[u8]) -> [u8; 32] {
    Blake2bMac256::new_with_salt_and_personal(Some(key), salt, KDF_PERSONAL)
        .expect("Blake2bMac256 with a valid key, salt and personal must not fail")
        .finalize_fixed()
        .into()
}

/// Error when turning a ratchet.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RatchetError {
    /// The requested counter is lower than the current counter (message replayed or too old).
    #[error("Target counter {target} is lower than the current counter {current}")]
    CounterTooLow {
        /// Current counter.
        current: u64,
        /// Requested counter.
        target: u64,
    },

    /// The requested counter is too far ahead.
    #[error("Target counter {target} is too far ahead of the current counter {current}")]
    CounterTooFarAhead {
        /// Current counter.
        current: u64,
        /// Requested counter.
        target: u64,
    },
}

/// A KDF ratchet: a chain key and its counter.
#[derive(Clone, Educe, ZeroizeOnDrop)]
#[educe(Debug)]
pub struct Ratchet {
    /// Current counter (starts at 1).
    #[zeroize(skip)]
    pub counter: u64,

    /// Current chain key.
    #[educe(Debug(ignore))]
    pub chain_key: [u8; 32],
}

impl Ratchet {
    /// Create a ratchet starting at counter 1 with the given initial chain key.
    #[must_use]
    pub fn new(initial_chain_key: [u8; 32]) -> Self {
        Self {
            counter: 1,
            chain_key: initial_chain_key,
        }
    }

    /// Turn the ratchet once.
    pub fn turn(&mut self) {
        let mut next = blake2b_256(&self.chain_key, KDF_SALT_CHAIN_KEY);
        self.chain_key.zeroize();
        self.chain_key = next;
        next.zeroize();
        self.counter = self.counter.saturating_add(1);
    }

    /// Turn the ratchet until it reaches `target`.
    ///
    /// # Errors
    ///
    /// Returns [`RatchetError`] if `target` is lower than the current counter or more than
    /// [`MAX_COUNTER_INCREMENT`] ahead.
    pub fn turn_until(&mut self, target: u64) -> Result<u64, RatchetError> {
        if target < self.counter {
            return Err(RatchetError::CounterTooLow {
                current: self.counter,
                target,
            });
        }
        let turns = target.saturating_sub(self.counter);
        if turns > MAX_COUNTER_INCREMENT {
            return Err(RatchetError::CounterTooFarAhead {
                current: self.counter,
                target,
            });
        }
        for _ in 0..turns {
            self.turn();
        }
        Ok(turns)
    }

    /// The message (encryption) key for the current counter.
    #[must_use]
    pub fn encryption_key(&self) -> [u8; 32] {
        blake2b_256(&self.chain_key, KDF_SALT_ENCRYPTION_KEY)
    }
}

#[cfg(test)]
mod tests {
    use data_encoding::HEXLOWER;

    use super::*;

    // Vectors from Android's `KDFRatchetTest.java` (which starts its ratchet at counter 0).
    const INITIAL_CHAIN_KEY: &str = "421e73cf324785dee4c4830f2efbb8cd4b258ed8520a608a6ce340aaa7400024";

    fn ratchet() -> Ratchet {
        Ratchet {
            counter: 0,
            chain_key: HEXLOWER
                .decode(INITIAL_CHAIN_KEY.as_bytes())
                .unwrap()
                .try_into()
                .unwrap(),
        }
    }

    #[test]
    fn single_turn_matches_android() {
        let mut ratchet = ratchet();
        ratchet.turn();
        assert_eq!(ratchet.counter, 1);
        assert_eq!(
            HEXLOWER.encode(&ratchet.encryption_key()),
            "60d3de2d849fa8b3d9799e8e50b09a7ef1d4e1e855c99fdb711bfe29466cdad3"
        );
    }

    #[test]
    fn turn_until_matches_android() {
        let mut ratchet = ratchet();
        assert_eq!(ratchet.turn_until(100), Ok(100));
        assert_eq!(
            HEXLOWER.encode(&ratchet.encryption_key()),
            "d2acf6e1c7262eb360ad92b6a74e0f9ad6ee129278742fc2462ae72916eaa334"
        );
    }

    #[test]
    fn turn_until_limits() {
        let mut ratchet = ratchet();
        assert!(matches!(
            ratchet.turn_until(1_000_000),
            Err(RatchetError::CounterTooFarAhead { .. })
        ));
        assert_eq!(ratchet.turn_until(5), Ok(5));
        assert_eq!(ratchet.turn_until(5), Ok(0));
        assert!(matches!(
            ratchet.turn_until(4),
            Err(RatchetError::CounterTooLow { .. })
        ));
    }
}
