// SPDX-License-Identifier: AGPL-3.0-only
//! A contact's poll setup and votes: the JSON through the receive steps, on a real store.
#![no_main]
use std::cell::RefCell;

use threesea_core::{polls, store::Store};

thread_local! {
    // A fresh store now and then: rows pile up otherwise and slow everything down.
    static STORE: RefCell<(Store, u32)> = RefCell::new((open(), 0));
}

fn open() -> Store {
    Store::open(std::path::Path::new(":memory:"), &[1; 32]).expect("in-memory store")
}

libfuzzer_sys::fuzz_target!(|text: &str| {
    STORE.with_borrow_mut(|(store, runs)| {
        *runs += 1;
        if *runs > 1000 {
            *store = open();
            *runs = 0;
        }
        // The text is the setup of a new poll, then a vote on it, then closes it.
        let id = u64::from(*runs);
        let setup = |message| {
            polls::receive_setup(store, "B0B0B0B0", "B0B0B0B0", false, message, 1, id, text)
        };
        let _ = setup(id * 2);
        let _ = polls::receive_vote(store, "B0B0B0B0", "C0C0C0C0", "B0B0B0B0", id, text);
        let _ = setup(id * 2 + 1);
    });
});
