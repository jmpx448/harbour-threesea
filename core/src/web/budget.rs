// SPDX-License-Identifier: AGPL-3.0-only
//! A budget for MessagePack from the browser, checked on the bytes before decoding (deep audit
//! 2026-10-27, D03): decoding builds a tree of values, each about 40 bytes, so a message of many
//! tiny elements needs dozens of times its size in memory. The scan counts values and nesting
//! without allocating; messages over the budget are dropped before they are decoded.

/// Values in one message from the browser. Its requests are small (a file arrives as one
/// binary value); 100 000 values decode to about 4 MB.
pub const MAX_VALUES: usize = 100_000;
/// Nesting depth (Threema Web's messages nest a few levels).
pub const MAX_DEPTH: usize = 32;

/// Whether `bytes` is one MessagePack value within the budget, with nothing after it.
#[must_use]
pub fn within(bytes: &[u8], max_values: usize, max_depth: usize) -> bool {
    // Values still expected per open container, innermost last.
    let mut open: Vec<usize> = Vec::new();
    let mut pending = 1_usize;
    let mut values = 0_usize;
    let mut at = 0_usize;
    let take = |at: &mut usize, n: usize| -> Option<usize> {
        let end = at.checked_add(n).filter(|end| *end <= bytes.len())?;
        let start = *at;
        *at = end;
        Some(start)
    };
    let number = |at: &mut usize, n: usize| -> Option<usize> {
        let start = take(at, n)?;
        let mut value = 0_usize;
        for byte in &bytes[start..start + n] {
            value = value.checked_mul(256)?.checked_add(usize::from(*byte))?;
        }
        Some(value)
    };
    loop {
        while pending == 0 {
            match open.pop() {
                Some(outer) => pending = outer,
                None => return at == bytes.len(),
            }
        }
        pending -= 1;
        values += 1;
        if values > max_values {
            return false;
        }
        let Some(marker) = bytes.get(at).copied() else {
            return false;
        };
        at += 1;
        // Children of a container (map: keys and values), and the bytes to skip otherwise.
        let (children, skip) = match marker {
            0x00..=0x7f | 0xe0..=0xff | 0xc0 | 0xc2 | 0xc3 => (0, 0),
            0x80..=0x8f => (usize::from(marker & 0x0f) * 2, 0),
            0x90..=0x9f => (usize::from(marker & 0x0f), 0),
            0xa0..=0xbf => (0, usize::from(marker & 0x1f)),
            0xcc | 0xd0 => (0, 1),
            0xcd | 0xd1 => (0, 2),
            0xca | 0xce | 0xd2 => (0, 4),
            0xcb | 0xcf | 0xd3 => (0, 8),
            0xd4 => (0, 2),
            0xd5 => (0, 3),
            0xd6 => (0, 5),
            0xd7 => (0, 9),
            0xd8 => (0, 17),
            0xc4 | 0xd9 => match number(&mut at, 1) {
                Some(n) => (0, n),
                None => return false,
            },
            0xc5 | 0xda => match number(&mut at, 2) {
                Some(n) => (0, n),
                None => return false,
            },
            0xc6 | 0xdb => match number(&mut at, 4) {
                Some(n) => (0, n),
                None => return false,
            },
            // ext: length, type byte, data.
            0xc7 => match number(&mut at, 1) {
                Some(n) => (0, n + 1),
                None => return false,
            },
            0xc8 => match number(&mut at, 2) {
                Some(n) => (0, n + 1),
                None => return false,
            },
            0xc9 => match number(&mut at, 4) {
                Some(n) => (0, n + 1),
                None => return false,
            },
            0xdc => match number(&mut at, 2) {
                Some(n) => (n, 0),
                None => return false,
            },
            0xdd => match number(&mut at, 4) {
                Some(n) => (n, 0),
                None => return false,
            },
            0xde => match number(&mut at, 2).and_then(|n| n.checked_mul(2)) {
                Some(n) => (n, 0),
                None => return false,
            },
            0xdf => match number(&mut at, 4).and_then(|n| n.checked_mul(2)) {
                Some(n) => (n, 0),
                None => return false,
            },
            // 0xc1 is never used.
            _ => return false,
        };
        if take(&mut at, skip).is_none() {
            return false;
        }
        if children > 0 {
            // Every child needs at least a byte; and the budget caps how many there can be.
            if children > bytes.len() - at || values + children > max_values {
                return false;
            }
            if open.len() + 1 > max_depth {
                return false;
            }
            open.push(pending);
            pending = children;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmpv::Value;

    fn encode(value: &Value) -> Vec<u8> {
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, value).unwrap();
        bytes
    }

    #[test]
    fn ordinary_messages_pass() {
        let message = Value::Map(vec![
            ("type".into(), "request".into()),
            ("subType".into(), "messages".into()),
            (
                "args".into(),
                Value::Map(vec![("id".into(), "ABCD1234".into())]),
            ),
            (
                "data".into(),
                Value::Array(vec![
                    Value::Binary(vec![7; 70_000]),
                    1.5.into(),
                    (-3).into(),
                    u64::MAX.into(),
                    Value::Nil,
                    true.into(),
                    Value::Ext(1, vec![1, 2, 3]),
                    "x".repeat(300).into(),
                ]),
            ),
        ]);
        assert!(within(&encode(&message), MAX_VALUES, MAX_DEPTH));
    }

    #[test]
    fn wide_deep_and_broken_messages_fail() {
        // The audit's example: 65 536 nils in 65 541 bytes.
        let mut wide = vec![0xdd];
        wide.extend_from_slice(&65_536_u32.to_be_bytes());
        wide.resize(5 + 65_536, 0xc0);
        assert!(within(&wide, 70_000, MAX_DEPTH));
        assert!(!within(&wide, MAX_VALUES / 2, MAX_DEPTH));
        // A claimed length beyond the bytes there are.
        let mut claimed = vec![0xdd];
        claimed.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(!within(&claimed, MAX_VALUES, MAX_DEPTH));
        let mut deep = Value::Nil;
        for _ in 0..40 {
            deep = Value::Array(vec![deep]);
        }
        assert!(!within(&encode(&deep), MAX_VALUES, MAX_DEPTH));
        assert!(!within(&[0xc1], MAX_VALUES, MAX_DEPTH));
        assert!(
            !within(&[0xa5, b'a'], MAX_VALUES, MAX_DEPTH),
            "string cut short"
        );
        assert!(
            !within(&[0xc0, 0xc0], MAX_VALUES, MAX_DEPTH),
            "trailing bytes"
        );
        assert!(!within(&[], MAX_VALUES, MAX_DEPTH));
    }
}
